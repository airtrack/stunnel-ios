use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::{Socket as TcpSocket, State as TcpState};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::info;

use crate::config::{AppConfig, TunnelMode};
use crate::connection::ConnectionManager;
use crate::engine::StunnelEngine;

pub struct TcpStream {
    handle: SocketHandle,
    engine: Arc<Mutex<StunnelEngine>>,
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        let mut engine = self.engine.lock().unwrap();
        engine.sockets.remove(self.handle);
        info!("TcpStream dropped, handle {:?} removed", self.handle);
    }
}

impl TcpStream {
    pub fn new(handle: SocketHandle, engine: Arc<Mutex<StunnelEngine>>) -> Self {
        Self { handle, engine }
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<TcpSocket>(self.handle);

        if socket.can_recv() {
            let result = socket.recv(|data| {
                let n = std::cmp::min(data.len(), buf.remaining());
                buf.put_slice(&data[..n]);
                (n, ())
            });

            match result {
                Ok(_) => Poll::Ready(Ok(())),
                Err(error) => Poll::Ready(Err(io::Error::other(error))),
            }
        } else {
            match socket.state() {
                TcpState::Closed | TcpState::CloseWait | TcpState::Closing | TcpState::LastAck => {
                    Poll::Ready(Ok(())) // EOF
                }
                _ => {
                    engine.register_waker(self.handle, cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<TcpSocket>(self.handle);

        if socket.can_send() {
            match socket.send_slice(buf) {
                Ok(n) => Poll::Ready(Ok(n)),
                Err(error) => Poll::Ready(Err(io::Error::other(error))),
            }
        } else {
            match socket.state() {
                TcpState::Closed | TcpState::CloseWait | TcpState::Closing | TcpState::LastAck => {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "connection closed",
                    )))
                }
                _ => {
                    engine.register_waker(self.handle, cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<TcpSocket>(self.handle);
        socket.close();
        Poll::Ready(Ok(()))
    }
}

pub async fn handle_tcp_direct_session(stream: &mut TcpStream, target: &str) -> io::Result<()> {
    let mut outbound = tokio::net::TcpStream::connect(target).await?;
    tokio::io::copy_bidirectional(stream, &mut outbound).await?;
    Ok(())
}

pub async fn handle_tcp_proxy_session(
    stream: &mut TcpStream,
    config: &AppConfig,
    target: &str,
    conn_manager: &ConnectionManager,
) -> io::Result<()> {
    match config.tunnel_mode()? {
        TunnelMode::S2nQuic => {
            let handle = conn_manager.get_s2n_handle(config).await?;
            let mut tunnel = stunnel::tunnel::client::connect_tcp_tunnel(handle, target)
                .await?
                .1;
            tokio::io::copy_bidirectional(stream, &mut tunnel).await?;
        }
        TunnelMode::TlsTcp => {
            let connector = conn_manager.get_tlstcp_connector(config).await?;
            let mut tunnel = stunnel::tunnel::client::connect_tcp_tunnel(connector, target)
                .await?
                .1;
            tokio::io::copy_bidirectional(stream, &mut tunnel).await?;
        }
    }

    Ok(())
}
