// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "HandyNative",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(
            name: "Handy",
            targets: ["HandyNative"]
        )
    ],
    dependencies: [
        .package(path: "ThirdParty/ArgmaxWhisperKit")
    ],
    targets: [
        .executableTarget(
            name: "HandyNative",
            dependencies: [
                .product(name: "WhisperKit", package: "ArgmaxWhisperKit")
            ],
            path: "Sources/HandyNative"
        ),
        .testTarget(
            name: "HandyNativeTests",
            dependencies: ["HandyNative"],
            path: "tests/HandyNativeTests"
        )
    ]
)
