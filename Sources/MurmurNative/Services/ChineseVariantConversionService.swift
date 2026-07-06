import Foundation

enum ChineseVariantConversionService {
    static func convertedText(_ text: String, selectedLanguage: String) -> String? {
        let transform: StringTransform
        switch selectedLanguage {
        case "zh-Hans":
            transform = StringTransform(rawValue: "Hant-Hans")
        case "zh-Hant":
            transform = StringTransform(rawValue: "Hans-Hant")
        default:
            return nil
        }

        return text.applyingTransform(transform, reverse: false)
    }
}
