import NetworkExtension
import os.log

private final class PacketFlowBridge {
    weak var provider: PacketTunnelProvider?

    init(provider: PacketTunnelProvider) {
        self.provider = provider
    }

    func writePacket(_ packetPtr: UnsafePointer<UInt8>, length: Int) {
        let data = Data(bytes: packetPtr, count: length)
        provider?.packetFlow.writePackets([data], withProtocols: [NSNumber(value: AF_INET)])
    }
}

class PacketTunnelProvider: NEPacketTunnelProvider {

    private var rustHandle: UnsafeMutableRawPointer?
    private var packetBridgeContext: UnsafeMutableRawPointer?
    private let runtimeCoordinator: PacketTunnelRuntimeCoordinator

    override init() {
        self.runtimeCoordinator = PacketTunnelRuntimeCoordinator()
        super.init()
    }

    init(runtimeCoordinator: PacketTunnelRuntimeCoordinator) {
        self.runtimeCoordinator = runtimeCoordinator
        super.init()
    }

    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        os_log(.info, "stunnel-ios: PacketTunnelProvider startTunnel initiated")

        // 1. Initialize logging in Stunnel core
        stunnel_init_logging()
        os_log(.info, "stunnel-ios: Stunnel core logging initialized")

        // 2. Load configuration from App Group
        os_log(.info, "stunnel-ios: Loading config from App Group...")
        let runtime: PacketTunnelRuntimeHandle
        do {
            installPacketBridge()
            runtime = try runtimeCoordinator.makeRuntime(
                packetContext: packetBridgeContext,
                callback: packetFlowCallback
            )
        } catch {
            releasePacketBridge()
            os_log(.error, "stunnel-ios: CRITICAL - Failed to prepare runtime: %{public}@", error.localizedDescription)
            completionHandler(error)
            return
        }
        let config = runtime.config
        self.rustHandle = runtime.handle

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

            os_log(.info, "stunnel-ios: Tunnel settings applied. Starting Stunnel core...")
            // 6. Start reading packets from TUN
            os_log(.info, "stunnel-ios: Starting packet read loop")
            self.readPackets()

            completionHandler(nil)
        }
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        os_log(.info, "stunnel-ios: stopTunnel reason: %d", reason.rawValue)

        stopCoreRuntime()
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

    private func installPacketBridge() {
        releasePacketBridge()

        let bridge = PacketFlowBridge(provider: self)
        packetBridgeContext = Unmanaged.passRetained(bridge).toOpaque()
    }

    private func releasePacketBridge() {
        guard let packetBridgeContext else {
            return
        }

        Unmanaged<PacketFlowBridge>.fromOpaque(packetBridgeContext).release()
        self.packetBridgeContext = nil
    }

    private func stopCoreRuntime() {
        runtimeCoordinator.stopRuntime(handle: rustHandle)
        rustHandle = nil

        releasePacketBridge()
    }
}

private let packetFlowCallback: StunnelPacketCallback = { context, packetPtr, length in
    guard let context, let packetPtr else {
        return
    }

    let bridge = Unmanaged<PacketFlowBridge>.fromOpaque(context).takeUnretainedValue()
    bridge.writePacket(packetPtr, length: length)
}
