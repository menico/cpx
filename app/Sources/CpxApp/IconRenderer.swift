// The menu bar icon: a small meter.
//
// Two lanes — session above, weekly below — each filled by how much
// remains, so a full bar means a fresh window. The image is a template, so
// macOS tints it for light and dark menu bars; a stale reading dims it and
// a provider incident adds a dot.

import AppKit
import CpxKit

struct IconInput: Equatable {
    /// Percent remaining in the session lane, or nil when unknown.
    var primary: Double?
    /// Percent remaining in the weekly lane, or nil when unknown.
    var secondary: Double?
    var stale: Bool
    var incident: Bool
    var provider: Provider?
}

enum IconRenderer {
    static let size = NSSize(width: 18, height: 18)

    static func image(for input: IconInput) -> NSImage {
        let image = NSImage(size: size, flipped: false) { _ in
            draw(input)
            return true
        }
        image.isTemplate = true
        return image
    }

    private static func draw(_ input: IconInput) {
        let base = NSColor.black
        let trackAlpha: CGFloat = input.stale ? 0.18 : 0.28
        let fillAlpha: CGFloat = input.stale ? 0.55 : 1.0
        let radius: CGFloat = input.provider == .codex ? 3 : 1

        // Lanes in points on an 18-point canvas, origin bottom-left.
        let hasSecondary = input.secondary != nil
        let top = hasSecondary ? NSRect(x: 1.5, y: 9.5, width: 15, height: 6) : NSRect(x: 1.5, y: 5, width: 15, height: 8)
        let bottom = NSRect(x: 1.5, y: 2.5, width: 15, height: 4)

        lane(top, remaining: input.primary, radius: radius, base: base, trackAlpha: trackAlpha, fillAlpha: fillAlpha)
        if hasSecondary {
            lane(bottom, remaining: input.secondary, radius: radius, base: base, trackAlpha: trackAlpha, fillAlpha: fillAlpha)
        }

        if input.incident {
            let dot = NSRect(x: 18 - 4.5, y: 18 - 4.5, width: 4, height: 4)
            // Punch a halo out of whatever is under the dot, so it reads on a full bar.
            if let context = NSGraphicsContext.current {
                context.saveGraphicsState()
                context.compositingOperation = .copy
                NSColor.clear.setFill()
                NSBezierPath(ovalIn: dot.insetBy(dx: -1, dy: -1)).fill()
                context.restoreGraphicsState()
            }
            base.withAlphaComponent(fillAlpha).setFill()
            NSBezierPath(ovalIn: dot).fill()
        }
    }

    private static func lane(_ rect: NSRect, remaining: Double?, radius: CGFloat, base: NSColor, trackAlpha: CGFloat, fillAlpha: CGFloat) {
        let track = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
        base.withAlphaComponent(trackAlpha).setFill()
        track.fill()

        guard let remaining else {
            // Unknown: a hollow lane.
            base.withAlphaComponent(trackAlpha + 0.1).setStroke()
            track.lineWidth = 1
            track.stroke()
            return
        }
        let fraction = CGFloat(max(0, min(100, remaining)) / 100)
        let width = (rect.width * fraction).rounded(.toNearestOrEven)
        guard width > 0 else { return }
        NSGraphicsContext.saveGraphicsState()
        track.addClip()
        base.withAlphaComponent(fillAlpha).setFill()
        NSRect(x: rect.minX, y: rect.minY, width: width, height: rect.height).fill()
        NSGraphicsContext.restoreGraphicsState()
    }
}
