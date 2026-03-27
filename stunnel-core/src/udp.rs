use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::udp::Socket as SmolUdpSocket;
use stunnel::tunnel::{AsyncReadDatagramExt, AsyncWriteDatagramExt, Tunnel};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant as TokioInstant;
use tracing::info;

use crate::config::{AppConfig, TunnelMode};
use crate::connection::ConnectionManager;
use crate::engine::StunnelEngine;

const UDP_BUFFER_SIZE: usize = 2048;
const UDP_IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(10);
const UDP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct UdpSocket {
    handle: SocketHandle,
    engine: Arc<Mutex<StunnelEngine>>,
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let mut engine = self.engine.lock().unwrap();
        engine.sockets.remove(self.handle);
        engine.udp_sessions.retain(|_, v| *v != self.handle);
        info!("UdpSocket dropped, handle {:?} removed", self.handle);
    }
}

impl UdpSocket {
    pub fn new(handle: SocketHandle, engine: Arc<Mutex<StunnelEngine>>) -> Self {
        Self { handle, engine }
    }

    pub fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<SmolUdpSocket>(self.handle);

        match socket.recv() {
            Ok((data, metadata)) => {
                let n = std::cmp::min(data.len(), buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                Poll::Ready(Ok((n, endpoint_to_socket_addr(metadata.endpoint))))
            }
            Err(smoltcp::socket::udp::RecvError::Exhausted) => {
                engine.register_waker(self.handle, cx.waker().clone());
                Poll::Pending
            }
            Err(error) => Poll::Ready(Err(io::Error::other(error))),
        }
    }

    pub fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<SmolUdpSocket>(self.handle);
        let endpoint = socket_addr_to_endpoint(target);

        match socket.send_slice(buf, endpoint) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(smoltcp::socket::udp::SendError::BufferFull) => {
                engine.register_waker(self.handle, cx.waker().clone());
                Poll::Pending
            }
            Err(error) => Poll::Ready(Err(io::Error::other(error))),
        }
    }
}

fn endpoint_to_socket_addr(endpoint: smoltcp::wire::IpEndpoint) -> SocketAddr {
    let ip = match endpoint.addr {
        smoltcp::wire::IpAddress::Ipv4(v4) => std::net::IpAddr::V4(v4.into()),
        smoltcp::wire::IpAddress::Ipv6(v6) => std::net::IpAddr::V6(v6.into()),
    };

    SocketAddr::new(ip, endpoint.port)
}

fn socket_addr_to_endpoint(target: SocketAddr) -> smoltcp::wire::IpEndpoint {
    match target {
        SocketAddr::V4(v4) => smoltcp::wire::IpEndpoint::new(
            smoltcp::wire::IpAddress::Ipv4((*v4.ip()).into()),
            v4.port(),
        ),
        SocketAddr::V6(v6) => smoltcp::wire::IpEndpoint::new(
            smoltcp::wire::IpAddress::Ipv6((*v6.ip()).into()),
            v6.port(),
        ),
    }
}

pub async fn handle_udp_direct_session(
    socket: UdpSocket,
    src_endpoint: SocketAddr,
    target_addr: SocketAddr,
) -> io::Result<()> {
    let outbound = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    outbound.connect(target_addr).await?;

    let last_activity = Arc::new(TokioMutex::new(TokioInstant::now()));

    let last_activity_s = Arc::clone(&last_activity);
    let s_task = async {
        let mut buf = vec![0u8; UDP_BUFFER_SIZE];
        loop {
            let (n, addr) = std::future::poll_fn(|cx| socket.poll_recv_from(cx, &mut buf)).await?;
            if addr == src_endpoint {
                *last_activity_s.lock().await = TokioInstant::now();
                outbound.send(&buf[..n]).await?;
            }
        }
    };

    let last_activity_r = Arc::clone(&last_activity);
    let r_task = async {
        let mut buf = vec![0u8; UDP_BUFFER_SIZE];
        loop {
            let n = outbound.recv(&mut buf).await?;
            *last_activity_r.lock().await = TokioInstant::now();
            std::future::poll_fn(|cx| socket.poll_send_to(cx, &buf[..n], target_addr)).await?;
        }
    };

    let timeout_task = async { idle_timeout(last_activity).await };

    tokio::select! {
        res = s_task => res,
        res = r_task => res,
        res = timeout_task => res,
    }
}

async fn idle_timeout(last_activity: Arc<TokioMutex<TokioInstant>>) -> io::Result<()> {
    loop {
        tokio::time::sleep(UDP_IDLE_CHECK_INTERVAL).await;
        let last = *last_activity.lock().await;
        if last.elapsed() > UDP_IDLE_TIMEOUT {
            return Ok(());
        }
    }
}

pub async fn handle_udp_proxy_session(
    socket: UdpSocket,
    src_endpoint: SocketAddr,
    config: &AppConfig,
    conn_manager: &ConnectionManager,
) -> io::Result<()> {
    match config.tunnel_mode()? {
        TunnelMode::S2nQuic => {
            let handle = conn_manager.get_s2n_handle(config).await?;
            let tunnel = stunnel::tunnel::client::connect_udp_tunnel(handle).await?;
            run_udp_forward(socket, src_endpoint, tunnel).await
        }
        TunnelMode::TlsTcp => {
            let connector = conn_manager.get_tlstcp_connector(config).await?;
            let tunnel = stunnel::tunnel::client::connect_udp_tunnel(connector).await?;
            run_udp_forward(socket, src_endpoint, tunnel).await
        }
    }
}

async fn run_udp_forward<S, R>(
    socket: UdpSocket,
    src_endpoint: SocketAddr,
    tunnel: Tunnel<S, R>,
) -> io::Result<()>
where
    S: AsyncWrite + Send + Unpin,
    R: AsyncRead + Send + Unpin,
{
    let (mut tun_send, mut tun_recv) = tunnel.split();
    let last_activity = Arc::new(TokioMutex::new(TokioInstant::now()));

    let last_activity_s = Arc::clone(&last_activity);
    let s_task = async {
        let mut buf = vec![0u8; UDP_BUFFER_SIZE];
        loop {
            let (n, addr) = std::future::poll_fn(|cx| socket.poll_recv_from(cx, &mut buf)).await?;
            if addr == src_endpoint {
                *last_activity_s.lock().await = TokioInstant::now();
                tun_send.send_datagram(&buf[..n], addr).await?;
            }
        }
    };

    let last_activity_r = Arc::clone(&last_activity);
    let r_task = async {
        let mut buf = vec![0u8; UDP_BUFFER_SIZE];
        loop {
            let (n, addr) = tun_recv.recv_datagram(&mut buf).await?;
            *last_activity_r.lock().await = TokioInstant::now();
            std::future::poll_fn(|cx| socket.poll_send_to(cx, &buf[..n], addr)).await?;
        }
    };

    let timeout_task = async { idle_timeout(last_activity).await };

    tokio::select! {
        res = s_task => res,
        res = r_task => res,
        res = timeout_task => res,
    }
}
