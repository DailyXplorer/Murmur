// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "MurmurNative",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(
            name: "Murmur",
            targets: ["MurmurNative"]
        )
    ],
    dependencies: [
        .package(path: "ThirdParty/ArgmaxWhisperKit")
    ],
    targets: [
        .executableTarget(
            name: "MurmurNative",
            dependencies: [
                .product(name: "WhisperKit", package: "ArgmaxWhisperKit")
            ],
            path: "Sources/MurmurNative"
        ),
        .testTarget(
            name: "MurmurNativeTests",
            dependencies: ["MurmurNative"],
            path: "tests/MurmurNativeTests"
        )
    ]
)
