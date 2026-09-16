// One account in full: usage, cost, and — for a profile — its configuration.

import CpxKit
import SwiftUI

struct AccountDetailView: View {
    @Bindable var state: AppState
    @Bindable var navigation: Navigation
    var account: Account
    @Binding var prompt: Prompt?

    @State private var detail: ProfileDetail?
    @State private var description = ""
    @State private var model = ""
    @State private var showAllDays = false

    private let modes = ["link", "copy", "own", "merge", "ignore"]
    private let modeHelp = [
        "link": "Shared with the source — edit once, every profile sees it",
        "copy": "Seeded once from the source, then this profile's own",
        "own": "Private to this profile; the source is never read",
        "merge": "Rebuilt from the source each time, with this profile's overrides on top",
        "ignore": "Not managed by cpx at all",
    ]

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button("‹ Accounts") { navigation.screen = .main }.buttonStyle(.borderless)
                Spacer()
                if state.busy { ProgressView().controlSize(.small) }
                Button("Show in Finder") { Desktop.reveal(account.directory) }.controlSize(.small)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if let error = state.configError {
                        ErrorBanner(text: error)
                    }
                    title
                    usageSection
                    if state.settings.showCost {
                        costSection
                    }
                    if let profile = account.profile {
                        profileSections(profile)
                    } else {
                        defaultSessionSections
                    }
                }
                .padding(12)
            }
        }
        .task(id: account.id) { await load() }
    }

    private var title: some View {
        HStack(spacing: 8) {
            ColorBar(hex: account.color).frame(height: 24)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(account.name).font(.title3.weight(.semibold))
                    ProviderTag(provider: account.provider)
                }
                Text(account.command).font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            Spacer()
        }
    }

    // MARK: - Usage

    @ViewBuilder
    private var usageSection: some View {
        SectionTitle(text: "Usage")
        if let snapshot = account.snapshot {
            HStack(spacing: 6) {
                if let email = snapshot.account { Text(email).font(.caption).foregroundStyle(.secondary) }
                if let plan = snapshot.plan { Text(plan).font(.caption.weight(.medium)) }
                Spacer()
                Text("via \(snapshot.source) · \(Format.age(of: snapshot.fetched))").font(.caption2).foregroundStyle(.secondary)
            }
            ForEach(snapshot.windows) { window in
                VStack(alignment: .leading, spacing: 2) {
                    UsageBar(window: window, showUsed: state.settings.showUsedNotRemaining, color: Color(hex: account.color))
                    HStack {
                        if let minutes = window.windowMinutes {
                            Text(Format.windowLength(minutes: minutes)).font(.caption2).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if let reset = window.resetDate {
                            Text("resets \(Format.clock(reset))").font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            if let extra = snapshot.extraUsage, extra.enabled {
                Field(label: "Extra usage") {
                    Text("\(Format.money(extra.usedUsd ?? 0)) of \(Format.money(extra.limitUsd ?? 0)) this month")
                        .font(.callout.monospacedDigit())
                }
            }
            if let credits = snapshot.credits {
                Field(label: "Credits") {
                    Text(credits.unlimited ? "unlimited" : credits.balance.map { String(format: "%.2f", $0) } ?? "none")
                        .font(.callout.monospacedDigit())
                }
            }
        }
        if let usage = account.usage, let error = usage.error {
            HStack(alignment: .top, spacing: 6) {
                StatusDot(tone: .warn).padding(.top, 5)
                if error.needsKeychainPermission {
                    Text("cpx has not been allowed to read this login yet. Allowing asks macOS once; choose Always Allow so it is not asked again.")
                        .font(.callout).foregroundStyle(.secondary)
                    Spacer()
                    Button("Allow…") { Task { await state.refreshUsage(manual: true, only: [account.id]) } }
                        .controlSize(.small)
                        .disabled(state.refreshing)
                } else {
                    Text(usage.message ?? error.kind).font(.callout).foregroundStyle(.secondary)
                    Spacer()
                    if error.isLoginProblem {
                        Button(account.signedIn ? "Refresh login…" : "Sign in…") { signIn() }.controlSize(.small)
                    }
                }
            }
        } else if account.snapshot == nil {
            Text(state.refreshing ? "Fetching…" : "Not fetched yet.").font(.caption).foregroundStyle(.secondary)
        }
    }

    // MARK: - Cost

    @ViewBuilder
    private var costSection: some View {
        if let cost = account.cost?.cost {
            SectionTitle(text: "Spend, from local transcripts")
            HStack(spacing: 16) {
                spend("Today", cost.today)
                spend("7 days", cost.last7Days)
                spend("30 days", cost.last30Days)
            }
            if !cost.models.isEmpty {
                ForEach(cost.models.prefix(5)) { model in
                    HStack {
                        Text(model.model).font(.caption.monospaced()).lineLimit(1)
                        Spacer()
                        Text(Format.tokens(model.tokens.total)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        Text(model.costUsd.map(Format.money) ?? "unpriced").font(.caption.monospacedDigit()).frame(width: 64, alignment: .trailing)
                    }
                }
            }
            if !cost.days.isEmpty {
                Button(showAllDays ? "Hide days" : "By day") { showAllDays.toggle() }
                    .buttonStyle(.link)
                    .font(.caption)
                if showAllDays {
                    ForEach(cost.days.reversed()) { day in
                        HStack {
                            Text(day.day).font(.caption.monospaced())
                            Spacer()
                            Text(Format.tokens(day.tokens.total)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                            Text((day.costUsd.map(Format.money) ?? "—") + (day.partlyUnpriced ? "+" : "")).font(.caption.monospacedDigit()).frame(width: 64, alignment: .trailing)
                        }
                    }
                }
            }
            Text("At list prices. A subscription is not billed per token; this is what the usage would cost on the API.")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
    }

    private func spend(_ label: String, _ value: Spend) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(label).font(.caption2).foregroundStyle(.secondary)
            Text(Format.money(value.costUsd) + (value.partlyUnpriced ? "+" : "")).font(.callout.monospacedDigit().weight(.medium))
            Text("\(Format.tokens(value.tokens.total)) tokens").font(.caption2).foregroundStyle(.secondary)
        }
    }

    // MARK: - Profile

    @ViewBuilder
    private func profileSections(_ profile: ProfileRow) -> some View {
        SectionTitle(text: "Profile")
        Field(label: "Name") {
            TextField("What this account is for", text: $description)
                .textFieldStyle(.roundedBorder)
                .onSubmit { setField("description", description) }
        }
        Field(label: "Colour") {
            HStack(spacing: 6) {
                ForEach(swatches, id: \.self) { hex in
                    Button {
                        setField("color", hex)
                    } label: {
                        Circle()
                            .fill(Color(hex: hex) ?? .gray)
                            .frame(width: 16, height: 16)
                            .overlay(Circle().stroke(Color.primary.opacity(profile.color == hex ? 0.8 : 0), lineWidth: 2))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        Field(label: "Model") {
            TextField("\(profile.provider.shortName)'s default", text: $model)
                .textFieldStyle(.roundedBorder)
                .onSubmit { setField("model", model) }
        }

        SectionTitle(text: "Account")
        Field(label: "Signed in as") { Text(profile.account ?? "—").font(.callout) }
        Field(label: "Login kept in") {
            Text(loginLocation(profile)).font(.callout)
        }
        Field(label: "") {
            HStack(spacing: 6) {
                Button(profile.authenticated ? "Check account…" : "Sign in…") { signIn() }
                    .controlSize(.small)
                    .disabled(!profile.applied)
                if profile.authenticated {
                    Button("Sign out…") { signOut() }.controlSize(.small)
                }
            }
        }
        if !profile.applied {
            Text("Apply your changes first — this profile has no directory yet.").font(.caption).foregroundStyle(.secondary)
        }

        if profile.provider == .claude {
            SectionTitle(text: "Statusline")
            StatuslineControlView(state: state, profile: profile.name, label: profile.name) {
                navigation.screen = .script(profile.name)
            }
        }

        SectionTitle(text: "Skills")
        Field(label: "Available") {
            HStack {
                Text(profile.provider == .claude ? "its own and its plugins'" : "its own").font(.callout)
                Spacer()
                Button("Manage…") { navigation.screen = .skills(profile.name) }.controlSize(.small)
            }
        }

        if let detail {
            SectionTitle(text: "What this profile gets")
            ForEach(detail.resources) { resource in
                HStack(spacing: 8) {
                    Text(resource.resource).font(.callout.monospaced())
                    if resource.hasPatch {
                        Text("+patch").font(.caption2).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Picker("", selection: Binding(
                        get: { resource.mode },
                        set: { mode in Task { _ = await state.perform { try await state.cli.setResource(profile: profile.name, resource: resource.resource, mode: mode) }; await load() } }
                    )) {
                        ForEach(modes.filter { $0 != "merge" || resource.supportsMerge }, id: \.self) { mode in
                            Text(mode).tag(mode)
                        }
                    }
                    .labelsHidden()
                    .frame(width: 90)
                    .help(modeHelp[resource.mode] ?? "")
                }
            }
            if !detail.env.isEmpty {
                SectionTitle(text: "Environment")
                ForEach(detail.env.keys.sorted(), id: \.self) { key in
                    Field(label: key) { Text(detail.env[key] ?? "").font(.callout.monospaced()) }
                }
            }
            SectionTitle(text: "Details")
            Field(label: "Directory") { Text(detail.directory).font(.caption.monospaced()).textSelection(.enabled) }
            Field(label: "Built from") { Text(detail.sourceDir).font(.caption.monospaced()).textSelection(.enabled) }
            Field(label: "Login") { Text(detail.credentialLocation).font(.caption.monospaced()).textSelection(.enabled) }
        }

        HStack(spacing: 8) {
            Button("Duplicate…") {
                prompt = .text(
                    title: "Duplicate \(profile.name)",
                    detail: "Copies its settings. The login is not copied — sign the new one in.",
                    placeholder: "\(profile.name)-2",
                    submit: "Duplicate"
                ) { name in
                    prompt = nil
                    Task { _ = await state.perform { try await state.cli.cloneProfile(from: profile.name, to: name) } }
                }
            }
            .controlSize(.small)
            Button("Remove…", role: .destructive) {
                prompt = .confirm(
                    title: "Remove \(profile.name)?",
                    detail: "Removes it from the config. \(profile.directory) stays on disk, with its login intact.",
                    confirm: "Remove",
                    destructive: true
                ) {
                    prompt = nil
                    Task {
                        if await state.perform({ try await state.cli.removeProfile(profile.name) }) {
                            navigation.screen = .main
                        }
                    }
                }
            }
            .controlSize(.small)
            Spacer()
            Toggle("Show in menu bar", isOn: Binding(
                get: { !state.settings.hiddenAccounts.contains(account.id) },
                set: { shown in
                    if shown { state.settings.hiddenAccounts.remove(account.id) } else { state.settings.hiddenAccounts.insert(account.id) }
                    state.onChange?()
                }))
                .controlSize(.small)
                .font(.caption)
        }
        .padding(.top, 6)
    }

    @ViewBuilder
    private var defaultSessionSections: some View {
        SectionTitle(text: "Not managed by cpx")
        Text(account.defaultSession?.isSource == true
             ? "A plain \(account.provider.binaryName) uses this directory, and your \(account.provider.shortName) profiles inherit from it. cpx reads it and never writes to it."
             : "A plain \(account.provider.binaryName) uses this directory. cpx reads it and never writes to it.")
            .font(.caption)
            .foregroundStyle(.secondary)
        Field(label: "Directory") { Text(account.directory).font(.caption.monospaced()).textSelection(.enabled) }
        Field(label: "Signed in as") { Text(account.accountEmail ?? (account.signedIn ? "signed in" : "not signed in")).font(.callout) }
        if account.provider == .claude {
            SectionTitle(text: "Statusline")
            Text("A badge here reads whichever profile is running, so a profile that inherits these settings still shows its own name.")
                .font(.caption)
                .foregroundStyle(.secondary)
            StatuslineControlView(state: state, profile: nil, label: "profile") {
                navigation.screen = .script(nil)
            }
        }
        Toggle("Show in menu bar", isOn: Binding(
            get: { !state.settings.hiddenAccounts.contains(account.id) },
            set: { shown in
                if shown { state.settings.hiddenAccounts.remove(account.id) } else { state.settings.hiddenAccounts.insert(account.id) }
                state.onChange?()
            }))
            .controlSize(.small)
            .font(.caption)
            .padding(.top, 6)
    }

    // MARK: - Actions

    private func load() async {
        guard let profile = account.profile else { return }
        description = profile.description
        model = profile.model ?? ""
        detail = try? await state.cli.show(profile.name)
    }

    private func setField(_ field: String, _ value: String) {
        guard let profile = account.profile else { return }
        let trimmed = value.trimmingCharacters(in: .whitespaces)
        Task {
            _ = await state.perform { try await state.cli.setField(profile: profile.name, field: field, value: trimmed.isEmpty ? nil : trimmed) }
            await load()
        }
    }

    private func loginLocation(_ profile: ProfileRow) -> String {
        switch profile.source {
        case "Keychain": "macOS Keychain"
        case "File": profile.provider == .codex ? "auth.json" : "credentials file"
        case "ApiKey": "an API key in auth.json"
        default: "not signed in"
        }
    }

    private func signIn() {
        let sub = account.provider == .claude ? (account.signedIn ? "auth status" : "auth login") : (account.signedIn ? "login status" : "login")
        Desktop.openInTerminal("\(Desktop.shellQuote(account.command)) \(sub)")
    }

    private func signOut() {
        let sub = account.provider == .claude ? "auth logout" : "logout"
        Desktop.openInTerminal("\(Desktop.shellQuote(account.command)) \(sub)")
    }
}
