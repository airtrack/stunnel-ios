import Foundation
import NetworkExtension
import Combine

class VPNManager: ObservableObject {
    static let shared = VPNManager()
    
    @Published var status: NEVPNStatus = .disconnected
    @Published var isEnabled: Bool = false
    
    private var manager: NETunnelProviderManager?
    
    init() {
        loadManager { _ in
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
    
    func loadManager(completion: @escaping (Error?) -> Void) {
        NETunnelProviderManager.loadAllFromPreferences { managers, error in
            if let error = error {
                completion(error)
                return
            }
            
            self.manager = managers?.first ?? NETunnelProviderManager()
            self.isEnabled = self.manager?.isEnabled ?? false
            completion(nil)
        }
    }
    
    func setupAndConnect(config: VPNConfig) {
        config.save()
        
        loadManager { error in
            guard let manager = self.manager else { return }
            
            manager.localizedDescription = "stunnel-ios"
            
            let protocolConfiguration = NETunnelProviderProtocol()
            protocolConfiguration.providerBundleIdentifier = "com.stunnel.ios.PacketTunnel"
            protocolConfiguration.serverAddress = config.serverAddr
            
            manager.protocolConfiguration = protocolConfiguration
            manager.isEnabled = true
            
            manager.saveToPreferences { error in
                if let error = error {
                    print("Failed to save VPN preferences: \(error)")
                    return
                }
                
                manager.loadFromPreferences { error in
                    guard let session = manager.connection as? NETunnelProviderSession else { return }
                    do {
                        try session.startTunnel(options: nil)
                    } catch {
                        print("Failed to start tunnel: \(error)")
                    }
                }
            }
        }
    }
    
    func disconnect() {
        guard let session = manager?.connection as? NETunnelProviderSession else { return }
        session.stopTunnel()
    }
    
    private func updateStatus() {
        if let status = manager?.connection.status {
            self.status = status
        }
    }
}
