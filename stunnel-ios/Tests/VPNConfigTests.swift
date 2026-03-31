import XCTest
@testable import stunnel_ios

final class VPNConfigTests: XCTestCase {
    private let suiteName = "group.me.airtrack.stunnel.tests"

    override func tearDownWithError() throws {
        UserDefaults(suiteName: suiteName)?.removePersistentDomain(forName: suiteName)
        try? FileManager.default.removeItem(at: makeTempDirectory())
    }

    func testSaveAndLoadRoundTripsConfiguration() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        let config = sampleConfig()

        try config.save(defaults: defaults)
        let loaded = try XCTUnwrap(VPNConfig.load(defaults: defaults))

        XCTAssertEqual(loaded.mode, config.mode)
        XCTAssertEqual(loaded.serverAddr, config.serverAddr)
        XCTAssertEqual(loaded.serverName, config.serverName)
        XCTAssertEqual(loaded.cert, config.cert)
        XCTAssertEqual(loaded.privKey, config.privKey)
    }

    func testRuntimeConfigWritesCertificateFilesAndReturnsTheirPaths() throws {
        let config = sampleConfig()
        let directory = makeTempDirectory()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let json = try config.makeRuntimeConfigJSON(containerURL: directory)
        let payload = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: String]
        )

        let certPath = try XCTUnwrap(payload["cert"])
        let keyPath = try XCTUnwrap(payload["priv_key"])

        XCTAssertEqual(try String(contentsOfFile: certPath, encoding: .utf8), config.cert)
        XCTAssertEqual(try String(contentsOfFile: keyPath, encoding: .utf8), config.privKey)
        XCTAssertEqual(payload["mode"], config.mode)
        XCTAssertEqual(payload["server_addr"], config.serverAddr)
        XCTAssertEqual(payload["server_name"], config.serverName)
    }

    func testRuntimeConfigFailsWithoutContainerDirectory() {
        let config = sampleConfig()

        XCTAssertThrowsError(try config.makeRuntimeConfigJSON(containerURL: nil)) { error in
            guard case VPNConfigError.sharedContainerUnavailable = error else {
                return XCTFail("Expected sharedContainerUnavailable, got \(error)")
            }
        }
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

    private func makeTempDirectory() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent(
            "stunnel-ios-tests",
            isDirectory: true
        )
    }
}
