use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::os::raw::c_void;
use std::task::Waker;

use bytes::Bytes;
use smoltcp::iface::{Config as SmolConfig, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
use smoltcp::wire::{IpCidr, Ipv4Address};

const TUN_ADDR: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const TUN_PREFIX_LEN: u8 = 24;
const TUN_MTU: usize = 1500;
const MAX_INBOUND_PACKETS: usize = 1024;

/// A simple buffer-backed device for smoltcp
pub struct TunDevice {
    pub inbound_packets: VecDeque<Bytes>,
    pub outbound_callback: Option<extern "C" fn(*mut c_void, *const u8, usize)>,
    pub outbound_context: usize,
}

impl TunDevice {
    fn new() -> Self {
        Self {
            inbound_packets: VecDeque::new(),
            outbound_callback: None,
            outbound_context: 0,
        }
    }
}

impl Device for TunDevice {
    type RxToken<'a>
        = RxToken
    where
        Self: 'a;
    type TxToken<'a>
        = TxToken<'a>
    where
        Self: 'a;

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
        caps.max_transmission_unit = TUN_MTU;
        caps.checksum = ChecksumCapabilities::ignored();
        caps
    }
}

pub struct RxToken {
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

pub struct TxToken<'a> {
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
            callback(
                self.device.outbound_context as *mut c_void,
                buffer.as_ptr(),
                buffer.len(),
            );
        }
        result
    }
}

/// The core engine managing the TCP/IP stack.
pub struct StunnelEngine {
    pub device: TunDevice,
    pub interface: Interface,
    pub sockets: SocketSet<'static>,
    pub wakers: HashMap<SocketHandle, Waker>,
    pub udp_sessions: HashMap<SocketAddr, SocketHandle>,
}

impl StunnelEngine {
    pub fn new() -> Self {
        let mut device = TunDevice::new();
        let config = SmolConfig::new(smoltcp::wire::HardwareAddress::Ip);
        let mut interface = Interface::new(config, &mut device, Instant::now());

        interface.update_ip_addrs(|addrs| {
            addrs
                .push(IpCidr::new(TUN_ADDR.into(), TUN_PREFIX_LEN))
                .unwrap();
        });

        interface
            .routes_mut()
            .add_default_ipv4_route(TUN_ADDR)
            .unwrap();

        Self {
            device,
            interface,
            sockets: SocketSet::new(vec![]),
            wakers: HashMap::new(),
            udp_sessions: HashMap::new(),
        }
    }

    pub fn poll(&mut self) {
        let timestamp = Instant::now();
        self.interface
            .poll(timestamp, &mut self.device, &mut self.sockets);

        for waker in self.wakers.values() {
            waker.wake_by_ref();
        }
        self.wakers.clear();
    }

    pub fn register_waker(&mut self, handle: SocketHandle, waker: Waker) {
        self.wakers.insert(handle, waker);
    }

    pub fn push_inbound_packet(&mut self, packet: Bytes) -> bool {
        if self.device.inbound_packets.len() >= MAX_INBOUND_PACKETS {
            return false;
        }

        self.device.inbound_packets.push_back(packet);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_INBOUND_PACKETS, StunnelEngine};

    use bytes::Bytes;

    #[test]
    fn inbound_queue_has_a_hard_cap() {
        let mut engine = StunnelEngine::new();

        for _ in 0..MAX_INBOUND_PACKETS {
            assert!(engine.push_inbound_packet(Bytes::from_static(&[1, 2, 3, 4])));
        }

        assert!(!engine.push_inbound_packet(Bytes::from_static(&[9, 9, 9, 9])));
        assert_eq!(engine.device.inbound_packets.len(), MAX_INBOUND_PACKETS);
    }
}
