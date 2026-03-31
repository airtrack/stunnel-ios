import XCTest
import NetworkExtension
@testable import stunnel_ios

final class VPNManagerTests: XCTestCase {
    func testSetupAndConnectStartsTunnelAfterSavingAndReloadingPreferences() {
        let store = FakeTunnelProviderStore()
        let manager = FakeTunnelProviderManager()
        store.loadedManagers = [manager]

        let vpnManager = VPNManager(store: store, configSaver: { _ in })
        let config = sampleConfig()

        vpnManager.setupAndConnect(config: config)

        XCTAssertEqual(manager.savedConfigurations.count, 1)
        XCTAssertEqual(manager.savedConfigurations.first?.serverAddress, config.serverAddr)
        XCTAssertTrue(manager.saveCalled)
        XCTAssertTrue(manager.reloadCalled)
        XCTAssertEqual(manager.session.startCount, 1)
        XCTAssertNil(vpnManager.lastErrorMessage)
    }

    func testSetupAndConnectSurfacesSessionStartFailure() {
        let store = FakeTunnelProviderStore()
        let manager = FakeTunnelProviderManager()
        manager.session.startError = NSError(domain: "test", code: 7, userInfo: [
            NSLocalizedDescriptionKey: "boom"
        ])
        store.loadedManagers = [manager]

        let vpnManager = VPNManager(store: store, configSaver: { _ in })
        vpnManager.setupAndConnect(config: sampleConfig())

        XCTAssertEqual(vpnManager.lastErrorMessage, "Failed to start tunnel: boom")
    }

    func testSetupAndConnectFailsWhenManagerStoreErrors() {
        let store = FakeTunnelProviderStore()
        store.loadError = NSError(domain: "test", code: 3, userInfo: [
            NSLocalizedDescriptionKey: "load failed"
        ])

        let vpnManager = VPNManager(store: store, configSaver: { _ in })
        vpnManager.setupAndConnect(config: sampleConfig())

        XCTAssertEqual(vpnManager.lastErrorMessage, "Failed to load manager: load failed")
    }

    private func sampleConfig() -> VPNConfig {
        VPNConfig(
            mode: "s2n-quic",
            serverAddr: "127.0.0.1:443",
            serverName: "localhost",
            cert: "CERT",
            privKey: "KEY"
        )
    }
}

private final class FakeTunnelProviderStore: TunnelProviderManagingStore {
    var loadedManagers: [TunnelProviderManaging] = []
    var loadError: Error?

    func loadManagers(completion: @escaping (Result<[TunnelProviderManaging], Error>) -> Void) {
        if let loadError {
            completion(.failure(loadError))
        } else {
            completion(.success(loadedManagers))
        }
    }

    func makeManager() -> TunnelProviderManaging {
        FakeTunnelProviderManager()
    }
}

private final class FakeTunnelProviderManager: TunnelProviderManaging {
    var protocolConfiguration: NETunnelProviderProtocol?
    var localizedDescription: String?
    var isEnabled: Bool = false
    var connectionStatus: NEVPNStatus = .disconnected
    var session = FakeTunnelSession()
    var saveError: Error?
    var reloadError: Error?
    var saveCalled = false
    var reloadCalled = false
    var savedConfigurations: [NETunnelProviderProtocol] = []

    var tunnelSession: TunnelSessionStarting? {
        session
    }

    func saveToPreferences(_ completion: @escaping (Error?) -> Void) {
        saveCalled = true
        if let protocolConfiguration {
            savedConfigurations.append(protocolConfiguration)
        }
        completion(saveError)
    }

    func loadFromPreferences(_ completion: @escaping (Error?) -> Void) {
        reloadCalled = true
        completion(reloadError)
    }
}

private final class FakeTunnelSession: TunnelSessionStarting {
    var startCount = 0
    var startError: Error?
    var stopCount = 0

    func startTunnel() throws {
        startCount += 1
        if let startError {
            throw startError
        }
    }

    func stopTunnel() {
        stopCount += 1
    }
}
