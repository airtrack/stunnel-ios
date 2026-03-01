import NetworkExtension
import os.log

class PacketTunnelProvider: NEPacketTunnelProvider {

    private var rustHandle: UnsafeMutableRawPointer?
    static var shared: PacketTunnelProvider?

    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        os_log(.info, "stunnel-ios: PacketTunnelProvider startTunnel initiated")

        // 1. Initialize logging in Rust
        stunnel_init_logging()
        os_log(.info, "stunnel-ios: Rust logging initialized")

        // 2. Load configuration from App Group
        os_log(.info, "stunnel-ios: Loading config from App Group...")
        guard let config = VPNConfig.load() else {
            os_log(.error, "stunnel-ios: CRITICAL - Failed to load VPNConfig from UserDefaults")
            completionHandler(NSError(domain: "stunnel-ios", code: 1, userInfo: [NSLocalizedDescriptionKey: "Failed to load configuration"]))
            return
        }
        
        guard let configJson = config.toRustConfigJSON() else {
            os_log(.error, "stunnel-ios: CRITICAL - Failed to generate Rust config JSON (cert write error?)")
            completionHandler(NSError(domain: "stunnel-ios", code: 2, userInfo: [NSLocalizedDescriptionKey: "Failed to write cert files"]))
            return
        }
        
        os_log(.info, "stunnel-ios: Config loaded successfully for mode: %{public}@", config.mode)

        // 3. Configure the tunnel
        let tunnelNetworkSettings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: config.serverAddr)
        
        let ipv4Settings = NEIPv4Settings(addresses: ["192.168.1.2"], subnetMasks: ["255.255.255.0"])
        ipv4Settings.includedRoutes = [NEIPv4Route.default()]
        tunnelNetworkSettings.ipv4Settings = ipv4Settings
        
        tunnelNetworkSettings.dnsSettings = NEDNSSettings(servers: ["8.8.8.8", "1.1.1.1"])
        tunnelNetworkSettings.mtu = 1500

        os_log(.info, "stunnel-ios: Setting tunnel network settings...")
        setTunnelNetworkSettings(tunnelNetworkSettings) { error in
            if let error = error {
                os_log(.error, "stunnel-ios: Failed to set tunnel network settings: %{public}@", error.localizedDescription)
                completionHandler(error)
                return
            }

            os_log(.info, "stunnel-ios: Tunnel settings applied. Starting Rust core...")
            PacketTunnelProvider.shared = self

            // 4. Start the Rust core
            self.rustHandle = stunnel_start(configJson)
            
            if self.rustHandle == nil {
                os_log(.error, "stunnel-ios: CRITICAL - Rust core failed to start (stunnel_start returned nil)")
                completionHandler(NSError(domain: "stunnel-ios", code: 3, userInfo: [NSLocalizedDescriptionKey: "Failed to start Rust core"]))
                return
            }

            // 5. Setup callback from Rust to Swift
            let callback: @convention(c) (UnsafePointer<UInt8>?, Int) -> Void = { (packetPtr, len) in
                guard let packetPtr = packetPtr else { return }
                let data = Data(bytes: packetPtr, count: len)
                PacketTunnelProvider.shared?.packetFlow.writePackets([data], withProtocols: [NSNumber(value: AF_INET)])
            }
            stunnel_set_packet_callback(self.rustHandle, callback)
            os_log(.info, "stunnel-ios: Rust packet callback registered")

            // 6. Start reading packets from TUN
            os_log(.info, "stunnel-ios: Starting packet read loop")
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
