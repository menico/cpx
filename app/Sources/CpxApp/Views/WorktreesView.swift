// Git worktrees across every account, with the sessions that worked in each.
//
// The point is finding what can go: a worktree no session is running in,
// with no uncommitted files, is safe to remove, and removing it keeps its
// branch. Anything else the command refuses, and the refusal is shown with
// the choice to go ahead anyway.

import AppKit
import CpxKit
import SwiftUI

struct WorktreesView: View {
    @Bindable var state: AppState

    @State private var query = ""
    @State private var unusedOnly = false
    @State private var expanded: Set<String> = []
    @State private var confirming: String?
    @State private var removing: String?
    @State private var refusals: [String: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                TextField("Search repo, branch or session", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .controlSize(.small)
                Toggle("Not in use", isOn: $unusedOnly)
                    .toggleStyle(.checkbox)
                    .controlSize(.small)
                    .help("Hide worktrees a running session is in")
            }
            if let report = state.worktrees {
                summary(report)
                let repos = filtered(report)
                if repos.isEmpty {
                    EmptyState(
                        title: report.repos.isEmpty ? "No worktrees" : "Nothing matches",
                        detail: report.repos.isEmpty
                            ? "None of the repositories your sessions worked in has a worktree besides its main one."
                            : "Try another search, or show worktrees in use.")
                }
                ForEach(repos) { repo in
                    SectionTitle(text: "\(repo.name) · \(repo.worktrees.count)")
                    ForEach(repo.worktrees) { tree in
                        row(tree, in: repo)
                    }
                }
            } else {
                ProgressView("Reading sessions…")
                    .controlSize(.small)
                    .frame(maxWidth: .infinity)
                    .padding()
            }
        }
        .task { await state.loadWorktrees() }
    }

    // MARK: - Summary

    private func summary(_ report: WorktreeReport) -> some View {
        let trees = report.repos.flatMap { added($0) }
        let free = trees.filter { !$0.inUse && $0.changes == 0 }
        let freeBytes = free.compactMap(\.sizeBytes).reduce(0, +)
        return HStack(spacing: 6) {
            Text("\(trees.count) worktrees in \(report.repos.count) repos")
            if state.measuringWorktrees {
                ProgressView().controlSize(.mini)
                Text("measuring…")
            } else if freeBytes > 0 {
                Text("· \(bytes(freeBytes)) in \(free.count) unused and clean")
            }
            Spacer()
            Button {
                Task { await state.loadWorktrees() }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .disabled(state.measuringWorktrees)
            .help("Read again")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    // MARK: - Rows

    private func row(_ tree: Worktree, in repo: WorktreeRepo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(tree.name).font(.callout.weight(.medium)).lineLimit(1).truncationMode(.middle)
                Spacer()
                facts(tree)
            }
            HStack(spacing: 4) {
                Image(systemName: "arrow.triangle.branch").font(.caption2)
                Text(tree.branch ?? "detached at \(tree.head.map { String($0.prefix(8)) } ?? "?")")
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .font(.caption.monospaced())
            .foregroundStyle(.secondary)
            .help(tree.path)

            sessions(tree)

            if let refusal = refusals[tree.path] {
                HStack(alignment: .top, spacing: 6) {
                    StatusDot(tone: .warn).padding(.top, 4)
                    Text(refusal).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Remove anyway", role: .destructive) { remove(tree, in: repo, force: true) }
                        .controlSize(.small)
                    Button("Cancel") { refusals[tree.path] = nil }
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                }
            }
            actions(tree, in: repo)
        }
        .padding(8)
        .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
        .opacity(removing == tree.path ? 0.5 : 1)
    }

    @ViewBuilder
    private func facts(_ tree: Worktree) -> some View {
        HStack(spacing: 6) {
            if tree.prunable {
                tag("gone", tone: .off)
            }
            if tree.locked {
                tag("locked", tone: .warn)
            }
            if let changes = tree.changes {
                tag(changes == 0 ? "clean" : "\(changes) changed", tone: changes == 0 ? .ok : .warn)
            }
            if let size = tree.sizeBytes {
                Text(bytes(size)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
        }
    }

    private func tag(_ text: String, tone: StatusDot.Tone) -> some View {
        HStack(spacing: 3) {
            StatusDot(tone: tone)
            Text(text)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    @ViewBuilder
    private func sessions(_ tree: Worktree) -> some View {
        if tree.sessions.isEmpty {
            Text("No session worked here").font(.caption).foregroundStyle(.tertiary)
        } else {
            let open = expanded.contains(tree.path)
            ForEach(open ? tree.sessions : Array(tree.sessions.prefix(2))) { session in
                sessionLine(session)
            }
            if tree.sessions.count > 2 {
                Button(open ? "Fewer" : "\(tree.sessions.count - 2) more session\(tree.sessions.count == 3 ? "" : "s")") {
                    if open { expanded.remove(tree.path) } else { expanded.insert(tree.path) }
                }
                .buttonStyle(.link)
                .font(.caption)
            }
        }
    }

    private func sessionLine(_ session: WorktreeSession) -> some View {
        HStack(spacing: 6) {
            ColorBar(hex: state.account(session.account)?.color).frame(height: 12)
            StatusDot(tone: session.live ? (session.status == "busy" ? .warn : .ok) : .off)
            Text(session.title ?? String(session.id.prefix(8)))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 4)
            Text(session.account).foregroundStyle(.secondary)
            Text(session.live ? (session.status ?? "running") : Format.age(of: session.lastActiveDate))
                .foregroundStyle(.secondary)
                .monospacedDigit()
        }
        .font(.caption)
        .contentShape(Rectangle())
        .contextMenu {
            Button("Resume in Terminal") { resume(session) }
            Button("Copy session ID") { copy(session.id) }
        }
        .help(session.live ? "Running now" : "Right-click to resume")
    }

    @ViewBuilder
    private func actions(_ tree: Worktree, in repo: WorktreeRepo) -> some View {
        HStack(spacing: 10) {
            if !tree.prunable {
                Button("Reveal") { Desktop.reveal(tree.path) }
                Button("Terminal") { Desktop.openInTerminal("cd \(Desktop.shellQuote(tree.path))") }
            }
            Button("Copy path") { copy(tree.path) }
            Spacer()
            if confirming == tree.path {
                Text(tree.branch.map { "keeps \($0)" } ?? "nothing else is touched")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Button("Remove", role: .destructive) { remove(tree, in: repo, force: false) }
                    .controlSize(.small)
                Button("Cancel") { confirming = nil }
            } else if removing == tree.path {
                ProgressView().controlSize(.small)
            } else {
                Button(tree.prunable ? "Prune…" : "Remove…") {
                    refusals[tree.path] = nil
                    confirming = tree.path
                }
                .disabled(removing != nil)
            }
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .font(.caption)
    }

    // MARK: - Work

    private func remove(_ tree: Worktree, in repo: WorktreeRepo, force: Bool) {
        confirming = nil
        refusals[tree.path] = nil
        removing = tree.path
        Task {
            do {
                try await state.removeWorktree(tree, repo: repo, force: force)
            } catch {
                refusals[tree.path] = error.localizedDescription
            }
            removing = nil
        }
    }

    private func resume(_ session: WorktreeSession) {
        let start = session.resumeDir.map { "cd \(Desktop.shellQuote($0)) && " } ?? ""
        Desktop.openInTerminal(start + session.resume)
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    // MARK: - Filtering

    private func added(_ repo: WorktreeRepo) -> [Worktree] {
        repo.worktrees.filter { !$0.main }
    }

    private func filtered(_ report: WorktreeReport) -> [WorktreeRepo] {
        let needle = query.trimmingCharacters(in: .whitespaces).lowercased()
        return report.repos.compactMap { repo in
            var repo = repo
            repo.worktrees = added(repo).filter { tree in
                if unusedOnly && tree.inUse { return false }
                guard !needle.isEmpty else { return true }
                let haystack = [repo.name, tree.path, tree.branch ?? ""]
                    + tree.sessions.flatMap { [$0.title ?? "", $0.account, $0.id] }
                return haystack.contains { $0.lowercased().contains(needle) }
            }
            return repo.worktrees.isEmpty ? nil : repo
        }
    }

    private func bytes(_ count: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(count), countStyle: .file)
    }
}
