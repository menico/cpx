// The Claude statusline badge, and the editor for the script behind it.

import CpxKit
import SwiftUI

/// Turns a profile badge on or off in front of the statusline already
/// configured. The existing statusline script is never edited: the badge
/// runs first and then hands the session straight to it.
struct StatuslineControlView: View {
    @Bindable var state: AppState
    var profile: String?
    var label: String
    var onEdit: () -> Void

    @State private var row: StatuslineRow?
    @State private var busy = false

    var body: some View {
        Group {
            if let row {
                Field(label: "Profile badge") {
                    HStack {
                        Text(row.badge ? "shown" : "not shown").font(.callout)
                        Spacer()
                        Button(row.badge ? "Remove" : "Add") { toggle(row) }.controlSize(.small).disabled(busy)
                    }
                }
                if row.badge {
                    Field(label: "Preview") {
                        HStack(spacing: 0) {
                            Text("● \(label)").font(.caption.monospaced()).foregroundStyle(Color(hex: state.account(profile ?? "")?.color) ?? .primary)
                            Text(" │ your statusline").font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                    }
                }
                Field(label: row.badge ? "In front of" : "Statusline") {
                    HStack {
                        Text(row.delegate ?? "none configured").font(.caption.monospaced()).lineLimit(2).truncationMode(.middle)
                        Spacer()
                        if row.delegate != nil {
                            Button("Edit…", action: onEdit).controlSize(.small)
                        }
                    }
                }
                if row.badge, row.needsApply {
                    Text("Recorded in your config; takes effect on the next apply.").font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .task(id: profile) { await load() }
    }

    private func load() async {
        let rows = (try? await state.cli.statuslines()) ?? []
        row = rows.first { $0.target == (profile ?? "(base)") }
    }

    private func toggle(_ row: StatuslineRow) {
        busy = true
        Task {
            _ = await state.perform {
                if row.badge {
                    try await state.cli.clearStatusline(profile: profile)
                } else {
                    try await state.cli.setStatusline(profile: profile)
                }
            }
            await load()
            busy = false
        }
    }
}

/// Edits the script a statusline runs.
///
/// A script installed by a package manager gets overwritten on its next
/// update, so editing one offers to take a copy first rather than quietly
/// letting the work be lost.
struct ScriptEditorView: View {
    @Bindable var state: AppState
    var profile: String?
    var onClose: () -> Void

    @State private var script: ScriptFile?
    @State private var loaded = false
    @State private var draft = ""
    @State private var busy = false
    @State private var saved = false

    private var dirty: Bool { script != nil && draft != script?.contents }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button("‹ Back", action: onClose).buttonStyle(.borderless)
                Spacer()
                if saved { Text("Saved").font(.caption).foregroundStyle(.secondary) }
                Button("Save") { save() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(!dirty || busy)
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            Divider()
            if !loaded {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let script {
                Text(script.path)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                if let managedBy = script.managedBy {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Something else installs this file and will overwrite it on its next update, so edits here would be lost. Take a copy and this profile runs that instead.")
                            .font(.caption)
                        Text(managedBy).font(.caption.monospaced()).foregroundStyle(.secondary)
                        Button("Copy to this profile") { fork() }.controlSize(.small).disabled(busy)
                    }
                    .padding(8)
                    .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 6))
                    .padding(.horizontal, 12)
                }
                TextEditor(text: $draft)
                    .font(.system(.caption, design: .monospaced))
                    .padding(.horizontal, 8)
                HStack {
                    Text("\(draft.split(separator: "\n", omittingEmptySubsequences: false).count) lines").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    if dirty {
                        Button("Revert") { draft = script.contents }.controlSize(.small)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
            } else {
                EmptyState(title: "Nothing to edit", detail: "This statusline does not run a script cpx can open — it may be a command on your PATH, or run through npx.")
                Spacer()
            }
        }
        .task { await load() }
    }

    private func load() async {
        script = try? await state.cli.statuslineScript(profile: profile)
        draft = script?.contents ?? ""
        loaded = true
    }

    private func save() {
        busy = true
        Task {
            if await state.perform({ try await state.cli.saveStatuslineScript(profile: profile, contents: draft) }) {
                script?.contents = draft
                saved = true
                try? await Task.sleep(nanoseconds: 1_600_000_000)
                saved = false
            }
            busy = false
        }
    }

    private func fork() {
        busy = true
        Task {
            _ = await state.perform { try await state.cli.forkStatuslineScript(profile: profile) }
            await load()
            busy = false
        }
    }
}
