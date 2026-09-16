// Things that belong to the desktop rather than the command.

import AppKit
import Foundation

enum Desktop {
    /// Run a profile's own command in Terminal, so signing in happens where
    /// the browser hand-off and the result can be seen.
    static func openInTerminal(_ command: String) {
        let script = "tell application \"Terminal\"\n  activate\n  do script \(appleScriptQuote(command))\nend tell"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script]
        try? process.run()
    }

    static func shellQuote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    static func appleScriptQuote(_ value: String) -> String {
        "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }

    static func reveal(_ path: String) {
        NSWorkspace.shared.selectFile(path, inFileViewerRootedAtPath: "")
    }

    static func open(_ path: String) {
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    static func open(_ url: URL) {
        NSWorkspace.shared.open(url)
    }

    /// Ask for a directory. Returns nil when cancelled.
    @MainActor
    static func pickDirectory(title: String) -> String? {
        let panel = NSOpenPanel()
        panel.title = title
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        NSApp.activate(ignoringOtherApps: true)
        return panel.runModal() == .OK ? panel.url?.path : nil
    }
}
