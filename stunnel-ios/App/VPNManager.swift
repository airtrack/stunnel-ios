import Foundation
import NetworkExtension
import Combine

class VPNManager: ObservableObject {
    static let shared = VPNManager()
    
    @Published var status: NEVPNStatus = .disconnected
    @Published var isEnabled: Bool = false
    
    private var manager: NETunnelProviderManager?
    
    init() {
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
        NETunnelProviderManager.loadAllFromPreferences { managers, error in
            if let error = error {
                print("stunnel-ios: loadAllFromPreferences error: \(error)")
                completion(error)
                return
            }
            
            if let existingManager = managers?.first {
                print("stunnel-ios: Found existing VPN manager")
                self.manager = existingManager
            } else {
                print("stunnel-ios: No existing manager found, creating new one")
                let newManager = NETunnelProviderManager()
                let protocolConfiguration = NETunnelProviderProtocol()
                protocolConfiguration.providerBundleIdentifier = "me.airtrack.stunnel.PacketTunnel"
                newManager.protocolConfiguration = protocolConfiguration
                newManager.localizedDescription = "stunnel-ios"
                self.manager = newManager
            }
            
            self.isEnabled = self.manager?.isEnabled ?? false
            completion(nil)
        }
    }
    
    func setupAndConnect(config: VPNConfig) {
        print("stunnel-ios: setupAndConnect start")
        config.save()
        print("stunnel-ios: Config saved to App Group")
        
        loadAndCreateManager { error in
            guard let manager = self.manager else {
                print("stunnel-ios: Manager is still nil after load")
                return
            }
            
            let protocolConfiguration = (manager.protocolConfiguration as? NETunnelProviderProtocol) ?? NETunnelProviderProtocol()
            protocolConfiguration.providerBundleIdentifier = "me.airtrack.stunnel.PacketTunnel"
            protocolConfiguration.serverAddress = config.serverAddr
            
            manager.protocolConfiguration = protocolConfiguration
            manager.localizedDescription = "stunnel-ios"
            manager.isEnabled = true
            
            print("stunnel-ios: Saving manager to preferences...")
            manager.saveToPreferences { error in
                if let error = error {
                    print("stunnel-ios: saveToPreferences failed: \(error.localizedDescription)")
                    return
                }
                
                print("stunnel-ios: Save successful, loading to apply...")
                manager.loadFromPreferences { error in
                    if let error = error {
                        print("stunnel-ios: loadFromPreferences after save failed: \(error)")
                        return
                    }
                    
                    print("stunnel-ios: Manager ready, status: \(manager.connection.status.rawValue). Starting tunnel...")
                    guard let session = manager.connection as? NETunnelProviderSession else {
                        print("stunnel-ios: Error - connection is not NETunnelProviderSession")
                        return
                    }
                    
                    do {
                        try session.startTunnel(options: nil)
                        print("stunnel-ios: startTunnel command sent to system")
                    } catch {
                        print("stunnel-ios: startTunnel exception: \(error)")
                    }
                }
            }
        }
    }
    
    func disconnect() {
        print("stunnel-ios: disconnect requested")
        (manager?.connection as? NETunnelProviderSession)?.stopTunnel()
    }
    
    private func updateStatus() {
        if let status = manager?.connection.status {
            print("stunnel-ios: VPN Status updated to \(status.rawValue)")
            self.status = status
        }
    }
}
