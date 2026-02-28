use bytes::Bytes;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State as TcpState};
use smoltcp::socket::AnySocket;
use smoltcp::time::Instant;
use smoltcp::wire::{IpCidr, Ipv4Address, TcpPacket};
use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::io;
use std::os::raw::{c_char, c_void};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::Runtime;
use tracing::{error, info, Level};
use tracing_subscriber;

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

        let config = Config::new(smoltcp::wire::HardwareAddress::Ip);
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

        // Wake up all registered wakers to check their socket status
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

/// The main handle for the C interface.
struct CoreContext {
    engine: Arc<Mutex<StunnelEngine>>,
    runtime: Runtime,
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .try_init();
    info!("stunnel-ios rust-core logging initialized");
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_start(config_json: *const c_char) -> *mut c_void {
    let _config_str = unsafe {
        if config_json.is_null() {
            return std::ptr::null_mut();
        }
        CStr::from_ptr(config_json).to_string_lossy()
    };

    info!("Starting stunnel-ios core engine...");

    let engine = Arc::new(Mutex::new(StunnelEngine::new()));
    let runtime = Runtime::new().unwrap();

    let ctx = Box::new(CoreContext { engine, runtime });
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

                    let stream = TcpStream::new(socket_handle, Arc::clone(&ctx.engine));
                    
                    // Spawn task to handle the stream
                    ctx.runtime.spawn(async move {
                        info!("Proxy task started for {}:{}", dst_addr, dst_port);
                        // TODO: Use tokio::io::copy_bidirectional with stunnel client
                        let mut _stream = stream;
                        // For now just prevent it from being dropped immediately
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        info!("Proxy task finished for {}:{}", dst_addr, dst_port);
                    });
                }
            }
        }
    }

    engine.device.inbound_packets.push_back(bytes);
    engine.poll();
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
