import Foundation

enum TranscriptionOutputFilterService {
    static func filter(
        _ text: String,
        appLanguage: String,
        customFillerWords: [String]?
    ) -> String {
        let fillerWords = customFillerWords ?? defaultFillerWords(for: appLanguage)
        var filtered = text

        for word in fillerWords where word.isEmpty == false {
            filtered = removingFillerWord(word, from: filtered)
        }

        filtered = collapseRepeatedStutters(in: filtered)
        filtered = collapseWhitespace(in: filtered)
        return filtered.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func defaultFillerWords(for language: String) -> [String] {
        let baseLanguage = language
            .split { $0 == "-" || $0 == "_" }
            .first
            .map(String.init) ?? language

        switch baseLanguage {
        case "en":
            return ["uh", "um", "uhm", "umm", "uhh", "uhhh", "ah", "hmm", "hm", "mmm", "mm", "mh", "eh", "ehh", "ha"]
        case "es":
            return ["ehm", "mmm", "hmm", "hm"]
        case "pt":
            return ["ahm", "hmm", "mmm", "hm"]
        case "fr":
            return ["euh", "hmm", "hm", "mmm"]
        case "de":
            return ["äh", "ähm", "hmm", "hm", "mmm"]
        case "it", "cs":
            return ["ehm", "hmm", "mmm", "hm"]
        case "pl", "tr", "vi":
            return ["hmm", "mmm", "hm"]
        case "ru", "uk":
            return ["хм", "ммм", "hmm", "mmm"]
        case "ar", "ja", "ko", "zh":
            return ["hmm", "mmm"]
        default:
            return ["uh", "uhm", "umm", "uhh", "uhhh", "ah", "hmm", "hm", "mmm", "mm", "mh", "ehh"]
        }
    }

    private static func removingFillerWord(_ word: String, from text: String) -> String {
        let escaped = NSRegularExpression.escapedPattern(for: word)
        let pattern = #"(?i)\b"# + escaped + #"\b[,.]?"#

        guard let regex = try? NSRegularExpression(pattern: pattern) else {
            return text
        }

        let range = NSRange(text.startIndex..<text.endIndex, in: text)
        return regex.stringByReplacingMatches(
            in: text,
            range: range,
            withTemplate: ""
        )
    }

    private static func collapseRepeatedStutters(in text: String) -> String {
        let words = text.split(whereSeparator: \.isWhitespace).map(String.init)
        guard words.isEmpty == false else {
            return text
        }

        var result: [String] = []
        var index = 0

        while index < words.count {
            let word = words[index]
            let lowercaseWord = word.lowercased()

            guard isAlphabetic(word) else {
                result.append(word)
                index += 1
                continue
            }

            var count = 1
            while index + count < words.count,
                  words[index + count].lowercased() == lowercaseWord {
                count += 1
            }

            result.append(word)
            index += max(count >= 3 ? count : 1, 1)
        }

        return result.joined(separator: " ")
    }

    private static func isAlphabetic(_ word: String) -> Bool {
        word.unicodeScalars.allSatisfy { CharacterSet.letters.contains($0) }
    }

    private static func collapseWhitespace(in text: String) -> String {
        guard let regex = try? NSRegularExpression(pattern: #"\s{2,}"#) else {
            return text
        }
        let range = NSRange(text.startIndex..<text.endIndex, in: text)
        return regex.stringByReplacingMatches(
            in: text,
            range: range,
            withTemplate: " "
        )
    }
}
