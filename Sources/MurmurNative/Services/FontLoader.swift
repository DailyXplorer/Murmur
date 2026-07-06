import CoreText
import Foundation

enum FontLoader {
    static func registerBundledFonts() {
        [
            "dm-sans-latin-wght-normal",
            "dm-sans-latin-wght-italic",
            "dm-sans-latin-ext-wght-normal",
            "dm-sans-latin-ext-wght-italic",
        ].forEach { resourceName in
            guard let url = Bundle.main.url(forResource: resourceName, withExtension: "woff2") else {
                return
            }
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }
}
