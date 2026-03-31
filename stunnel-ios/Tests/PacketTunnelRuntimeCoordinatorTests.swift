import XCTest
@testable import stunnel_ios

final class PacketTunnelRuntimeCoordinatorTests: XCTestCase {
    func testMakeRuntimeLoadsConfigAndStartsCore() throws {
        let loader = FakeVPNConfigLoader(config: sampleConfig())
        let core = FakePacketTunnelCoreManager(createHandle: UnsafeMutableRawPointer(bitPattern: 0x1))
        let coordinator = PacketTunnelRuntimeCoordinator(
            configLoader: loader,
            coreManager: core,
            runtimeConfigBuilder: { _ in "{\"mode\":\"s2n-quic\"}" }
        )

        let runtime = try coordinator.makeRuntime(packetContext: nil, callback: testPacketCallback)

        XCTAssertEqual(runtime.config.mode, "s2n-quic")
        XCTAssertEqual(core.startCalls, 1)
        XCTAssertEqual(core.callbackRegistrations, 1)
        XCTAssertFalse(core.didClearCallback)
        XCTAssertFalse(core.didStop)
    }

    func testMakeRuntimeStopsCoreWhenStartFails() {
        let loader = FakeVPNConfigLoader(config: sampleConfig())
        let core = FakePacketTunnelCoreManager(
            createHandle: UnsafeMutableRawPointer(bitPattern: 0x2),
            startResult: false
        )
        let coordinator = PacketTunnelRuntimeCoordinator(
            configLoader: loader,
            coreManager: core,
            runtimeConfigBuilder: { _ in "{\"mode\":\"s2n-quic\"}" }
        )

        XCTAssertThrowsError(try coordinator.makeRuntime(packetContext: nil, callback: testPacketCallback)) { error in
            guard case PacketTunnelRuntimeError.coreStartFailed = error else {
                return XCTFail("Expected coreStartFailed, got \(error)")
            }
        }

        XCTAssertTrue(core.didClearCallback)
        XCTAssertTrue(core.didStop)
    }

    func testMakeRuntimeFailsWhenConfigIsMissing() {
        let loader = FakeVPNConfigLoader(config: nil)
        let core = FakePacketTunnelCoreManager(createHandle: UnsafeMutableRawPointer(bitPattern: 0x3))
        let coordinator = PacketTunnelRuntimeCoordinator(
            configLoader: loader,
            coreManager: core,
            runtimeConfigBuilder: { _ in "{\"mode\":\"s2n-quic\"}" }
        )

        XCTAssertThrowsError(try coordinator.makeRuntime(packetContext: nil, callback: testPacketCallback)) { error in
            guard case PacketTunnelRuntimeError.missingConfig = error else {
                return XCTFail("Expected missingConfig, got \(error)")
            }
        }

        XCTAssertEqual(core.startCalls, 0)
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

private struct FakeVPNConfigLoader: VPNConfigLoading {
    let config: VPNConfig?

    func load() throws -> VPNConfig? {
        config
    }
}

private final class FakePacketTunnelCoreManager: PacketTunnelCoreManaging {
    let createHandle: UnsafeMutableRawPointer?
    let startResult: Bool
    var startCalls = 0
    var callbackRegistrations = 0
    var didClearCallback = false
    var didStop = false

    init(createHandle: UnsafeMutableRawPointer?, startResult: Bool = true) {
        self.createHandle = createHandle
        self.startResult = startResult
    }

    func create(configJson: String) -> UnsafeMutableRawPointer? {
        createHandle
    }

    func setPacketCallback(
        handle: UnsafeMutableRawPointer,
        context: UnsafeMutableRawPointer?,
        callback: StunnelPacketCallback
    ) {
        callbackRegistrations += 1
    }

    func start(handle: UnsafeMutableRawPointer) -> Bool {
        startCalls += 1
        return startResult
    }

    func clearPacketCallback(handle: UnsafeMutableRawPointer) {
        didClearCallback = true
    }

    func stop(handle: UnsafeMutableRawPointer) {
        didStop = true
    }
}

private let testPacketCallback: StunnelPacketCallback = { _, _, _ in }
