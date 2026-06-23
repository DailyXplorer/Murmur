import Foundation

enum AppVersionProvider {
    static let fallbackVersion = "0.1.0"

    static func currentBundleVersion(bundle: Bundle = .main) -> String {
        bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? fallbackVersion
    }
}
