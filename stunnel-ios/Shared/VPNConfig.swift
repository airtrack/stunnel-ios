import Foundation

struct VPNConfig: Codable {
    var mode: String
    var serverAddr: String
    var serverName: String
    var cert: String // Certificate content
    var privKey: String // Private key content
    
    static let sharedSuiteName = "group.me.airtrack.stunnel"
    static let configKey = "vpn_configuration"
    
    // Get the shared container base URL
    static var sharedContainerURL: URL? {
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: sharedSuiteName)
    }
    
    // Save content as a file in the shared directory and return its absolute path
    private func saveFile(name: String, content: String) -> String? {
        guard let containerURL = VPNConfig.sharedContainerURL else { return nil }
        let fileURL = containerURL.appendingPathComponent(name)
        try? content.write(to: fileURL, atomically: true, encoding: .utf8)
        return fileURL.path
    }
    
    func save() {
        let encoder = JSONEncoder()
        if let data = try? encoder.encode(self) {
            UserDefaults(suiteName: VPNConfig.sharedSuiteName)?.set(data, forKey: VPNConfig.configKey)
        }
    }
    
    static func load() -> VPNConfig? {
        guard let data = UserDefaults(suiteName: sharedSuiteName)?.data(forKey: configKey) else {
            return nil
        }
        let decoder = JSONDecoder()
        return try? decoder.decode(VPNConfig.self, from: data)
    }
    
    // Generate JSON for core engine, converting content to actual sandbox paths
    func toStunnelConfigJSON() -> String? {
        guard let certPath = saveFile(name: "client.crt", content: cert),
              let keyPath = saveFile(name: "client.key", content: privKey) else {
            return nil
        }
        
        let rustConfig: [String: String] = [
            "mode": mode,
            "server_addr": serverAddr,
            "server_name": serverName,
            "cert": certPath,
            "priv_key": keyPath
        ]
        
        guard let data = try? JSONSerialization.data(withJSONObject: rustConfig, options: []),
              let jsonString = String(data: data, encoding: .utf8) else {
            return nil
        }
        return jsonString
    }
}
