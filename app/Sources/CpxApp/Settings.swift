// What the app remembers about how you like it.
//
// Everything here is app-local preference: it says nothing about profiles,
// which live in config.toml and belong to the command.

import CpxKit
import Foundation
import Observation
import ServiceManagement

enum IconMode: String, CaseIterable, Codable {
    /// One status item showing the chosen account.
    case merged
    /// One status item per account.
    case perAccount

    var label: String {
        switch self {
        case .merged: "One icon"
        case .perAccount: "One icon per account"
        }
    }
}

enum MenuBarMetric: String, CaseIterable, Codable {
    case auto
    case session
    case weekly

    var label: String {
        switch self {
        case .auto: "Whichever is closer to its limit"
        case .session: "Session"
        case .weekly: "Weekly"
        }
    }
}

/// When cpx may put a Keychain dialog on the screen. macOS asks once per
/// Claude profile, and again whenever Claude Code rewrites its item.
enum KeychainPromptPolicy: String, CaseIterable, Codable {
    case manualOnly
    case never
    case always

    var label: String {
        switch self {
        case .manualOnly: "Only when I refresh by hand"
        case .never: "Never"
        case .always: "Whenever needed, even in the background"
        }
    }
}

@MainActor
@Observable
final class Settings {
    private let defaults: UserDefaults

    var cadence: RefreshCadence { didSet { save("cadence", cadence.rawValue) } }
    var iconMode: IconMode { didSet { save("iconMode", iconMode.rawValue) } }
    var metric: MenuBarMetric { didSet { save("metric", metric.rawValue) } }
    var showPercentInMenuBar: Bool { didSet { save("showPercent", showPercentInMenuBar) } }
    var showUsedNotRemaining: Bool { didSet { save("showUsed", showUsedNotRemaining) } }
    var showCost: Bool { didSet { save("showCost", showCost) } }
    var statusChecks: Bool { didSet { save("statusChecks", statusChecks) } }
    var notificationsEnabled: Bool { didSet { save("notifications", notificationsEnabled) } }
    var sessionAlerts: Bool { didSet { save("sessionAlerts", sessionAlerts) } }
    var weeklyResetAlerts: Bool { didSet { save("weeklyResetAlerts", weeklyResetAlerts) } }
    var thresholds: [Int] { didSet { save("thresholds", thresholds) } }
    /// The account the merged icon shows; nil means the busiest one.
    var pinnedAccount: String? { didSet { save("pinnedAccount", pinnedAccount) } }
    var hiddenAccounts: Set<String> { didSet { save("hiddenAccounts", Array(hiddenAccounts)) } }
    var costDays: Int { didSet { save("costDays", costDays) } }
    var keychainPrompts: KeychainPromptPolicy { didSet { save("keychainPrompts", keychainPrompts.rawValue) } }
    /// Keep an owner-only copy of each Claude login under the cpx root, so
    /// the Keychain is read once per profile and renewed by cpx.
    var keepLogins: Bool { didSet { save("keepLogins", keepLogins) } }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        cadence = RefreshCadence(rawValue: defaults.string(forKey: "cadence") ?? "") ?? .adaptive
        iconMode = IconMode(rawValue: defaults.string(forKey: "iconMode") ?? "") ?? .merged
        metric = MenuBarMetric(rawValue: defaults.string(forKey: "metric") ?? "") ?? .auto
        showPercentInMenuBar = defaults.object(forKey: "showPercent") as? Bool ?? true
        showUsedNotRemaining = defaults.bool(forKey: "showUsed")
        showCost = defaults.object(forKey: "showCost") as? Bool ?? true
        statusChecks = defaults.object(forKey: "statusChecks") as? Bool ?? true
        notificationsEnabled = defaults.object(forKey: "notifications") as? Bool ?? true
        sessionAlerts = defaults.object(forKey: "sessionAlerts") as? Bool ?? true
        weeklyResetAlerts = defaults.object(forKey: "weeklyResetAlerts") as? Bool ?? false
        thresholds = QuotaThresholds.sanitized(defaults.array(forKey: "thresholds") as? [Int] ?? [])
        pinnedAccount = defaults.string(forKey: "pinnedAccount")
        hiddenAccounts = Set(defaults.stringArray(forKey: "hiddenAccounts") ?? [])
        costDays = max(7, defaults.object(forKey: "costDays") as? Int ?? 30)
        keychainPrompts = KeychainPromptPolicy(rawValue: defaults.string(forKey: "keychainPrompts") ?? "") ?? .manualOnly
        keepLogins = defaults.bool(forKey: "keepLogins")
    }

    private func save(_ key: String, _ value: Any?) {
        if let value {
            defaults.set(value, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }

    // Launch at login goes through the system, not UserDefaults, so it is
    // read back rather than remembered.
    var launchAtLogin: Bool {
        get { Bundle.main.bundleIdentifier != nil && SMAppService.mainApp.status == .enabled }
        set {
            guard Bundle.main.bundleIdentifier != nil else { return }
            do {
                if newValue {
                    try SMAppService.mainApp.register()
                } else {
                    try SMAppService.mainApp.unregister()
                }
            } catch {
                NSLog("launch at login: \(error)")
            }
        }
    }
}
