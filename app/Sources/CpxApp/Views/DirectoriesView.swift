// Bound directories, health findings, and the pending plan.

import CpxKit
import SwiftUI

struct DirectoriesView: View {
    @Bindable var state: AppState
    @Binding var prompt: Prompt?

    var body: some View {
        if state.profiles.isEmpty {
            EmptyState(title: "No profiles yet", detail: "Add a profile before binding a directory to one.")
        } else if state.bindings.isEmpty {
            EmptyState(
                title: "No directories bound",
                detail: "Bind a directory and everything you run there — including a plain claude or codex — uses that profile.",
                action: bind,
                actionTitle: "Bind a directory…")
        } else {
            ForEach(state.bindings) { binding in
                HStack(alignment: .top, spacing: 10) {
                    ColorBar(hex: binding.color)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(basename(binding.path)).font(.callout.weight(.medium))
                        Text(binding.path).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        if let problem = binding.problem {
                            HStack(spacing: 4) {
                                StatusDot(tone: .warn)
                                Text(problem).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 4) {
                        HStack(spacing: 4) {
                            Text(binding.profile).font(.caption)
                            ProviderTag(provider: binding.provider)
                        }
                        Button("Unbind") {
                            Task { _ = await state.perform { try await state.cli.unbind(dir: binding.path, provider: binding.provider) } }
                        }
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                    }
                }
                .padding(8)
                .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
            }
            Button("Bind another directory…", action: bind)
                .controlSize(.small)
                .padding(.top, 4)
        }
    }

    private func bind() {
        guard let dir = Desktop.pickDirectory(title: "Choose a directory to bind") else { return }
        if state.profiles.count == 1, let only = state.profiles.first {
            Task { _ = await state.perform { try await state.cli.bind(profile: only.name, dir: dir) } }
            return
        }
        prompt = .choose(
            title: "Which profile should this use?",
            detail: dir,
            options: state.profiles.map { PromptOption(id: $0.name, label: $0.name, hint: $0.account ?? $0.provider.displayName, color: $0.color) }
        ) { name in
            prompt = nil
            Task { _ = await state.perform { try await state.cli.bind(profile: name, dir: dir) } }
        }
    }

    private func basename(_ path: String) -> String {
        let trimmed = path.hasSuffix("/") ? String(path.dropLast()) : path
        return trimmed.split(separator: "/").last.map(String.init) ?? path
    }
}

struct HealthView: View {
    var checks: [Check]

    var body: some View {
        let findings = checks.filter { $0.severity != "Ok" }
        if findings.isEmpty {
            EmptyState(title: "All clear", detail: "\(checks.count) checks passed.")
        } else {
            ForEach(findings) { check in
                HStack(alignment: .top, spacing: 8) {
                    StatusDot(tone: check.severity == "Error" ? .fail : .warn).padding(.top, 5)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(check.name).font(.callout.weight(.medium))
                        Text(check.detail).font(.caption).foregroundStyle(.secondary)
                        if let remedy = check.remedy {
                            Text(remedy).font(.caption.monospaced()).textSelection(.enabled)
                        }
                    }
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
            }
        }
    }
}

/// The same gutter glyphs `cpx apply --dry-run` prints, so the app and the
/// terminal never describe the same change differently.
struct PlanSheetView: View {
    var plan: Plan

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(plan.actions.enumerated()), id: \.offset) { _, line in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(gutter(line.risk)).frame(width: 10).foregroundStyle(line.risk == "foreign" ? .red : .secondary)
                        Text(line.verb).frame(width: 48, alignment: .leading).foregroundStyle(.secondary)
                        Text(line.target).lineLimit(1).truncationMode(.middle)
                    }
                    .font(.caption.monospaced())
                }
                ForEach(plan.notes, id: \.self) { note in
                    Text(note).font(.caption).foregroundStyle(.secondary)
                }
                if plan.requiresForce {
                    Text("Lines marked ! would replace files cpx did not write. Each one is backed up first.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func gutter(_ risk: String) -> String {
        switch risk {
        case "generated": "~"
        case "foreign": "!"
        default: ""
        }
    }
}
