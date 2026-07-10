import Foundation

enum CustomWordCorrectionService {
    static func applyCustomWords(
        to text: String,
        customWords: [String],
        threshold: Double
    ) -> String {
        guard customWords.isEmpty == false else {
            return text
        }

        let normalizedCustomWords = AppSettings.normalizedCustomWordsForImport(customWords)
        guard normalizedCustomWords.isEmpty == false else {
            return text
        }

        let customWordsNoSpace = normalizedCustomWords
            .map { $0.lowercased().replacingOccurrences(of: " ", with: "") }

        return text
            .components(separatedBy: "\n")
            .map {
                applyCustomWordsToLine(
                    $0,
                    normalizedCustomWords: normalizedCustomWords,
                    customWordsNoSpace: customWordsNoSpace,
                    threshold: threshold
                )
            }
            .joined(separator: "\n")
    }

    private static func applyCustomWordsToLine(
        _ line: String,
        normalizedCustomWords: [String],
        customWordsNoSpace: [String],
        threshold: Double
    ) -> String {
        let words = line.split(whereSeparator: \.isWhitespace).map(String.init)
        var result: [String] = []
        var index = 0

        while index < words.count {
            var matched = false

            for ngramLength in stride(from: 3, through: 1, by: -1) {
                guard index + ngramLength <= words.count else {
                    continue
                }

                let ngramWords = Array(words[index..<(index + ngramLength)])
                let candidate = buildNGram(ngramWords)

                guard let replacement = bestMatch(
                    candidate: candidate,
                    customWords: normalizedCustomWords,
                    customWordsNoSpace: customWordsNoSpace,
                    threshold: threshold
                ) else {
                    continue
                }

                let punctuationPrefix = punctuation(in: ngramWords[0]).prefix
                let punctuationSuffix = punctuation(in: ngramWords[ngramLength - 1]).suffix
                let corrected = preserveCasePattern(original: ngramWords[0], replacement: replacement)
                result.append("\(punctuationPrefix)\(corrected)\(punctuationSuffix)")
                index += ngramLength
                matched = true
                break
            }

            if matched == false {
                result.append(words[index])
                index += 1
            }
        }

        return result.joined(separator: " ")
    }

    private static func buildNGram(_ words: [String]) -> String {
        words
            .map {
                $0.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
                    .lowercased()
            }
            .joined()
    }

    private static func bestMatch(
        candidate: String,
        customWords: [String],
        customWordsNoSpace: [String],
        threshold: Double
    ) -> String? {
        guard candidate.isEmpty == false,
              candidate.count <= 50
        else {
            return nil
        }

        var bestWord: String?
        var bestScore = Double.greatestFiniteMagnitude

        for (index, customWord) in customWordsNoSpace.enumerated() {
            let lengthDifference = abs(candidate.count - customWord.count)
            let maxLength = max(candidate.count, customWord.count)
            let maxAllowedDifference = max(Double(maxLength) * 0.25, 2)
            guard Double(lengthDifference) <= maxAllowedDifference else {
                continue
            }

            let distance = levenshteinDistance(candidate, customWord)
            let score = maxLength == 0 ? 1 : Double(distance) / Double(maxLength)

            if score < threshold, score < bestScore {
                bestWord = customWords[index]
                bestScore = score
            }
        }

        return bestWord
    }

    private static func punctuation(in word: String) -> (prefix: String, suffix: String) {
        let characters = Array(word)
        let prefix = characters
            .prefix { $0.isLetter == false && $0.isNumber == false }
        let suffix = characters
            .reversed()
            .prefix { $0.isLetter == false && $0.isNumber == false }
            .reversed()
        return (String(prefix), String(suffix))
    }

    private static func preserveCasePattern(original: String, replacement: String) -> String {
        let letters = original.filter(\.isLetter)

        if letters.isEmpty == false, letters.allSatisfy(\.isUppercase) {
            return replacement.uppercased()
        }

        if original.first?.isUppercase == true {
            guard let first = replacement.first else {
                return replacement
            }
            return first.uppercased() + String(replacement.dropFirst())
        }

        return replacement
    }

    private static func levenshteinDistance(_ lhs: String, _ rhs: String) -> Int {
        let lhs = Array(lhs)
        let rhs = Array(rhs)

        if lhs.isEmpty { return rhs.count }
        if rhs.isEmpty { return lhs.count }

        var previous = Array(0...rhs.count)
        var current = Array(repeating: 0, count: rhs.count + 1)

        for lhsIndex in 1...lhs.count {
            current[0] = lhsIndex

            for rhsIndex in 1...rhs.count {
                if lhs[lhsIndex - 1] == rhs[rhsIndex - 1] {
                    current[rhsIndex] = previous[rhsIndex - 1]
                } else {
                    current[rhsIndex] = min(
                        previous[rhsIndex] + 1,
                        current[rhsIndex - 1] + 1,
                        previous[rhsIndex - 1] + 1
                    )
                }
            }

            previous = current
        }

        return previous[rhs.count]
    }
}
