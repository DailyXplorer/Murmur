import Foundation

extension FileHandle {
    func writeLine(_ line: String) {
        guard let data = "\(line)\n".data(using: .utf8) else {
            return
        }
        write(data)
    }
}
