import Foundation
import NetworkExtension
import os.log

extension PacketTunnelProvider {
    
    // Function to set up the callback from Rust
    func setupRustPacketCallback() {
        guard let handle = rustHandle else { return }
        
        // This is a global closure to handle the callback from Rust.
        // Rust will call this when it wants to send a packet back to the TUN interface.
        // Since Rust callbacks are C-style, we use a global function pointer or a static closure.
        
        let packetCallback: @convention(c) (UnsafePointer<UInt8>?, Int) -> Void = { (packetPtr, len) in
            guard let packetPtr = packetPtr else { return }
            let data = Data(bytes: packetPtr, count: len)
            
            // We need a way to access the current PacketTunnelProvider instance.
            // Since this is a C-callback, we don't have direct access to 'self'.
            // In a real implementation, we would store a reference in a singleton or a global variable.
            
            PacketTunnelProvider.shared?.writePacket(data)
        }
        
        stunnel_set_packet_callback(handle, packetCallback)
    }
    
    // We'll need a way to write the packet back to the flow.
    func writePacket(_ data: Data) {
        // In the Network Extension, we send back raw IP packets via packetFlow.
        // We need to specify the protocol (IPv4 = 2, IPv6 = 10 as per Apple's AF_INET/AF_INET6 constants in NEPacketTunnelFlow).
        // For simplicity, we assume IPv4 (2). Real logic should check the packet header.
        packetFlow.writePackets([data], withProtocols: [NSNumber(value: AF_INET)])
    }
}

// Global/Singleton to handle the Rust-to-Swift callback
extension PacketTunnelProvider {
    static var shared: PacketTunnelProvider?
}
