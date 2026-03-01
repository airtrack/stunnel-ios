import Foundation

struct VPNConfig: Codable {
    var mode: String
    var serverAddr: String
    var serverName: String
    var cert: String // 证书内容
    var privKey: String // 私钥内容
    
    static let sharedSuiteName = "group.com.airtrack.stunnel"
    static let configKey = "vpn_configuration"
    
    // 获取共享容器的基础 URL
    static var sharedContainerURL: URL? {
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: sharedSuiteName)
    }
    
    // 将内容保存为共享目录下的文件，并返回该文件的绝对路径
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
    
    // 生成传给 Rust 的 JSON，此时需要将内容转为实际的沙盒路径
    func toRustConfigJSON() -> String? {
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
