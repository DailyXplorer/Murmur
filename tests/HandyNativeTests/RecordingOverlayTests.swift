import XCTest
@testable import HandyNative

final class RecordingOverlayTests: XCTestCase {
    func testLevelMapperProducesNineClampedBars() {
        let levels = RecordingOverlayLevelMapper.levels(from: 2)

        XCTAssertEqual(levels.count, 9)
        XCTAssertTrue(levels.allSatisfy { $0 >= 0 && $0 <= 1 })
        XCTAssertGreaterThan(levels.max() ?? 0, 0)
    }

    func testLevelMapperKeepsSilenceAtZero() {
        XCTAssertEqual(RecordingOverlayLevelMapper.levels(from: 0), Array(repeating: 0, count: 9))
    }
}
