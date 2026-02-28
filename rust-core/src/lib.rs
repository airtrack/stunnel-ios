use bytes::{Bytes, BytesMut};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::socket::AnySocket;
use smoltcp::time::Instant;
use smoltcp::wire::{IpCidr, Ipv4Address, TcpPacket};
use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
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

/// Represents an active TCP session being proxied.
struct Session {
    to_proxy_tx: mpsc::Sender<Bytes>,
    from_proxy_rx: mpsc::Receiver<Bytes>,
}

/// The core engine managing the TCP/IP stack and proxy logic.
struct StunnelEngine {
    device: TunDevice,
    interface: Interface,
    sockets: SocketSet<'static>,
    runtime: Runtime,
    sessions: HashMap<SocketHandle, Session>,
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

        let sockets = SocketSet::new(vec![]);
        let runtime = Runtime::new().unwrap();

        StunnelEngine {
            device,
            interface,
            sockets,
            runtime,
            sessions: HashMap::new(),
        }
    }

    fn handle_new_packet(&mut self, packet_data: Bytes) {
        if let Ok(ip_packet) = smoltcp::wire::Ipv4Packet::new_checked(&packet_data) {
            if ip_packet.next_header() == smoltcp::wire::IpProtocol::Tcp {
                if let Ok(tcp_packet) = TcpPacket::new_checked(ip_packet.payload()) {
                    if tcp_packet.syn() && !tcp_packet.ack() {
                        let dst_addr = ip_packet.dst_addr();
                        let dst_port = tcp_packet.dst_port();

                        info!("Intercepted TCP SYN to {}:{}", dst_addr, dst_port);
                        self.create_socket(dst_addr, dst_port);
                    }
                }
            }
        }

        self.device.inbound_packets.push_back(packet_data);
        self.poll();
    }

    fn create_socket(&mut self, dst_addr: Ipv4Address, dst_port: u16) {
        let tcp_rx_buffer = TcpSocketBuffer::new(vec![0; 65536]);
        let tcp_tx_buffer = TcpSocketBuffer::new(vec![0; 65536]);
        let mut socket = TcpSocket::new(tcp_rx_buffer, tcp_tx_buffer);

        if let Err(e) = socket.listen((dst_addr, dst_port)) {
            error!("Failed to listen on socket: {:?}", e);
        } else {
            self.sockets.add(socket);
        }
    }

    fn poll(&mut self) {
        let timestamp = Instant::now();
        self.interface
            .poll(timestamp, &mut self.device, &mut self.sockets);
        self.manage_sessions();
    }

    fn manage_sessions(&mut self) {
        let mut to_remove = Vec::new();
        let mut to_start = Vec::new();

        // 1. Identify connections to start or pump
        for (handle, socket) in self.sockets.iter_mut() {
            if let Some(tcp_socket) = TcpSocket::downcast_mut(socket) {
                if tcp_socket.is_active() {
                    if !self.sessions.contains_key(&handle) {
                        to_start.push(handle);
                    } else {
                        // Pump data Smoltcp -> MPSC
                        if tcp_socket.can_recv() {
                            let session = self.sessions.get_mut(&handle).unwrap();
                            let _ = tcp_socket.recv(|data| {
                                if !data.is_empty() {
                                    let _ = session.to_proxy_tx.try_send(Bytes::copy_from_slice(data));
                                }
                                (data.len(), ())
                            });
                        }
                    }
                } else if self.sessions.contains_key(&handle) {
                    to_remove.push(handle);
                }
            }
        }

        // 2. Start new sessions
        for handle in to_start {
            let socket = self.sockets.get_mut::<TcpSocket>(handle);
            let remote_endpoint = socket.remote_endpoint();
            info!("Starting proxy session for {:?}", remote_endpoint);

            let (to_proxy_tx, mut to_proxy_rx) = mpsc::channel::<Bytes>(100);
            let (_from_proxy_tx, from_proxy_rx) = mpsc::channel::<Bytes>(100);

            self.sessions.insert(
                handle,
                Session {
                    to_proxy_tx,
                    from_proxy_rx,
                },
            );

            self.runtime.spawn(async move {
                while let Some(data) = to_proxy_rx.recv().await {
                    info!("Outbound task received {} bytes", data.len());
                }
            });
        }

        // 3. Pump MPSC -> Smoltcp
        for (handle, session) in self.sessions.iter_mut() {
            if let Ok(data) = session.from_proxy_rx.try_recv() {
                let socket = self.sockets.get_mut::<TcpSocket>(*handle);
                if socket.can_send() {
                    let _ = socket.send_slice(&data);
                }
            }
        }

        // 4. Remove closed sessions
        for handle in to_remove {
            info!("Closing session for handle {:?}", handle);
            self.sessions.remove(&handle);
            self.sockets.remove(handle);
        }
    }
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

    let engine = Box::new(Mutex::new(StunnelEngine::new()));
    Box::into_raw(engine) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_stop(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    info!("Stopping stunnel-ios core engine");
    unsafe {
        let _ = Box::from_raw(handle as *mut Mutex<StunnelEngine>);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_process_packet(handle: *mut c_void, packet: *const u8, len: usize) {
    if handle.is_null() || packet.is_null() {
        return;
    }

    let packet_data = unsafe { std::slice::from_raw_parts(packet, len) };
    let bytes = Bytes::copy_from_slice(packet_data);
    
    let engine_mutex = unsafe { &*(handle as *const Mutex<StunnelEngine>) };

    if let Ok(mut engine) = engine_mutex.lock() {
        engine.handle_new_packet(bytes);
    }
}

type PacketCallback = extern "C" fn(*const u8, usize);

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_set_packet_callback(handle: *mut c_void, callback: PacketCallback) {
    if handle.is_null() {
        return;
    }

    let engine_mutex = unsafe { &*(handle as *const Mutex<StunnelEngine>) };
    if let Ok(mut engine) = engine_mutex.lock() {
        engine.device.outbound_callback = Some(callback);
        info!("Rust core: Packet callback registered");
    }
}
