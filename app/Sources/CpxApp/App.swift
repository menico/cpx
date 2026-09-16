// The menu bar app. Presentation only: every decision comes from `cpx`.

import AppKit
import CpxKit
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    var state: AppState!
    var statusItems: StatusItemController!
    var settingsWindow: SettingsWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        // A Finder-launched app has a stub PATH; the command needs the real one
        // to find `claude`, `codex` and `direnv`.
        var environment = ProcessInfo.processInfo.environment
        if let path = CpxCLI.loginPath() {
            let current = environment["PATH"] ?? ""
            var seen = Set<String>()
            let merged = (path.split(separator: ":") + current.split(separator: ":"))
                .map(String.init)
                .filter { !$0.isEmpty && seen.insert($0).inserted }
            environment["PATH"] = merged.joined(separator: ":")
        }
        // Never let the app's own environment steer a profile.
        for key in ["CLAUDE_CONFIG_DIR", "CLAUDE_PROFILE", "CODEX_HOME", "CODEX_PROFILE"] {
            environment.removeValue(forKey: key)
        }

        guard let executable = CpxCLI.locate(bundleExecutable: Bundle.main.executableURL, environment: environment) else {
            let alert = NSAlert()
            alert.messageText = "The cpx command was not found"
            alert.informativeText = "The app drives the `cpx` command and needs it next to itself in the bundle, or on PATH. Install it with `brew install cpx` or `make install`."
            alert.runModal()
            NSApp.terminate(nil)
            return
        }

        let cli = CpxCLI(executable: executable, environment: environment)
        let settings = Settings()
        let notifier = Notifier()
        state = AppState(cli: cli, settings: settings, notifier: notifier)
        statusItems = StatusItemController(state: state, delegate: self)
        state.onChange = { [weak self] in self?.statusItems.update() }

        Task {
            await state.reloadConfig()
            if let paths = try? await cli.paths() {
                state.watchConfig(at: paths.config)
            }
            // A launch is nobody clicking anything: never a Keychain dialog.
            await state.refreshUsage(manual: false)
            if settings.notificationsEnabled {
                notifier.requestAuthorisation()
            }
            // Development hooks: open the popover on launch, optionally on
            // one account, since a status item cannot be clicked from a script.
            if let account = ProcessInfo.processInfo.environment["CPX_DEV_SHOW"] {
                statusItems.showPopover(account: account.isEmpty || account == "1" ? nil : account)
            }
        }
    }

    func showSettings() {
        if settingsWindow == nil {
            settingsWindow = SettingsWindowController(state: state)
        }
        settingsWindow?.show()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }
}

@main
struct CpxMain {
    /// The delegate must outlive `run()`; NSApplication does not retain it.
    @MainActor static var delegate: AppDelegate?

    static func main() {
        MainActor.assumeIsolated {
            let delegate = AppDelegate()
            Self.delegate = delegate
            let app = NSApplication.shared
            app.delegate = delegate
            app.run()
        }
    }
}
