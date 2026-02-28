use bytes::Bytes;
use serde::{Deserialize, Serialize};
use smoltcp::iface::{Config as SmolConfig, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State as TcpState};
use smoltcp::socket::udp::{PacketBuffer as UdpPacketBuffer, PacketMetadata as UdpPacketMetadata, Socket as UdpSocket};
use smoltcp::time::Instant;
use smoltcp::wire::{IpCidr, Ipv4Address, IpProtocol, TcpPacket, UdpPacket};
use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::io;
use std::net::SocketAddr;
use std::os::raw::{c_char, c_void};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use stunnel::tunnel::{AsyncReadDatagramExt, AsyncWriteDatagramExt, Tunnel};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex as TokioMutex, OnceCell};
use tracing::{error, info, Level};
use tracing_subscriber;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub mode: String, // "tlstcp", "quinn-quic", "s2n-quic"
    pub server_addr: String,
    pub server_name: String,
    pub cert: String,
    pub priv_key: String,
}

/// A simple buffer-backed device for smoltcp
struct TunDevice {
    inbound_packets: VecDeque<Bytes>,
    outbound_callback: Option<PacketCallback>,
}

impl Device for TunDevice {
    type RxToken<'a> = RxToken where Self: 'a;
    type TxToken<'a> = TxToken<'a> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if let Some(packet) = self.inbound_packets.pop_front() {
            Some((RxToken { packet }, TxToken { device: self }))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(TxToken { device: self })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = 1500;
        caps.checksum = ChecksumCapabilities::ignored();
        caps
    }
}

struct RxToken {
    packet: Bytes,
}

impl smoltcp::phy::RxToken for RxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.packet)
    }
}

struct TxToken<'a> {
    device: &'a mut TunDevice,
}

impl<'a> smoltcp::phy::TxToken for TxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer);
        if let Some(callback) = self.device.outbound_callback {
            callback(buffer.as_ptr(), buffer.len());
        }
        result
    }
}

/// The core engine managing the TCP/IP stack.
struct StunnelEngine {
    device: TunDevice,
    interface: Interface,
    sockets: SocketSet<'static>,
    wakers: HashMap<SocketHandle, Waker>,
}

impl StunnelEngine {
    fn new() -> Self {
        let mut device = TunDevice {
            inbound_packets: VecDeque::new(),
            outbound_callback: None,
        };

        let config = SmolConfig::new(smoltcp::wire::HardwareAddress::Ip);
        let mut interface = Interface::new(config, &mut device, Instant::now());

        interface.update_ip_addrs(|addrs| {
            addrs
                .push(IpCidr::new(Ipv4Address::new(192, 168, 1, 1).into(), 24))
                .unwrap();
        });

        interface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::new(192, 168, 1, 1))
            .unwrap();

        StunnelEngine {
            device,
            interface,
            sockets: SocketSet::new(vec![]),
            wakers: HashMap::new(),
        }
    }

    fn poll(&mut self) {
        let timestamp = Instant::now();
        self.interface
            .poll(timestamp, &mut self.device, &mut self.sockets);

        for waker in self.wakers.values() {
            waker.wake_by_ref();
        }
        self.wakers.clear();
    }

    fn register_waker(&mut self, handle: SocketHandle, waker: Waker) {
        self.wakers.insert(handle, waker);
    }
}

/// A virtual TcpStream that bridges smoltcp and tokio.
pub struct TcpStream {
    handle: SocketHandle,
    engine: Arc<Mutex<StunnelEngine>>,
}

impl TcpStream {
    fn new(handle: SocketHandle, engine: Arc<Mutex<StunnelEngine>>) -> Self {
        TcpStream { handle, engine }
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
                Err(e) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
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
                Err(e) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
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

impl Drop for TcpStream {
    fn drop(&mut self) {
        let mut engine = self.engine.lock().unwrap();
        engine.sockets.remove(self.handle);
        info!("TcpStream dropped, handle {:?} removed", self.handle);
    }
}

/// A virtual UdpSocket that bridges smoltcp and tokio.
pub struct UdpProxySocket {
    handle: SocketHandle,
    engine: Arc<Mutex<StunnelEngine>>,
}

impl UdpProxySocket {
    fn new(handle: SocketHandle, engine: Arc<Mutex<StunnelEngine>>) -> Self {
        UdpProxySocket { handle, engine }
    }

    pub fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<UdpSocket>(self.handle);

        match socket.recv() {
            Ok((data, metadata)) => {
                let n = std::cmp::min(data.len(), buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                let endpoint = metadata.endpoint;
                let addr = match endpoint {
                    smoltcp::wire::IpEndpoint { addr, port } => {
                        let ip = match addr {
                            smoltcp::wire::IpAddress::Ipv4(v4) => std::net::IpAddr::V4(v4.into()),
                            smoltcp::wire::IpAddress::Ipv6(v6) => std::net::IpAddr::V6(v6.into()),
                        };
                        SocketAddr::new(ip, port)
                    }
                };
                Poll::Ready(Ok((n, addr)))
            }
            Err(smoltcp::socket::udp::RecvError::Exhausted) => {
                engine.register_waker(self.handle, cx.waker().clone());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
        }
    }

    pub fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        let mut engine = self.engine.lock().unwrap();
        let socket = engine.sockets.get_mut::<UdpSocket>(self.handle);

        let endpoint = match target {
            SocketAddr::V4(v4) => smoltcp::wire::IpEndpoint::new(smoltcp::wire::IpAddress::Ipv4(v4.ip().clone().into()), v4.port()),
            SocketAddr::V6(v6) => smoltcp::wire::IpEndpoint::new(smoltcp::wire::IpAddress::Ipv6(v6.ip().clone().into()), v6.port()),
        };

        match socket.send_slice(buf, endpoint) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(smoltcp::socket::udp::SendError::BufferFull) => {
                engine.register_waker(self.handle, cx.waker().clone());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
        }
    }
}

impl Drop for UdpProxySocket {
    fn drop(&mut self) {
        let mut engine = self.engine.lock().unwrap();
        engine.sockets.remove(self.handle);
        info!("UdpProxySocket dropped, handle {:?} removed", self.handle);
    }
}

trait ConnectionState {
    fn in_good_condition(&mut self) -> io::Result<()>;
}

impl ConnectionState for s2n_quic::connection::Handle {
    fn in_good_condition(&mut self) -> io::Result<()> {
        self.keep_alive(true)
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionReset, e))
    }
}

impl ConnectionState for stunnel::tlstcp::client::Connector {
    fn in_good_condition(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Manages the lifecycle of outbound connections.
struct ConnectionManager {
    s2n_client: OnceCell<s2n_quic::Client>,
    s2n_handle: TokioMutex<Option<s2n_quic::connection::Handle>>,
    tlstcp_connector: OnceCell<stunnel::tlstcp::client::Connector>,
}

impl ConnectionManager {
    fn new() -> Self {
        Self {
            s2n_client: OnceCell::new(),
            s2n_handle: TokioMutex::new(None),
            tlstcp_connector: OnceCell::new(),
        }
    }

    async fn get_s2n_handle(&self, config: &AppConfig) -> io::Result<s2n_quic::connection::Handle> {
        let mut lock = self.s2n_handle.lock().await;

        if let Some(mut handle) = lock.clone() {
            if handle.in_good_condition().is_ok() {
                return Ok(handle);
            }
            info!("QUIC connection broken, reconnecting...");
        }

        let client = self.s2n_client
            .get_or_try_init(|| async {
                let quic_config = stunnel::quic::Config {
                    addr: "0.0.0.0:0".to_string(),
                    cert: config.cert.clone(),
                    priv_key: config.priv_key.clone(),
                    loss_threshold: 10,
                };
                stunnel::quic::s2n_quic::client::new(&quic_config)
            })
            .await?;

        let addr: SocketAddr = config.server_addr.parse().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid addr: {:?}", e))
        })?;
        let connect =
            s2n_quic::client::Connect::new(addr).with_server_name(config.server_name.as_str());

        let mut conn = client.connect(connect).await.map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("QUIC connect error: {:?}", e))
        })?;
        conn.keep_alive(true).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("QUIC keep-alive error: {:?}", e))
        })?;

        let handle = conn.handle();
        *lock = Some(handle.clone());
        Ok(handle)
    }

    async fn get_tlstcp_connector(
        &self,
        config: &AppConfig,
    ) -> io::Result<stunnel::tlstcp::client::Connector> {
        self.tlstcp_connector
            .get_or_try_init(|| async {
                let tls_config = stunnel::tlstcp::client::Config {
                    server_addr: config.server_addr.clone(),
                    server_name: config.server_name.clone(),
                    cert: config.cert.clone(),
                    priv_key: config.priv_key.clone(),
                };
                Ok(stunnel::tlstcp::client::new(&tls_config))
            })
            .await
            .cloned()
    }
}

/// The main handle for the C interface.
struct CoreContext {
    engine: Arc<Mutex<StunnelEngine>>,
    runtime: Runtime,
    config: AppConfig,
    conn_manager: Arc<ConnectionManager>,
    udp_sessions: Arc<Mutex<HashMap<SocketAddr, SocketHandle>>>,
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .try_init();
    info!("stunnel-ios rust-core logging initialized");
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_start(config_json_ptr: *const c_char) -> *mut c_void {
    let config_json = unsafe {
        if config_json_ptr.is_null() {
            return std::ptr::null_mut();
        }
        CStr::from_ptr(config_json_ptr).to_string_lossy()
    };

    let config: AppConfig = match serde_json::from_str(&config_json) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to parse config: {:?}", e);
            return std::ptr::null_mut();
        }
    };

    info!("Starting stunnel-ios core engine with mode: {}", config.mode);

    let engine = Arc::new(Mutex::new(StunnelEngine::new()));
    let runtime = Runtime::new().unwrap();
    let conn_manager = Arc::new(ConnectionManager::new());
    let udp_sessions = Arc::new(Mutex::new(HashMap::new()));

    let ctx = Box::new(CoreContext {
        engine,
        runtime,
        config,
        conn_manager,
        udp_sessions,
    });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_stop(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    info!("Stopping stunnel-ios core engine");
    unsafe {
        let _ = Box::from_raw(handle as *mut CoreContext);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_process_packet(handle: *mut c_void, packet: *const u8, len: usize) {
    if handle.is_null() || packet.is_null() {
        return;
    }

    let ctx = unsafe { &*(handle as *const CoreContext) };
    let packet_data = unsafe { std::slice::from_raw_parts(packet, len) };
    let bytes = Bytes::copy_from_slice(packet_data);

    let mut engine = ctx.engine.lock().unwrap();

    if let Ok(ip_packet) = smoltcp::wire::Ipv4Packet::new_checked(&bytes) {
        let src_addr = ip_packet.src_addr();
        let dst_addr = ip_packet.dst_addr();

        match ip_packet.next_header() {
            IpProtocol::Tcp => {
                if let Ok(tcp_packet) = TcpPacket::new_checked(ip_packet.payload()) {
                    if tcp_packet.syn() && !tcp_packet.ack() {
                        let dst_port = tcp_packet.dst_port();
                        info!("Intercepted TCP SYN to {}:{}", dst_addr, dst_port);

                        let tcp_rx_buffer = TcpSocketBuffer::new(vec![0; 65536]);
                        let tcp_tx_buffer = TcpSocketBuffer::new(vec![0; 65536]);
                        let mut socket = TcpSocket::new(tcp_rx_buffer, tcp_tx_buffer);
                        socket.listen((dst_addr, dst_port)).unwrap();
                        let socket_handle = engine.sockets.add(socket);

                        let mut stream = TcpStream::new(socket_handle, Arc::clone(&ctx.engine));
                        let config = ctx.config.clone();
                        let target = format!("{}:{}", dst_addr, dst_port);
                        let conn_manager = Arc::clone(&ctx.conn_manager);

                        ctx.runtime.spawn(async move {
                            let result = handle_tcp_proxy_session(&mut stream, &config, &target, &conn_manager).await;
                            if let Err(e) = result {
                                error!("TCP proxy session failed for {}: {:?}", target, e);
                            }
                        });
                    }
                }
            }
            IpProtocol::Udp => {
                if let Ok(udp_packet) = UdpPacket::new_checked(ip_packet.payload()) {
                    let src_port = udp_packet.src_port();
                    let dst_port = udp_packet.dst_port();
                    let src_endpoint = SocketAddr::new(std::net::IpAddr::V4(src_addr.into()), src_port);
                    let dst_endpoint = SocketAddr::new(std::net::IpAddr::V4(dst_addr.into()), dst_port);

                    let mut udp_sessions = ctx.udp_sessions.lock().unwrap();
                    if !udp_sessions.contains_key(&src_endpoint) {
                        info!("Intercepted new UDP session from {} to {}", src_endpoint, dst_endpoint);
                        
                        let udp_rx_buffer = UdpPacketBuffer::new(vec![UdpPacketMetadata::EMPTY; 16], vec![0; 65536]);
                        let udp_tx_buffer = UdpPacketBuffer::new(vec![UdpPacketMetadata::EMPTY; 16], vec![0; 65536]);
                        let mut socket = UdpSocket::new(udp_rx_buffer, udp_tx_buffer);
                        socket.bind((dst_addr, dst_port)).unwrap();
                        let socket_handle = engine.sockets.add(socket);
                        udp_sessions.insert(src_endpoint, socket_handle);

                        let proxy_socket = UdpProxySocket::new(socket_handle, Arc::clone(&ctx.engine));
                        let config = ctx.config.clone();
                        let conn_manager = Arc::clone(&ctx.conn_manager);
                        let udp_sessions_clone = Arc::clone(&ctx.udp_sessions);

                        ctx.runtime.spawn(async move {
                            let result = handle_udp_proxy_session(proxy_socket, src_endpoint, &config, &conn_manager).await;
                            if let Err(e) = result {
                                error!("UDP proxy session failed for {}: {:?}", src_endpoint, e);
                            }
                            udp_sessions_clone.lock().unwrap().remove(&src_endpoint);
                        });
                    }
                }
            }
            _ => {}
        }
    }

    engine.device.inbound_packets.push_back(bytes);
    engine.poll();
}

async fn handle_tcp_proxy_session(
    stream: &mut TcpStream,
    config: &AppConfig,
    target: &str,
    conn_manager: &ConnectionManager,
) -> io::Result<()> {
    if config.mode == "s2n-quic" {
        let handle = conn_manager.get_s2n_handle(config).await?;
        let mut tunnel = stunnel::tunnel::client::connect_tcp_tunnel(handle, target).await?.1;
        tokio::io::copy_bidirectional(stream, &mut tunnel).await?;
    } else if config.mode == "tlstcp" {
        let connector = conn_manager.get_tlstcp_connector(config).await?;
        let mut tunnel = stunnel::tunnel::client::connect_tcp_tunnel(connector, target).await?.1;
        tokio::io::copy_bidirectional(stream, &mut tunnel).await?;
    }
    Ok(())
}

async fn handle_udp_proxy_session(
    socket: UdpProxySocket,
    src_endpoint: SocketAddr,
    config: &AppConfig,
    conn_manager: &ConnectionManager,
) -> io::Result<()> {
    if config.mode == "s2n-quic" {
        let handle = conn_manager.get_s2n_handle(config).await?;
        let tunnel = stunnel::tunnel::client::connect_udp_tunnel(handle).await?;
        run_udp_forward(socket, src_endpoint, tunnel).await
    } else {
        let connector = conn_manager.get_tlstcp_connector(config).await?;
        let tunnel = stunnel::tunnel::client::connect_udp_tunnel(connector).await?;
        run_udp_forward(socket, src_endpoint, tunnel).await
    }
}

async fn run_udp_forward<S, R>(
    socket: UdpProxySocket,
    src_endpoint: SocketAddr,
    tunnel: Tunnel<S, R>,
) -> io::Result<()>
where
    S: AsyncWrite + Send + Unpin,
    R: AsyncRead + Send + Unpin,
{
    let (mut tun_send, mut tun_recv) = tunnel.split();

    // 1. Tun -> Proxy
    let s_task = async {
        let mut buf = vec![0u8; 2048];
        loop {
            let (n, addr) = std::future::poll_fn(|cx| socket.poll_recv_from(cx, &mut buf)).await?;
            if addr == src_endpoint {
                tun_send.send_datagram(&buf[..n], addr).await?;
            }
        }
    };

    // 2. Proxy -> Tun
    let r_task = async {
        let mut buf = vec![0u8; 2048];
        loop {
            let (n, addr) = tun_recv.recv_datagram(&mut buf).await?;
            std::future::poll_fn(|cx| socket.poll_send_to(cx, &buf[..n], addr)).await?;
        }
    };

    tokio::select! {
        res = s_task => res,
        res = r_task => res,
        _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => Ok(()), // Timeout
    }
}

type PacketCallback = extern "C" fn(*const u8, usize);

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_set_packet_callback(handle: *mut c_void, callback: PacketCallback) {
    if handle.is_null() {
        return;
    }

    let ctx = unsafe { &*(handle as *const CoreContext) };
    let mut engine = ctx.engine.lock().unwrap();
    engine.device.outbound_callback = Some(callback);
    info!("Rust core: Packet callback registered");
}
