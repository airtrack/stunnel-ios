import NetworkExtension
import os.log

class PacketTunnelProvider: NEPacketTunnelProvider {

    private var rustHandle: UnsafeMutableRawPointer?

    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        os_log(.info, "stunnel-ios: startTunnel")

        // 1. Initialize logging in Rust
        stunnel_init_logging()

        // 2. Configure the tunnel (example static config)
        let tunnelNetworkSettings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
        
        let ipv4Settings = NEIPv4Settings(addresses: ["192.168.1.1"], subnetMasks: ["255.255.255.0"])
        ipv4Settings.includedRoutes = [NEIPv4Route.default()]
        tunnelNetworkSettings.ipv4Settings = ipv4Settings
        
        tunnelNetworkSettings.dnsSettings = NEDNSSettings(servers: ["8.8.8.8", "1.1.1.1"])

        // 3. Set the network settings
        setTunnelNetworkSettings(tunnelNetworkSettings) { error in
            if let error = error {
                os_log(.error, "stunnel-ios: Failed to set tunnel network settings: %{public}@", error.localizedDescription)
                completionHandler(error)
                return
            }

            PacketTunnelProvider.shared = self

            // 4. Start the Rust core
            let configJson = "{}" // Placeholder for real config
            self.rustHandle = stunnel_start(configJson)
            
            if self.rustHandle == nil {
                os_log(.error, "stunnel-ios: Failed to start Rust core")
                completionHandler(NSError(domain: "stunnel-ios", code: 1, userInfo: [NSLocalizedDescriptionKey: "Failed to start Rust core"]))
                return
            }

            self.setupRustPacketCallback()

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
            
            // Continue reading
            self.readPackets()
        }
    }

    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)? = nil) {
        // Handle messages from the main app (e.g., status requests, config updates)
        completionHandler?(nil)
    }

    override func sleep(completionHandler: @escaping () -> Void) {
        completionHandler()
    }

    override func wake() {
    }
}
