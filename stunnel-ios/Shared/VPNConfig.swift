import Foundation

struct VPNConfig: Codable {
    var mode: String // "s2n-quic", "tlstcp"
    var serverAddr: String
    var serverName: String
    var cert: String
    var privKey: String
    
    static let sharedSuiteName = "group.com.stunnel.ios"
    static let configKey = "vpn_configuration"
    
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
    
    func toJSONString() -> String? {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        guard let data = try? encoder.encode(self) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
