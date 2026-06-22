@testable import HandyNative
import XCTest

final class SidebarHugeIconTests: XCTestCase {
    func testSidebarSectionIconsMatchCurrentHugeicons() {
        XCTAssertEqual(
            AppSection.allCases.map { SidebarHugeIconKind(section: $0) },
            [.hand, .cpu, .cog, .history, .sparkles, .flaskConical]
        )
    }

    func testEverySidebarHugeIconHasRenderablePaths() {
        for kind in SidebarHugeIconKind.allCases {
            let paths = kind.paths

            XCTAssertEqual(paths.count, kind.pathData.count)
            XCTAssertTrue(paths.allSatisfy { !$0.isEmpty }, "\(kind) should render at least one path segment")
        }
    }

    func testPathParserHandlesPackedMoveLineAndCurveCommands() {
        let path = HugeIconPathParser.parse("M11.995 4V2M13 9L9 13C8 14 7 15 6 16Z")

        XCTAssertFalse(path.isEmpty)
    }
}
