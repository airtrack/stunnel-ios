use bytes::Bytes;
use serde::{Deserialize, Serialize};
use smoltcp::iface::{Config as SmolConfig, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State as TcpState};
use smoltcp::time::Instant;
use smoltcp::wire::{IpCidr, Ipv4Address, TcpPacket};
use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::io;
use std::net::SocketAddr;
use std::os::raw::{c_char, c_void};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::Runtime;
use tokio::sync::OnceCell;
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

#[derive(Clone)]
enum OutboundConnector {
    S2nQuic(s2n_quic::connection::Handle),
    TlsTcp(stunnel::tlstcp::client::Connector),
}

/// The main handle for the C interface.
struct CoreContext {
    engine: Arc<Mutex<StunnelEngine>>,
    runtime: Runtime,
    config: AppConfig,
    connector: OnceCell<OutboundConnector>,
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

    let ctx = Box::new(CoreContext {
        engine,
        runtime,
        config,
        connector: OnceCell::new(),
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

    // Intercept TCP SYN
    if let Ok(ip_packet) = smoltcp::wire::Ipv4Packet::new_checked(&bytes) {
        if ip_packet.next_header() == smoltcp::wire::IpProtocol::Tcp {
            if let Ok(tcp_packet) = TcpPacket::new_checked(ip_packet.payload()) {
                if tcp_packet.syn() && !tcp_packet.ack() {
                    let dst_addr = ip_packet.dst_addr();
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
                    let connector_cell = ctx.connector.clone();

                    ctx.runtime.spawn(async move {
                        info!("Proxy task started for {}", target);
                        
                        // Get or initialize the connector
                        let connector = match get_connector(&connector_cell, &config).await {
                            Ok(c) => c,
                            Err(e) => {
                                error!("Failed to get connector: {:?}", e);
                                return;
                            }
                        };

                        let result = handle_proxy_session(&mut stream, connector, &target).await;
                        if let Err(e) = result {
                            error!("Proxy session failed for {}: {:?}", target, e);
                        }
                        info!("Proxy task finished for {}", target);
                    });
                }
            }
        }
    }

    engine.device.inbound_packets.push_back(bytes);
    engine.poll();
}

async fn get_connector(
    cell: &OnceCell<OutboundConnector>,
    config: &AppConfig,
) -> io::Result<OutboundConnector> {
    cell.get_or_try_init(|| async {
        match config.mode.as_str() {
            "s2n-quic" => {
                let quic_config = stunnel::quic::Config {
                    addr: "0.0.0.0:0".to_string(),
                    cert: config.cert.clone(),
                    priv_key: config.priv_key.clone(),
                    loss_threshold: 10,
                };
                let client = stunnel::quic::s2n_quic::client::new(&quic_config)?;
                let addr: SocketAddr = config.server_addr.parse().map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid addr: {:?}", e))
                })?;
                let connect = s2n_quic::client::Connect::new(addr).with_server_name(config.server_name.as_str());
                let mut conn = client.connect(connect).await.map_err(|e| {
                    io::Error::new(io::ErrorKind::Other, format!("QUIC connect error: {:?}", e))
                })?;
                conn.keep_alive(true).map_err(|e| {
                    io::Error::new(io::ErrorKind::Other, format!("QUIC keep-alive error: {:?}", e))
                })?;
                Ok(OutboundConnector::S2nQuic(conn.handle()))
            }
            "tlstcp" => {
                let tls_config = stunnel::tlstcp::client::Config {
                    server_addr: config.server_addr.clone(),
                    server_name: config.server_name.clone(),
                    cert: config.cert.clone(),
                    priv_key: config.priv_key.clone(),
                };
                let connector = stunnel::tlstcp::client::new(&tls_config);
                Ok(OutboundConnector::TlsTcp(connector))
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Unsupported mode")),
        }
    }).await.cloned()
}

async fn handle_proxy_session(
    stream: &mut TcpStream,
    connector: OutboundConnector,
    target: &str,
) -> io::Result<()> {
    match connector {
        OutboundConnector::S2nQuic(handle) => {
            let mut tunnel = stunnel::tunnel::client::connect_tcp_tunnel(handle, target).await?.1;
            tokio::io::copy_bidirectional(stream, &mut tunnel).await?;
        }
        OutboundConnector::TlsTcp(connector) => {
            let mut tunnel = stunnel::tunnel::client::connect_tcp_tunnel(connector, target).await?.1;
            tokio::io::copy_bidirectional(stream, &mut tunnel).await?;
        }
    }
    Ok(())
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
