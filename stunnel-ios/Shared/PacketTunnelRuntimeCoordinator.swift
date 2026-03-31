import Foundation

typealias StunnelPacketCallback = @convention(c) (
    UnsafeMutableRawPointer?,
    UnsafePointer<UInt8>?,
    Int
) -> Void

protocol VPNConfigLoading {
    func load() throws -> VPNConfig?
}

struct SharedVPNConfigLoader: VPNConfigLoading {
    func load() throws -> VPNConfig? {
        try VPNConfig.load()
    }
}

protocol PacketTunnelCoreManaging {
    func create(configJson: String) -> UnsafeMutableRawPointer?
    func setPacketCallback(
        handle: UnsafeMutableRawPointer,
        context: UnsafeMutableRawPointer?,
        callback: StunnelPacketCallback
    )
    func start(handle: UnsafeMutableRawPointer) -> Bool
    func clearPacketCallback(handle: UnsafeMutableRawPointer)
    func stop(handle: UnsafeMutableRawPointer)
}

struct StunnelCoreManager: PacketTunnelCoreManaging {
    func create(configJson: String) -> UnsafeMutableRawPointer? {
        stunnel_create(configJson)
    }

    func setPacketCallback(
        handle: UnsafeMutableRawPointer,
        context: UnsafeMutableRawPointer?,
        callback: StunnelPacketCallback
    ) {
        stunnel_set_packet_callback(handle, context, callback)
    }

    func start(handle: UnsafeMutableRawPointer) -> Bool {
        stunnel_start(handle)
    }

    func clearPacketCallback(handle: UnsafeMutableRawPointer) {
        stunnel_clear_packet_callback(handle)
    }

    func stop(handle: UnsafeMutableRawPointer) {
        stunnel_stop(handle)
    }
}

struct PacketTunnelRuntimeHandle {
    let config: VPNConfig
    let handle: UnsafeMutableRawPointer
}

enum PacketTunnelRuntimeError: LocalizedError {
    case missingConfig
    case coreCreateFailed
    case coreStartFailed

    var errorDescription: String? {
        switch self {
        case .missingConfig:
            return "Failed to load configuration"
        case .coreCreateFailed:
            return "Failed to create Stunnel core runtime"
        case .coreStartFailed:
            return "Failed to start Stunnel core runtime"
        }
    }
}

final class PacketTunnelRuntimeCoordinator {
    private let configLoader: VPNConfigLoading
    private let coreManager: PacketTunnelCoreManaging
    private let runtimeConfigBuilder: (VPNConfig) throws -> String

    init(
        configLoader: VPNConfigLoading = SharedVPNConfigLoader(),
        coreManager: PacketTunnelCoreManaging = StunnelCoreManager(),
        runtimeConfigBuilder: @escaping (VPNConfig) throws -> String = { config in
            try config.makeRuntimeConfigJSON()
        }
    ) {
        self.configLoader = configLoader
        self.coreManager = coreManager
        self.runtimeConfigBuilder = runtimeConfigBuilder
    }

    func makeRuntime(
        packetContext: UnsafeMutableRawPointer?,
        callback: StunnelPacketCallback
    ) throws -> PacketTunnelRuntimeHandle {
        guard let config = try configLoader.load() else {
            throw PacketTunnelRuntimeError.missingConfig
        }

        let configJson = try runtimeConfigBuilder(config)
        guard let handle = coreManager.create(configJson: configJson) else {
            throw PacketTunnelRuntimeError.coreCreateFailed
        }

        coreManager.setPacketCallback(handle: handle, context: packetContext, callback: callback)
        guard coreManager.start(handle: handle) else {
            coreManager.clearPacketCallback(handle: handle)
            coreManager.stop(handle: handle)
            throw PacketTunnelRuntimeError.coreStartFailed
        }

        return PacketTunnelRuntimeHandle(config: config, handle: handle)
    }

    func stopRuntime(handle: UnsafeMutableRawPointer?) {
        guard let handle else {
            return
        }

        coreManager.clearPacketCallback(handle: handle)
        coreManager.stop(handle: handle)
    }
}
