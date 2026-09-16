// Everything a profile can call on.
//
// Skills reach a profile two ways and the two have different levers: your
// own live in the profile and can be switched off one at a time, while
// plugin skills come with their plugin and can only be turned off together
// — that is the granularity Claude's settings offer, so it is the one shown.

import CpxKit
import SwiftUI

struct SkillsView: View {
    @Bindable var state: AppState
    var profile: String
    var onClose: () -> Void

    @State private var inventory: SkillInventory?
    @State private var busy: String?
    @State private var needsApply = false
    @State private var expanded: String?
    @State private var confirming: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button("‹ Back", action: onClose).buttonStyle(.borderless)
                Spacer()
                if needsApply { Text("apply to take effect").font(.caption).foregroundStyle(.secondary) }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    if let inventory {
                        SectionTitle(text: "In this profile")
                        if inventory.shared {
                            Text("This profile shares its skills directory, so turning one off turns it off everywhere. Give it its own copy first if that is not what you want.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        if inventory.own.isEmpty {
                            Text("No skills of its own.").font(.caption).foregroundStyle(.secondary)
                        }
                        if confirming != nil {
                            Text("Removing moves the skill to skills.removed/ in this profile. Nothing is deleted.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        ForEach(inventory.own) { skill in
                            HStack(alignment: .top, spacing: 8) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(skill.name).font(.callout.weight(.medium)).foregroundStyle(skill.enabled ? .primary : .secondary)
                                    if let description = skill.description {
                                        Text(description).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                                Button(skill.enabled ? "Turn off" : "Turn on") {
                                    act(skill.name) { try await state.cli.setSkillEnabled(profile: profile, skill: skill.name, enabled: !skill.enabled) }
                                }
                                .controlSize(.small)
                                .disabled(busy == skill.name)
                                if confirming == skill.name {
                                    Button("Confirm", role: .destructive) {
                                        confirming = nil
                                        act(skill.name) { try await state.cli.removeSkill(profile: profile, skill: skill.name) }
                                    }
                                    .controlSize(.small)
                                } else {
                                    Button("Remove…") { confirming = skill.name }.buttonStyle(.borderless).controlSize(.small)
                                }
                            }
                            .padding(8)
                            .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
                        }

                        let fromPlugins = inventory.plugins.filter { $0.skills > 0 }
                        if !fromPlugins.isEmpty {
                            SectionTitle(text: "From plugins · \(fromPlugins.reduce(0) { $0 + $1.skills }) skills")
                            Text("A plugin's skills come and go together — that is the only switch Claude has for them.")
                                .font(.caption).foregroundStyle(.secondary)
                            ForEach(fromPlugins) { plugin in
                                HStack(alignment: .top, spacing: 8) {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(plugin.plugin).font(.callout.weight(.medium)).foregroundStyle(plugin.enabled ? .primary : .secondary)
                                        Button("\(plugin.skills) skill\(plugin.skills == 1 ? "" : "s") \(expanded == plugin.key ? "▾" : "▸")") {
                                            expanded = expanded == plugin.key ? nil : plugin.key
                                        }
                                        .buttonStyle(.link)
                                        .font(.caption)
                                        if expanded == plugin.key {
                                            Text(plugin.names.joined(separator: ", ")).font(.caption).foregroundStyle(.secondary)
                                        }
                                    }
                                    Spacer()
                                    Button(plugin.enabled ? "Turn off" : "Turn on") {
                                        act(plugin.key) {
                                            let change = try await state.cli.setPluginEnabled(profile: profile, key: plugin.key, enabled: !plugin.enabled)
                                            if change.needsApply { needsApply = true }
                                        }
                                    }
                                    .controlSize(.small)
                                    .disabled(busy == plugin.key)
                                }
                                .padding(8)
                                .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
                            }
                        }
                    } else if let error = state.configError {
                        ErrorBanner(text: error)
                    } else {
                        ProgressView().frame(maxWidth: .infinity).padding()
                    }
                }
                .padding(12)
            }
        }
        .task { await load() }
    }

    private func load() async {
        do {
            inventory = try await state.cli.skills(profile: profile)
        } catch {
            state.configError = error.localizedDescription
        }
    }

    private func act(_ id: String, _ work: @escaping () async throws -> Void) {
        busy = id
        Task {
            _ = await state.perform(work)
            await load()
            busy = nil
        }
    }
}
