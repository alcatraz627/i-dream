
// ─── HUD content view ────────────────────────────────────────────────────────
/// Custom contentView for the floating HUD panel. Forwards right-clicks
/// to the BarDelegate so the menubar menu (theMenu) is shown on right-click,
/// matching the user's expectation that right-click === left-click on menubar.
private final class HUDContentView: NSView {
    weak var delegate: BarDelegate?
    override func rightMouseDown(with event: NSEvent) {
        delegate?.popUpHUDContextMenu(with: event, from: self)
    }
}

// ─── HUD hover-aware button ──────────────────────────────────────────────────
/// NSButton subclass with no chrome by default — paints a subtle rounded
/// background only while the cursor is over it, and pushes its `hoverLabel`
/// into the BarDelegate's `hudHoverLabel` text field on enter / clears on
/// exit. Used for the action button row at the bottom of the floating HUD.
private final class HoverButton: NSButton {
    var hoverLabel: String  = ""
    var tintColor:  NSColor = .systemCyan
    weak var delegate: BarDelegate?
    private var trackingArea: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        isBordered = false
        bezelStyle = .regularSquare
        layer?.cornerRadius = 6
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) not used") }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let ta = trackingArea { removeTrackingArea(ta) }
        let ta = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self, userInfo: nil)
        addTrackingArea(ta)
        trackingArea = ta
    }

    // Clickable things show the hand — the affordance the sidebar footer
    // buttons and theme icons were missing.
    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
        delegate?.setHUDHoverLabel(hoverLabel, color: tintColor)
    }
    override func mouseExited(with event: NSEvent) {
        isHovered = false
        delegate?.setHUDHoverLabel("", color: .tertiaryLabelColor)
    }

    override func draw(_ dirtyRect: NSRect) {
        if isHovered {
            tintColor.withAlphaComponent(0.18).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill()
        }
        super.draw(dirtyRect)
    }
}

// ─── Mini bar-chart view for HUD token history ────────────────────────────────
/// Draws a compact histogram of recent token-usage values using NSBezierPath.
/// Bars are colored cyan→yellow→orange based on relative load; newest bar is brightest.
///
/// Interaction: hovering a bar pushes a "tokens · timeAgo" string into the
/// HUD hover label; clicking a bar fires `delegate?.barChartClicked(at:entry:)`
/// which currently opens the dashboard. Cursor switches to a pointing hand
/// over hovered bars.
private class MiniBarChartView: NSView {
    var values: [Int] = [] { didSet { needsDisplay = true } }
    /// Parallel array — entries[i] is the JournalEntry the bar at index i was
    /// drawn from. Optional so callers (legacy or test) can still drive the
    /// chart with just values.
    var entries: [JournalEntry] = []
    weak var delegate: BarDelegate?
    private var trackingArea: NSTrackingArea?
    private var hoveredIndex: Int? = nil { didSet { if oldValue != hoveredIndex { needsDisplay = true } } }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let ta = trackingArea { removeTrackingArea(ta) }
        let ta = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways, .inVisibleRect],
            owner: self, userInfo: nil)
        addTrackingArea(ta)
        trackingArea = ta
    }
    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .pointingHand)
    }

    /// Translate a point in view coordinates → bar index.
    private func barIndex(at point: NSPoint) -> Int? {
        guard !values.isEmpty, point.x >= 0, point.x < bounds.width else { return nil }
        let n   = values.count
        let gap: CGFloat = 2.0
        let barW = max(3, (bounds.width - gap * CGFloat(n - 1)) / CGFloat(n))
        let idx = Int(point.x / (barW + gap))
        return (0..<n).contains(idx) ? idx : nil
    }

    override func mouseMoved(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let idx = barIndex(at: p)
        hoveredIndex = idx
        if let i = idx, i < entries.count {
            let e = entries[i]
            let tokStr = e.tokensUsed >= 1000
                ? String(format: "%.1fk", Double(e.tokensUsed) / 1000)
                : "\(e.tokensUsed)"
            let when = timeAgo(e.timestamp)
            delegate?.setHUDHoverLabel("\(tokStr) tokens · \(when) — click for details", color: .systemCyan)
        } else {
            delegate?.setHUDHoverLabel("", color: .tertiaryLabelColor)
        }
    }
    override func mouseExited(with event: NSEvent) {
        hoveredIndex = nil
        delegate?.setHUDHoverLabel("", color: .tertiaryLabelColor)
    }
    override func mouseDown(with event: NSEvent) {
        // Single-click: select / show details on hover label (already wired via mouseMoved).
        // Double-click only opens the dashboard — single click was too aggressive
        // (the bar chart sits next to controls users may want to tap).
        guard event.clickCount >= 2 else { return }
        let p = convert(event.locationInWindow, from: nil)
        guard let idx = barIndex(at: p) else { return }
        let entry = (idx < entries.count) ? entries[idx] : nil
        delegate?.barChartClicked(at: idx, entry: entry)
    }

    override func draw(_ dirtyRect: NSRect) {
        guard !values.isEmpty else { return }
        let n      = values.count
        let maxVal = values.max() ?? 1
        let gap:  CGFloat = 2.0
        let barW: CGFloat = max(3, (bounds.width - gap * CGFloat(n - 1)) / CGFloat(n))

        for (i, v) in values.enumerated() {
            let fraction  = maxVal > 0 ? CGFloat(v) / CGFloat(maxVal) : 0
            let barH      = max(2, fraction * (bounds.height - 4))
            let x         = CGFloat(i) * (barW + gap)
            let recency   = CGFloat(i) / max(1, CGFloat(n - 1))   // 0=oldest, 1=newest
            var alpha     = 0.25 + recency * 0.70
            if hoveredIndex == i { alpha = 1.0 }

            let color: NSColor
            if fraction > 0.75      { color = NSColor.systemOrange.withAlphaComponent(alpha) }
            else if fraction > 0.45 { color = NSColor.systemYellow.withAlphaComponent(alpha) }
            else                    { color = NSColor.systemCyan.withAlphaComponent(alpha) }

            let barRect = NSRect(x: x, y: 2, width: max(1, barW - 1), height: barH)
            let path    = NSBezierPath(roundedRect: barRect, xRadius: 1.5, yRadius: 1.5)
            color.setFill()
            path.fill()
        }
    }
}

// ─── Calendar heat map ───────────────────────────────────────────────────────
/// GitHub-style contribution grid showing consolidation activity by day.
/// Each cell represents one day; intensity is based on token usage relative to max.
private class CalendarHeatMapView: NSView {
    /// (date, tokenCount) pairs — one per consolidation cycle
    var entries: [(date: Date, tokens: Int)] = [] { didSet { needsDisplay = true } }

    private let cellSize: CGFloat = 12
    private let gap: CGFloat      = 3
    private let weeksToShow       = 16   // ~4 months

    override var intrinsicContentSize: NSSize {
        let w = CGFloat(weeksToShow) * (cellSize + gap) + 40  // +40 for day labels
        let h = 7 * (cellSize + gap) + 20                     // +20 for month labels
        return NSSize(width: w, height: h)
    }

    override func draw(_ dirtyRect: NSRect) {
        let cal = Calendar.current
        let today = cal.startOfDay(for: Date())

        // Build a day → token total map
        var dayMap: [Date: Int] = [:]
        for e in entries {
            let day = cal.startOfDay(for: e.date)
            dayMap[day, default: 0] += e.tokens
        }
        let maxTokens = max(1, dayMap.values.max() ?? 1)

        // Calculate start date: go back weeksToShow weeks from the end of this week
        let todayWeekday = cal.component(.weekday, from: today) // 1=Sun
        let daysToEndOfWeek = 7 - todayWeekday
        let endDate = cal.date(byAdding: .day, value: daysToEndOfWeek, to: today)!
        let startDate = cal.date(byAdding: .weekOfYear, value: -weeksToShow, to: endDate)!

        let originX: CGFloat = 24  // leave room for day-of-week labels
        let originY: CGFloat = 0

        // Day-of-week labels
        let dayLabels = ["", "M", "", "W", "", "F", ""]
        let labelFont = NSFont.systemFont(ofSize: 9)
        let labelAttrs: [NSAttributedString.Key: Any] = [
            .font: labelFont, .foregroundColor: NSColor.tertiaryLabelColor]
        for (i, lbl) in dayLabels.enumerated() {
            guard !lbl.isEmpty else { continue }
            let y = originY + CGFloat(6 - i) * (cellSize + gap)
            NSAttributedString(string: lbl, attributes: labelAttrs)
                .draw(at: CGPoint(x: 2, y: y + 1))
        }

        // Draw cells
        var currentDate = startDate
        var week = 0
        var lastMonth = -1

        while currentDate <= endDate {
            let weekday = cal.component(.weekday, from: currentDate) - 1 // 0=Sun
            let row = 6 - weekday  // Sun at bottom, Sat at top
            let x = originX + CGFloat(week) * (cellSize + gap)
            let y = originY + CGFloat(row) * (cellSize + gap)

            let tokens = dayMap[currentDate] ?? 0
            let intensity = Double(tokens) / Double(maxTokens)

            let color: NSColor
            if tokens == 0 {
                color = NSColor.separatorColor.withAlphaComponent(0.15)
            } else if intensity > 0.75 {
                color = NSColor.systemGreen.withAlphaComponent(0.9)
            } else if intensity > 0.45 {
                color = NSColor.systemGreen.withAlphaComponent(0.6)
            } else if intensity > 0.2 {
                color = NSColor.systemGreen.withAlphaComponent(0.35)
            } else {
                color = NSColor.systemGreen.withAlphaComponent(0.18)
            }

            let rect = NSRect(x: x, y: y, width: cellSize, height: cellSize)
            let path = NSBezierPath(roundedRect: rect, xRadius: 2, yRadius: 2)
            color.setFill()
            path.fill()

            // Month label at start of each new month
            let month = cal.component(.month, from: currentDate)
            let dayOfMonth = cal.component(.day, from: currentDate)
            if month != lastMonth && dayOfMonth <= 7 {
                lastMonth = month
                let fmt = DateFormatter()
                fmt.dateFormat = "MMM"
                let monthStr = fmt.string(from: currentDate)
                let monthY = originY + 7 * (cellSize + gap) + 2
                NSAttributedString(string: monthStr, attributes: labelAttrs)
                    .draw(at: CGPoint(x: x, y: monthY))
            }

            // Advance
            currentDate = cal.date(byAdding: .day, value: 1, to: currentDate)!
            if cal.component(.weekday, from: currentDate) == 1 { week += 1 }
        }
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

// ═══════════════════ Dashboard v3 — SwiftUI pane layer ═══════════════════
// Browse is the single paradigm for every knowledge type. It reads the
// engine's honest derived views (~/.claude/i-dream/derived/views/) so the
// ×N cluster badges, item ages, and "showing N of M" totals come from the
// data layer, not per-tab formatting. SwiftUI lives ONLY inside the
// dashboard window — menus stay pure AppKit (the sibling widget's A-3
// lesson: hosting SwiftUI in NSMenuItem views crashes).
// Design: docs/23-widget-v3-plan.md Stage 2.
