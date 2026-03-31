import Foundation
import NetworkExtension

protocol TunnelSessionStarting {
    func startTunnel() throws
    func stopTunnel()
}

protocol TunnelProviderManaging: AnyObject {
    var protocolConfiguration: NETunnelProviderProtocol? { get set }
    var localizedDescription: String? { get set }
    var isEnabled: Bool { get set }
    var connectionStatus: NEVPNStatus { get }
    var tunnelSession: TunnelSessionStarting? { get }

    func saveToPreferences(_ completion: @escaping (Error?) -> Void)
    func loadFromPreferences(_ completion: @escaping (Error?) -> Void)
}

protocol TunnelProviderManagingStore {
    func loadManagers(completion: @escaping (Result<[TunnelProviderManaging], Error>) -> Void)
    func makeManager() -> TunnelProviderManaging
}

final class SystemTunnelProviderStore: TunnelProviderManagingStore {
    func loadManagers(completion: @escaping (Result<[TunnelProviderManaging], Error>) -> Void) {
        NETunnelProviderManager.loadAllFromPreferences { managers, error in
            if let error {
                completion(.failure(error))
                return
            }

            let wrappedManagers = (managers ?? []).map(SystemTunnelProviderManager.init)
            completion(.success(wrappedManagers))
        }
    }

    func makeManager() -> TunnelProviderManaging {
        SystemTunnelProviderManager(NETunnelProviderManager())
    }
}

private final class SystemTunnelProviderManager: TunnelProviderManaging {
    private let manager: NETunnelProviderManager

    init(_ manager: NETunnelProviderManager) {
        self.manager = manager
    }

    var protocolConfiguration: NETunnelProviderProtocol? {
        get { manager.protocolConfiguration as? NETunnelProviderProtocol }
        set { manager.protocolConfiguration = newValue }
    }

    var localizedDescription: String? {
        get { manager.localizedDescription }
        set { manager.localizedDescription = newValue }
    }

    var isEnabled: Bool {
        get { manager.isEnabled }
        set { manager.isEnabled = newValue }
    }

    var connectionStatus: NEVPNStatus {
        manager.connection.status
    }

    var tunnelSession: TunnelSessionStarting? {
        guard let session = manager.connection as? NETunnelProviderSession else {
            return nil
        }

        return SystemTunnelSession(session)
    }

    func saveToPreferences(_ completion: @escaping (Error?) -> Void) {
        manager.saveToPreferences(completionHandler: completion)
    }

    func loadFromPreferences(_ completion: @escaping (Error?) -> Void) {
        manager.loadFromPreferences(completionHandler: completion)
    }
}

private struct SystemTunnelSession: TunnelSessionStarting {
    private let session: NETunnelProviderSession

    init(_ session: NETunnelProviderSession) {
        self.session = session
    }

    func startTunnel() throws {
        try session.startTunnel(options: nil)
    }

    func stopTunnel() {
        session.stopTunnel()
    }
}
