import NetworkExtension
import os.log

class PacketTunnelProvider: NEPacketTunnelProvider {

    private var rustHandle: UnsafeMutableRawPointer?
    static var shared: PacketTunnelProvider?

    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        os_log(.info, "stunnel-ios: startTunnel")

        // 1. Initialize logging in Rust
        stunnel_init_logging()

        // 2. Configure the tunnel
        let tunnelNetworkSettings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
        
        // Internal IP for the virtual interface
        let ipv4Settings = NEIPv4Settings(addresses: ["192.168.1.2"], subnetMasks: ["255.255.255.0"])
        ipv4Settings.includedRoutes = [NEIPv4Route.default()]
        tunnelNetworkSettings.ipv4Settings = ipv4Settings
        
        tunnelNetworkSettings.dnsSettings = NEDNSSettings(servers: ["8.8.8.8", "1.1.1.1"])
        tunnelNetworkSettings.mtu = 1500

        setTunnelNetworkSettings(tunnelNetworkSettings) { error in
            if let error = error {
                os_log(.error, "stunnel-ios: Failed to set tunnel network settings: %{public}@", error.localizedDescription)
                completionHandler(error)
                return
            }

            PacketTunnelProvider.shared = self

            // 3. Start the Rust core with real configuration
            // In a real app, this config would come from the main app via App Groups
            let configJson = """
            {
                "mode": "s2n-quic",
                "server_addr": "your-proxy-server:443",
                "server_name": "your-domain.com",
                "cert": "path-to-cert",
                "priv_key": "path-to-key"
            }
            """
            self.rustHandle = stunnel_start(configJson)
            
            if self.rustHandle == nil {
                os_log(.error, "stunnel-ios: Failed to start Rust core")
                completionHandler(NSError(domain: "stunnel-ios", code: 1, userInfo: [NSLocalizedDescriptionKey: "Failed to start Rust core"]))
                return
            }

            // 4. Setup callback from Rust to Swift
            let callback: @convention(c) (UnsafePointer<UInt8>?, Int) -> Void = { (packetPtr, len) in
                guard let packetPtr = packetPtr else { return }
                let data = Data(bytes: packetPtr, count: len)
                PacketTunnelProvider.shared?.packetFlow.writePackets([data], withProtocols: [NSNumber(value: AF_INET)])
            }
            stunnel_set_packet_callback(self.rustHandle, callback)

            // 5. Start reading packets from TUN
            self.readPackets()

            completionHandler(nil)
        }
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        os_log(.info, "stunnel-ios: stopTunnel reason: %d", reason.rawValue)
        
        if let handle = rustHandle {
            stunnel_stop(handle)
            rustHandle = nil
        }
        
        PacketTunnelProvider.shared = nil
        completionHandler()
    }

    private func readPackets() {
        packetFlow.readPackets { [weak self] (packets, protocols) in
            guard let self = self, let handle = self.rustHandle else { return }
            
            for packet in packets {
                packet.withUnsafeBytes { ptr in
                    if let baseAddress = ptr.baseAddress {
                        stunnel_process_packet(handle, baseAddress.assumingMemoryBound(to: UInt8.self), packet.count)
                    }
                }
            }
            
            self.readPackets()
        }
    }
}
