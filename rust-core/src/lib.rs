pub mod config;
pub mod connection;
pub mod engine;
pub mod tcp;
pub mod udp;
pub mod utils;

use crate::config::AppConfig;
use crate::connection::ConnectionManager;
use crate::engine::StunnelEngine;
use crate::tcp::{TcpStream, handle_tcp_direct_session, handle_tcp_proxy_session};
use crate::udp::{UdpSocket, handle_udp_direct_session, handle_udp_proxy_session};
use crate::utils::is_private_v4;

use bytes::Bytes;
use smoltcp::socket::tcp::{Socket as SmolTcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::socket::udp::{
    PacketBuffer as UdpPacketBuffer, PacketMetadata as UdpPacketMetadata, Socket as SmolUdpSocket,
};
use smoltcp::wire::{IpProtocol, TcpPacket, UdpPacket};
use std::ffi::CStr;
use std::net::SocketAddr;
use std::os::raw::{c_char, c_void};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tracing::{Level, error, info};
use tracing_subscriber;

/// The main handle for the C interface.
struct CoreContext {
    engine: Arc<Mutex<StunnelEngine>>,
    runtime: Runtime,
    config: AppConfig,
    conn_manager: Arc<ConnectionManager>,
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

    info!(
        "Starting stunnel-ios core engine with mode: {}",
        config.mode
    );

    let engine = Arc::new(Mutex::new(StunnelEngine::new()));
    let runtime = Runtime::new().unwrap();
    let conn_manager = Arc::new(ConnectionManager::new());

    let ctx = Box::new(CoreContext {
        engine,
        runtime,
        config,
        conn_manager,
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
        let is_direct = is_private_v4(dst_addr);

        match ip_packet.next_header() {
            IpProtocol::Tcp => {
                if let Ok(tcp_packet) = TcpPacket::new_checked(ip_packet.payload()) {
                    if tcp_packet.syn() && !tcp_packet.ack() {
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
                                handle_tcp_proxy_session(
                                    &mut stream,
                                    &config,
                                    &target,
                                    &conn_manager,
                                )
                                .await
                            };
                            if let Err(e) = result {
                                error!("TCP session failed for {}: {:?}", target, e);
                            }
                        });
                    }
                }
            }
            IpProtocol::Udp => {
                if let Ok(udp_packet) = UdpPacket::new_checked(ip_packet.payload()) {
                    let src_port = udp_packet.src_port();
                    let dst_port = udp_packet.dst_port();
                    let src_endpoint =
                        SocketAddr::new(std::net::IpAddr::V4(src_addr.into()), src_port);

                    if !engine.udp_sessions.contains_key(&src_endpoint) {
                        info!(
                            "Intercepted new UDP session from {} (direct: {})",
                            src_endpoint, is_direct
                        );

                        let udp_rx_buffer = UdpPacketBuffer::new(
                            vec![UdpPacketMetadata::EMPTY; 16],
                            vec![0; 65536],
                        );
                        let udp_tx_buffer = UdpPacketBuffer::new(
                            vec![UdpPacketMetadata::EMPTY; 16],
                            vec![0; 65536],
                        );
                        let mut socket = SmolUdpSocket::new(udp_rx_buffer, udp_tx_buffer);
                        socket.bind((dst_addr, dst_port)).unwrap();
                        let socket_handle = engine.sockets.add(socket);
                        engine.udp_sessions.insert(src_endpoint, socket_handle);

                        let proxy_socket = UdpSocket::new(socket_handle, Arc::clone(&ctx.engine));
                        let config = ctx.config.clone();
                        let conn_manager = Arc::clone(&ctx.conn_manager);
                        let target_addr =
                            SocketAddr::new(std::net::IpAddr::V4(dst_addr.into()), dst_port);

                        ctx.runtime.spawn(async move {
                            let result = if is_direct {
                                handle_udp_direct_session(proxy_socket, src_endpoint, target_addr)
                                    .await
                            } else {
                                handle_udp_proxy_session(
                                    proxy_socket,
                                    src_endpoint,
                                    &config,
                                    &conn_manager,
                                )
                                .await
                            };
                            if let Err(e) = result {
                                error!("UDP session failed for {}: {:?}", src_endpoint, e);
                            }
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
