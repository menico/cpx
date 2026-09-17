// Everything the interface shows, and the loop that keeps it current.
//
// Two kinds of data live here. The configuration — profiles, bindings,
// health, the pending plan — is cheap to read and reloaded after every
// change. Usage and cost are slower and come from the network and the
// transcripts, so they refresh on a cadence and survive between refreshes.

import AppKit
import CpxKit
import Foundation
import Observation

/// One row of the accounts list: a profile or a default session, with
/// whatever usage and cost have been fetched for it.
struct Account: Identifiable, Hashable {
    var id: String
    var provider: Provider
    var name: String
    var profile: ProfileRow?
    var defaultSession: DefaultSession?
    var usage: UsageAccount?
    var cost: CostAccount?

    var color: String? { profile?.color }
    var isDefaultSession: Bool { profile == nil }
    var directory: String { profile?.directory ?? defaultSession?.directory ?? "" }
    var command: String { profile?.command ?? provider.binaryName }
    var signedIn: Bool { profile?.authenticated ?? defaultSession?.signedIn ?? false }
    var accountEmail: String? { usage?.usage?.account ?? profile?.account ?? defaultSession?.account }
    var plan: String? { usage?.usage?.plan ?? profile?.plan }
    var snapshot: UsageSnapshot? { usage?.usage }
    var usageError: UsageError? { usage?.error }
}

@MainActor
@Observable
final class AppState {
    let cli: CpxCLI
    let settings: Settings
    let notifier: Notifier

    var initialised: Bool?
    var profiles: [ProfileRow] = []
    var defaultSessions: [DefaultSession] = []
    var bindings: [BindingRow] = []
    var checks: [Check] = []
    var plan: Plan = .empty
    var adoptable: [AdoptionRow] = []

    var usage: [String: UsageAccount] = [:]
    var cost: [String: CostAccount] = [:]
    var providerStatus: [Provider: ProviderStatus] = [:]

    var configError: String?
    var busy = false
    var refreshing = false
    var lastUsageRefresh: Date?
    var nextRefreshAt: Date?
    var lastMenuOpenAt: Date?

    /// Alert state per account and window, keyed `account/window`.
    private var alertStates: [String: WindowAlertState] = [:]
    private var refreshTimer: Timer?
    private var configWatcher: DispatchSourceFileSystemObject?

    /// Called after every configuration reload and usage refresh, so the
    /// status item can redraw.
    var onChange: (() -> Void)?

    init(cli: CpxCLI, settings: Settings, notifier: Notifier) {
        self.cli = cli
        self.settings = settings
        self.notifier = notifier
    }

    // MARK: - Accounts

    var accounts: [Account] {
        var rows: [Account] = profiles.map { profile in
            Account(id: profile.name, provider: profile.provider, name: profile.name, profile: profile,
                    usage: usage[profile.name], cost: cost[profile.name])
        }
        for session in defaultSessions where session.claimedBy == nil {
            let id = "@\(session.provider.rawValue)"
            rows.append(Account(id: id, provider: session.provider, name: session.provider.binaryName,
                                defaultSession: session, usage: usage[id], cost: cost[id]))
        }
        return rows
    }

    var visibleAccounts: [Account] {
        accounts.filter { !settings.hiddenAccounts.contains($0.id) }
    }

    func account(_ id: String) -> Account? {
        accounts.first { $0.id == id }
    }

    /// The account the merged icon shows: the pinned one, else whichever is
    /// closest to a limit, else the first.
    var iconAccount: Account? {
        let visible = visibleAccounts
        if let pinned = settings.pinnedAccount, let found = visible.first(where: { $0.id == pinned }) {
            return found
        }
        return visible.max { a, b in
            (a.snapshot.map { usedPercent(for: $0) } ?? -1) < (b.snapshot.map { usedPercent(for: $0) } ?? -1)
        } ?? visible.first
    }

    /// The window the menu bar reports for a snapshot, per the metric setting.
    func menuWindow(for snapshot: UsageSnapshot) -> RateWindow? {
        switch settings.metric {
        case .session: snapshot.session ?? snapshot.weekly
        case .weekly: snapshot.weekly ?? snapshot.session
        case .auto:
            [snapshot.session, snapshot.weekly].compactMap { $0 }.max { $0.usedPercent < $1.usedPercent }
        }
    }

    func usedPercent(for snapshot: UsageSnapshot) -> Double {
        menuWindow(for: snapshot)?.usedPercent ?? 0
    }

    // MARK: - Configuration

    func reloadConfig() async {
        do {
            let paths = try await cli.paths()
            initialised = paths.initialised
            guard paths.initialised else {
                profiles = []
                defaultSessions = []
                onChange?()
                return
            }
            async let list = cli.list()
            async let bound = cli.bindings()
            async let health = cli.checks()
            async let pending = cli.plan()
            async let candidates = cli.adoptionCandidates()
            let (listed, boundRows, healthRows, pendingPlan, candidateRows) = try await (list, bound, health, pending, candidates)
            profiles = listed.profiles
            defaultSessions = listed.defaultSessions
            bindings = boundRows
            checks = healthRows
            plan = pendingPlan
            adoptable = candidateRows
            configError = nil
        } catch {
            configError = error.localizedDescription
        }
        onChange?()
    }

    /// Run a change, then reload so the interface shows its effect.
    func perform(_ work: () async throws -> Void) async -> Bool {
        busy = true
        defer { busy = false }
        do {
            try await work()
            configError = nil
        } catch {
            configError = error.localizedDescription
            await reloadConfig()
            return false
        }
        await reloadConfig()
        return true
    }

    func initialise() async {
        _ = await perform { try await cli.initialise() }
        await refreshUsage(manual: false)
    }

    // MARK: - Usage

    /// Whether a refresh may show a Keychain dialog, per the policy.
    func mayPrompt(manual: Bool) -> Bool {
        switch settings.keychainPrompts {
        case .never: false
        case .always: true
        case .manualOnly: manual
        }
    }

    /// Fetch usage, cost and provider status for every account, or for
    /// `only` those named.
    func refreshUsage(manual: Bool, only: [String] = []) async {
        guard initialised == true, !refreshing else { return }
        refreshing = true
        defer {
            refreshing = false
            scheduleNextRefresh()
        }

        let prompt = mayPrompt(manual: manual)
        async let usageReport = try? cli.usage(accounts: only, prompt: prompt, keepLogins: settings.keepLogins)
        async let costReport = settings.showCost ? (try? cli.cost(accounts: only, days: settings.costDays)) : nil
        async let statuses: [Provider: ProviderStatus] = settings.statusChecks ? fetchStatuses() : [:]

        if let report = await usageReport {
            var fresh = only.isEmpty ? [:] : usage
            for account in report.accounts {
                // A failed fetch keeps the last good snapshot visible, with the error beside it.
                var row = account
                if row.usage == nil, let previous = usage[account.id]?.usage {
                    row.usage = previous
                }
                fresh[account.id] = row
            }
            usage = fresh
            lastUsageRefresh = Date()
        }
        if let report = await costReport {
            var fresh = only.isEmpty ? [:] : cost
            for account in report.accounts {
                fresh[account.id] = account
            }
            cost = fresh
        }
        let fetchedStatuses = await statuses
        for (provider, status) in fetchedStatuses {
            providerStatus[provider] = status
        }

        evaluateAlerts()
        onChange?()
    }

    private func fetchStatuses() async -> [Provider: ProviderStatus] {
        var found: [Provider: ProviderStatus] = [:]
        let wanted = Set(accounts.map(\.provider))
        for provider in wanted {
            if let status = await StatusFeed.fetch(provider) {
                found[provider] = status
            }
        }
        return found
    }

    // MARK: - Refresh loop

    func scheduleNextRefresh() {
        refreshTimer?.invalidate()
        refreshTimer = nil
        nextRefreshAt = nil

        let delay: TimeInterval
        switch settings.cadence {
        case .manual:
            return
        case .adaptive:
            let input = RefreshInput(
                now: Date(),
                lastMenuOpenAt: lastMenuOpenAt,
                lastCodingActivityAt: nil,
                lowPowerMode: ProcessInfo.processInfo.isLowPowerModeEnabled,
                thermalPressure: [.serious, .critical].contains(ProcessInfo.processInfo.thermalState))
            delay = RefreshPolicy.nextDelay(for: input).delay
        default:
            delay = settings.cadence.fixedInterval ?? 300
        }

        nextRefreshAt = Date().addingTimeInterval(delay)
        let timer = Timer(timeInterval: delay, repeats: false) { [weak self] _ in
            Task { @MainActor in
                await self?.refreshUsage(manual: false)
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        refreshTimer = timer
    }

    /// The menu opened: note it for the cadence, and bring a slow cadence forward.
    func menuOpened() {
        lastMenuOpenAt = Date()
        if settings.cadence == .adaptive, let next = nextRefreshAt, next.timeIntervalSinceNow > RefreshPolicy.recentInteractionDelay {
            scheduleNextRefresh()
        }
        // The configuration may have changed under us in an editor.
        Task { await reloadConfig() }
    }

    /// Watch config.toml so an edit in a text editor shows up without a click.
    func watchConfig(at path: String) {
        configWatcher?.cancel()
        let descriptor = open(path, O_EVTONLY)
        guard descriptor >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write, .rename, .delete], queue: .main)
        source.setEventHandler { [weak self] in
            Task { @MainActor in
                await self?.reloadConfig()
                // Editors replace the file; watch the new inode.
                self?.watchConfig(at: path)
            }
        }
        source.setCancelHandler { close(descriptor) }
        source.resume()
        configWatcher = source
    }

    // MARK: - Alerts

    private func evaluateAlerts() {
        guard settings.notificationsEnabled else { return }
        for account in accounts {
            guard let snapshot = account.snapshot else { continue }
            for window in snapshot.windows where window.id == "session" || window.id == "weekly" {
                let key = "\(account.id)/\(window.id)"
                let outcome = QuotaAlertLogic.evaluate(
                    remaining: window.remainingPercent,
                    thresholds: settings.thresholds,
                    state: alertStates[key] ?? WindowAlertState())
                alertStates[key] = outcome.state

                if let threshold = outcome.warning {
                    notifier.post(
                        id: "quota-\(key)",
                        title: "\(account.name) \(window.title.lowercased()) quota low",
                        body: "\(Format.percent(window.remainingPercent)) left. Reached your \(threshold)% \(window.title.lowercased()) warning threshold.")
                }
                if window.id == "session", settings.sessionAlerts {
                    switch outcome.transition {
                    case .depleted:
                        notifier.post(id: "session-\(account.id)", title: "\(account.name) session depleted", body: "0% left. You will be told when it is available again.")
                    case .restored:
                        notifier.post(id: "session-\(account.id)", title: "\(account.name) session restored", body: "Session quota is available again.")
                    case .none:
                        break
                    }
                }
                // A weekly window that was well used and is suddenly nearly
                // empty has reset.
                if window.id == "weekly", settings.weeklyResetAlerts,
                   let previous = previousUsed[key], previous >= 25, window.usedPercent <= previous - 25 {
                    notifier.post(id: "weekly-\(account.id)", title: "\(account.name) weekly limit reset", body: "A fresh week of usage is available.")
                }
                previousUsed[key] = window.usedPercent
            }
        }
    }

    private var previousUsed: [String: Double] = [:]
}
