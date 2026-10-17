// swift-tools-version: 6.2
import PackageDescription

// On-device analysis for Murmur meetings, run as a separate process: it needs
// macOS 14, and a crash in Core ML must not take the app down.
let package = Package(
    name: "MurmurMeetingsHelper",
    platforms: [.macOS(.v14)],
    dependencies: [
        // No traits: the NeMo text-normalization engine is not needed and
        // would bring a second Rust runtime.
        .package(
            url: "https://github.com/FluidInference/FluidAudio", exact: "0.17.7", traits: [])
    ],
    targets: [
        .executableTarget(
            name: "murmur-meetings-helper",
            dependencies: [.product(name: "FluidAudio", package: "FluidAudio")],
            path: "Sources/Helper"
        )
    ]
)
