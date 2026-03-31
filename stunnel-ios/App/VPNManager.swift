import Foundation
import NetworkExtension
import Combine

class VPNManager: ObservableObject {
    static let shared = VPNManager()
    
    @Published var status: NEVPNStatus = .disconnected
    @Published var isEnabled: Bool = false
    @Published var lastErrorMessage: String?
    
    private let store: TunnelProviderManagingStore
    private var tunnelManager: TunnelProviderManaging?
    private let configSaver: (VPNConfig) throws -> Void

    init(
        store: TunnelProviderManagingStore = SystemTunnelProviderStore(),
        configSaver: @escaping (VPNConfig) throws -> Void = { config in try config.save() }
    ) {
        self.store = store
        self.configSaver = configSaver
        print("stunnel-ios: VPNManager init")
        loadAndCreateManager { _ in
            self.updateStatus()
        }
        
        NotificationCenter.default.addObserver(
            forName: .NEVPNStatusDidChange,
            object: nil,
            queue: .main
        ) { _ in
            self.updateStatus()
        }
    }
    
    func loadAndCreateManager(completion: @escaping (Error?) -> Void) {
        store.loadManagers { result in
            switch result {
            case .failure(let error):
                print("stunnel-ios: loadAllFromPreferences error: \(error)")
                completion(error)
            case .success(let managers):
                if let existingManager = managers.first {
                    print("stunnel-ios: Found existing VPN manager")
                    self.tunnelManager = existingManager
                } else {
                    print("stunnel-ios: No existing manager found, creating new one")
                    let newManager = self.store.makeManager()
                    let protocolConfiguration = NETunnelProviderProtocol()
                    protocolConfiguration.providerBundleIdentifier = "me.airtrack.stunnel.PacketTunnel"
                    newManager.protocolConfiguration = protocolConfiguration
                    newManager.localizedDescription = "stunnel-ios"
                    self.tunnelManager = newManager
                }

                self.isEnabled = self.tunnelManager?.isEnabled ?? false
                completion(nil)
            }
        }
    }

    func disconnect() {
        print("stunnel-ios: disconnect requested")
        tunnelManager?.tunnelSession?.stopTunnel()
    }

    private func updateStatus() {
        if let status = tunnelManager?.connectionStatus {
            print("stunnel-ios: VPN Status updated to \(status.rawValue)")
            self.status = status
        }
    }

    private func configure(manager: TunnelProviderManaging, config: VPNConfig) {
        let protocolConfiguration = manager.protocolConfiguration ?? NETunnelProviderProtocol()
        protocolConfiguration.providerBundleIdentifier = "me.airtrack.stunnel.PacketTunnel"
        protocolConfiguration.serverAddress = config.serverAddr

        manager.protocolConfiguration = protocolConfiguration
        manager.localizedDescription = "stunnel-ios"
        manager.isEnabled = true
    }

    private func saveManagerPreferences(_ manager: TunnelProviderManaging) {
        print("stunnel-ios: Saving manager to preferences...")
        manager.saveToPreferences { error in
            if let error {
                self.fail(step: "save preferences", error: error)
                return
            }

            self.reloadManagerPreferences(manager)
        }
    }

    private func reloadManagerPreferences(_ manager: TunnelProviderManaging) {
        print("stunnel-ios: Save successful, loading to apply...")
        manager.loadFromPreferences { error in
            if let error {
                self.fail(step: "reload preferences", error: error)
                return
            }

            self.startTunnelSession(manager)
        }
    }

    private func startTunnelSession(_ manager: TunnelProviderManaging) {
        print("stunnel-ios: Manager ready, status: \(manager.connectionStatus.rawValue). Starting tunnel...")
        guard let session = manager.tunnelSession else {
            fail(step: "start tunnel", message: "VPN connection is not a tunnel provider session")
            return
        }

        do {
            try session.startTunnel()
            clearError()
            print("stunnel-ios: startTunnel command sent to system")
        } catch {
            fail(step: "start tunnel", error: error)
        }
    }

    func setupAndConnect(config: VPNConfig) {
        print("stunnel-ios: setupAndConnect start")
        clearError()

        do {
            try configSaver(config)
            print("stunnel-ios: Config saved to App Group")
        } catch {
            fail(step: "save config", error: error)
            return
        }

        loadAndCreateManager { error in
            if let error {
                self.fail(step: "load manager", error: error)
                return
            }

            guard let manager = self.tunnelManager else {
                self.fail(
                    step: "load manager",
                    message: "VPN manager is unavailable after loading preferences"
                )
                return
            }

            self.configure(manager: manager, config: config)
            self.saveManagerPreferences(manager)
        }
    }

    private func clearError() {
        DispatchQueue.main.async {
            self.lastErrorMessage = nil
        }
    }

    private func fail(step: String, error: Error) {
        fail(step: step, message: error.localizedDescription)
        print("stunnel-ios: \(step) failed: \(error)")
    }

    private func fail(step: String, message: String) {
        print("stunnel-ios: \(step) failed: \(message)")
        DispatchQueue.main.async {
            self.lastErrorMessage = "Failed to \(step): \(message)"
            self.status = .disconnected
        }
    }
}
