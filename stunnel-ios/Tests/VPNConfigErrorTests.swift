import XCTest
@testable import stunnel_ios

final class VPNConfigErrorTests: XCTestCase {
    func testErrorDescriptionsAreUserReadable() {
        XCTAssertEqual(
            VPNConfigError.sharedDefaultsUnavailable.errorDescription,
            "Shared App Group defaults are unavailable"
        )
        XCTAssertEqual(
            VPNConfigError.sharedContainerUnavailable.errorDescription,
            "Shared App Group container is unavailable"
        )
        XCTAssertEqual(
            VPNConfigError.runtimeJSONEncodingFailed.errorDescription,
            "Failed to encode runtime stunnel configuration"
        )
    }
}
