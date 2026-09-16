// Numbers as people read them.

import Foundation

public enum Format {
    /// "12%" — clamped to 0...100 and rounded.
    public static func percent(_ value: Double) -> String {
        "\(Int(min(100, max(0, value)).rounded()))%"
    }

    /// "in 2h 05m", "in 3d 4h", "now".
    public static func countdown(to date: Date, from now: Date = Date()) -> String {
        let seconds = Int(date.timeIntervalSince(now))
        if seconds <= 0 { return "now" }
        let minutes = seconds / 60
        if minutes < 60 { return "in \(minutes)m" }
        let hours = minutes / 60
        if hours < 48 { return String(format: "in %dh %02dm", hours, minutes % 60) }
        return "in \(hours / 24)d \(hours % 24)h"
    }

    /// "Tue 14:05" or "14:05" for today.
    public static func clock(_ date: Date, now: Date = Date(), calendar: Calendar = .current) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.locale = Locale.current
        if calendar.isDate(date, inSameDayAs: now) {
            formatter.setLocalizedDateFormatFromTemplate("HHmm")
        } else {
            formatter.setLocalizedDateFormatFromTemplate("EEE HHmm")
        }
        return formatter.string(from: date)
    }

    /// "$12.34", "$0.50", "$1,234".
    public static func money(_ usd: Double) -> String {
        if usd >= 1000 {
            let formatter = NumberFormatter()
            formatter.numberStyle = .decimal
            formatter.maximumFractionDigits = 0
            formatter.roundingMode = .halfUp
            return "$" + (formatter.string(from: NSNumber(value: usd)) ?? String(Int(usd)))
        }
        return String(format: "$%.2f", usd)
    }

    /// "1.2M", "34k", "980".
    public static func tokens(_ count: Int) -> String {
        if count >= 1_000_000 { return String(format: "%.1fM", Double(count) / 1_000_000) }
        if count >= 10_000 { return "\(count / 1000)k" }
        if count >= 1000 { return String(format: "%.1fk", Double(count) / 1000) }
        return "\(count)"
    }

    /// "5 hours", "1 week", "3 days" for a window length.
    public static func windowLength(minutes: Int) -> String {
        if minutes % (7 * 24 * 60) == 0 {
            let weeks = minutes / (7 * 24 * 60)
            return weeks == 1 ? "1 week" : "\(weeks) weeks"
        }
        if minutes % (24 * 60) == 0 {
            let days = minutes / (24 * 60)
            return days == 1 ? "1 day" : "\(days) days"
        }
        if minutes % 60 == 0 {
            let hours = minutes / 60
            return hours == 1 ? "1 hour" : "\(hours) hours"
        }
        return "\(minutes) min"
    }

    /// "just now", "4 min ago", "2 h ago".
    public static func age(of date: Date, now: Date = Date()) -> String {
        let seconds = Int(now.timeIntervalSince(date))
        if seconds < 60 { return "just now" }
        if seconds < 3600 { return "\(seconds / 60) min ago" }
        if seconds < 86400 { return "\(seconds / 3600) h ago" }
        return "\(seconds / 86400) d ago"
    }
}
