import AppKit

/// The menu bar face: the sleep-wave glyph, the hours since the last cycle
/// that produced something, and one dot only when something needs the owner.
enum StatusFace {
    /// Two stacked sine waves, the lower one fainter: a sleeper's breathing.
    /// A template image, so the menu bar tints it for dark, light and
    /// highlighted states.
    static let glyph: NSImage = {
        let size = NSSize(width: 18, height: 16)
        let img = NSImage(size: size, flipped: false) { _ in
            func wave(y: CGFloat, amp: CGFloat, alpha: CGFloat, width w: CGFloat) {
                let p = NSBezierPath()
                p.lineWidth = w
                p.lineCapStyle = .round
                p.lineJoinStyle = .round
                let x0: CGFloat = 1.5, x1: CGFloat = 16.5
                p.move(to: NSPoint(x: x0, y: y))
                let steps = 40
                for i in 1...steps {
                    let t = CGFloat(i) / CGFloat(steps)
                    let x = x0 + (x1 - x0) * t
                    p.line(to: NSPoint(x: x, y: y + amp * sin(t * 2 * .pi)))
                }
                NSColor.black.withAlphaComponent(alpha).setStroke()
                p.stroke()
            }
            wave(y: 9.6, amp: 3.0, alpha: 1.0, width: 1.6)
            wave(y: 4.2, amp: 1.7, alpha: 0.55, width: 1.4)
            return true
        }
        img.isTemplate = true
        img.accessibilityDescription = "i-dream"
        return img
    }()

    static func dot(_ color: NSColor) -> NSImage {
        let img = NSImage(size: NSSize(width: 6, height: 6), flipped: false) { r in
            color.setFill(); NSBezierPath(ovalIn: r).fill(); return true
        }
        return img
    }

    /// The text beside the glyph. Quiet: the hours alone. Waiting on the
    /// owner: an amber dot and the count of items. Broken: a red dot, and the
    /// number is the age of what broke.
    static func title(_ r: Record?, reading: ReadingState) -> NSAttributedString {
        let font = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .regular)
        let out = NSMutableAttributedString()
        func text(_ s: String) { out.append(NSAttributedString(string: s, attributes: [.font: font])) }
        func pip(_ c: NSColor) {
            let a = NSTextAttachment()
            a.image = dot(c)
            // Centre the dot on the digits' x-height, not the baseline.
            a.bounds = NSRect(x: 0, y: (font.xHeight - 6) / 2 + 0.5, width: 6, height: 6)
            out.append(NSAttributedString(attachment: a))
        }
        guard let r else {
            if case .unavailable = reading { text(" —") } else if case .failed = reading { text(" !") }
            return out
        }
        let broken = r.statusSev == .bad
        if broken {
            let age: TimeInterval? = !r.daemonRunning ? r.lastCycle.map { r.now.timeIntervalSince($0) } : r.dead.first?.writtenAge
            text(" " + (age.map { "\(max(1, Int($0 / 86400)))d" } ?? "—") + " ")
            pip(.systemRed)
            return out
        }
        let h = r.hoursSinceProductive
        text(" " + (h.map { $0 < 48 ? "\(Int($0))h" : "\(Int($0 / 24))d" } ?? "—"))
        if r.awaitingItems > 0 {
            text(" ")
            pip(NSColor(srgbRed: 0.85, green: 0.64, blue: 0.11, alpha: 1))
            text(" \(r.awaitingItems)")
        }
        return out
    }

    static func tooltip(_ r: Record?) -> String {
        guard let r else { return "i-dream" }
        var parts = ["last productive cycle " + ageText(r.lastProductive.map { r.now.timeIntervalSince($0) }) + " ago"]
        if r.awaitingItems > 0 { parts.append("\(plural(r.awaitingItems, "item")) waiting on you") }
        if !r.dead.isEmpty { parts.append("\(plural(r.dead.count, "dead source"))") }
        if !r.daemonRunning { parts.append("daemon \(r.daemonWord)") }
        return parts.joined(separator: " · ")
    }
}
