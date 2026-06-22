// swift-tools-version: 5.10

import PackageDescription

let package = Package(
    name: "ArgmaxWhisperKit",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
        .watchOS(.v10),
        .visionOS(.v1)
    ],
    products: [
        .library(
            name: "WhisperKit",
            targets: ["WhisperKit"]
        )
    ],
    targets: [
        .target(
            name: "ArgmaxCore",
            swiftSettings: swiftSettings()
        ),
        .target(
            name: "WhisperKit",
            dependencies: ["ArgmaxCore"],
            swiftSettings: swiftSettings()
        )
    ],
    swiftLanguageVersions: [.v5]
)

func swiftSettings() -> [SwiftSetting] {
    [.enableExperimentalFeature("StrictConcurrency")]
}
