pub mod config;
pub mod connection;
pub mod engine;
pub mod tcp;
pub mod udp;
pub mod utils;

use std::ffi::CStr;
use std::net::SocketAddr;
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use smoltcp::socket::tcp::{Socket as SmolTcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::socket::udp::{
    PacketBuffer as UdpPacketBuffer, PacketMetadata as UdpPacketMetadata, Socket as SmolUdpSocket,
};
use smoltcp::wire::{IpProtocol, TcpPacket, UdpPacket};
use tokio::runtime::Runtime;
use tracing::{Level, error, info};
use tracing_subscriber;

use crate::config::AppConfig;
use crate::connection::ConnectionManager;
use crate::engine::StunnelEngine;
use crate::tcp::{TcpStream, handle_tcp_direct_session, handle_tcp_proxy_session};
use crate::udp::{UdpSocket, handle_udp_direct_session, handle_udp_proxy_session};
use crate::utils::is_private_v4;

/// The main handle for the C interface.
struct CoreContext {
    engine: Arc<Mutex<StunnelEngine>>,
    runtime: Runtime,
    config: AppConfig,
    conn_manager: Arc<ConnectionManager>,
    started: AtomicBool,
}

impl CoreContext {
    fn new(config: AppConfig) -> Self {
        Self {
            engine: Arc::new(Mutex::new(StunnelEngine::new())),
            runtime: Runtime::new().unwrap(),
            config,
            conn_manager: Arc::new(ConnectionManager::new()),
            started: AtomicBool::new(false),
        }
    }
}

type PacketCallback = extern "C" fn(*mut c_void, *const u8, usize);

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .try_init();
    info!("stunnel-ios stunnel-core logging initialized");
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_create(config_json_ptr: *const c_char) -> *mut c_void {
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

    if let Err(error) = config.tunnel_mode() {
        error!("Invalid config: {:?}", error);
        return std::ptr::null_mut();
    }

    info!(
        "Creating stunnel-ios core engine with mode: {}",
        config.mode
    );

    let ctx = Box::new(CoreContext::new(config));
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_start(handle: *mut c_void) -> bool {
    if handle.is_null() {
        return false;
    }

    let ctx = unsafe { &*(handle as *const CoreContext) };
    let engine = ctx.engine.lock().unwrap();
    if engine.device.outbound_callback.is_none() {
        error!("Cannot start stunnel-ios core without a packet callback");
        return false;
    }

    ctx.started.store(true, Ordering::Release);
    info!("stunnel-ios core runtime started");
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_set_packet_callback(
    handle: *mut c_void,
    context: *mut c_void,
    callback: PacketCallback,
) {
    if handle.is_null() {
        return;
    }

    let ctx = unsafe { &*(handle as *const CoreContext) };
    let mut engine = ctx.engine.lock().unwrap();
    engine.device.outbound_callback = Some(callback);
    engine.device.outbound_context = context as usize;
    info!("Rust core: Packet callback registered");
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_clear_packet_callback(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }

    let ctx = unsafe { &*(handle as *const CoreContext) };
    let mut engine = ctx.engine.lock().unwrap();
    engine.device.outbound_callback = None;
    engine.device.outbound_context = 0;
    info!("Rust core: Packet callback cleared");
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_process_packet(handle: *mut c_void, packet: *const u8, len: usize) {
    if handle.is_null() || packet.is_null() {
        return;
    }

    let ctx = unsafe { &*(handle as *const CoreContext) };
    if !ctx.started.load(Ordering::Acquire) {
        return;
    }

    let packet_data = unsafe { std::slice::from_raw_parts(packet, len) };
    let bytes = Bytes::copy_from_slice(packet_data);

    let mut engine = ctx.engine.lock().unwrap();

    if let Ok(ip_packet) = smoltcp::wire::Ipv4Packet::new_checked(&bytes) {
        let src_addr = ip_packet.src_addr();
        let dst_addr = ip_packet.dst_addr();
        let is_direct = is_private_v4(dst_addr);

        match ip_packet.next_header() {
            IpProtocol::Tcp => {
                handle_tcp_packet(ctx, &mut engine, ip_packet.payload(), dst_addr, is_direct)
            }
            IpProtocol::Udp => handle_udp_packet(
                ctx,
                &mut engine,
                ip_packet.payload(),
                src_addr,
                dst_addr,
                is_direct,
            ),
            _ => {}
        }
    }

    if !engine.push_inbound_packet(bytes) {
        error!("Dropping inbound packet because the queue is full");
        return;
    }

    engine.poll();
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

fn handle_tcp_packet(
    ctx: &CoreContext,
    engine: &mut StunnelEngine,
    payload: &[u8],
    dst_addr: smoltcp::wire::Ipv4Address,
    is_direct: bool,
) {
    let Ok(tcp_packet) = TcpPacket::new_checked(payload) else {
        return;
    };

    if !tcp_packet.syn() || tcp_packet.ack() {
        return;
    }

    let dst_port = tcp_packet.dst_port();
    info!(
        "Intercepted TCP SYN to {}:{} (direct: {})",
        dst_addr, dst_port, is_direct
    );

    let tcp_rx_buffer = TcpSocketBuffer::new(vec![0; 65536]);
    let tcp_tx_buffer = TcpSocketBuffer::new(vec![0; 65536]);
    let mut socket = SmolTcpSocket::new(tcp_rx_buffer, tcp_tx_buffer);
    socket.listen((dst_addr, dst_port)).unwrap();

    let socket_handle = engine.sockets.add(socket);
    let mut stream = TcpStream::new(socket_handle, Arc::clone(&ctx.engine));
    let config = ctx.config.clone();
    let target = format!("{}:{}", dst_addr, dst_port);
    let conn_manager = Arc::clone(&ctx.conn_manager);

    ctx.runtime.spawn(async move {
        let result = if is_direct {
            handle_tcp_direct_session(&mut stream, &target).await
        } else {
            handle_tcp_proxy_session(&mut stream, &config, &target, &conn_manager).await
        };

        if let Err(error) = result {
            error!("TCP session failed for {}: {:?}", target, error);
        }
    });
}

fn handle_udp_packet(
    ctx: &CoreContext,
    engine: &mut StunnelEngine,
    payload: &[u8],
    src_addr: smoltcp::wire::Ipv4Address,
    dst_addr: smoltcp::wire::Ipv4Address,
    is_direct: bool,
) {
    let Ok(udp_packet) = UdpPacket::new_checked(payload) else {
        return;
    };

    let src_port = udp_packet.src_port();
    let dst_port = udp_packet.dst_port();
    let src_endpoint = SocketAddr::new(std::net::IpAddr::V4(src_addr.into()), src_port);

    if engine.udp_sessions.contains_key(&src_endpoint) {
        return;
    }

    info!(
        "Intercepted new UDP session from {} (direct: {})",
        src_endpoint, is_direct
    );

    let udp_rx_buffer = UdpPacketBuffer::new(vec![UdpPacketMetadata::EMPTY; 16], vec![0; 65536]);
    let udp_tx_buffer = UdpPacketBuffer::new(vec![UdpPacketMetadata::EMPTY; 16], vec![0; 65536]);
    let mut socket = SmolUdpSocket::new(udp_rx_buffer, udp_tx_buffer);
    socket.bind((dst_addr, dst_port)).unwrap();

    let socket_handle = engine.sockets.add(socket);
    engine.udp_sessions.insert(src_endpoint, socket_handle);

    let proxy_socket = UdpSocket::new(socket_handle, Arc::clone(&ctx.engine));
    let config = ctx.config.clone();
    let conn_manager = Arc::clone(&ctx.conn_manager);
    let target_addr = SocketAddr::new(std::net::IpAddr::V4(dst_addr.into()), dst_port);

    ctx.runtime.spawn(async move {
        let result = if is_direct {
            handle_udp_direct_session(proxy_socket, src_endpoint, target_addr).await
        } else {
            handle_udp_proxy_session(proxy_socket, src_endpoint, &config, &conn_manager).await
        };

        if let Err(error) = result {
            error!("UDP session failed for {}: {:?}", src_endpoint, error);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::os::raw::c_void;
    use std::ptr;
    use std::sync::atomic::Ordering;

    use bytes::Bytes;

    use super::{
        CoreContext, stunnel_clear_packet_callback, stunnel_create, stunnel_process_packet,
        stunnel_set_packet_callback, stunnel_start, stunnel_stop,
    };

    extern "C" fn noop_callback(_context: *mut c_void, _packet: *const u8, _len: usize) {}

    fn valid_config_json() -> CString {
        CString::new(
            r#"{"mode":"s2n-quic","server_addr":"127.0.0.1:443","server_name":"localhost","cert":"client.crt","priv_key":"client.key"}"#,
        )
        .unwrap()
    }

    #[test]
    fn create_rejects_invalid_json() {
        let invalid = CString::new("{").unwrap();
        let handle = stunnel_create(invalid.as_ptr());

        assert!(handle.is_null());
    }

    #[test]
    fn start_requires_callback_registration() {
        let handle = stunnel_create(valid_config_json().as_ptr());
        assert!(!handle.is_null());

        assert!(!stunnel_start(handle));

        unsafe {
            let ctx = &*(handle as *const CoreContext);
            assert!(!ctx.started.load(Ordering::Acquire));
        }

        stunnel_stop(handle);
    }

    #[test]
    fn start_succeeds_after_callback_registration() {
        let handle = stunnel_create(valid_config_json().as_ptr());
        assert!(!handle.is_null());

        stunnel_set_packet_callback(handle, ptr::null_mut(), noop_callback);
        assert!(stunnel_start(handle));

        unsafe {
            let ctx = &*(handle as *const CoreContext);
            assert!(ctx.started.load(Ordering::Acquire));
        }

        stunnel_clear_packet_callback(handle);
        stunnel_stop(handle);
    }

    #[test]
    fn create_rejects_unsupported_tunnel_mode() {
        let invalid = CString::new(
            r#"{"mode":"invalid","server_addr":"127.0.0.1:443","server_name":"localhost","cert":"client.crt","priv_key":"client.key"}"#,
        )
        .unwrap();
        let handle = stunnel_create(invalid.as_ptr());

        assert!(handle.is_null());
    }

    #[test]
    fn process_packet_is_a_no_op_until_started() {
        let handle = stunnel_create(valid_config_json().as_ptr());
        assert!(!handle.is_null());

        let packet = Bytes::from_static(&[0, 1, 2, 3]);
        stunnel_process_packet(handle, packet.as_ptr(), packet.len());

        unsafe {
            let ctx = &*(handle as *const CoreContext);
            let engine = ctx.engine.lock().unwrap();
            assert!(engine.device.inbound_packets.is_empty());
        }

        stunnel_stop(handle);
    }
}
