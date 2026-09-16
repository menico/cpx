// Every account on the machine, with what it has used.

import CpxKit
import SwiftUI

struct AccountsView: View {
    @Bindable var state: AppState
    @Bindable var navigation: Navigation
    @Binding var prompt: Prompt?

    var body: some View {
        let accounts = state.visibleAccounts
        if accounts.isEmpty, state.adoptable.allSatisfy(\.taken) {
            EmptyState(
                title: "No accounts yet",
                detail: "A profile is one Claude Code or Codex account, with its own login and its own command.")
        }
        ForEach(accounts) { account in
            AccountCard(state: state, account: account) {
                navigation.screen = .account(account.id)
            }
        }
        if state.settings.statusChecks {
            ForEach(Provider.allCases, id: \.self) { provider in
                if let status = state.providerStatus[provider], status.indicator.isIncident {
                    HStack(spacing: 6) {
                        StatusDot(tone: status.indicator == .minor || status.indicator == .maintenance ? .warn : .fail)
                        Text("\(provider.displayName): \(status.description ?? status.indicator.label)")
                            .font(.caption)
                        Spacer()
                        Button("Status page") { Desktop.open(StatusFeed.page(for: provider)) }
                            .buttonStyle(.link)
                            .font(.caption)
                    }
                    .padding(8)
                    .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                }
            }
        }
        AdoptableSection(state: state)
    }
}

struct AccountCard: View {
    @Bindable var state: AppState
    var account: Account
    var open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(alignment: .top, spacing: 10) {
                ColorBar(hex: account.color)
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 6) {
                        Text(account.name).font(.headline)
                        ProviderTag(provider: account.provider)
                        if account.isDefaultSession {
                            Text("not managed").font(.caption2).foregroundStyle(.secondary)
                        } else if account.profile?.adopted == true {
                            Text("in place").font(.caption2).foregroundStyle(.secondary)
                        }
                        Spacer()
                        stateLabel
                    }
                    if account.accountEmail != nil || account.plan != nil {
                        HStack(spacing: 6) {
                            if let email = account.accountEmail {
                                Text(email).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            if let plan = account.plan {
                                Text(plan).font(.caption.weight(.medium)).foregroundStyle(.secondary)
                            }
                        }
                    }
                    if let snapshot = account.snapshot {
                        ForEach([snapshot.session, snapshot.weekly].compactMap { $0 }) { window in
                            UsageBar(window: window, showUsed: state.settings.showUsedNotRemaining, color: Color(hex: account.color), compact: true)
                        }
                        if snapshot.windows.count > 2 {
                            Text("+\(snapshot.windows.count - 2) more limit\(snapshot.windows.count - 2 == 1 ? "" : "s")")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                    if let usage = account.usage, let error = usage.error {
                        HStack(spacing: 6) {
                            StatusDot(tone: .warn)
                            if error.needsKeychainPermission {
                                Text("Needs permission to read this login.").font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                Button("Allow…") {
                                    Task { await state.refreshUsage(manual: true, only: [account.id]) }
                                }
                                .controlSize(.mini)
                                .disabled(state.refreshing)
                            } else {
                                Text(usage.message ?? error.kind).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                            }
                        }
                    }
                    if state.settings.showCost, let cost = account.cost?.cost {
                        HStack(spacing: 10) {
                            costLabel("today", cost.today)
                            costLabel("7d", cost.last7Days)
                            costLabel("30d", cost.last30Days)
                        }
                    }
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var stateLabel: some View {
        let (tone, text): (StatusDot.Tone, String) = {
            if let profile = account.profile, !profile.applied { return (.off, "not built") }
            if !account.signedIn { return (.warn, "signed out") }
            if account.usageError?.isLoginProblem == true { return (.warn, "needs attention") }
            return (.ok, "ready")
        }()
        return HStack(spacing: 4) {
            StatusDot(tone: tone)
            Text(text).font(.caption).foregroundStyle(.secondary)
        }
    }

    private func costLabel(_ label: String, _ spend: Spend) -> some View {
        HStack(spacing: 3) {
            Text(label).font(.caption2).foregroundStyle(.secondary)
            Text(Format.money(spend.costUsd) + (spend.partlyUnpriced ? "+" : "")).font(.caption.monospacedDigit())
        }
    }
}

/// Directories that already exist and could be managed where they are.
struct AdoptableSection: View {
    @Bindable var state: AppState

    var body: some View {
        let available = state.adoptable.filter { !$0.taken }
        if !available.isEmpty {
            SectionTitle(text: "Already on this machine")
            Text("Managed where they are. Nothing inside them changes, and each keeps the account it is already signed into.")
                .font(.caption)
                .foregroundStyle(.secondary)
            ForEach(available) { candidate in
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(candidate.name).font(.callout.weight(.medium))
                            ProviderTag(provider: candidate.provider)
                        }
                        Text(candidate.dir).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        if !candidate.found.isEmpty {
                            Text("keeps \(candidate.found.joined(separator: ", "))").font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button("Adopt") {
                        Task { _ = await state.perform { try await state.cli.adopt(dir: candidate.dir) } }
                    }
                    .controlSize(.small)
                }
                .padding(8)
                .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
            }
        }
    }
}
