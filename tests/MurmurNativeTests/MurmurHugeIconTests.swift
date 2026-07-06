@testable import MurmurNative
import XCTest

final class MurmurHugeIconTests: XCTestCase {
    func testEverySharedHugeIconHasRenderablePaths() {
        for kind in MurmurHugeIconKind.allCases {
            let paths = kind.paths

            XCTAssertEqual(paths.count, kind.pathData.count)
            XCTAssertTrue(paths.allSatisfy { !$0.isEmpty }, "\(kind) should render at least one path segment")
        }
    }

    func testSharedHugeIconsCoverCurrentSettingAndPermissionIcons() {
        XCTAssertEqual(
            MurmurHugeIconKind.allCases,
            [
                .informationCircle,
                .mic,
                .keyboard,
                .check,
                .loading,
                .folderOpen,
                .copy,
                .delete,
                .play,
                .pause,
                .rotateLeft,
                .star,
                .alertCircle,
                .alertTriangle,
                .cancelCircle,
                .checkCircle,
                .chevronDown,
                .download,
                .globe,
                .hardDrive,
                .languages,
                .refresh,
            ]
        )
    }

    func testSharedHugeIconsCoverCurrentHistoryIcons() {
        XCTAssertEqual(
            [
                MurmurHugeIconKind.folderOpen,
                .copy,
                .check,
                .star,
                .rotateLeft,
                .delete,
                .play,
                .pause,
            ].map(\.pathData.isEmpty),
            Array(repeating: false, count: 8)
        )
    }

    func testSharedHugeIconsCoverCurrentModelPostProcessAndAdvancedIcons() {
        XCTAssertEqual(
            [
                MurmurHugeIconKind.alertCircle,
                .alertTriangle,
                .cancelCircle,
                .check,
                .checkCircle,
                .chevronDown,
                .download,
                .globe,
                .hardDrive,
                .languages,
                .loading,
                .play,
                .refresh,
                .delete,
            ].map(\.pathData.isEmpty),
            Array(repeating: false, count: 14)
        )
    }
}
