import Foundation

enum AppSection: String, CaseIterable, Identifiable {
    case general
    case models
    case advanced
    case history
    case postProcessing
    case debug

    var id: String { rawValue }

    static func visibleSections(settings: AppSettings) -> [AppSection] {
        allCases.filter { section in
            switch section {
            case .postProcessing:
                settings.postProcessEnabled
            case .debug:
                settings.debugMode
            default:
                true
            }
        }
    }

    var title: String {
        switch self {
        case .general: "General"
        case .models: "Models"
        case .advanced: "Advanced"
        case .history: "History"
        case .postProcessing: "Post Process"
        case .debug: "Debug"
        }
    }

}
