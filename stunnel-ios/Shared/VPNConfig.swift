import Foundation

struct VPNConfig: Codable {
    var mode: String
    var serverAddr: String
    var serverName: String
    var cert: String // Certificate content
    var privKey: String // Private key content
    
    static let sharedSuiteName = "group.me.airtrack.stunnel"
    static let configKey = "vpn_configuration"

    func save() throws {
        try save(defaults: UserDefaults(suiteName: VPNConfig.sharedSuiteName))
    }

    func save(defaults: UserDefaults?) throws {
        let encoder = JSONEncoder()
        let data = try encoder.encode(self)

        guard let defaults else {
            throw VPNConfigError.sharedDefaultsUnavailable
        }

        defaults.set(data, forKey: VPNConfig.configKey)
    }

    static func load() throws -> VPNConfig? {
        try load(defaults: UserDefaults(suiteName: sharedSuiteName))
    }

    static func load(defaults: UserDefaults?) throws -> VPNConfig? {
        guard let defaults else {
            throw VPNConfigError.sharedDefaultsUnavailable
        }

        guard let data = defaults.data(forKey: configKey) else {
            return nil
        }

        let decoder = JSONDecoder()
        return try decoder.decode(VPNConfig.self, from: data)
    }

    func makeRuntimeConfigJSON() throws -> String {
        try StunnelRuntimeConfigWriter().write(config: self)
    }

    func makeRuntimeConfigJSON(containerURL: URL?) throws -> String {
        try StunnelRuntimeConfigWriter(containerURL: containerURL).write(config: self)
    }
}

enum VPNConfigError: LocalizedError {
    case sharedDefaultsUnavailable
    case sharedContainerUnavailable
    case certWriteFailed(Error)
    case keyWriteFailed(Error)
    case runtimeJSONEncodingFailed

    var errorDescription: String? {
        switch self {
        case .sharedDefaultsUnavailable:
            return "Shared App Group defaults are unavailable"
        case .sharedContainerUnavailable:
            return "Shared App Group container is unavailable"
        case .certWriteFailed(let error):
            return "Failed to write client certificate: \(error.localizedDescription)"
        case .keyWriteFailed(let error):
            return "Failed to write client key: \(error.localizedDescription)"
        case .runtimeJSONEncodingFailed:
            return "Failed to encode runtime stunnel configuration"
        }
    }
}

struct StunnelRuntimeConfigWriter {
    private static let certFileName = "client.crt"
    private static let keyFileName = "client.key"
    private let containerURL: URL?

    init(containerURL: URL? = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: VPNConfig.sharedSuiteName
    )) {
        self.containerURL = containerURL
    }

    func write(config: VPNConfig) throws -> String {
        let certPath = try writeFile(
            name: Self.certFileName,
            content: config.cert,
            mapError: VPNConfigError.certWriteFailed
        )
        let keyPath = try writeFile(
            name: Self.keyFileName,
            content: config.privKey,
            mapError: VPNConfigError.keyWriteFailed
        )

        let rustConfig: [String: String] = [
            "mode": config.mode,
            "server_addr": config.serverAddr,
            "server_name": config.serverName,
            "cert": certPath,
            "priv_key": keyPath,
        ]

        guard let data = try? JSONSerialization.data(withJSONObject: rustConfig, options: []),
              let jsonString = String(data: data, encoding: .utf8) else {
            throw VPNConfigError.runtimeJSONEncodingFailed
        }

        return jsonString
    }

    private func writeFile(
        name: String,
        content: String,
        mapError: (Error) -> VPNConfigError
    ) throws -> String {
        guard let containerURL else {
            throw VPNConfigError.sharedContainerUnavailable
        }

        let fileURL = containerURL.appendingPathComponent(name)
        do {
            try content.write(to: fileURL, atomically: true, encoding: .utf8)
        } catch {
            throw mapError(error)
        }

        return fileURL.path
    }
}
