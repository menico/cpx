// The popover: accounts, directories, health, and the pending plan.

import CpxKit
import SwiftUI

struct MainView: View {
    @Bindable var state: AppState
    @Bindable var navigation: Navigation
    var openSettings: () -> Void

    @State private var prompt: Prompt?
    @State private var showPlan = false

    var body: some View {
        Group {
            if state.initialised == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if state.initialised == false {
                welcome
            } else {
                switch navigation.screen {
                case .main:
                    main
                case .account(let id):
                    if let account = state.account(id) {
                        AccountDetailView(state: state, navigation: navigation, account: account, prompt: $prompt)
                    } else {
                        main.onAppear { navigation.screen = .main }
                    }
                case .script(let profile):
                    ScriptEditorView(state: state, profile: profile) { navigation.screen = profile.map { .account($0) } ?? .main }
                case .skills(let profile):
                    SkillsView(state: state, profile: profile) { navigation.screen = .account(profile) }
                }
            }
        }
        .frame(width: 400, height: 560)
        .sheet(item: $prompt) { prompt in
            PromptSheet(prompt: prompt) { self.prompt = nil }
        }
    }

    private var welcome: some View {
        VStack {
            header
            EmptyState(
                title: "cpx",
                detail: "Run several Claude Code and Codex accounts side by side. Each gets its own login, its own command, and can be bound to a directory.",
                action: { Task { await state.initialise() } },
                actionTitle: "Create configuration")
            if let error = state.configError {
                ErrorBanner(text: error).padding(.horizontal, 12)
            }
            Spacer()
        }
    }

    private var main: some View {
        VStack(spacing: 0) {
            header
            Picker("", selection: $navigation.tab) {
                Text("Accounts \(state.visibleAccounts.count)").tag(Navigation.Tab.accounts)
                Text("Directories \(state.bindings.count)").tag(Navigation.Tab.directories)
                healthLabel.tag(Navigation.Tab.health)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 12)
            .padding(.bottom, 8)

            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    if let error = state.configError {
                        ErrorBanner(text: error)
                    }
                    switch navigation.tab {
                    case .accounts:
                        AccountsView(state: state, navigation: navigation, prompt: $prompt)
                    case .directories:
                        DirectoriesView(state: state, prompt: $prompt)
                    case .health:
                        HealthView(checks: state.checks)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
            }
            footer
        }
    }

    private var healthLabel: some View {
        let failing = state.checks.filter { $0.severity == "Error" }.count
        let warning = state.checks.filter { $0.severity == "Warning" }.count
        return Text(failing > 0 ? "Health ✕" : warning > 0 ? "Health !" : "Health ✓")
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("cpx").font(.headline)
            if let last = state.lastUsageRefresh {
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    Text(Format.age(of: last, now: context.date))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if state.refreshing || state.busy {
                ProgressView().controlSize(.small)
            }
            Button {
                Task { await state.refreshUsage(manual: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .help("Refresh usage now")
            .disabled(state.refreshing)
            Button(action: openSettings) {
                Image(systemName: "gearshape")
            }
            .buttonStyle(.borderless)
            .help("Settings")
            Menu {
                Button("Edit config.toml…") {
                    Task {
                        if let paths = try? await state.cli.paths() { Desktop.open(paths.config) }
                    }
                }
                Button("Reveal profiles folder") {
                    Task {
                        if let paths = try? await state.cli.paths() { Desktop.reveal(paths.root) }
                    }
                }
                Divider()
                Button("Quit cpx") { NSApp.terminate(nil) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .frame(width: 24)
        }
        .padding(.horizontal, 12)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }

    private var footer: some View {
        VStack(spacing: 0) {
            Divider()
            if showPlan, state.plan.pendingActions > 0 {
                PlanSheetView(plan: state.plan)
                    .frame(maxHeight: 200)
                Divider()
            }
            HStack(spacing: 8) {
                Button("New profile…") { newProfile() }
                    .controlSize(.small)
                Spacer()
                if state.plan.pendingActions == 0 {
                    Text("Up to date").font(.caption).foregroundStyle(.secondary)
                } else {
                    Button(showPlan ? "Hide changes" : "\(state.plan.pendingActions) change\(state.plan.pendingActions == 1 ? "" : "s")") {
                        showPlan.toggle()
                    }
                    .buttonStyle(.borderless)
                    .controlSize(.small)
                    Button(state.plan.requiresForce ? "Apply & back up" : "Apply") {
                        Task {
                            let force = state.plan.requiresForce
                            _ = await state.perform { _ = try await state.cli.apply(force: force, sync: false) }
                            showPlan = false
                            await state.refreshUsage(manual: false)
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(state.busy)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
    }

    private func newProfile() {
        prompt = .choose(
            title: "Which tool is the new profile for?",
            detail: "A profile is one account, with its own login and its own command.",
            options: Provider.allCases.map { PromptOption(id: $0.rawValue, label: $0.displayName, hint: "gives you \($0.binaryName)-<name>", color: nil) }
        ) { chosen in
            let provider = Provider(rawValue: chosen) ?? .claude
            prompt = .text(
                title: "New \(provider.displayName) profile",
                detail: "The name becomes its command: work gives you \(provider.binaryName)-work.",
                placeholder: "work",
                submit: "Create"
            ) { name in
                prompt = nil
                Task {
                    let used = Set(state.profiles.compactMap(\.color))
                    let color = swatches.first { !used.contains($0) } ?? swatches[0]
                    _ = await state.perform {
                        try await state.cli.addProfile(name: name, provider: provider)
                        try await state.cli.setField(profile: name, field: "color", value: color)
                    }
                }
            }
        }
    }
}
