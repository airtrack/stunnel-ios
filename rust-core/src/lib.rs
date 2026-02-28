use std::os::raw::{c_char, c_void};
use std::ffi::CStr;
use tracing::{info, Level};
use tracing_subscriber;

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_init_logging() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();
    info!("stunnel-ios rust-core logging initialized");
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_start(config_json: *const c_char) -> *mut c_void {
    let config_str = unsafe {
        if config_json.is_null() {
            return std::ptr::null_mut();
        }
        CStr::from_ptr(config_json).to_string_lossy()
    };
    
    info!("Starting stunnel-ios core with config: {}", config_str);
    
    // Placeholder for actual engine start logic.
    // In a real implementation, we'd spawn tokio tasks and return an opaque handle.
    
    let handle = Box::new(()); // Dummy handle
    Box::into_raw(handle) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_stop(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    info!("Stopping stunnel-ios core");
    unsafe {
        let _ = Box::from_raw(handle as *mut ());
    }
}

/// Send a packet from the TUN interface to the Rust stack.
#[unsafe(no_mangle)]
pub extern "C" fn stunnel_process_packet(handle: *mut c_void, packet: *const u8, len: usize) {
    if handle.is_null() || packet.is_null() {
        return;
    }
    let _packet_data = unsafe { std::slice::from_raw_parts(packet, len) };
    // info!("Processing packet of length: {}", len);
    
    // Here we will feed the packet to smoltcp and the proxy logic.
}

/// Callback for the Rust core to send a packet back to the TUN interface.
type PacketCallback = extern "C" fn(*const u8, usize);

#[unsafe(no_mangle)]
pub extern "C" fn stunnel_set_packet_callback(handle: *mut c_void, _callback: PacketCallback) {
    if handle.is_null() {
        return;
    }
    info!("Packet callback set");
    // Store callback for when we need to send packets back to TUN.
}
