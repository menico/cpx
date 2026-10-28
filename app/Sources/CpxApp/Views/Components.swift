// Small pieces the screens share.

import CpxKit
import SwiftUI

/// Where the interface is, so the popover can resume where it was left.
@MainActor
@Observable
final class Navigation {
    enum Screen: Equatable {
        case main
        case account(String)
        case script(String?)
        case skills(String)
    }

    var screen: Screen = .main
    var tab: Tab = .accounts

    enum Tab: String, CaseIterable {
        case accounts
        case directories
        case worktrees
        case health
    }
}

/// Eight identity colours, spaced far enough apart to tell apart at 3px wide.
let swatches = ["#5c8dff", "#5dc794", "#d69552", "#c96ec9", "#5bbcd6", "#d4676b", "#9b8ce0", "#8a9099"]

extension Color {
    init?(hex: String?) {
        guard let hex, hex.count == 7, hex.hasPrefix("#"), let value = UInt32(hex.dropFirst(), radix: 16) else { return nil }
        self.init(
            red: Double((value >> 16) & 0xff) / 255,
            green: Double((value >> 8) & 0xff) / 255,
            blue: Double(value & 0xff) / 255)
    }
}

/// The identity bar on the left of a row.
struct ColorBar: View {
    var hex: String?
    var body: some View {
        RoundedRectangle(cornerRadius: 1.5)
            .fill(Color(hex: hex) ?? Color.secondary.opacity(0.25))
            .frame(width: 3)
    }
}

struct ProviderTag: View {
    var provider: Provider
    var body: some View {
        Text(provider.shortName)
            .font(.caption2.weight(.medium))
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(Color.secondary.opacity(0.15), in: Capsule())
            .foregroundStyle(.secondary)
    }
}

struct StatusDot: View {
    enum Tone { case ok, warn, fail, off }
    var tone: Tone
    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 7, height: 7)
    }
    private var color: Color {
        switch tone {
        case .ok: .green
        case .warn: .orange
        case .fail: .red
        case .off: .secondary.opacity(0.4)
        }
    }
}

/// One rate window as a bar with its numbers.
struct UsageBar: View {
    var window: RateWindow
    var showUsed: Bool
    var color: Color?
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(window.title)
                    .font(compact ? .caption : .callout)
                    .foregroundStyle(.primary)
                Spacer()
                if let reset = window.resetDate {
                    TimelineView(.periodic(from: .now, by: 60)) { context in
                        Text("resets \(Format.countdown(to: reset, from: context.date))")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Text(showUsed ? "\(Format.percent(window.usedPercent)) used" : "\(Format.percent(window.remainingPercent)) left")
                    .font(compact ? .caption.monospacedDigit() : .callout.monospacedDigit())
                    .foregroundStyle(tone)
            }
            GeometryReader { geometry in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.secondary.opacity(0.18))
                    Capsule()
                        .fill(color ?? tone)
                        .frame(width: geometry.size.width * CGFloat(fraction))
                }
            }
            .frame(height: compact ? 4 : 6)
        }
    }

    private var fraction: Double {
        let value = showUsed ? window.usedPercent : window.remainingPercent
        return max(0, min(100, value)) / 100
    }

    private var tone: Color {
        let remaining = window.remainingPercent
        if remaining <= 10 { return .red }
        if remaining <= 25 { return .orange }
        return .primary
    }
}

struct SectionTitle: View {
    var text: String
    var body: some View {
        Text(text.uppercased())
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.top, 6)
    }
}

struct EmptyState: View {
    var title: String
    var detail: String
    var action: (() -> Void)?
    var actionTitle: String = ""

    var body: some View {
        VStack(spacing: 8) {
            Text(title).font(.headline)
            Text(detail)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let action {
                Button(actionTitle, action: action)
                    .buttonStyle(.borderedProminent)
                    .padding(.top, 4)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 28)
        .padding(.horizontal, 24)
    }
}

struct ErrorBanner: View {
    var text: String
    var body: some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.red)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .background(Color.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
    }
}

/// A labelled field, the way the detail screens lay things out.
struct Field<Content: View>: View {
    var label: String
    @ViewBuilder var content: Content

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(label)
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(width: 96, alignment: .trailing)
            content
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
