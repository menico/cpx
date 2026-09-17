// The settings window.

import AppKit
import CpxKit
import SwiftUI

@MainActor
final class SettingsWindowController {
    private let window: NSWindow

    init(state: AppState) {
        let hosting = NSHostingController(rootView: SettingsView(state: state))
        window = NSWindow(contentViewController: hosting)
        window.title = "cpx Settings"
        window.styleMask = [.titled, .closable, .miniaturizable]
        window.setContentSize(NSSize(width: 460, height: 520))
        window.isReleasedWhenClosed = false
        window.center()
    }

    func show() {
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }
}

struct SettingsView: View {
    @Bindable var state: AppState

    var body: some View {
        @Bindable var settings = state.settings
        Form {
            Section("Refresh") {
                Picker("Check usage", selection: $settings.cadence) {
                    ForEach(RefreshCadence.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                .onChange(of: settings.cadence) { state.scheduleNextRefresh() }
                if let next = state.nextRefreshAt {
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        Text("Next check \(Format.countdown(to: next, from: context.date))").font(.caption).foregroundStyle(.secondary)
                    }
                }
                Toggle("Check provider status pages", isOn: $settings.statusChecks)
                Toggle("Estimate spend from local transcripts", isOn: $settings.showCost)
                if settings.showCost {
                    Stepper("Spend history: \(settings.costDays) days", value: $settings.costDays, in: 7...365, step: 1)
                }
                Toggle("Launch at login", isOn: Binding(get: { settings.launchAtLogin }, set: { settings.launchAtLogin = $0 }))
                    .disabled(Bundle.main.bundleIdentifier == nil)
            }

            Section("Menu bar") {
                Picker("Icons", selection: $settings.iconMode) {
                    ForEach(IconMode.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                .onChange(of: settings.iconMode) { state.onChange?() }
                if settings.iconMode == .merged {
                    Picker("Show", selection: Binding(
                        get: { settings.pinnedAccount ?? "" },
                        set: { settings.pinnedAccount = $0.isEmpty ? nil : $0; state.onChange?() }
                    )) {
                        Text("Busiest account").tag("")
                        ForEach(state.visibleAccounts) { Text($0.name).tag($0.id) }
                    }
                }
                Picker("Percentage from", selection: $settings.metric) {
                    ForEach(MenuBarMetric.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                .onChange(of: settings.metric) { state.onChange?() }
                Toggle("Show the percentage next to the icon", isOn: $settings.showPercentInMenuBar)
                    .onChange(of: settings.showPercentInMenuBar) { state.onChange?() }
                Toggle("Show usage as used rather than remaining", isOn: $settings.showUsedNotRemaining)
                    .onChange(of: settings.showUsedNotRemaining) { state.onChange?() }
                VStack(alignment: .leading, spacing: 4) {
                    Text("Accounts in the menu bar").font(.callout)
                    ForEach(state.accounts) { account in
                        Toggle(isOn: Binding(
                            get: { !settings.hiddenAccounts.contains(account.id) },
                            set: { shown in
                                if shown { settings.hiddenAccounts.remove(account.id) } else { settings.hiddenAccounts.insert(account.id) }
                                state.onChange?()
                            })) {
                            HStack(spacing: 6) {
                                Text(account.name)
                                ProviderTag(provider: account.provider)
                            }
                        }
                    }
                }
            }

            Section("Notifications") {
                Toggle("Warn when a quota runs low", isOn: $settings.notificationsEnabled)
                    .onChange(of: settings.notificationsEnabled) { if $1 { state.notifier.requestAuthorisation() } }
                HStack {
                    Text("Warn at")
                    Spacer()
                    ForEach([90, 75, 50, 25, 20, 10], id: \.self) { threshold in
                        Toggle("\(threshold)%", isOn: Binding(
                            get: { settings.thresholds.contains(threshold) },
                            set: { on in
                                var set = Set(settings.thresholds)
                                if on { set.insert(threshold) } else { set.remove(threshold) }
                                settings.thresholds = QuotaThresholds.sanitized(Array(set))
                            }))
                            .toggleStyle(.button)
                            .controlSize(.small)
                    }
                }
                .disabled(!settings.notificationsEnabled)
                Text("Thresholds are percent remaining. Each fires once on the way down and re-arms when the window recovers.")
                    .font(.caption).foregroundStyle(.secondary)
                Toggle("Say when a session runs out, and when it is back", isOn: $settings.sessionAlerts)
                    .disabled(!settings.notificationsEnabled)
                Toggle("Say when a weekly limit resets", isOn: $settings.weeklyResetAlerts)
                    .disabled(!settings.notificationsEnabled)
            }

            Section("Claude logins") {
                Toggle("Keep a private copy of each Claude login", isOn: $settings.keepLogins)
                    .onChange(of: settings.keepLogins) { _, on in
                        if on {
                            // Copy each login now, so the questions come once, together.
                            Task { await state.refreshUsage(manual: true) }
                        } else {
                            Task { try? await state.cli.forgetLogins() }
                        }
                    }
                Text("With a copy, the Keychain is read once per profile and cpx renews the copy itself, the way Claude Code renews its own; macOS is not asked again. The copy lives under ~/.claude-profiles/cache, readable only by you — the same footing as Codex's own login file. Turning this off deletes the copies.")
                    .font(.caption).foregroundStyle(.secondary)
                Picker("Ask to read a Claude login", selection: $settings.keychainPrompts) {
                    ForEach(KeychainPromptPolicy.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                Text("cpx reads a login only to ask its provider for usage. Background refreshes never ask; an account that needs permission shows an Allow button. With copies kept, a question comes at most once per profile.")
                    .font(.caption).foregroundStyle(.secondary)
            }

            Section("About") {
                LabeledContent("Command", value: state.cli.executable.path)
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
    }
}
