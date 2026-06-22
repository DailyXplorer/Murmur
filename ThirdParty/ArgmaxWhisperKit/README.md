# Argmax WhisperKit Vendor Package

This package vendors the `ArgmaxCore` and `WhisperKit` targets from `argmaxinc/argmax-oss-swift` 1.0.0.

Handy uses this local package instead of the full upstream SwiftPM manifest because the upstream package also declares CLI targets that pull `swift-argument-parser` plugins. On the current macOS/Xcode setup, those plugins can hang during debug-symbol generation even when Handy only depends on the `WhisperKit` library product.

Keep `LICENSE` and `NOTICES` with the vendored source when updating.
