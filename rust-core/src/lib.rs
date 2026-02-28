use std::os::raw::{c_char, c_void};
use std::ffi::CStr;
use std::sync::Mutex;
use tracing::{info, Level};
use tracing_subscriber;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, ChecksumCapabilities};
use smoltcp::wire::{IpCidr, Ipv4Address};
use smoltcp::time::Instant;
use std::collections::VecDeque;

/// A simple buffer-backed device for smoltcp
struct TunDevice {
    inbound_packets: VecDeque<Vec<u8>>,
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
    packet: Vec<u8>,
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

/// The core engine managing the TCP/IP stack and proxy logic.
struct StunnelEngine {
    device: TunDevice,
    interface: Interface,
    sockets: SocketSet<'static>,
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
            addrs.push(IpCidr::new(Ipv4Address::new(192, 168, 1, 1).into(), 24)).unwrap();
        });

        StunnelEngine {
            device,
            interface,
            sockets: SocketSet::new(vec![]),
        }
    }

    fn poll(&mut self) {
        let timestamp = Instant::now();
        self.interface.poll(timestamp, &mut self.device, &mut self.sockets);
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
    
    let packet_data = unsafe { std::slice::from_raw_parts(packet, len) }.to_vec();
    let engine_mutex = unsafe { &*(handle as *const Mutex<StunnelEngine>) };
    
    if let Ok(mut engine) = engine_mutex.lock() {
        engine.device.inbound_packets.push_back(packet_data);
        engine.poll();
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
