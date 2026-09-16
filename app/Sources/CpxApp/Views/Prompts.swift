// Questions the interface asks.

import SwiftUI

/// A question the interface needs answered, shown as a sheet.
enum Prompt: Identifiable {
    case text(title: String, detail: String?, placeholder: String, submit: String, onSubmit: (String) -> Void)
    case choose(title: String, detail: String?, options: [PromptOption], onChoose: (String) -> Void)
    case confirm(title: String, detail: String?, confirm: String, destructive: Bool, onConfirm: () -> Void)

    var id: String {
        switch self {
        case .text(let title, _, _, _, _), .choose(let title, _, _, _), .confirm(let title, _, _, _, _): title
        }
    }
}

struct PromptOption: Identifiable {
    var id: String
    var label: String
    var hint: String?
    var color: String?
}

struct PromptSheet: View {
    var prompt: Prompt
    var onDismiss: () -> Void
    @State private var value = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            switch prompt {
            case .text(let title, let detail, let placeholder, let submit, let onSubmit):
                header(title, detail)
                TextField(placeholder, text: $value)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { commit(onSubmit) }
                HStack {
                    Spacer()
                    Button("Cancel", action: onDismiss).keyboardShortcut(.cancelAction)
                    Button(submit) { commit(onSubmit) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(value.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            case .choose(let title, let detail, let options, let onChoose):
                header(title, detail)
                VStack(spacing: 4) {
                    ForEach(options) { option in
                        Button {
                            onChoose(option.id)
                        } label: {
                            HStack(spacing: 8) {
                                ColorBar(hex: option.color).frame(height: 28)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(option.label).font(.callout)
                                    if let hint = option.hint {
                                        Text(hint).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .padding(6)
                        .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                    }
                }
                HStack {
                    Spacer()
                    Button("Cancel", action: onDismiss).keyboardShortcut(.cancelAction)
                }
            case .confirm(let title, let detail, let confirm, let destructive, let onConfirm):
                header(title, detail)
                HStack {
                    Spacer()
                    Button("Cancel", action: onDismiss).keyboardShortcut(.cancelAction)
                    Button(confirm, role: destructive ? .destructive : nil, action: onConfirm)
                        .keyboardShortcut(.defaultAction)
                        .tint(destructive ? .red : nil)
                }
            }
        }
        .padding(16)
        .frame(width: 340)
    }

    private func header(_ title: String, _ detail: String?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.headline)
            if let detail {
                Text(detail).font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private func commit(_ onSubmit: (String) -> Void) {
        let trimmed = value.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        onSubmit(trimmed)
    }
}
