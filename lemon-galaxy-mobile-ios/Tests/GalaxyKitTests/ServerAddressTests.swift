import XCTest
@testable import GalaxyKit

final class ServerAddressTests: XCTestCase {
    let fp = "sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8"

    func testParsesHostPortFingerprint() {
        let a = ServerAddress.parse("127.0.0.1:17443#\(fp)")
        XCTAssertEqual(a?.host, "127.0.0.1")
        XCTAssertEqual(a?.port, 17443)
        XCTAssertEqual(a?.fingerprint, fp)
        XCTAssertEqual(a?.text, "127.0.0.1:17443#\(fp)")
        XCTAssertEqual(ServerAddress.parse("  galaxy.example.com:7443#\(fp)\n")?.host, "galaxy.example.com")
    }

    func testIPv6NeedsBrackets() {
        let a = ServerAddress.parse("[240e:3b0::1]:7443#\(fp)")
        XCTAssertEqual(a?.host, "240e:3b0::1")
        XCTAssertEqual(a?.hostPort, "[240e:3b0::1]:7443")
        XCTAssertNil(ServerAddress.parse("240e:3b0::1:7443#\(fp)"))
        XCTAssertNil(ServerAddress.parse("[galaxy]:7443#\(fp)"))
    }

    func testRejectsBadInput() {
        XCTAssertNil(ServerAddress.parse("galaxy.example.com:7443"))
        XCTAssertNil(ServerAddress.parse("galaxy.example.com#\(fp)"))
        XCTAssertNil(ServerAddress.parse("galaxy.example.com:0#\(fp)"))
        XCTAssertNil(ServerAddress.parse("galaxy.example.com:99999#\(fp)"))
        XCTAssertNil(ServerAddress.parse("galaxy.example.com:7443#md5-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8"))
        XCTAssertNil(ServerAddress.parse("galaxy.example.com:7443#sha256-short"))
        XCTAssertNil(ServerAddress.parse("https://galaxy.example.com:7443#\(fp)"))
        XCTAssertNil(ServerAddress.parse("-bad.example.com:7443#\(fp)"))
        XCTAssertNil(ServerAddress.parse("host:７４４３#\(fp)"))
    }

    func testScannedPayloads() {
        XCTAssertEqual(ServerAddress.parseScanned("10.0.0.5:7443#\(fp)"), "10.0.0.5:7443#\(fp)")
        let encoded = "10.0.0.5:7443%23\(fp)"
        XCTAssertEqual(ServerAddress.parseScanned("lemongalaxy://login?server=\(encoded)"), "10.0.0.5:7443#\(fp)")
        XCTAssertNil(ServerAddress.parseScanned("https://example.com/?server=\(encoded)"))
        XCTAssertNil(ServerAddress.parseScanned("hello"))
    }
}
