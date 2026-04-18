// i-dream-bar.swift  (v4)
// Standalone macOS menu-bar widget for the i-dream consolidation daemon.
//
// Compile & run:
//   bash tools/menubar/build.sh              # build + launch
//   bash tools/menubar/build.sh --install    # build + register LaunchAgent
//   bash tools/menubar/build.sh --status     # show running state

import AppKit
import Foundation

// ─── Paths ────────────────────────────────────────────────────────────────────

private let home      = FileManager.default.homeDirectoryForCurrentUser.path
private let subDir    = home + "/.claude/subconscious"
private let statePath = subDir + "/state.json"
private let pidPath   = subDir + "/daemon.pid"
private let iDream    = home + "/.cargo/bin/i-dream"
private let debugLog  = "/tmp/i-dream-bar.log"
private let tracesDir   = subDir + "/dreams/traces"
private let activityFile = subDir + "/.last-activity"
private let signalsFile  = subDir + "/logs/signals.jsonl"

/// Falls back to the mtime of .last-activity since state.json always has
/// last_activity = null (the daemon writes the file but not the JSON field).
private func lastActivityDate() -> Date? {
    let attrs = try? FileManager.default.attributesOfItem(atPath: activityFile)
    return attrs?[.modificationDate] as? Date
}

/// Count of user-signal entries written by the UserPromptSubmit hook.
private func signalsCount() -> Int {
    guard let content = try? String(contentsOfFile: signalsFile, encoding: .utf8) else { return 0 }
    return content.components(separatedBy: "\n").filter { !$0.isEmpty }.count
}

private func todayLogPath() -> String {
    let fmt = DateFormatter()
    fmt.dateFormat = "yyyy-MM-dd"
    return subDir + "/logs/i-dream.log." + fmt.string(from: Date())
}

/// Returns today's log if it exists, otherwise the most recent log file.
private func bestLogPath() -> String {
    let today = todayLogPath()
    if FileManager.default.fileExists(atPath: today) { return today }
    let logsDir = subDir + "/logs"
    let files   = (try? FileManager.default.contentsOfDirectory(atPath: logsDir)) ?? []
    if let latest = files.filter({ $0.hasPrefix("i-dream.log.") }).sorted().last {
        return logsDir + "/" + latest
    }
    return today
}

// ─── Debug logging ────────────────────────────────────────────────────────────

private func dlog(_ msg: String) {
    let ts   = ISO8601DateFormatter().string(from: Date())
    let line = "  \(ts) [bar] \(msg)\n"
    guard let data = line.data(using: .utf8) else { return }
    if let fh = FileHandle(forWritingAtPath: debugLog) {
        fh.seekToEndOfFile(); fh.write(data); fh.closeFile()
    } else {
        try? data.write(to: URL(fileURLWithPath: debugLog))
    }
}

// ─── Date formatting ──────────────────────────────────────────────────────────

private func isoDate(_ s: String?) -> Date? {
    guard let s = s else { return nil }
    let fmt1 = ISO8601DateFormatter()
    fmt1.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let d = fmt1.date(from: s) { return d }
    return ISO8601DateFormatter().date(from: s)
}

private func fmtDate(_ s: String?) -> String {
    guard let date = isoDate(s) else { return "–" }
    return fmtDateDirect(date)
}

private func fmtDateDirect(_ date: Date) -> String {
    let fmt = DateFormatter()
    fmt.dateFormat = "MMM d, h:mm a"
    return fmt.string(from: date)
}

private func timeAgo(_ s: String?) -> String {
    guard let date = isoDate(s) else { return "–" }
    let d = Date().timeIntervalSince(date)
    switch d {
    case ..<60:    return "just now"
    case ..<3600:  return "\(Int(d / 60))m ago"
    case ..<86400: return "\(Int(d / 3600))h ago"
    default:       return "\(Int(d / 86400))d ago"
    }
}

private func fmtDateWithAge(_ s: String?) -> String {
    guard let _ = isoDate(s) else { return "–" }
    return "\(fmtDate(s))  (\(timeAgo(s)))"
}

private func fmtNum(_ n: Int) -> String {
    switch n {
    case 1_000_000...: return String(format: "%.1fM", Double(n) / 1_000_000)
    case 1_000...:     return String(format: "%.0fK", Double(n) / 1_000)
    default:           return "\(n)"
    }
}

private func fmtBytes(_ n: UInt64) -> String {
    switch n {
    case 1_073_741_824...: return String(format: "%.1f GB", Double(n) / 1_073_741_824)
    case 1_048_576...:     return String(format: "%.1f MB", Double(n) / 1_048_576)
    case 1_024...:         return String(format: "%.0f KB", Double(n) / 1_024)
    default:               return "\(n) B"
    }
}

private func fmtElapsed(_ secs: TimeInterval) -> String {
    let s = Int(secs)
    return s < 60 ? "\(s)s" : "\(s / 60)m \(s % 60)s"
}

// ─── Data models ─────────────────────────────────────────────────────────────

private struct UsageLimitStatus: Codable {
    let outputTokens5h:    Int
    let outputTokens7d:    Int
    let limit5h:           Int
    let limit7d:           Int
    let pct5h:             Double
    let pct7d:             Double
    let overWarnThreshold: Bool
    let checkedAt:         String
    enum CodingKeys: String, CodingKey {
        case outputTokens5h    = "output_tokens_5h"
        case outputTokens7d    = "output_tokens_7d"
        case limit5h           = "limit_5h"
        case limit7d           = "limit_7d"
        case pct5h             = "pct_5h"
        case pct7d             = "pct_7d"
        case overWarnThreshold = "over_warn_threshold"
        case checkedAt         = "checked_at"
    }

    /// Human-readable warning line for menus and dialogs.
    var warningLine: String {
        var parts: [String] = []
        if limit5h > 0 { parts.append("5h: \(Int(pct5h * 100))% of \(limit5h / 1000)k tokens") }
        if limit7d > 0 { parts.append("7d: \(Int(pct7d * 100))% of \(limit7d / 1000)k tokens") }
        return parts.joined(separator: "  ·  ")
    }
}

private struct DaemonState: Codable {
    let lastActivity:      String?
    let lastConsolidation: String?
    let totalCycles:       Int
    let totalTokensUsed:   Int
    let usage:             UsageLimitStatus?
    enum CodingKeys: String, CodingKey {
        case lastActivity      = "last_activity"
        case lastConsolidation = "last_consolidation"
        case totalCycles       = "total_cycles"
        case totalTokensUsed   = "total_tokens_used"
        case usage             = "usage"
    }
}

private struct BoardData {
    let dreamsProcessed:  Int
    let metacogProcessed: Int
    let dreamsPatterns:   Int
    let associations:     Int
    let metacogAudits:    Int
    let lastError:        String?
}

private struct Pattern: Codable {
    let id:        String?
    let pattern:    String
    let valence:    String
    let confidence: Double
    let category:   String
    let firstSeen:  String?
    enum CodingKeys: String, CodingKey {
        case id, pattern, valence, confidence, category
        case firstSeen = "first_seen"
    }
}

private struct JournalEntry: Codable {
    let id:                String?
    let timestamp:         String
    let sessionsAnalyzed:  Int
    let patternsExtracted: Int
    let associationsFound: Int
    let insightsPromoted:  Int
    let tokensUsed:        Int
    enum CodingKeys: String, CodingKey {
        case id, timestamp
        case sessionsAnalyzed  = "sessions_analyzed"
        case patternsExtracted = "patterns_extracted"
        case associationsFound = "associations_found"
        case insightsPromoted  = "insights_promoted"
        case tokensUsed        = "tokens_used"
    }
}

private struct Association: Codable {
    let id:            String
    let hypothesis:    String
    let confidence:    Double
    let actionable:    Bool
    let suggestedRule: String?
    let patternsLinked: [String]?
    enum CodingKeys: String, CodingKey {
        case id, hypothesis, confidence, actionable
        case suggestedRule  = "suggested_rule"
        case patternsLinked = "patterns_linked"
    }
}

private struct MetacogAudit: Codable {
    let calibrationScore:     Double?
    let overconfidentCount:   Int?
    let underconfidentCount:  Int?
    let wellCalibratedCount:  Int?
    let biasesDetected:       [String]?
    let recommendations:      [String]?
    enum CodingKeys: String, CodingKey {
        case calibrationScore    = "calibration_score"
        case overconfidentCount  = "overconfident_count"
        case underconfidentCount = "underconfident_count"
        case wellCalibratedCount = "well_calibrated_count"
        case biasesDetected      = "biases_detected"
        case recommendations
    }
    /// True only when at least the core calibration data is present.
    var hasContent: Bool { calibrationScore != nil || biasesDetected != nil }
}

/// Outer wrapper: the module stores { "response": "```json\n{...}\n```", "sessions": [...] }
private struct MetacogAuditFile: Codable {
    let response: String?
}

// ─── Rich text builder ────────────────────────────────────────────────────────

/// Fluent builder for NSAttributedString with semantic styling methods.
/// Converts plain-string content from detail views into visually structured text.
final class RichText {
    private let buf = NSMutableAttributedString()

    @discardableResult func header(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 16, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func subheader(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 14, weight: .medium),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func body(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func dim(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: NSColor.secondaryLabelColor,
        ])); return self
    }
    @discardableResult func mono(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func ok(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemGreen,
        ])); return self
    }
    @discardableResult func warn(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemOrange,
        ])); return self
    }
    @discardableResult func err(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemRed,
        ])); return self
    }
    @discardableResult func accent(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemBlue,
        ])); return self
    }
    @discardableResult func divider() -> RichText {
        buf.append(NSAttributedString(string: String(repeating: "─", count: 60) + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 10),
            .foregroundColor: NSColor.separatorColor,
        ])); return self
    }
    @discardableResult func spacer() -> RichText {
        buf.append(NSAttributedString(string: "\n")); return self
    }
    /// Clickable blue subheader — link value is passed to the text view delegate on click.
    @discardableResult func linkSubheader(_ text: String, linkValue: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font:            NSFont.systemFont(ofSize: 14, weight: .medium),
            .foregroundColor: NSColor.systemBlue,
            .link:            linkValue as AnyObject,
        ])); return self
    }
    /// Arbitrary color line — used for heat-map value rows in the dream journal.
    @discardableResult func coloredLine(_ text: String, color: NSColor) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: color,
        ])); return self
    }
    /// Append a pre-built attributed string (no trailing newline added).
    @discardableResult func raw(_ attributedString: NSAttributedString) -> RichText {
        buf.append(attributedString); return self
    }
    func build() -> NSAttributedString { buf }
}

// ─── Journal link delegate ───────────────────────────────────────────────────
// Thin NSTextViewDelegate wrapper that intercepts clicks on link-attributed text
// (NSLinkAttributeName with a String value = journal entry timestamp).
// Avoids making BarDelegate globally conform to NSTextViewDelegate.

private class JournalLinkDelegate: NSObject, NSTextViewDelegate {
    let onLink: (String) -> Void
    init(_ onLink: @escaping (String) -> Void) { self.onLink = onLink; super.init() }
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        if let ts = link as? String { onLink(ts); return true }
        return false
    }
}

// Intercepts insight feedback link clicks ("insight-up:<id>" / "insight-down:<id>").
private class InsightFeedbackDelegate: NSObject, NSTextViewDelegate {
    let onFeedback: (String, String) -> Void   // (insightId, "up"|"down")
    init(_ onFeedback: @escaping (String, String) -> Void) { self.onFeedback = onFeedback; super.init() }
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        guard let linkStr = link as? String else { return false }
        if linkStr.hasPrefix("insight-up:") {
            onFeedback(String(linkStr.dropFirst("insight-up:".count)), "up")
            return true
        }
        if linkStr.hasPrefix("insight-down:") {
            onFeedback(String(linkStr.dropFirst("insight-down:".count)), "down")
            return true
        }
        return false
    }
}

// ─── Pattern network view ─────────────────────────────────────────────────────
// Ring-of-rings layout: categories on outer circle, their patterns on inner circles.
// Interactive: pan (drag), zoom (pinch / scroll-wheel), hover (faint connection lines),
// click to inspect a node, double-click to reset view.

final class PatternGraphView: NSView {
    private struct Node {
        let pattern:  Pattern
        var position: CGPoint
        let radius:   CGFloat
    }

    private struct CategoryArc {
        let name:     String
        let midAngle: CGFloat  // radians, graph coordinate space
        let ringR:    CGFloat
    }

    private var nodes:          [Node]       = []
    private var categoryArcs:   [CategoryArc] = []
    private var popover:        NSPopover?
    private var selectedIdx:    Int?      = nil
    private var hoveredIdx:     Int?      = nil
    private var trackingArea:   NSTrackingArea?

    // Pan / zoom state
    private var panOffset:      CGPoint   = .zero
    private var zoomScale:      CGFloat   = 1.0
    private let zoomMin:        CGFloat   = 0.25
    private let zoomMax:        CGFloat   = 4.0

    // Click-vs-drag detection
    private var mouseDownLoc:   CGPoint?  = nil
    private var isDragging:     Bool      = false
    private let dragThreshold:  CGFloat   = 5.0

    fileprivate init(frame: NSRect, patterns: [Pattern]) {
        super.init(frame: frame)
        wantsLayer = true
        buildLayout(patterns)
    }
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { true }

    private func buildLayout(_ patterns: [Pattern]) {
        let cx = bounds.midX
        let cy = bounds.midY
        let ringR: CGFloat = min(bounds.width, bounds.height) * 0.44

        let categories = Array(Set(patterns.map { $0.category })).sorted()
        let catCount   = max(categories.count, 1)
        var byCategory: [String: [Pattern]] = [:]
        for p in patterns { byCategory[p.category, default: []].append(p) }
        // Sort within category: highest confidence first
        for cat in byCategory.keys { byCategory[cat]?.sort { $0.confidence > $1.confidence } }

        // Each category gets a proportional arc; 6° gap between adjacent categories
        let gapRad:       CGFloat = 6.0 * .pi / 180.0
        let totalGap:     CGFloat = gapRad * CGFloat(catCount)
        let availableArc: CGFloat = 2 * .pi - totalGap
        let total: CGFloat = CGFloat(max(patterns.count, 1))

        nodes        = []
        categoryArcs = []
        var startAngle: CGFloat = -.pi / 2   // 12 o'clock

        for cat in categories {
            let pats = byCategory[cat] ?? []
            guard !pats.isEmpty else { continue }
            let catArc = availableArc * (CGFloat(pats.count) / total)
            let midAngle = startAngle + catArc / 2

            for (j, p) in pats.enumerated() {
                // Distribute nodes evenly across this category's arc slice
                let t: CGFloat = pats.count == 1 ? 0.5 :
                    CGFloat(j) / CGFloat(pats.count - 1)
                let angle = startAngle + t * catArc
                let x = cx + ringR * cos(angle)
                let y = cy + ringR * sin(angle)
                let r: CGFloat = 5.0 + CGFloat(p.confidence) * 7.0  // 5–12 pt
                nodes.append(Node(pattern: p, position: CGPoint(x: x, y: y), radius: r))
            }

            categoryArcs.append(CategoryArc(name: cat, midAngle: midAngle, ringR: ringR))
            startAngle += catArc + gapRad
        }
    }

    // MARK: – Hover tracking area

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let ta = trackingArea { removeTrackingArea(ta) }
        trackingArea = NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self, userInfo: nil)
        addTrackingArea(trackingArea!)
    }

    override func mouseMoved(with event: NSEvent) {
        let gp   = viewToGraph(convert(event.locationInWindow, from: nil))
        let prev = hoveredIdx
        hoveredIdx = hitNode(at: gp)
        if hoveredIdx != prev { needsDisplay = true }
    }

    override func mouseExited(with event: NSEvent) {
        if hoveredIdx != nil { hoveredIdx = nil; needsDisplay = true }
    }

    // MARK: – Zoom (pinch gesture + scroll wheel)

    override func scrollWheel(with event: NSEvent) {
        if event.hasPreciseScrollingDeltas {
            // Trackpad two-finger swipe → pan
            panOffset.x += event.scrollingDeltaX
            panOffset.y += event.scrollingDeltaY
        } else {
            // Mouse wheel → zoom
            let factor: CGFloat = event.deltaY > 0 ? 1.10 : 0.91
            zoomScale = max(zoomMin, min(zoomMax, zoomScale * factor))
        }
        needsDisplay = true
    }

    override func magnify(with event: NSEvent) {
        zoomScale = max(zoomMin, min(zoomMax, zoomScale * (1.0 + event.magnification)))
        needsDisplay = true
    }

    // MARK: – Pan (drag) + click detection

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            // Double-click → reset to default view
            panOffset = .zero; zoomScale = 1.0
            popover?.close(); popover = nil
            selectedIdx = nil
            needsDisplay = true
            return
        }
        window?.makeFirstResponder(self)
        mouseDownLoc = convert(event.locationInWindow, from: nil)
        isDragging   = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard let start = mouseDownLoc else { return }
        let cur = convert(event.locationInWindow, from: nil)
        if !isDragging && hypot(cur.x - start.x, cur.y - start.y) > dragThreshold {
            isDragging = true
        }
        if isDragging {
            panOffset.x += event.deltaX
            panOffset.y -= event.deltaY   // AppKit y-axis is bottom-up; deltaY is screen-down positive
            needsDisplay = true
        }
    }

    override func mouseUp(with event: NSEvent) {
        defer { mouseDownLoc = nil; isDragging = false }
        guard !isDragging else { return }
        // It was a clean click — hit-test in graph space
        let gp = viewToGraph(convert(event.locationInWindow, from: nil))
        if let hit = hitNode(at: gp) {
            selectedIdx = hit
            needsDisplay = true
            showPopover(for: hit)
        } else {
            popover?.close(); popover = nil
            selectedIdx = nil
            needsDisplay = true
        }
    }

    // MARK: – Coordinate helpers

    /// Convert a point in view (screen) space to the underlying graph coordinate space.
    private func viewToGraph(_ pt: CGPoint) -> CGPoint {
        let cx = bounds.midX, cy = bounds.midY
        return CGPoint(
            x: (pt.x - panOffset.x - cx) / zoomScale + cx,
            y: (pt.y - panOffset.y - cy) / zoomScale + cy)
    }

    /// Hit-test: returns the index of the first node whose radius (+ 6pt padding) contains graphPt.
    private func hitNode(at graphPt: CGPoint) -> Int? {
        for (i, node) in nodes.enumerated() {
            if hypot(graphPt.x - node.position.x, graphPt.y - node.position.y) <= node.radius + 6 {
                return i
            }
        }
        return nil
    }

    private func nodeColor(_ p: Pattern) -> NSColor {
        return p.confidence >= 0.85 ? .systemGreen
             : p.confidence >= 0.65 ? .systemBlue
             : .secondaryLabelColor
    }

    // MARK: – Drawing

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }

        // Background (outside the transform so it always fills the view)
        NSColor.textBackgroundColor.setFill()
        NSBezierPath.fill(bounds)

        ctx.saveGState()

        // Apply pan + zoom — zoom is centred on the view's midpoint
        let cx = bounds.midX, cy = bounds.midY
        ctx.translateBy(x: panOffset.x + cx, y: panOffset.y + cy)
        ctx.scaleBy(x: zoomScale, y: zoomScale)
        ctx.translateBy(x: -cx, y: -cy)

        // ── Ring guide ────────────────────────────────────────────────────────
        // Faint circle so the ring "track" is always visible even at gap sections.
        if !nodes.isEmpty {
            let ringR = categoryArcs.first?.ringR ?? min(bounds.width, bounds.height) * 0.44
            ctx.setStrokeColor(NSColor.quaternaryLabelColor.cgColor)
            ctx.setLineWidth(0.5)
            ctx.strokeEllipse(in: CGRect(x: cx - ringR, y: cy - ringR,
                                         width: ringR * 2, height: ringR * 2))
        }

        // ── Connection lines ──────────────────────────────────────────────────
        // All edges hidden by default. On hover: show only the 15 nearest nodes
        // sorted by Euclidean distance, regardless of category — this prevents the
        // N×(N-1)/2 combinatorial blob when categories have 80+ members.
        if let hov = hoveredIdx {
            let hovPos = nodes[hov].position
            let hovCat = nodes[hov].pattern.category
            let nearest = (0 ..< nodes.count)
                .filter { $0 != hov }
                .sorted { hypot(nodes[$0].position.x - hovPos.x, nodes[$0].position.y - hovPos.y)
                        < hypot(nodes[$1].position.x - hovPos.x, nodes[$1].position.y - hovPos.y) }
                .prefix(15)
            for j in nearest {
                let sameCategory = nodes[j].pattern.category == hovCat
                let alpha: CGFloat = sameCategory ? 0.55 : 0.22
                let c1 = nodeColor(nodes[hov].pattern)
                let c2 = nodeColor(nodes[j].pattern)
                let blended = c1.blended(withFraction: 0.5, of: c2) ?? c1
                ctx.setStrokeColor(blended.withAlphaComponent(alpha).cgColor)
                ctx.setLineWidth(sameCategory ? 1.2 : 0.7)
                ctx.move(to: hovPos)
                ctx.addLine(to: nodes[j].position)
                ctx.strokePath()
            }
        }

        // ── Nodes ─────────────────────────────────────────────────────────────
        for (idx, node) in nodes.enumerated() {
            let p          = node.pattern
            let baseColor  = nodeColor(p)
            let isHovered  = idx == hoveredIdx
            let isSelected = idx == selectedIdx
            let fillAlpha: CGFloat = isSelected ? 1.0 : isHovered ? 0.92 : 0.72
            let r          = node.radius

            // Glow ring for hovered / selected nodes
            if isHovered || isSelected {
                ctx.setStrokeColor(baseColor.withAlphaComponent(isSelected ? 0.45 : 0.28).cgColor)
                ctx.setLineWidth(5.5)
                let gr = r + 4
                ctx.strokeEllipse(in: CGRect(x: node.position.x - gr, y: node.position.y - gr,
                                             width: gr * 2, height: gr * 2))
            }

            let rect = CGRect(x: node.position.x - r, y: node.position.y - r,
                              width: r * 2, height: r * 2)
            // Thin white halo so nodes stand out against dark ring arcs
            if !isHovered && !isSelected {
                ctx.setStrokeColor(NSColor.white.withAlphaComponent(0.25).cgColor)
                ctx.setLineWidth(1.5)
                let hr = r + 1.0
                ctx.strokeEllipse(in: CGRect(x: node.position.x - hr, y: node.position.y - hr,
                                             width: hr * 2, height: hr * 2))
            }
            ctx.setFillColor(baseColor.withAlphaComponent(fillAlpha).cgColor)
            ctx.setStrokeColor(baseColor.cgColor)
            ctx.setLineWidth(isSelected ? 2.5 : isHovered ? 2.0 : 1.2)
            ctx.fillEllipse(in: rect)
            ctx.strokeEllipse(in: rect)

            // Short label above node — only when hovered, selected, or zoomed in.
            // Suppressing labels at low zoom eliminates the text fog with many nodes.
            if isHovered || isSelected || zoomScale > 2.0 {
                let label = p.pattern.components(separatedBy: " ").prefix(3).joined(separator: " ")
                let attrs: [NSAttributedString.Key: Any] = [
                    .font:            NSFont.systemFont(ofSize: 9),
                    .foregroundColor: isHovered ? NSColor.labelColor : NSColor.secondaryLabelColor,
                ]
                let str = NSAttributedString(string: label, attributes: attrs)
                let sz  = str.size()
                str.draw(at: CGPoint(x: node.position.x - sz.width / 2,
                                     y: node.position.y + r + 2))
            }
        }

        // ── Category labels at arc midpoints (outside the ring) ──────────────
        let catAttrs: [NSAttributedString.Key: Any] = [
            .font:            NSFont.systemFont(ofSize: 12, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
        ]
        for arc in categoryArcs {
            let labelR = arc.ringR + 32
            let lx = cx + labelR * cos(arc.midAngle)
            let ly = cy + labelR * sin(arc.midAngle)
            let str = NSAttributedString(string: arc.name, attributes: catAttrs)
            let sz  = str.size()
            str.draw(at: CGPoint(x: lx - sz.width / 2, y: ly - sz.height / 2))
        }

        ctx.restoreGState()

        // ── Zoom / pan indicator (outside transform — always screen-space) ────
        if abs(zoomScale - 1.0) > 0.02 || panOffset.x != 0 || panOffset.y != 0 {
            let hint = "\(Int(zoomScale * 100))%  ·  dbl-click to reset"
            let hAttrs: [NSAttributedString.Key: Any] = [
                .font:            NSFont.systemFont(ofSize: 9),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ]
            NSAttributedString(string: hint, attributes: hAttrs)
                .draw(at: CGPoint(x: bounds.maxX - 140, y: 6))
        }
    }

    // MARK: – Popover

    private func showPopover(for idx: Int) {
        popover?.close()
        let p  = nodes[idx].pattern

        let vc = NSViewController()
        let vw = NSView(frame: NSRect(x: 0, y: 0, width: 400, height: 162))
        let tv = NSTextView(frame: NSRect(x: 12, y: 8, width: 376, height: 146))
        tv.isEditable = false; tv.backgroundColor = .clear

        let rt = RichText()
        rt.subheader(p.pattern)

        // Confidence bar (▮ filled, ░ empty)
        let confPct = Int(p.confidence * 100)
        let filled  = String(repeating: "▮", count: confPct / 10)
        let empty   = String(repeating: "░", count: 10 - confPct / 10)
        rt.dim("\(p.category)  ·  \(filled)\(empty) \(confPct)%")

        if p.valence != "neutral" {
            let valColor: NSColor = p.valence == "positive" ? .systemGreen
                                  : p.valence == "negative" ? .systemOrange
                                  : .secondaryLabelColor
            rt.coloredLine("valence: \(p.valence)", color: valColor)
        }
        if let first = p.firstSeen { rt.dim("first seen: \(fmtDate(first))") }

        // Same-category siblings
        let siblings = nodes.filter { $0.pattern.category == p.category && $0.pattern.pattern != p.pattern }
        if !siblings.isEmpty {
            let names = siblings.prefix(3)
                .map { $0.pattern.pattern.components(separatedBy: " ").prefix(4).joined(separator: " ") }
                .joined(separator: " · ")
            rt.dim("related: \(names)\(siblings.count > 3 ? " + \(siblings.count - 3) more" : "")")
        }

        tv.textStorage?.setAttributedString(rt.build())
        vw.addSubview(tv)
        vc.view = vw

        let pop = NSPopover()
        pop.contentViewController = vc
        pop.behavior              = .transient
        pop.contentSize           = vw.frame.size

        // Convert node centre from graph space to view space for the anchor rect
        let n    = nodes[idx]
        let vx   = (n.position.x - bounds.midX) * zoomScale + bounds.midX + panOffset.x
        let vy   = (n.position.y - bounds.midY) * zoomScale + bounds.midY + panOffset.y
        let vr   = n.radius * zoomScale
        pop.show(relativeTo: CGRect(x: vx - vr, y: vy - vr, width: vr * 2, height: vr * 2),
                 of: self, preferredEdge: .maxY)
        popover = pop
    }
}

// ─── Association Network Graph ────────────────────────────────────────────────
// Interactive graph of cross-pattern hypotheses (associations).
// Nodes = associations, sized by confidence.
// Edges = shared patternsLinked IDs, thickness ∝ overlap count.
// Three concentric rings: inner ≥0.75 confidence, middle ≥0.50, outer <0.50.
// Pan / zoom / hover / click identical to PatternGraphView.

final class AssociationGraphView: NSView {
    private struct Edge { let a: Int; let b: Int; let weight: Int }
    private struct Node {
        let assoc:    Association
        var position: CGPoint
        let radius:   CGFloat
    }

    private var nodes:        [Node]  = []
    private var edges:        [Edge]  = []
    private var popover:      NSPopover?
    private var selectedIdx:  Int?    = nil
    private var hoveredIdx:   Int?    = nil
    private var trackingArea: NSTrackingArea?

    private var panOffset:   CGPoint = .zero
    private var zoomScale:   CGFloat = 1.0
    private let zoomMin:     CGFloat = 0.25
    private let zoomMax:     CGFloat = 4.0
    private var mouseDownLoc: CGPoint? = nil
    private var isDragging:  Bool    = false
    private let dragThreshold: CGFloat = 5.0

    fileprivate init(frame: NSRect, associations: [Association]) {
        super.init(frame: frame)
        wantsLayer = true
        buildLayout(associations)
    }
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { true }

    private func buildLayout(_ associations: [Association]) {
        let cx = bounds.midX, cy = bounds.midY
        let outerR: CGFloat  = min(bounds.width, bounds.height) * 0.40
        let middleR: CGFloat = outerR * 0.65
        let innerR:  CGFloat = outerR * 0.30

        // Sort descending confidence so high-confidence get inner ring
        let sorted = associations.sorted { $0.confidence > $1.confidence }
        let inner  = sorted.filter { $0.confidence >= 0.75 }
        let mid    = sorted.filter { $0.confidence >= 0.50 && $0.confidence < 0.75 }
        let outer  = sorted.filter { $0.confidence <  0.50 }

        nodes = []
        func place(_ group: [Association], ringR: CGFloat) {
            guard !group.isEmpty else { return }
            for (i, a) in group.enumerated() {
                let angle = CGFloat(i) / CGFloat(group.count) * 2 * .pi - .pi / 2
                let x = cx + ringR * cos(angle)
                let y = cy + ringR * sin(angle)
                let r: CGFloat = 8.0 + CGFloat(a.confidence) * 10.0
                nodes.append(Node(assoc: a, position: CGPoint(x: x, y: y), radius: r))
            }
        }
        place(inner, ringR: innerR)
        place(mid,   ringR: middleR)
        place(outer, ringR: outerR)

        // Build edges: shared patternsLinked IDs
        edges = []
        for i in 0 ..< nodes.count {
            let aIds = Set(nodes[i].assoc.patternsLinked ?? [])
            for j in (i+1) ..< nodes.count {
                let bIds = Set(nodes[j].assoc.patternsLinked ?? [])
                let overlap = aIds.intersection(bIds).count
                if overlap > 0 {
                    edges.append(Edge(a: i, b: j, weight: overlap))
                }
            }
        }
    }

    // MARK: – Hover

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let ta = trackingArea { removeTrackingArea(ta) }
        trackingArea = NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self, userInfo: nil)
        addTrackingArea(trackingArea!)
    }

    override func mouseMoved(with event: NSEvent) {
        let gp = viewToGraph(convert(event.locationInWindow, from: nil))
        let prev = hoveredIdx
        hoveredIdx = hitNode(at: gp)
        if hoveredIdx != prev { needsDisplay = true }
    }

    override func mouseExited(with event: NSEvent) {
        if hoveredIdx != nil { hoveredIdx = nil; needsDisplay = true }
    }

    // MARK: – Zoom

    override func scrollWheel(with event: NSEvent) {
        if event.hasPreciseScrollingDeltas {
            panOffset.x += event.scrollingDeltaX
            panOffset.y += event.scrollingDeltaY
        } else {
            let factor: CGFloat = event.deltaY > 0 ? 1.10 : 0.91
            zoomScale = max(zoomMin, min(zoomMax, zoomScale * factor))
        }
        needsDisplay = true
    }

    override func magnify(with event: NSEvent) {
        zoomScale = max(zoomMin, min(zoomMax, zoomScale * (1.0 + event.magnification)))
        needsDisplay = true
    }

    // MARK: – Pan + click

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            panOffset = .zero; zoomScale = 1.0
            popover?.close(); popover = nil
            selectedIdx = nil; needsDisplay = true; return
        }
        window?.makeFirstResponder(self)
        mouseDownLoc = convert(event.locationInWindow, from: nil)
        isDragging   = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard let start = mouseDownLoc else { return }
        let cur = convert(event.locationInWindow, from: nil)
        if !isDragging && hypot(cur.x - start.x, cur.y - start.y) > dragThreshold {
            isDragging = true
        }
        if isDragging {
            panOffset.x += event.deltaX
            panOffset.y -= event.deltaY
            needsDisplay = true
        }
    }

    override func mouseUp(with event: NSEvent) {
        defer { mouseDownLoc = nil; isDragging = false }
        guard !isDragging else { return }
        let gp = viewToGraph(convert(event.locationInWindow, from: nil))
        if let hit = hitNode(at: gp) {
            selectedIdx = hit; needsDisplay = true; showPopover(for: hit)
        } else {
            popover?.close(); popover = nil; selectedIdx = nil; needsDisplay = true
        }
    }

    // MARK: – Coordinate helpers

    private func viewToGraph(_ pt: CGPoint) -> CGPoint {
        let cx = bounds.midX, cy = bounds.midY
        return CGPoint(
            x: (pt.x - panOffset.x - cx) / zoomScale + cx,
            y: (pt.y - panOffset.y - cy) / zoomScale + cy)
    }

    private func hitNode(at gp: CGPoint) -> Int? {
        nodes.enumerated().first { hypot(gp.x - $1.position.x, gp.y - $1.position.y) <= $1.radius + 6 }?.offset
    }

    private func nodeColor(_ a: Association) -> NSColor {
        if a.actionable && a.confidence >= 0.75 { return .systemGreen }
        if a.actionable                         { return .systemBlue  }
        return a.confidence >= 0.65 ? .secondaryLabelColor : .tertiaryLabelColor
    }

    // MARK: – Drawing

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }

        NSColor.textBackgroundColor.setFill()
        NSBezierPath.fill(bounds)

        ctx.saveGState()
        let cx = bounds.midX, cy = bounds.midY
        ctx.translateBy(x: panOffset.x + cx, y: panOffset.y + cy)
        ctx.scaleBy(x: zoomScale, y: zoomScale)
        ctx.translateBy(x: -cx, y: -cy)

        // Ring guide circles (faint dashed)
        let outerR: CGFloat  = min(bounds.width, bounds.height) * 0.40
        for (r, label) in [(outerR, "low"), (outerR * 0.65, "mid"), (outerR * 0.30, "high")] {
            ctx.setStrokeColor(NSColor.separatorColor.withAlphaComponent(0.3).cgColor)
            ctx.setLineWidth(0.5)
            ctx.setLineDash(phase: 0, lengths: [4, 4])
            ctx.strokeEllipse(in: CGRect(x: cx - r, y: cy - r, width: r * 2, height: r * 2))
            ctx.setLineDash(phase: 0, lengths: [])
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 8.5),
                .foregroundColor: NSColor.tertiaryLabelColor
            ]
            let s = NSAttributedString(string: label, attributes: attrs)
            let sz = s.size()
            s.draw(at: CGPoint(x: cx + r - sz.width - 4, y: cy - sz.height / 2))
        }

        // ── Edges ─────────────────────────────────────────────────────────────
        // Hidden by default; on hover show only edges connecting the hovered node,
        // capped at 12 to prevent blob when nodes share many pattern links.
        if let hov = hoveredIdx {
            let maxWeight = edges.map { $0.weight }.max() ?? 1
            let hovEdges  = edges
                .filter { $0.a == hov || $0.b == hov }
                .sorted { $0.weight > $1.weight }
                .prefix(12)
            for edge in hovEdges {
                let ca = nodeColor(nodes[edge.a].assoc)
                let cb = nodeColor(nodes[edge.b].assoc)
                let blended = ca.blended(withFraction: 0.5, of: cb) ?? ca
                ctx.setStrokeColor(blended.withAlphaComponent(0.60).cgColor)
                let w = 1.0 + 2.0 * CGFloat(edge.weight) / CGFloat(maxWeight)
                ctx.setLineWidth(w)
                ctx.move(to: nodes[edge.a].position)
                ctx.addLine(to: nodes[edge.b].position)
                ctx.strokePath()
            }
        }

        // ── Nodes ─────────────────────────────────────────────────────────────
        for (idx, node) in nodes.enumerated() {
            let a          = node.assoc
            let baseColor  = nodeColor(a)
            let isHovered  = idx == hoveredIdx
            let isSelected = idx == selectedIdx
            let r          = node.radius

            if isHovered || isSelected {
                ctx.setStrokeColor(baseColor.withAlphaComponent(isSelected ? 0.45 : 0.28).cgColor)
                ctx.setLineWidth(5.5)
                let gr = r + 4
                ctx.strokeEllipse(in: CGRect(x: node.position.x - gr, y: node.position.y - gr,
                                              width: gr * 2, height: gr * 2))
            }

            let fillAlpha: CGFloat = isSelected ? 1.0 : isHovered ? 0.92 : 0.70
            ctx.setFillColor(baseColor.withAlphaComponent(fillAlpha).cgColor)
            ctx.setStrokeColor(baseColor.cgColor)
            ctx.setLineWidth(isSelected ? 2.5 : isHovered ? 2.0 : 1.0)
            let rect = CGRect(x: node.position.x - r, y: node.position.y - r,
                               width: r * 2, height: r * 2)
            ctx.fillEllipse(in: rect)
            ctx.strokeEllipse(in: rect)

            // Diamond marker for actionable nodes
            if a.actionable {
                let dm: CGFloat = 4
                let dp = node.position
                ctx.setFillColor(NSColor.white.withAlphaComponent(isHovered ? 0.9 : 0.7).cgColor)
                ctx.move(to: CGPoint(x: dp.x, y: dp.y + dm))
                ctx.addLine(to: CGPoint(x: dp.x + dm, y: dp.y))
                ctx.addLine(to: CGPoint(x: dp.x, y: dp.y - dm))
                ctx.addLine(to: CGPoint(x: dp.x - dm, y: dp.y))
                ctx.closePath()
                ctx.fillPath()
            }

            // Short label — only when hovered, selected, or zoomed in enough.
            // Suppressing at default zoom eliminates text fog with 170+ nodes.
            if isHovered || isSelected || zoomScale > 1.8 {
                let label = a.hypothesis.components(separatedBy: " ").prefix(4).joined(separator: " ")
                let labelAttrs: [NSAttributedString.Key: Any] = [
                    .font:            NSFont.systemFont(ofSize: isHovered ? 10 : 9),
                    .foregroundColor: isHovered ? NSColor.labelColor : NSColor.secondaryLabelColor,
                ]
                let str = NSAttributedString(string: label, attributes: labelAttrs)
                let sz  = str.size()
                str.draw(at: CGPoint(x: node.position.x - sz.width / 2,
                                      y: node.position.y + r + 2))
            }
        }

        ctx.restoreGState()

        // Zoom indicator
        if abs(zoomScale - 1.0) > 0.02 || panOffset.x != 0 || panOffset.y != 0 {
            let hAttrs: [NSAttributedString.Key: Any] = [
                .font:            NSFont.systemFont(ofSize: 9),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ]
            NSAttributedString(string: "\(Int(zoomScale * 100))%  ·  dbl-click to reset",
                               attributes: hAttrs)
                .draw(at: CGPoint(x: bounds.maxX - 150, y: 6))
        }
    }

    // MARK: – Popover

    private func showPopover(for idx: Int) {
        popover?.close()
        let a  = nodes[idx].assoc

        let vc = NSViewController()
        let vw = NSView(frame: NSRect(x: 0, y: 0, width: 440, height: 168))
        let tv = NSTextView(frame: NSRect(x: 12, y: 8, width: 416, height: 152))
        tv.isEditable = false; tv.backgroundColor = .clear

        let rt = RichText()
        rt.subheader(a.hypothesis)

        let confPct = Int(a.confidence * 100)
        let filled  = String(repeating: "▮", count: confPct / 10)
        let empty   = String(repeating: "░", count: 10 - confPct / 10)
        let tag     = a.actionable ? "  · actionable ◆" : ""
        rt.dim("\(filled)\(empty) \(confPct)%\(tag)")

        if let rule = a.suggestedRule, !rule.isEmpty {
            rt.accent("→ Rule: \(rule)")
        }

        let linkedCount = a.patternsLinked?.count ?? 0
        if linkedCount > 0 { rt.dim("linked patterns: \(linkedCount)") }

        // Neighbours sharing edges
        let neighbours = edges
            .filter { $0.a == idx || $0.b == idx }
            .sorted { $0.weight > $1.weight }
        if !neighbours.isEmpty {
            let names = neighbours.prefix(3).map { e -> String in
                let other = e.a == idx ? e.b : e.a
                let h = nodes[other].assoc.hypothesis
                return h.components(separatedBy: " ").prefix(4).joined(separator: " ")
            }.joined(separator: " · ")
            rt.dim("connected: \(names)\(neighbours.count > 3 ? " + \(neighbours.count - 3) more" : "")")
        }

        tv.textStorage?.setAttributedString(rt.build())
        vw.addSubview(tv)
        vc.view = vw

        let pop = NSPopover()
        pop.contentViewController = vc
        pop.behavior              = .transient
        pop.contentSize           = vw.frame.size

        let n  = nodes[idx]
        let vx = (n.position.x - bounds.midX) * zoomScale + bounds.midX + panOffset.x
        let vy = (n.position.y - bounds.midY) * zoomScale + bounds.midY + panOffset.y
        let vr = n.radius * zoomScale
        pop.show(relativeTo: CGRect(x: vx - vr, y: vy - vr, width: vr * 2, height: vr * 2),
                 of: self, preferredEdge: .maxY)
        popover = pop
    }
}

// ─── Icon choices ─────────────────────────────────────────────────────────────

private let iconChoices: [(label: String, symbol: String)] = [
    // Sleep / dream
    ("Moon ZZZ",   "moon.zzz.fill"),
    ("Moon",       "moon.fill"),
    ("Cloud Moon", "cloud.moon.fill"),
    ("Stars",      "moon.stars.fill"),
    ("Zzz",        "zzz"),
    // Intelligence
    ("Brain",      "brain"),
    ("Brain Head", "brain.head.profile"),
    ("Sparkles",   "sparkles"),
    ("Wand Stars", "wand.and.stars"),
    ("Lightbulb",  "lightbulb.fill"),
    ("Magnify",    "magnifyingglass"),
    // Data / signal
    ("Waveform",   "waveform.path"),
    ("Network",    "network"),
    ("CPU",        "cpu"),
    ("Infinity",   "infinity"),
    ("Chart Bar",  "chart.bar.fill"),
    ("Chart XY",   "chart.xyaxis.line"),
    ("Antenna",    "antenna.radiowaves.left.and.right"),
    // Actions / motion
    ("Bolt",       "bolt.fill"),
    ("Arrow Cycle","arrow.triangle.2.circlepath"),
    ("Clock",      "clock.fill"),
    // UI / documents
    ("Star",       "star.fill"),
    ("Eye",        "eye.fill"),
    ("Document",   "doc.richtext"),
    ("Book",       "book.fill"),
    ("Clipboard",  "list.bullet.clipboard.fill"),
    ("Hexagon",    "hexagon.fill"),
    // Social / nature
    ("Bubble",     "bubble.left.and.bubble.right.fill"),
    ("Globe",      "globe"),
    ("Flame",      "flame.fill"),
    ("Leaf",       "leaf.fill"),
    ("Rainbow",    "rainbow"),
    ("Heart",      "heart.fill"),
    ("Gear",       "gearshape.fill"),
    ("Cloud",      "cloud.fill"),
]

private let iconDefaultsKey   = "dev.i-dream.bar.icon"
private let defaultIconSymbol = "moon.zzz.fill"
private let hudVisibleKey     = "dev.i-dream.bar.hudVisible"
private let hudAlwaysOnTopKey = "dev.i-dream.bar.hudOnTop"

private func currentIconSymbol() -> String {
    UserDefaults.standard.string(forKey: iconDefaultsKey) ?? defaultIconSymbol
}

// Color gradient used during dreaming animation (warm → cool → warm)
private let dreamAnimColors: [NSColor] = [
    .systemYellow, .systemOrange, .systemPink,
    .systemPurple, .systemBlue,   .systemTeal,
    .systemGreen,  .systemYellow,
]

// ─── Readers ──────────────────────────────────────────────────────────────────

private func readState() -> DaemonState? {
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: statePath)) else { return nil }
    return try? JSONDecoder().decode(DaemonState.self, from: data)
}

private func isDaemonRunning() -> Bool {
    guard
        let raw = try? String(contentsOfFile: pidPath, encoding: .utf8),
        let pid = Int32(raw.trimmingCharacters(in: .whitespacesAndNewlines))
    else { dlog("isDaemonRunning: no pid file or unparseable"); return false }
    let alive = kill(pid, 0) == 0
    dlog("isDaemonRunning: pid=\(pid) alive=\(alive)")
    return alive
}

private func countJsonArray(at path: String) -> Int {
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let arr  = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
    else { return 0 }
    return arr.count
}

private func countProcessedSessions(at path: String) -> Int {
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let obj  = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let map  = obj["sessions"] as? [String: Any]
    else { return 0 }
    return map.count
}

/// Returns the most recent daemon error message, but only if it occurred
/// after the last successful consolidation (i.e., the error is still "live").
/// Log lines are prefixed with an ISO8601 timestamp, e.g.:
///   2026-04-16T23:41:47.123456Z  ERROR ...
private func lastDaemonError() -> String? {
    guard let content = try? String(contentsOfFile: bestLogPath(), encoding: .utf8) else { return nil }

    // Parse last consolidation date for comparison
    let lastConsolidationDate: Date? = {
        guard let data  = try? Data(contentsOf: URL(fileURLWithPath: statePath)),
              let state = try? JSONDecoder().decode(DaemonState.self, from: data)
        else { return nil }
        return isoDate(state.lastConsolidation)
    }()

    let iso = ISO8601DateFormatter()
    iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let iso2 = ISO8601DateFormatter()  // without fractional seconds fallback

    for line in content.components(separatedBy: "\n").reversed() {
        guard line.contains(" ERROR "), let errRange = line.range(of: " ERROR ") else { continue }

        // Try to parse the line's leading timestamp (first token)
        if let lastConsolidation = lastConsolidationDate {
            let firstToken = String(line.prefix(32).components(separatedBy: " ").first ?? "")
            let lineDate = iso.date(from: firstToken) ?? iso2.date(from: firstToken)
            if let lineDate = lineDate, lineDate < lastConsolidation {
                // Error is older than last successful cycle — no longer relevant
                return nil
            }
        }

        let msg = String(line[errRange.upperBound...])
            .replacingOccurrences(of: "API request failed: API request failed \\(\\d+ [^)]+\\): ",
                                   with: "", options: .regularExpression)
            .trimmingCharacters(in: .whitespaces)
        return msg.count > 100 ? String(msg.prefix(97)) + "…" : msg
    }
    return nil
}

private func readBoard() -> BoardData {
    BoardData(
        dreamsProcessed:  countProcessedSessions(at: subDir + "/dreams/processed.json"),
        metacogProcessed: countProcessedSessions(at: subDir + "/metacog/processed.json"),
        dreamsPatterns:   countJsonArray(at: subDir + "/dreams/patterns.json"),
        associations:     countJsonArray(at: subDir + "/dreams/associations.json"),
        metacogAudits:    (try? FileManager.default.contentsOfDirectory(
                               atPath: subDir + "/metacog/audits"))?.count ?? 0,
        lastError:        lastDaemonError()
    )
}

// ─── Payload color parser ─────────────────────────────────────────────────────
//
// Shared utility: colorizes a raw payload string (JSON, Markdown, plain text)
// into an NSAttributedString with syntax-aware highlighting.
//
// Usage (from std::claude reference — see ~/.claude/skills/shared/README.md):
//   colorizePayload(text, baseColor: phaseColor, bgColor: bgAlpha, indentStyle: style)
//
// Color param roles:
//   baseColor  — primary text / string values / prose
//   bgColor    — background tint for the whole block
//   Keys/numbers/booleans always use fixed semantic colors (systemCyan, systemOrange, etc.)
//   to stay readable regardless of baseColor. Only string values inherit baseColor.

private func colorizePayload(
    _ text:        String,
    baseColor:     NSColor,
    bgColor:       NSColor,
    indentStyle:   NSParagraphStyle
) -> NSAttributedString {
    let baseFontSize: CGFloat = 9

    // ── Detect format ──────────────────────────────────────────────────────
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    let isJSON     = trimmed.hasPrefix("{") || trimmed.hasPrefix("[")
    let isMarkdown = !isJSON && (trimmed.hasPrefix("#") || trimmed.contains("\n##") || trimmed.contains("\n- "))

    let buf = NSMutableAttributedString()
    let baseAttrs: [NSAttributedString.Key: Any] = [
        .font:            NSFont.monospacedSystemFont(ofSize: baseFontSize, weight: .regular),
        .foregroundColor: baseColor,
        .backgroundColor: bgColor,
        .paragraphStyle:  indentStyle,
    ]

    if isJSON {
        // ── JSON coloring ──────────────────────────────────────────────────
        // Token-level coloring: keys=cyan, strings=baseColor, numbers=orange,
        // booleans/null=yellow, punctuation=dim.
        let lines = text.components(separatedBy: "\n")
        for (li, rawLine) in lines.enumerated() {
            let lineBuf = NSMutableAttributedString()
            var i = rawLine.startIndex

            while i < rawLine.endIndex {
                let ch = rawLine[i]

                // JSON string token
                if ch == "\"" {
                    var j = rawLine.index(after: i)
                    while j < rawLine.endIndex {
                        if rawLine[j] == "\\" && rawLine.index(after: j) < rawLine.endIndex {
                            j = rawLine.index(j, offsetBy: 2)
                        } else if rawLine[j] == "\"" {
                            j = rawLine.index(after: j)
                            break
                        } else {
                            j = rawLine.index(after: j)
                        }
                    }
                    let token = String(rawLine[i..<j])
                    // If followed (after whitespace) by ":" it's a key, else a value
                    var peek = j
                    while peek < rawLine.endIndex, rawLine[peek] == " " { peek = rawLine.index(after: peek) }
                    let isKey = peek < rawLine.endIndex && rawLine[peek] == ":"
                    let color: NSColor = isKey ? .systemCyan : baseColor
                    lineBuf.append(NSAttributedString(string: token, attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: baseFontSize, weight: isKey ? .medium : .regular),
                        .foregroundColor: color, .backgroundColor: bgColor, .paragraphStyle: indentStyle,
                    ]))
                    i = j
                    continue
                }

                // Number
                if ch.isNumber || (ch == "-" && rawLine.index(after: i) < rawLine.endIndex && rawLine[rawLine.index(after: i)].isNumber) {
                    var j = rawLine.index(after: i)
                    while j < rawLine.endIndex && (rawLine[j].isNumber || rawLine[j] == "." || rawLine[j] == "e" || rawLine[j] == "-") {
                        j = rawLine.index(after: j)
                    }
                    lineBuf.append(NSAttributedString(string: String(rawLine[i..<j]), attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: baseFontSize, weight: .regular),
                        .foregroundColor: NSColor.systemOrange, .backgroundColor: bgColor, .paragraphStyle: indentStyle,
                    ]))
                    i = j
                    continue
                }

                // Boolean / null keywords
                let remaining = String(rawLine[i...])
                if remaining.hasPrefix("true") || remaining.hasPrefix("false") || remaining.hasPrefix("null") {
                    let kw = remaining.hasPrefix("true") ? "true" : remaining.hasPrefix("false") ? "false" : "null"
                    lineBuf.append(NSAttributedString(string: kw, attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: baseFontSize, weight: .bold),
                        .foregroundColor: NSColor.systemYellow, .backgroundColor: bgColor, .paragraphStyle: indentStyle,
                    ]))
                    i = rawLine.index(i, offsetBy: kw.count)
                    continue
                }

                // Punctuation / whitespace
                let punctColor: NSColor = (ch == "{" || ch == "}" || ch == "[" || ch == "]" || ch == "," || ch == ":")
                    ? NSColor.secondaryLabelColor : baseColor
                lineBuf.append(NSAttributedString(string: String(ch), attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: baseFontSize, weight: .regular),
                    .foregroundColor: punctColor, .backgroundColor: bgColor, .paragraphStyle: indentStyle,
                ]))
                i = rawLine.index(after: i)
            }

            if li < lines.count - 1 {
                lineBuf.append(NSAttributedString(string: "\n", attributes: baseAttrs))
            }
            buf.append(lineBuf)
        }

    } else if isMarkdown {
        // ── Markdown coloring ──────────────────────────────────────────────
        // Headers=bold+bright, bullet points=accent, code spans=monospace+dim,
        // emphasis=italic, plain prose=baseColor.
        for (li, line) in text.components(separatedBy: "\n").enumerated() {
            let trim = line.trimmingCharacters(in: .whitespaces)
            let lineAttrs: [NSAttributedString.Key: Any]

            if trim.hasPrefix("### ") {
                lineAttrs = [.font: NSFont.systemFont(ofSize: baseFontSize + 1, weight: .semibold),
                             .foregroundColor: baseColor, .backgroundColor: bgColor, .paragraphStyle: indentStyle]
            } else if trim.hasPrefix("## ") {
                lineAttrs = [.font: NSFont.systemFont(ofSize: baseFontSize + 2, weight: .bold),
                             .foregroundColor: NSColor.labelColor, .backgroundColor: bgColor, .paragraphStyle: indentStyle]
            } else if trim.hasPrefix("# ") {
                lineAttrs = [.font: NSFont.systemFont(ofSize: baseFontSize + 3, weight: .heavy),
                             .foregroundColor: NSColor.labelColor, .backgroundColor: bgColor, .paragraphStyle: indentStyle]
            } else if trim.hasPrefix("- ") || trim.hasPrefix("* ") || trim.hasPrefix("• ") {
                lineAttrs = [.font: NSFont.systemFont(ofSize: baseFontSize, weight: .regular),
                             .foregroundColor: baseColor.blended(withFraction: 0.3, of: .labelColor) ?? baseColor,
                             .backgroundColor: bgColor, .paragraphStyle: indentStyle]
            } else if trim.hasPrefix(">") {
                lineAttrs = [.font: NSFont.systemFont(ofSize: baseFontSize, weight: .light),
                             .foregroundColor: NSColor.secondaryLabelColor, .backgroundColor: bgColor, .paragraphStyle: indentStyle]
            } else if trim.hasPrefix("```") || trim.hasSuffix("```") {
                lineAttrs = [.font: NSFont.monospacedSystemFont(ofSize: baseFontSize - 1, weight: .regular),
                             .foregroundColor: NSColor.tertiaryLabelColor, .backgroundColor: bgColor, .paragraphStyle: indentStyle]
            } else {
                lineAttrs = baseAttrs
            }

            buf.append(NSAttributedString(string: line + (li < text.components(separatedBy: "\n").count - 1 ? "\n" : ""),
                                          attributes: lineAttrs))
        }

    } else {
        // ── Plain text fallback ────────────────────────────────────────────
        buf.append(NSAttributedString(string: text, attributes: baseAttrs))
    }

    return buf
}

// ─── Sparklines & metrics ─────────────────────────────────────────────────────

/// Maps a sequence of integers to a Unicode sparkline string (▁▂▃▄▅▆▇█).
/// The tallest value is always █; an empty input returns "".
private func fmtSparkline(_ values: [Int], width: Int = 10) -> String {
    guard !values.isEmpty else { return "" }
    let bars = "▁▂▃▄▅▆▇█"
    let window = Array(values.suffix(width))
    let maxVal = window.max() ?? 1
    return window.map { v in
        let idx = maxVal == 0 ? 0 : min(Int(Double(v) / Double(maxVal) * 7.0), 7)
        return String(bars[bars.index(bars.startIndex, offsetBy: idx)])
    }.joined()
}

/// Returns a 0–1 score estimating cognitive load from recent journal entries.
/// Blends token velocity (60%) and pattern extraction rate (40%).
private func cognitiveLoadScore(journal: [JournalEntry]) -> Double {
    guard !journal.isEmpty else { return 0 }
    let recent   = Array(journal.suffix(5))
    let avgTok   = Double(recent.map { $0.tokensUsed }.reduce(0, +)) / Double(recent.count)
    let avgPat   = Double(recent.map { $0.patternsExtracted }.reduce(0, +)) / Double(recent.count)
    let tokLoad  = min(avgTok / 8000.0, 1.0)
    let patLoad  = min(avgPat / 10.0,   1.0)
    return tokLoad * 0.6 + patLoad * 0.4
}

/// Renders a 5-slot filled/empty gauge: score 0.0 → "○○○○○", 1.0 → "●●●●●".
private func fmtLoadGauge(_ score: Double) -> String {
    let filled = Int(score * 5 + 0.5)
    return String(repeating: "●", count: filled) + String(repeating: "○", count: 5 - filled)
}

private func recentPatterns(limit: Int = 3) -> [Pattern] {
    let path = subDir + "/dreams/patterns.json"
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let arr  = try? JSONDecoder().decode([Pattern].self, from: data)
    else { return [] }
    return Array(arr.suffix(limit))
}

private func allPatterns() -> [Pattern] {
    let path = subDir + "/dreams/patterns.json"
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let arr  = try? JSONDecoder().decode([Pattern].self, from: data)
    else { return [] }
    return arr
}

private func recentJournal(limit: Int = 3) -> [JournalEntry] {
    let path = subDir + "/dreams/journal.jsonl"
    guard let content = try? String(contentsOfFile: path, encoding: .utf8) else { return [] }
    return content.components(separatedBy: "\n").filter { !$0.isEmpty }.suffix(limit)
        .compactMap { line -> JournalEntry? in
            guard let d = line.data(using: .utf8) else { return nil }
            return try? JSONDecoder().decode(JournalEntry.self, from: d)
        }
}

private func allJournal() -> [JournalEntry] {
    let path = subDir + "/dreams/journal.jsonl"
    guard let content = try? String(contentsOfFile: path, encoding: .utf8) else { return [] }
    return content.components(separatedBy: "\n").filter { !$0.isEmpty }
        .compactMap { line -> JournalEntry? in
            guard let d = line.data(using: .utf8) else { return nil }
            return try? JSONDecoder().decode(JournalEntry.self, from: d)
        }
}

private func allAssociations() -> [Association] {
    let path = subDir + "/dreams/associations.json"
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let arr  = try? JSONDecoder().decode([Association].self, from: data)
    else { return [] }
    return arr
}

/// Read the insight-digest prose paragraph (strips the markdown header/metadata lines).
private func readInsightDigest() -> String? {
    let path = subDir + "/dreams/insight-digest.md"
    guard let raw = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
    let prose = raw.components(separatedBy: "\n")
        .filter { !$0.hasPrefix("#") && !$0.hasPrefix("_") && !$0.hasPrefix("##") }
        .joined(separator: "\n")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    return prose.isEmpty ? nil : prose
}

/// Read the sentiment field from dreams/digest-meta.json.
/// Returns "positive", "neutral", or "negative" (defaults to "neutral" if absent).
private func readDigestSentiment() -> String {
    let path = subDir + "/dreams/digest-meta.json"
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let obj  = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let s    = obj["sentiment"] as? String
    else { return "neutral" }
    return s
}

/// Read all dream insights from dreams/insights.md as a raw string.
private func readAllInsights() -> String? {
    let path = subDir + "/dreams/insights.md"
    return try? String(contentsOfFile: path, encoding: .utf8)
}

/// Read the current dream frequency from settings.json (hours). Returns nil if unset.
private func readDreamFrequency() -> Double? {
    let path = subDir + "/settings.json"
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let obj  = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let h    = obj["dream_frequency_hours"] as? Double,
          h > 0
    else { return nil }
    return h
}

/// Persist the dream frequency to settings.json.
private func writeDreamFrequency(_ hours: Double) {
    let path = subDir + "/settings.json"
    var obj: [String: Any] = [:]
    if let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
       let existing = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
        obj = existing
    }
    if hours > 0 { obj["dream_frequency_hours"] = hours }
    else { obj.removeValue(forKey: "dream_frequency_hours") }
    if let data = try? JSONSerialization.data(withJSONObject: obj, options: .prettyPrinted) {
        try? data.write(to: URL(fileURLWithPath: path))
    }
}

/// Return the Date when the next dream cycle will fire (activity + threshold).
/// Returns nil if no activity file exists.
private func nextDreamDate(thresholdHours: Double) -> Date? {
    let attrs = try? FileManager.default.attributesOfItem(atPath: activityFile)
    guard let mod = attrs?[.modificationDate] as? Date else { return nil }
    return mod.addingTimeInterval(thresholdHours * 3600)
}

/// Format a countdown to a future date: "in 2h 15m", "in 45m", "now".
private func fmtCountdown(_ target: Date) -> String {
    let secs = target.timeIntervalSinceNow
    if secs <= 0 { return "now" }
    let h = Int(secs) / 3600
    let m = (Int(secs) % 3600) / 60
    if h > 0 { return "in \(h)h \(m)m" }
    return "in \(m)m"
}

// ─── Store health ─────────────────────────────────────────────────────────────

private struct StoreFile {
    let label:     String
    let path:      String
    let entries:   Int
    let sizeBytes: UInt64
    /// Matches the dashboard's 5 MB warning threshold.
    var isLarge: Bool { sizeBytes >= 5 * 1024 * 1024 }
}

private func countJsonlLines(at path: String) -> Int {
    guard let content = try? String(contentsOfFile: path, encoding: .utf8) else { return 0 }
    return content.components(separatedBy: "\n").filter { !$0.isEmpty }.count
}

private func readStoreFiles() -> [StoreFile] {
    let watched: [(String, String)] = [
        ("Hook events",      subDir + "/logs/events.jsonl"),
        ("Metacog activity", subDir + "/metacog/activity.jsonl"),
        ("Signals",          subDir + "/logs/signals.jsonl"),
        ("Dream journal",    subDir + "/dreams/journal.jsonl"),
    ]
    return watched.map { label, path in
        let attrs = try? FileManager.default.attributesOfItem(atPath: path)
        let size  = (attrs?[.size] as? UInt64) ?? 0
        return StoreFile(label: label, path: path,
                         entries: countJsonlLines(at: path), sizeBytes: size)
    }
}

private func readLatestAudit() -> (audit: MetacogAudit?, filename: String?) {
    let auditsDir = subDir + "/metacog/audits"
    guard let files = try? FileManager.default.contentsOfDirectory(atPath: auditsDir) else {
        return (nil, nil)
    }
    guard let latest = files.filter({ $0.hasSuffix(".json") }).sorted().last else {
        return (nil, nil)
    }
    let path = auditsDir + "/" + latest
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)) else { return (nil, nil) }

    // Try wrapper format first (current daemon output):
    // { "response": "```json\n{...}\n```", "sessions": [...] }
    if let wrapper  = try? JSONDecoder().decode(MetacogAuditFile.self, from: data),
       let response = wrapper.response {
        let stripped = response
            .replacingOccurrences(of: "```json\n", with: "")
            .replacingOccurrences(of: "```json",   with: "")
            .replacingOccurrences(of: "\n```",     with: "")
            .replacingOccurrences(of: "```",       with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let innerData = stripped.data(using: .utf8),
           let audit = try? JSONDecoder().decode(MetacogAudit.self, from: innerData),
           audit.hasContent {
            return (audit, latest)
        }
    }

    // Fallback: flat format (future / manual writes)
    if let audit = try? JSONDecoder().decode(MetacogAudit.self, from: data), audit.hasContent {
        return (audit, latest)
    }

    return (nil, latest)
}

/// Inspect the latest dream trace to identify current phase + completion.
/// Returns (phaseLabel, elapsedSecs, isDone).
private func detectDreamProgress(since start: Date) -> (phase: String, elapsed: TimeInterval, isDone: Bool) {
    let elapsed = Date().timeIntervalSince(start)
    let fm = FileManager.default
    guard let files = try? fm.contentsOfDirectory(atPath: tracesDir) else {
        return ("…", elapsed, false)
    }
    guard let latestFile = files.filter({ $0.hasSuffix(".jsonl") }).sorted().last else {
        return ("…", elapsed, false)
    }
    let latestPath = tracesDir + "/" + latestFile
    // Only consider this trace if it's recent enough to be from our trigger
    if let attrs = try? fm.attributesOfItem(atPath: latestPath),
       let mod   = attrs[.modificationDate] as? Date,
       mod < start.addingTimeInterval(-30) {
        return ("…", elapsed, false)
    }
    guard let content = try? String(contentsOfFile: latestPath, encoding: .utf8) else {
        return ("…", elapsed, false)
    }
    var lastPhase = "init"
    var isDone    = false
    for line in content.components(separatedBy: "\n").filter({ !$0.isEmpty }).suffix(10) {
        guard let d   = line.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any]
        else { continue }
        if let p = obj["phase"] as? String { lastPhase = p }
        if let k = obj["kind"]  as? String, k == "cycle_complete" { isDone = true }
    }
    let label: String
    switch lastPhase {
    case "init": label = "Initializing"
    case "sws":  label = "SWS — extracting learnings"
    case "rem":  label = "REM — finding patterns"
    case "wake": label = "Wake — consolidating"
    default:     label = lastPhase
    }
    return (label, elapsed, isDone)
}

// ─── AppleScript helper ───────────────────────────────────────────────────────

private func openInTerminal(_ command: String) {
    let esc = command
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
    let src = "tell application \"Terminal\"\n    do script \"\(esc)\"\n    activate\nend tell"
    var err: NSDictionary?
    NSAppleScript(source: src)?.executeAndReturnError(&err)
}

// ─── App delegate ─────────────────────────────────────────────────────────────

final class BarDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    var statusItem: NSStatusItem!
    var timer: Timer?

    private var cachedRunning        = false
    private var cachedState:         DaemonState?
    private var cachedBoard:         BoardData?
    private var cachedPatterns:      [Pattern]      = []
    private var cachedJournal:       [JournalEntry] = []
    private var cachedStoreFiles:    [StoreFile]    = []
    private var cachedDigest:        String?
    private var cachedFrequencyHours: Double?
    private var cachedPatternCount:  Int = 0
    private var cachedHighConfCount: Int = 0

    // Persistent resizable detail panel (replaces NSAlert popups)
    private var detailPanel:          NSPanel?
    private var detailFilePath:       String?
    private var journalLinkDelegate:  JournalLinkDelegate?
    private var cycleDetailPanel:     NSPanel?

    // Dream completion card (auto-dismissing overlay)
    private var completionCard: NSPanel?

    // Pattern network graph panel
    private var networkPanel:            NSPanel?

    // Association network graph panel
    private var associationNetworkPanel: NSPanel?

    // Insight feedback panel
    private var feedbackPanel: NSPanel?

    // Ambient HUD — always-visible mini status window
    private var hudPanel:        NSPanel?
    private var hudUpdateTimer:  Timer?
    private var hudBarChart:     MiniBarChartView?
    private var hudPinBtn:       NSButton?
    private var hudTimeRangeBtn: NSButton?
    /// 0 = 7d, 1 = 30d, 2 = all
    private var hudTimeRangeIndex: Int = 0

    // Dream replay — event-by-event trace playback
    private var replayPanel:      NSPanel?
    private var replayTimer:      Timer?
    private var replayEvents:     [[String: Any]] = []
    private var replayIndex:      Int             = 0
    private var replayTextView:   NSTextView?
    private var replayPauseBtn:   NSButton?
    private var replayTracePopup: NSPopUpButton?
    private var replayTraceFiles: [String]        = []
    private var insightFeedbackDelegate: InsightFeedbackDelegate?

    // Dreaming animation
    private var isCycling       = false
    private var cycleStartTime: Date?
    private var animFrame       = 0
    private var animTimer:      Timer?

    // Persistent menu instance (rebuilt in-place via NSMenuDelegate)
    private var theMenu: NSMenu!

    func applicationDidFinishLaunching(_ note: Notification) {
        dlog("launched PID=\(ProcessInfo.processInfo.processIdentifier) build=\(BuildInfo.commitHash)/\(BuildInfo.sourceHash) at=\(BuildInfo.builtAt)")
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)

        theMenu                  = NSMenu()
        theMenu.autoenablesItems = false
        theMenu.delegate         = self
        statusItem.menu          = theMenu

        refresh()
        // Restore HUD if it was visible in the previous session
        if UserDefaults.standard.bool(forKey: hudVisibleKey) { showHUD() }

        // Full refresh every 30s (state.json, board, patterns)
        timer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            self?.refresh()
        }
        // Lightweight running-status poll every 10s to keep button accurate
        Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            let nowRunning = isDaemonRunning()
            if nowRunning != self.cachedRunning {
                self.cachedRunning = nowRunning
                self.updateButton()
            }
        }
    }

    // Called by AppKit right before the menu is shown — always up-to-date.
    func menuNeedsUpdate(_ menu: NSMenu) {
        cachedRunning          = isDaemonRunning()
        cachedState            = readState()
        cachedBoard            = readBoard()
        cachedPatterns         = recentPatterns(limit: 5)
        cachedJournal          = recentJournal(limit: 20)
        cachedStoreFiles       = readStoreFiles()
        cachedDigest           = readInsightDigest()
        cachedFrequencyHours   = readDreamFrequency()
        let allPats            = allPatterns()
        cachedPatternCount     = allPats.count
        cachedHighConfCount    = allPats.filter { $0.confidence >= 0.8 }.count
        updateButton()
        menu.removeAllItems()
        populateMenuItems(menu)
    }

    @objc func refresh() {
        cachedRunning          = isDaemonRunning()
        cachedState            = readState()
        cachedBoard            = readBoard()
        cachedPatterns         = recentPatterns(limit: 5)
        cachedJournal          = recentJournal(limit: 20)
        cachedStoreFiles       = readStoreFiles()
        cachedDigest           = readInsightDigest()
        cachedFrequencyHours   = readDreamFrequency()
        let allPats            = allPatterns()
        cachedPatternCount     = allPats.count
        cachedHighConfCount    = allPats.filter { $0.confidence >= 0.8 }.count
        dlog("refresh: running=\(cachedRunning) cycles=\(cachedState?.totalCycles ?? -1)")
        checkCycleCompletion()
        updateButton()
        // Keep HUD current if visible
        if let panel = hudPanel, let tv = panel.contentView?.subviews.first as? NSTextView {
            updateHUDContent(tv)
        }
    }

    // ── Dreaming animation ─────────────────────────────────────────────────────

    private func startDreamAnimation() {
        animFrame = 0
        animTimer?.invalidate()
        animTimer = Timer.scheduledTimer(withTimeInterval: 0.4, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            self.animFrame = (self.animFrame + 1) % dreamAnimColors.count
            self.checkCycleCompletion()
            self.updateButton()
        }
    }

    private func stopDreamAnimation() {
        animTimer?.invalidate()
        animTimer      = nil
        isCycling      = false
        cycleStartTime = nil
        updateButton()
    }

    // ── Dream completion card ─────────────────────────────────────────────────
    // Slides in from the bottom-right corner, auto-dismisses after 6s with fade.

    private func showCompletionCard() {
        completionCard?.orderOut(nil)
        completionCard = nil

        let cardW: CGFloat = 400
        let cardH: CGFloat = 210
        guard let screen = NSScreen.main else { return }
        let sv = screen.visibleFrame
        let ox = sv.maxX - cardW - 16
        let oy = sv.minY + 16

        let panel = NSPanel(
            contentRect: NSRect(x: ox, y: oy, width: cardW, height: cardH),
            styleMask:   [.nonactivatingPanel, .titled, .closable, .fullSizeContentView],
            backing: .buffered, defer: false)
        panel.level                     = .floating
        panel.isMovableByWindowBackground = true
        panel.titlebarAppearsTransparent = true
        panel.backgroundColor           = NSColor.windowBackgroundColor.withAlphaComponent(0.96)
        panel.hasShadow                 = true

        // Animate in from slightly below
        var startFrame = panel.frame
        startFrame.origin.y -= 30
        panel.setFrame(startFrame, display: false)
        panel.orderFront(nil)
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.3
            panel.animator().setFrame(NSRect(x: ox, y: oy, width: cardW, height: cardH), display: true)
        }

        // Content
        let entry   = cachedJournal.last
        let n       = cachedState?.totalCycles ?? 0
        let rt      = RichText()
        rt.header("✓  Dream cycle \(n) complete")
        rt.divider()
        if let e = entry {
            let parts = [
                e.sessionsAnalyzed  > 0 ? "\(e.sessionsAnalyzed) sessions"  : nil,
                e.patternsExtracted > 0 ? "\(e.patternsExtracted) patterns" : nil,
                e.associationsFound > 0 ? "\(e.associationsFound) associations" : nil,
                e.insightsPromoted  > 0 ? "\(e.insightsPromoted) insights promoted" : nil,
            ].compactMap { $0 }.joined(separator: "  ·  ")
            if !parts.isEmpty { rt.ok(parts) }
            rt.dim("\(fmtNum(e.tokensUsed)) tokens used")
        }
        if let digest = cachedDigest {
            rt.spacer()
            let snippet = String(digest.prefix(160))
            rt.body(snippet.count < digest.count ? snippet + "…" : snippet)
        }

        let tv = NSTextView(frame: NSRect(x: 16, y: 12, width: cardW - 32, height: cardH - 36))
        tv.isEditable = false
        tv.isSelectable = false
        tv.backgroundColor = .clear
        tv.textStorage?.setAttributedString(rt.build())
        panel.contentView?.addSubview(tv)
        completionCard = panel

        // Fade out after 4s, remove after 6s
        DispatchQueue.main.asyncAfter(deadline: .now() + 4) { [weak self] in
            NSAnimationContext.runAnimationGroup({ ctx in
                ctx.duration = 2
                panel.animator().alphaValue = 0
            }, completionHandler: {
                panel.orderOut(nil)
                self?.completionCard = nil
            })
        }
        dlog("completion card shown for cycle \(n)")
    }

    private func checkCycleCompletion() {
        guard isCycling, let start = cycleStartTime else { return }
        // Safety timeout: 3 minutes
        if Date().timeIntervalSince(start) > 180 {
            dlog("cycle animation timeout"); stopDreamAnimation(); return
        }
        let progress = detectDreamProgress(since: start)
        if progress.isDone {
            dlog("cycle complete — trace detected")
            stopDreamAnimation()
            refresh()
            showCompletionCard()
        }
    }

    // ── Status bar button ──────────────────────────────────────────────────────

    private func updateButton() {
        guard let btn = statusItem.button else { return }
        // Icon: user-chosen symbol unless there's an error, then always exclamation
        let hasError = cachedBoard?.lastError != nil
        let baseSym  = hasError && !isCycling ? "exclamationmark.circle.fill" : currentIconSymbol()
        if let img = NSImage(systemSymbolName: baseSym, accessibilityDescription: "i-dream") {
            img.isTemplate = true
            btn.image = img
            btn.imagePosition = .imageLeft
        }
        if isCycling {
            // Colour-cycling indicator — elapsed time updates every 0.4s live in the status bar
            let color   = dreamAnimColors[animFrame % dreamAnimColors.count]
            let elapsed = cycleStartTime.map { fmtElapsed(Date().timeIntervalSince($0)) } ?? "…"
            btn.attributedTitle = NSAttributedString(string: " ◉ \(elapsed)", attributes: [
                .foregroundColor: color,
                .font: NSFont.systemFont(ofSize: 12, weight: .medium),
            ])
            btn.toolTip = "i-dream: dreaming… (\(elapsed))"
        } else if cachedRunning {
            let n = cachedState?.totalCycles ?? 0
            // Star-glow: show recency of last dream as fading sparkle (2h window, 3 tiers)
            var suffix = ""
            if let lastConsolid = isoDate(cachedState?.lastConsolidation) {
                let age = Date().timeIntervalSince(lastConsolid) / 7200.0  // 0–1 over 2h
                if age < 0.33 {
                    suffix = " ✦✦✦"
                } else if age < 0.66 {
                    suffix = " ✦✦"
                } else if age < 1.0 {
                    suffix = " ✦"
                }
            }
            btn.title   = " \(n)\(suffix)"
            btn.toolTip = "i-dream: running · \(n) cycles  [build: \(BuildInfo.commitHash)/\(BuildInfo.sourceHash)]"
        } else {
            btn.title   = ""
            btn.toolTip = "i-dream: stopped — click to manage  [build: \(BuildInfo.commitHash)/\(BuildInfo.sourceHash)]"
        }
    }

    // ── Menu construction ──────────────────────────────────────────────────────

    private func populateMenuItems(_ menu: NSMenu) {
        let running = cachedRunning
        let s       = cachedState
        let b       = cachedBoard

        // ─ Dreaming indicator ─────────────────────────────────────────────────
        if isCycling, let start = cycleStartTime {
            let progress = detectDreamProgress(since: start)
            let color    = dreamAnimColors[animFrame % dreamAnimColors.count]
            addColored(menu, "◉  Dreaming   \(fmtElapsed(progress.elapsed))",
                       color: color, font: .systemFont(ofSize: 13, weight: .semibold))
            addDim(menu, "  Phase: \(progress.phase)")
            menu.addItem(.separator())
        }

        // ─ Status header ──────────────────────────────────────────────────────
        let statusColor: NSColor = running ? .systemGreen : .systemOrange
        let statusText  = running ? "◉  i-dream  —  Running" : "○  i-dream  —  Stopped"
        addColored(menu, statusText, color: statusColor,
                   font: .systemFont(ofSize: 13, weight: .semibold))
        // Cognitive load gauge — inline with status
        if !cachedJournal.isEmpty {
            let load      = cognitiveLoadScore(journal: cachedJournal)
            let gauge     = fmtLoadGauge(load)
            let loadColor: NSColor = load > 0.75 ? .systemOrange : load > 0.4 ? .systemBlue : .secondaryLabelColor
            addRow(menu, "  Cognitive load", gauge, valueColor: loadColor)
        }
        // ─ Daemon controls ────────────────────────────────────────────────────
        if running {
            let s = add(menu, "Stop Daemon", #selector(stopDaemon))
            setIcon(s, "stop.fill")
        } else {
            let s = add(menu, "Start Daemon", #selector(startDaemon))
            setIcon(s, "play.fill")
        }
        let t = add(menu, "Trigger Dream Cycle", #selector(triggerCycleWithUsageCheck))
        setIcon(t, "arrow.triangle.2.circlepath")
        t.isEnabled = running && !isCycling

        // Usage limit warning row (only when over threshold)
        if let usage = s?.usage, usage.overWarnThreshold {
            let warn = NSMenuItem(title: "⚠ High usage — \(usage.warningLine)", action: nil, keyEquivalent: "")
            warn.isEnabled = false
            warn.attributedTitle = NSAttributedString(string: "⚠ High usage — \(usage.warningLine)", attributes: [
                .foregroundColor: NSColor.systemOrange,
                .font: NSFont.systemFont(ofSize: 11),
            ])
            menu.addItem(warn)
        }

        menu.addItem(.separator())

        // ─ Activity ───────────────────────────────────────────────────────────
        addSection(menu, "Activity")
        if let s = s {
            addRow(menu, "Cycles",      "\(s.totalCycles)",        valueColor: .systemBlue)
            // Usage window stats if limits are configured
            if let usage = s.usage, usage.limit5h > 0 || usage.limit7d > 0 {
                let usageStr = usage.warningLine
                let usageColor: NSColor = usage.overWarnThreshold ? .systemOrange : .systemGreen
                addRow(menu, "Usage", usageStr, valueColor: usageColor)
            }
            // Sparkline of token usage over last 20 cycles
            let spark = fmtSparkline(cachedJournal.map { $0.tokensUsed })
            let tokLabel = spark.isEmpty ? fmtNum(s.totalTokensUsed) : "\(fmtNum(s.totalTokensUsed))  \(spark)"
            addRow(menu, "Tokens used", tokLabel, valueColor: .systemBlue)
            addRow(menu, "Last run",    fmtDateWithAge(s.lastConsolidation))
            // last_activity in state.json is always null — read file mtime instead
            let lastAct = lastActivityDate()
            let lastActStr: String = lastAct.map { d in
                let d2 = Date().timeIntervalSince(d)
                let ago: String
                switch d2 {
                case ..<60:    ago = "just now"
                case ..<3600:  ago = "\(Int(d2 / 60))m ago"
                case ..<86400: ago = "\(Int(d2 / 3600))h ago"
                default:       ago = "\(Int(d2 / 86400))d ago"
                }
                return "\(fmtDateDirect(d))  (\(ago))"
            } ?? "—"
            addRow(menu, "Last active", lastActStr)
            let sigs = signalsCount()
            if sigs > 0 {
                addRow(menu, "User signals", "\(sigs)", valueColor: .systemPurple)
            }
        } else {
            addDim(menu, "  state.json not found")
        }

        // ─ Dream Frequency ────────────────────────────────────────────────────
        menu.addItem(.separator())
        addSection(menu, "Dream Frequency")
        let effectiveHz = cachedFrequencyHours ?? 4.0
        let freqLabel: String
        if effectiveHz < 1.0 {
            freqLabel = "\(Int(effectiveHz * 60))m"
        } else if effectiveHz == effectiveHz.rounded() {
            freqLabel = "\(Int(effectiveHz))h"
        } else {
            freqLabel = String(format: "%.1fh", effectiveHz)
        }
        let nextDream = nextDreamDate(thresholdHours: effectiveHz)
        let nextStr   = nextDream.map { fmtCountdown($0) } ?? "—"
        addRow(menu, "  Frequency", freqLabel, valueColor: .systemBlue)
        addRow(menu, "  Next dream", nextStr)

        // Submenu with frequency choices
        let freqMenu = NSMenu()
        let freqOptions: [(label: String, hours: Double)] = [
            ("30 minutes", 0.5),
            ("1 hour",     1.0),
            ("2 hours",    2.0),
            ("3 hours",    3.0),
            ("4 hours (default)", 4.0),
            ("6 hours",    6.0),
            ("9 hours",    9.0),
            ("12 hours",  12.0),
            ("18 hours",  18.0),
            ("24 hours",  24.0),
            ("36 hours",  36.0),
            ("48 hours",  48.0),
        ]
        for opt in freqOptions {
            let item = NSMenuItem(title: opt.label, action: #selector(setDreamFrequency(_:)),
                                  keyEquivalent: "")
            item.target = self
            item.representedObject = opt.hours
            item.state = (opt.hours == (cachedFrequencyHours ?? 4.0)) ? .on : .off
            freqMenu.addItem(item)
        }
        let freqParent = NSMenuItem(title: "  Change Frequency →", action: nil, keyEquivalent: "")
        setIcon(freqParent, "clock")
        menu.addItem(freqParent)
        menu.setSubmenu(freqMenu, for: freqParent)

        // ─ Knowledge Base ─────────────────────────────────────────────────────
        menu.addItem(.separator())
        addSection(menu, "Knowledge Base  (tap to explore)")
        if let b = b {
            let pi = addClickable(menu, "  Patterns",    "\(b.dreamsPatterns)",
                                  valueColor: .systemBlue, action: #selector(showPatternsDetail))
            setIcon(pi, "brain")
            let ai = addClickable(menu, "  Associations", "\(b.associations)",
                                  valueColor: .systemBlue, action: #selector(showAssociationsDetail))
            setIcon(ai, "link")
            let si = addClickable(menu, "  Sessions",
                                  "\(b.dreamsProcessed) dreams  ·  \(b.metacogProcessed) metacog",
                                  action: #selector(showSessionsDetail))
            setIcon(si, "book.fill")
            if b.metacogAudits > 0 {
                let mi = addClickable(menu, "  Metacog audits", "\(b.metacogAudits)",
                                      action: #selector(showMetacogDetail))
                setIcon(mi, "checkmark.seal.fill")
            }
        }

        // ─ Recent inferences ──────────────────────────────────────────────────
        if !cachedJournal.isEmpty || !cachedPatterns.isEmpty || cachedDigest != nil {
            menu.addItem(.separator())
            addSection(menu, "Recent Inferences")

            // Insight digest — "Recent Dreams Inference": prose synthesis of last 5 dream insights.
            // Sentiment is read from dreams/digest-meta.json { "sentiment": "positive"|"neutral"|"negative" }
            if let digest = cachedDigest {
                let sentiment = readDigestSentiment()
                let sentimentColor: NSColor = sentiment == "positive" ? .systemGreen
                                           : sentiment == "negative" ? .systemOrange
                                           : .labelColor
                let digestItem = NSMenuItem()
                let digestAttr = NSMutableAttributedString()
                let truncDigest = digest.count > 220 ? String(digest.prefix(217)) + "…" : digest
                digestAttr.append(NSAttributedString(string: "  \(truncDigest)\n",
                    attributes: [.font: NSFont.systemFont(ofSize: 13),
                                 .foregroundColor: sentimentColor]))
                digestAttr.append(NSAttributedString(string: "  Recent Dreams Inference  ·  updated every 3h",
                    attributes: [.font: NSFont.systemFont(ofSize: 11),
                                 .foregroundColor: NSColor.tertiaryLabelColor]))
                digestItem.attributedTitle = digestAttr
                digestItem.isEnabled = false
                // Golden-yellow sparkles icon tinted at render time
                if let img = NSImage(systemSymbolName: "sparkles", accessibilityDescription: "insights") {
                    let tintedImg = img.copy() as! NSImage
                    tintedImg.isTemplate = false
                    let gold = NSColor(red: 1.0, green: 0.80, blue: 0.10, alpha: 1.0)
                    let tinted = NSImage(size: tintedImg.size, flipped: false) { _ in
                        gold.setFill()
                        img.draw(in: NSRect(origin: .zero, size: tintedImg.size),
                                 from: .zero, operation: .sourceOver, fraction: 1.0)
                        return true
                    }
                    digestItem.image = tinted
                }
                menu.addItem(digestItem)

                // Re-trigger "Recent Dreams Inference" button
                let reInferItem = add(menu, "  ↺ Re-run Recent Dreams Inference",
                                      #selector(triggerRecentDreamsInference))
                setIcon(reInferItem, "arrow.clockwise.circle")
                reInferItem.indentationLevel = 1
            }

            // Show last cycle summary — with how long ago it happened
            if let latest = cachedJournal.last {
                let parts = [
                    latest.sessionsAnalyzed > 0 ? "\(latest.sessionsAnalyzed) sessions" : nil,
                    latest.patternsExtracted > 0 ? "\(latest.patternsExtracted) patterns" : nil,
                    latest.associationsFound > 0 ? "\(latest.associationsFound) associations" : nil,
                    latest.insightsPromoted  > 0 ? "\(latest.insightsPromoted) insights" : nil,
                ].compactMap { $0 }.joined(separator: "  ·  ")
                let summary = parts.isEmpty ? "skipped — no sessions" : parts
                addTwoLine(menu,
                           top:    "  Last cycle  \(fmtDate(latest.timestamp))  (\(timeAgo(latest.timestamp)))",
                           bottom: "  \(summary)  ·  \(fmtNum(latest.tokensUsed)) tokens")
            }
            // Show recent pattern learnings — hover to expand submenu with full details
            if !cachedPatterns.isEmpty {
                for p in cachedPatterns {
                    let truncated = p.pattern.count > 180 ? String(p.pattern.prefix(177)) + "…" : p.pattern
                    let sym  = valenceSymbol(p.valence)
                    // Confidence colour: green ≥85%, blue ≥65%, muted <65%
                    let confColor: NSColor = p.confidence >= 0.85 ? .systemGreen
                                          : p.confidence >= 0.65 ? .systemBlue
                                          : .secondaryLabelColor
                    let confDot = p.confidence >= 0.85 ? "●" : p.confidence >= 0.65 ? "◕" : "○"
                    let dateStr = p.firstSeen != nil ? "  ·  \(fmtDateWithAge(p.firstSeen))" : ""
                    let item = NSMenuItem()
                    let full = NSMutableAttributedString()
                    full.append(NSAttributedString(string: "  \(sym) \"\(truncated)\"\n",
                                                   attributes: [.font: NSFont.systemFont(ofSize: 14)]))
                    full.append(NSAttributedString(string: "  \(confDot) \(Int(p.confidence * 100))%  ·  \(p.category)\(dateStr)",
                                                   attributes: [
                                                       .font: NSFont.systemFont(ofSize: 12),
                                                       .foregroundColor: confColor,
                                                   ]))
                    item.attributedTitle = full
                    item.isEnabled = true
                    item.submenu = makePatternSubmenu(p)
                    setIcon(item, "sparkle")
                    menu.addItem(item)
                }
                // View All Insights link
                let viewAll = addClickable(menu, "  View All Insights →", "",
                                           action: #selector(showInsightsDetail))
                setIcon(viewAll, "list.bullet.rectangle")
            }
        }

        // ─ Last error ─────────────────────────────────────────────────────────
        if let err = b?.lastError {
            menu.addItem(.separator())
            addSection(menu, "⚠  Last Error  (today)")
            let errItem = NSMenuItem()
            let errFull = NSMutableAttributedString()
            let truncErr = err.count > 90 ? String(err.prefix(87)) + "…" : err
            errFull.append(NSAttributedString(string: "  " + truncErr + "\n",
                                              attributes: [
                                                  .foregroundColor: NSColor.systemRed,
                                                  .font: NSFont.systemFont(ofSize: 13),
                                              ]))
            errFull.append(NSAttributedString(string: "  click to copy",
                                              attributes: [
                                                  .font: NSFont.systemFont(ofSize: 11),
                                                  .foregroundColor: NSColor.tertiaryLabelColor,
                                              ]))
            errItem.attributedTitle = errFull
            errItem.action = #selector(copyItemText(_:))
            errItem.target = self
            errItem.isEnabled = true
            errItem.representedObject = err
            setIcon(errItem, "doc.on.clipboard")
            menu.addItem(errItem)
        }

        // ─ Store Health ───────────────────────────────────────────────────────
        if !cachedStoreFiles.isEmpty {
            menu.addItem(.separator())
            let hasWarnings = cachedStoreFiles.contains { $0.isLarge }
            addSection(menu, hasWarnings ? "⚠  Store Health" : "Store Health")
            for f in cachedStoreFiles {
                let prefix     = f.isLarge ? "⚠ " : "✓ "
                let valueColor: NSColor = f.isLarge ? .systemOrange : .secondaryLabelColor
                addRow(menu, "  \(prefix)\(f.label)",
                       "\(f.entries) entries · \(fmtBytes(f.sizeBytes))",
                       valueColor: valueColor)
            }
            if hasWarnings {
                let pruneItem = add(menu, "  Run Prune in Terminal…", #selector(runPrune))
                setIcon(pruneItem, "arrow.3.trianglepath")
            }
        }

        menu.addItem(.separator())

        // ─ Tools ──────────────────────────────────────────────────────────────
        // Ambient HUD toggle
        let hudVisible = UserDefaults.standard.bool(forKey: hudVisibleKey)
        let hudTitle = hudVisible ? "Hide Ambient HUD" : "Show Ambient HUD"
        let hudItem = add(menu, hudTitle, #selector(toggleHUD))
        setIcon(hudItem, hudVisible ? "eye.slash.fill" : "eye.fill")

        if hudVisible {
            let onTop   = UserDefaults.standard.bool(forKey: hudAlwaysOnTopKey)
            let pinItem = add(menu, onTop ? "  ✓ Always on Top" : "  Always on Top",
                              #selector(toggleHUDOnTop))
            pinItem.indentationLevel = 1
            _ = pinItem
        }

        let replay = add(menu, "Dream Replay…", #selector(showDreamReplay))
        setIcon(replay, "play.circle.fill")
        // Disable if no traces exist
        let traceFiles = (try? FileManager.default.contentsOfDirectory(atPath: tracesDir))?.filter { $0.hasSuffix(".jsonl") } ?? []
        replay.isEnabled = !traceFiles.isEmpty

        let dash = add(menu, "Open Dashboard", #selector(openDashboard))
        setIcon(dash, "chart.bar.doc.horizontal.fill")

        let howTo = add(menu, "Show How-To…", #selector(showHowTo))
        setIcon(howTo, "questionmark.circle.fill")

        let glossary = add(menu, "Terminology Glossary…", #selector(showTerminologyGlossary))
        setIcon(glossary, "text.book.closed.fill")

        let gh = add(menu, "View on GitHub", #selector(openGitHub))
        setIcon(gh, "arrow.up.right.square")

        let cfg = add(menu, "Edit Config in VS Code", #selector(openConfigInVSCode))
        setIcon(cfg, "gearshape.fill")

        // Logs submenu
        let logsMenu = NSMenu()
        let openLogsTermItem = NSMenuItem(title: "Open in Terminal", action: #selector(openLogs), keyEquivalent: "")
        openLogsTermItem.target = self; openLogsTermItem.isEnabled = true
        setIcon(openLogsTermItem, "terminal.fill")
        logsMenu.addItem(openLogsTermItem)
        let openLogsVSCItem = NSMenuItem(title: "Open in VS Code", action: #selector(openLogsInVSCode), keyEquivalent: "")
        openLogsVSCItem.target = self; openLogsVSCItem.isEnabled = true
        setIcon(openLogsVSCItem, "chevron.left.forwardslash.chevron.right")
        logsMenu.addItem(openLogsVSCItem)
        let openDebugItem = NSMenuItem(title: "Open Debug Log", action: #selector(openDebugLog), keyEquivalent: "")
        openDebugItem.target = self; openDebugItem.isEnabled = true
        setIcon(openDebugItem, "ant.fill")
        logsMenu.addItem(openDebugItem)
        let logsParent = NSMenuItem(title: "Logs", action: nil, keyEquivalent: "")
        setIcon(logsParent, "doc.text.magnifyingglass")
        menu.addItem(logsParent); menu.setSubmenu(logsMenu, for: logsParent)

        menu.addItem(.separator())

        // ─ Change Icon submenu ────────────────────────────────────────────────
        let iconMenu = NSMenu()
        let current  = currentIconSymbol()
        for choice in iconChoices {
            let i = NSMenuItem(title: choice.label, action: #selector(changeIcon(_:)),
                               keyEquivalent: "")
            i.target            = self
            i.representedObject = choice.symbol
            i.state             = (choice.symbol == current) ? .on : .off
            if let img = NSImage(systemSymbolName: choice.symbol,
                                 accessibilityDescription: nil) {
                img.isTemplate = true; i.image = img
            }
            iconMenu.addItem(i)
        }
        let iconParent = NSMenuItem(title: "Change Icon", action: nil, keyEquivalent: "")
        setIcon(iconParent, "paintbrush.pointed.fill")
        menu.addItem(iconParent); menu.setSubmenu(iconMenu, for: iconParent)

        menu.addItem(.separator())

        let r = add(menu, "Refresh", #selector(refresh))
        setIcon(r, "arrow.clockwise")
        r.keyEquivalent = "r"
        let q = NSMenuItem(title: "Quit",
                           action: #selector(NSApplication.terminate(_:)),
                           keyEquivalent: "q")
        setIcon(q, "power")
        menu.addItem(q)
    }

    // ── Menu item helpers ──────────────────────────────────────────────────────

    @discardableResult
    private func add(_ menu: NSMenu, _ title: String, _ sel: Selector) -> NSMenuItem {
        let i = NSMenuItem()
        i.attributedTitle = NSAttributedString(string: title,
                                               attributes: [.font: NSFont.systemFont(ofSize: 14)])
        i.action = sel; i.target = self; i.isEnabled = true
        menu.addItem(i); return i
    }

    private func addSection(_ menu: NSMenu, _ title: String) {
        let i = NSMenuItem()
        i.attributedTitle = NSAttributedString(string: title.uppercased(), attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .semibold),
            .foregroundColor: NSColor.labelColor.withAlphaComponent(0.7),
        ])
        i.isEnabled = false; menu.addItem(i)
    }

    private func addColored(_ menu: NSMenu, _ title: String,
                            color: NSColor, font: NSFont = .systemFont(ofSize: 15)) {
        let i = NSMenuItem()
        i.attributedTitle = NSAttributedString(string: title, attributes: [
            .font: font, .foregroundColor: color,
        ])
        i.isEnabled = false; menu.addItem(i)
    }

    private func addRow(_ menu: NSMenu, _ label: String, _ value: String,
                        valueColor: NSColor? = nil) {
        let i    = NSMenuItem()
        let full = NSMutableAttributedString()
        let pad  = max(1, 24 - label.count)
        full.append(NSAttributedString(string: "  \(label)" + String(repeating: " ", count: pad),
                                       attributes: [
                                           .font: NSFont.systemFont(ofSize: 14),
                                           .foregroundColor: NSColor.labelColor,
                                       ]))
        full.append(NSAttributedString(string: value, attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 14, weight: .regular),
            .foregroundColor: valueColor ?? NSColor.labelColor,
        ]))
        i.attributedTitle = full; i.isEnabled = false; menu.addItem(i)
    }

    /// Like addRow but clickable — shows a subtle › arrow and has an action.
    @discardableResult
    private func addClickable(_ menu: NSMenu, _ label: String, _ value: String,
                               valueColor: NSColor? = nil, action: Selector) -> NSMenuItem {
        let i    = NSMenuItem()
        let full = NSMutableAttributedString()
        let pad  = max(1, 24 - label.count)
        full.append(NSAttributedString(string: "\(label)" + String(repeating: " ", count: pad),
                                       attributes: [.font: NSFont.systemFont(ofSize: 14)]))
        full.append(NSAttributedString(string: value, attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 14, weight: .regular),
            .foregroundColor: valueColor ?? NSColor.labelColor,
        ]))
        full.append(NSAttributedString(string: "  ›", attributes: [
            .font: NSFont.systemFont(ofSize: 14),
            .foregroundColor: NSColor.tertiaryLabelColor,
        ]))
        i.attributedTitle = full; i.action = action; i.target = self; i.isEnabled = true
        menu.addItem(i); return i
    }

    private func addTwoLine(_ menu: NSMenu, top: String, bottom: String) {
        let i    = NSMenuItem()
        let full = NSMutableAttributedString()
        full.append(NSAttributedString(string: top + "\n",
                                       attributes: [
                                           .font: NSFont.systemFont(ofSize: 14),
                                           .foregroundColor: NSColor.labelColor,
                                       ]))
        full.append(NSAttributedString(string: bottom, attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.labelColor.withAlphaComponent(0.6),
        ]))
        i.attributedTitle = full; i.isEnabled = false; menu.addItem(i)
    }

    private func addDim(_ menu: NSMenu, _ title: String) {
        let i = NSMenuItem()
        i.attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: NSColor.labelColor.withAlphaComponent(0.6),
            .font: NSFont.systemFont(ofSize: 13),
        ])
        i.isEnabled = false; menu.addItem(i)
    }

    private func valenceSymbol(_ v: String) -> String {
        switch v {
        case "positive": return "+"
        case "negative": return "−"
        default:         return "◦"
        }
    }

    // ── Pattern detail submenu ────────────────────────────────────────────────
    // Built per-pattern item in "Recent Inferences". Shows full text, metadata
    // rows, and action items (copy, view all). Hover → submenu appears at right.

    private func makePatternSubmenu(_ p: Pattern) -> NSMenu {
        let sub = NSMenu()

        // ── Full text (non-truncated) ──────────────────────────────────────────
        let textItem = NSMenuItem()
        let textAttr = NSMutableAttributedString()
        textAttr.append(NSAttributedString(string: "  " + p.pattern,
            attributes: [
                .font:            NSFont.systemFont(ofSize: 13),
                .foregroundColor: NSColor.labelColor,
            ]))
        textItem.attributedTitle = textAttr
        textItem.isEnabled = false
        sub.addItem(textItem)

        sub.addItem(.separator())

        // ── Metadata rows ──────────────────────────────────────────────────────
        addRow(sub, "Category",   p.category)

        let confColor: NSColor = p.confidence >= 0.85 ? .systemGreen
                               : p.confidence >= 0.65 ? .systemBlue
                               : .secondaryLabelColor
        let confDot = p.confidence >= 0.85 ? "●●●●●"
                    : p.confidence >= 0.65 ? "●●●○○" : "●●○○○"
        addRow(sub, "Confidence", "\(confDot)  \(Int(p.confidence * 100))%",
               valueColor: confColor)

        let sym = valenceSymbol(p.valence)
        addRow(sub, "Valence", "\(sym)  \(p.valence)")

        if let fs = p.firstSeen, !fs.isEmpty {
            addRow(sub, "First seen", fmtDateWithAge(fs))
        }

        if let pid = p.id, !pid.isEmpty {
            addRow(sub, "ID", pid)
        }

        sub.addItem(.separator())

        // ── Actions ────────────────────────────────────────────────────────────
        let copyItem = NSMenuItem()
        copyItem.attributedTitle = NSAttributedString(string: "  Copy text",
            attributes: [.font: NSFont.systemFont(ofSize: 13)])
        copyItem.action = #selector(copyItemText(_:))
        copyItem.target = self
        copyItem.isEnabled = true
        copyItem.representedObject =
            "\(sym) \(p.pattern)\nCategory: \(p.category) | Confidence: \(Int(p.confidence * 100))%"
        setIcon(copyItem, "doc.on.clipboard")
        sub.addItem(copyItem)

        let viewAllItem = NSMenuItem()
        viewAllItem.attributedTitle = NSAttributedString(string: "  View All Insights →",
            attributes: [.font: NSFont.systemFont(ofSize: 13)])
        viewAllItem.action = #selector(showInsightsDetail)
        viewAllItem.target = self
        viewAllItem.isEnabled = true
        setIcon(viewAllItem, "list.bullet.rectangle")
        sub.addItem(viewAllItem)

        return sub
    }

    // ── SF Symbol icon helper ──────────────────────────────────────────────────

    private func setIcon(_ item: NSMenuItem, _ symbol: String) {
        if var img = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) {
            let cfg = NSImage.SymbolConfiguration(pointSize: 15, weight: .medium)
            img = img.withSymbolConfiguration(cfg) ?? img
            img.isTemplate = true
            item.image = img
        }
    }

    // ── Resizable detail panel ─────────────────────────────────────────────────

    /// Present a floating, resizable NSPanel with rich attributed text content.
    /// Replaces the old fixed-size NSAlert popups. If `filePath` is given, an
    /// "Open File" button is shown in the toolbar.
    private func showResizablePanel(title: String, content: NSAttributedString,
                                     filePath: String? = nil) {
        // Close and release any existing detail panel
        detailPanel?.close()
        detailPanel    = nil
        detailFilePath = filePath

        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 680),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing:     .buffered,
            defer:       false
        )
        panel.title                = title
        panel.isReleasedWhenClosed = false
        panel.level                = .floating
        panel.center()

        // ── Layout: frame-based (no Auto Layout) ────────────────────────────
        // Auto Layout + NSScrollView/NSTextView inside NSPanel has sizing
        // issues — the unconstrained contentView collapses to zero width
        // before constraints resolve. autoresizingMask is the correct pattern.
        let panW: CGFloat = 900
        let panH: CGFloat = 680
        let barH: CGFloat = 48

        // Scroll view fills panel minus toolbar at bottom
        let sv = NSScrollView(frame: NSRect(x: 0, y: barH, width: panW, height: panH - barH))
        sv.autoresizingMask      = [.width, .height]
        sv.hasVerticalScroller   = true
        sv.hasHorizontalScroller = false
        sv.autohidesScrollers    = true
        sv.borderType            = .noBorder

        let contentSize = sv.contentSize
        let tv = NSTextView(frame: NSRect(x: 0, y: 0,
                                         width: contentSize.width,
                                         height: contentSize.height))
        tv.minSize             = NSSize(width: 0, height: contentSize.height)
        tv.maxSize             = NSSize(width: CGFloat.greatestFiniteMagnitude,
                                       height: CGFloat.greatestFiniteMagnitude)
        tv.autoresizingMask    = .width
        tv.isEditable          = false
        tv.isSelectable        = true
        tv.backgroundColor     = .textBackgroundColor
        tv.textContainerInset  = NSSize(width: 14, height: 14)
        tv.isVerticallyResizable   = true
        tv.isHorizontallyResizable = false
        tv.textContainer?.containerSize = NSSize(width: contentSize.width,
                                                 height: CGFloat.greatestFiniteMagnitude)
        tv.textContainer?.widthTracksTextView = true
        sv.documentView = tv
        tv.textStorage?.setAttributedString(content)

        // Toolbar bar at bottom
        let bar = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: barH))
        bar.autoresizingMask = [.width]

        // Thin separator at top edge of bar
        let sep = NSBox(frame: NSRect(x: 0, y: barH - 1, width: panW, height: 1))
        sep.boxType          = .separator
        sep.autoresizingMask = [.width]
        bar.addSubview(sep)

        let closeBtn = NSButton(title: "Close", target: self,
                                action: #selector(closeDetailPanel))
        closeBtn.frame      = NSRect(x: panW - 92, y: 8, width: 80, height: 32)
        closeBtn.autoresizingMask = [.minXMargin]
        closeBtn.bezelStyle = .rounded
        bar.addSubview(closeBtn)

        if filePath != nil {
            let openBtn = NSButton(title: "Open File", target: self,
                                   action: #selector(openDetailFile))
            openBtn.frame      = NSRect(x: panW - 184, y: 8, width: 84, height: 32)
            openBtn.autoresizingMask = [.minXMargin]
            openBtn.bezelStyle = .rounded
            bar.addSubview(openBtn)
        }

        panel.contentView?.addSubview(sv)
        panel.contentView?.addSubview(bar)

        detailPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    @objc private func closeDetailPanel() {
        detailPanel?.close()
        detailPanel    = nil
        detailFilePath = nil
    }

    @objc private func openDetailFile() {
        if let fp = detailFilePath {
            NSWorkspace.shared.open(URL(fileURLWithPath: fp))
        }
    }

    @objc private func showPatternsDetail() {
        let patterns = allPatterns()
        guard !patterns.isEmpty else {
            alert("Patterns", "No patterns have been extracted yet."); return
        }
        let rt = RichText()
        rt.header("Behavioral & Cognitive Patterns")
        rt.dim("\(patterns.count) total patterns")
        rt.spacer()
        for p in patterns.suffix(15).reversed() {
            let val   = p.valence == "positive" ? "+" : p.valence == "negative" ? "−" : "◦"
            let since = p.firstSeen.map { "  ·  first seen \(fmtDate($0))" } ?? ""
            let label = "\(val)  \(p.pattern)"
            if p.valence == "positive"      { rt.ok(label) }
            else if p.valence == "negative" { rt.warn(label) }
            else                            { rt.subheader(label) }
            rt.dim("  \(p.category)  ·  \(Int(p.confidence * 100))% confident\(since)")
            rt.spacer()
        }
        if patterns.count > 15 { rt.dim("… and \(patterns.count - 15) earlier patterns") }
        showResizablePanel(title: "Patterns (\(patterns.count))",
                           content: rt.build(),
                           filePath: subDir + "/dreams/patterns.json")
        // Add "Network View →" and "Rate Insights →" buttons to the toolbar
        if let panel = detailPanel,
           let bar = panel.contentView?.subviews.first(where: { $0.frame.height == 48 && $0.frame.origin.y == 0 }) {
            let panW = panel.contentView?.bounds.width ?? 900
            let netBtn = NSButton(title: "Network View →", target: self,
                                  action: #selector(showPatternNetwork))
            netBtn.frame      = NSRect(x: 12, y: 8, width: 130, height: 32)
            netBtn.autoresizingMask = []
            netBtn.bezelStyle = .rounded
            bar.addSubview(netBtn)
            let rateBtn = NSButton(title: "Rate Insights →", target: self,
                                   action: #selector(showInsightsFeedback))
            rateBtn.frame      = NSRect(x: 155, y: 8, width: 130, height: 32)
            rateBtn.autoresizingMask = []
            rateBtn.bezelStyle = .rounded
            bar.addSubview(rateBtn)
            _ = panW
        }
    }

    // ── Insight Feedback ──────────────────────────────────────────────────────
    // Opens a panel with top-15 patterns; user can rate each thumbs-up/down.

    @objc private func showInsightsFeedback() {
        let patterns = allPatterns()
        guard !patterns.isEmpty else {
            alert("Rate Insights", "No patterns to rate yet."); return
        }
        feedbackPanel?.close(); feedbackPanel = nil

        let topPatterns = Array(patterns.sorted { $0.confidence > $1.confidence }.prefix(15))

        let panW: CGFloat = 620
        let panH: CGFloat = 600
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.title                = "Rate Insights"
        panel.isReleasedWhenClosed = false
        panel.level                = .floating
        panel.center()

        // Pre-measure each row's required text height so rows size to their content.
        let labelFont   = NSFont.systemFont(ofSize: 12)
        let textWidth   = panW - 96 - 124           // left margin + button column
        let measureAttrs: [NSAttributedString.Key: Any] = [.font: labelFont]
        let rowPadding: CGFloat = 20                // vertical padding per row

        let rowHeights: [CGFloat] = topPatterns.map { p in
            let measured = (p.pattern as NSString).boundingRect(
                with: NSSize(width: textWidth, height: .greatestFiniteMagnitude),
                options: [.usesLineFragmentOrigin, .usesFontLeading],
                attributes: measureAttrs)
            return max(20, ceil(measured.height)) + rowPadding
        }
        let totalContentH = rowHeights.reduce(0, +)
        let containerView = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: totalContentH))

        // Build rows bottom-up (NSView origin is bottom-left).
        // Item 0 (highest confidence) sits at the top of the scroll view.
        var yOffset: CGFloat = 0
        for i in stride(from: topPatterns.count - 1, through: 0, by: -1) {
            let p    = topPatterns[i]
            let rowH = rowHeights[i]
            let rowView = NSView(frame: NSRect(x: 0, y: yOffset, width: panW, height: rowH))
            rowView.autoresizingMask = [.width]

            // Confidence bar (vertically centred)
            let barW    = p.confidence * 80
            let barY    = (rowH - 16) / 2
            let confBar = NSView(frame: NSRect(x: 8, y: barY, width: barW, height: 16))
            let barColor: NSColor = p.valence == "positive" ? .systemGreen
                                  : p.valence == "negative" ? .systemRed : .systemBlue
            confBar.wantsLayer = true
            confBar.layer?.backgroundColor = barColor.withAlphaComponent(0.7).cgColor
            confBar.layer?.cornerRadius    = 3
            rowView.addSubview(confBar)

            // Pattern text — full text, wrapping allowed
            let textH   = rowH - rowPadding
            let label   = NSTextField(wrappingLabelWithString: p.pattern)
            label.font              = labelFont
            label.textColor         = .labelColor
            label.frame             = NSRect(x: 96, y: rowPadding / 2,
                                             width: textWidth, height: textH)
            label.autoresizingMask  = [.width]
            rowView.addSubview(label)

            // Thumbs-up button
            let upBtn = NSButton(title: "👍", target: self, action: #selector(insightRateUp(_:)))
            upBtn.frame             = NSRect(x: panW - 118, y: (rowH - 30) / 2, width: 50, height: 30)
            upBtn.autoresizingMask  = [.minXMargin]
            upBtn.bezelStyle        = .rounded
            upBtn.tag               = i
            rowView.addSubview(upBtn)

            // Thumbs-down button
            let downBtn = NSButton(title: "👎", target: self, action: #selector(insightRateDown(_:)))
            downBtn.frame           = NSRect(x: panW - 64, y: (rowH - 30) / 2, width: 50, height: 30)
            downBtn.autoresizingMask = [.minXMargin]
            downBtn.bezelStyle      = .rounded
            downBtn.tag             = i
            rowView.addSubview(downBtn)

            // Separator
            let sep = NSBox(frame: NSRect(x: 0, y: 0, width: panW, height: 1))
            sep.boxType = .separator; sep.autoresizingMask = [.width]
            rowView.addSubview(sep)

            containerView.addSubview(rowView)
            yOffset += rowH
        }

        // Store top patterns in a property so action handlers can reference them
        _feedbackPatterns = topPatterns

        let sv = NSScrollView(frame: NSRect(x: 0, y: 0, width: panW, height: panH))
        sv.autoresizingMask    = [.width, .height]
        sv.hasVerticalScroller = true
        sv.autohidesScrollers  = true
        sv.borderType          = .noBorder
        sv.documentView        = containerView
        panel.contentView?.addSubview(sv)

        feedbackPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    private var _feedbackPatterns: [Pattern] = []

    @objc private func insightRateUp(_ sender: NSButton) {
        let idx = sender.tag
        guard idx < _feedbackPatterns.count else { return }
        let p = _feedbackPatterns[idx]
        recordFeedback(patternId: p.pattern, rating: 1)
        markFeedbackRow(button: sender, rating: 1)
    }

    @objc private func insightRateDown(_ sender: NSButton) {
        let idx = sender.tag
        guard idx < _feedbackPatterns.count else { return }
        let p = _feedbackPatterns[idx]
        recordFeedback(patternId: p.pattern, rating: -1)
        markFeedbackRow(button: sender, rating: -1)
    }

    private func markFeedbackRow(button: NSButton, rating: Int) {
        guard let rowView = button.superview else { return }
        // Dim the row
        rowView.alphaValue = 0.45
        // Show "✓ rated" label
        let doneLabel = NSTextField(labelWithString: rating > 0 ? "✓ 👍" : "✓ 👎")
        doneLabel.font      = .systemFont(ofSize: 12, weight: .medium)
        doneLabel.textColor = .secondaryLabelColor
        doneLabel.frame     = NSRect(x: rowView.bounds.width - 116, y: 10, width: 104, height: 22)
        doneLabel.autoresizingMask = [.minXMargin]
        rowView.addSubview(doneLabel)
        // Disable both rating buttons in this row
        for sub in rowView.subviews {
            if let btn = sub as? NSButton { btn.isEnabled = false }
        }
    }

    private func recordFeedback(patternId: String, rating: Int) {
        let feedbackPath = subDir + "/dreams/insight-feedback.jsonl"
        let iso: String = {
            let fmt = ISO8601DateFormatter()
            fmt.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            return fmt.string(from: Date())
        }()
        let entry: [String: Any] = [
            "ts": iso,
            "pattern_id": patternId,
            "rating": rating,
            "source": "widget"
        ]
        guard let jsonData = try? JSONSerialization.data(withJSONObject: entry),
              let jsonStr  = String(data: jsonData, encoding: .utf8) else { return }
        let line = jsonStr + "\n"
        if FileManager.default.fileExists(atPath: feedbackPath) {
            if let fh = FileHandle(forWritingAtPath: feedbackPath) {
                fh.seekToEndOfFile()
                fh.write(line.data(using: .utf8) ?? Data())
                fh.closeFile()
            }
        } else {
            try? line.write(toFile: feedbackPath, atomically: true, encoding: .utf8)
        }
    }

    @objc private func showPatternNetwork() {
        let patterns = allPatterns()
        guard !patterns.isEmpty else {
            alert("Pattern Network", "No patterns to visualize yet."); return
        }
        networkPanel?.close()
        let panW: CGFloat = 860
        let panH: CGFloat = 660
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.title                = "Pattern Network  (\(patterns.count) patterns)"
        panel.isReleasedWhenClosed = false
        panel.level                = .floating
        panel.center()

        let graphH = panH - 48
        let graphView = PatternGraphView(
            frame: NSRect(x: 0, y: 48, width: panW, height: graphH),
            patterns: patterns)
        graphView.autoresizingMask = [.width, .height]
        panel.contentView?.addSubview(graphView)

        // Toolbar
        let bar = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: 48))
        bar.autoresizingMask = [.width]
        let sep = NSBox(frame: NSRect(x: 0, y: 47, width: panW, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]
        bar.addSubview(sep)
        let hint = NSTextField(labelWithString: "Click node to inspect  ·  Drag to pan  ·  Pinch / scroll to zoom  ·  Hover to see connections  ·  Dbl-click to reset  ·  \(patterns.count) patterns / \(Set(patterns.map { $0.category }).count) categories")
        hint.font        = .systemFont(ofSize: 11)
        hint.textColor   = .secondaryLabelColor
        hint.frame       = NSRect(x: 14, y: 14, width: panW - 200, height: 20)
        hint.autoresizingMask = [.width]
        bar.addSubview(hint)
        let closeBtn = NSButton(title: "Close", target: self, action: #selector(closeNetworkPanel))
        closeBtn.frame      = NSRect(x: panW - 92, y: 8, width: 80, height: 32)
        closeBtn.autoresizingMask = [.minXMargin]
        closeBtn.bezelStyle = .rounded
        bar.addSubview(closeBtn)
        panel.contentView?.addSubview(bar)

        networkPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    @objc private func closeNetworkPanel() {
        networkPanel?.close()
        networkPanel = nil
    }

    @objc private func showAssociationNetwork() {
        let assocs = allAssociations()
        guard !assocs.isEmpty else {
            alert("Association Network", "No associations to visualize yet."); return
        }
        associationNetworkPanel?.close()
        let panW: CGFloat = 860, panH: CGFloat = 660
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.title                = "Association Network  (\(assocs.count) hypotheses)"
        panel.isReleasedWhenClosed = false
        panel.level                = .floating
        panel.center()

        let graphH = panH - 48
        let graphView = AssociationGraphView(
            frame: NSRect(x: 0, y: 48, width: panW, height: graphH),
            associations: assocs)
        graphView.autoresizingMask = [.width, .height]
        panel.contentView?.addSubview(graphView)

        let actionableCount = assocs.filter { $0.actionable }.count
        let bar = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: 48))
        bar.autoresizingMask = [.width]
        let sep = NSBox(frame: NSRect(x: 0, y: 47, width: panW, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]
        bar.addSubview(sep)
        let legend = "Green=actionable ◆  Blue=non-actionable  ·  Rings: inner≥75% · mid≥50% · outer<50%  ·  Edges=shared patterns  ·  Dbl-click to reset"
        let hint = NSTextField(labelWithString: legend)
        hint.font        = .systemFont(ofSize: 10.5)
        hint.textColor   = .secondaryLabelColor
        hint.frame       = NSRect(x: 14, y: 14, width: panW - 200, height: 20)
        hint.autoresizingMask = [.width]
        bar.addSubview(hint)
        let sub = NSTextField(labelWithString: "\(assocs.count) associations  ·  \(actionableCount) actionable")
        sub.font = .systemFont(ofSize: 10); sub.textColor = .tertiaryLabelColor
        sub.frame = NSRect(x: 14, y: 0, width: 260, height: 14)
        bar.addSubview(sub)
        let closeBtn = NSButton(title: "Close", target: self, action: #selector(closeAssociationNetworkPanel))
        closeBtn.frame = NSRect(x: panW - 92, y: 8, width: 80, height: 32)
        closeBtn.autoresizingMask = [.minXMargin]; closeBtn.bezelStyle = .rounded
        bar.addSubview(closeBtn)
        panel.contentView?.addSubview(bar)

        associationNetworkPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    @objc private func closeAssociationNetworkPanel() {
        associationNetworkPanel?.close()
        associationNetworkPanel = nil
    }

    // ── Dream replay ─────────────────────────────────────────────────────────
    // Auto-plays through a trace JSONL file event by event at 500ms intervals.
    // Shows phase colour, kind, timestamp, and detail for each event.

    @objc private func showDreamReplay() {
        replayTimer?.invalidate(); replayTimer = nil
        replayPanel?.close();      replayPanel = nil

        let fm  = FileManager.default
        let all = ((try? fm.contentsOfDirectory(atPath: tracesDir))?.filter { $0.hasSuffix(".jsonl") }.sorted() ?? []).reversed() as [String]
        // newest-first
        let sorted = Array(all)
        guard let latest = sorted.first else {
            alert("Dream Replay", "No trace files found in \(tracesDir)"); return
        }
        replayTraceFiles = sorted

        func loadTrace(_ filename: String) {
            let tracePath = tracesDir + "/" + filename
            guard let raw = try? String(contentsOfFile: tracePath, encoding: .utf8) else { return }
            replayEvents = raw.components(separatedBy: "\n").filter { !$0.isEmpty }.compactMap { line in
                guard let d = line.data(using: .utf8),
                      let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any]
                else { return nil }
                return obj
            }
            replayIndex = 0
        }
        loadTrace(latest)

        let panW: CGFloat = 800
        let panH: CGFloat = 620
        let barH: CGFloat = 48
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.title                = "Dream Replay"
        panel.isReleasedWhenClosed = false
        panel.level                = .floating
        panel.center()

        // Scrollable text view
        let sv = NSScrollView(frame: NSRect(x: 0, y: barH, width: panW, height: panH - barH))
        sv.autoresizingMask    = [.width, .height]
        sv.hasVerticalScroller = true; sv.autohidesScrollers = true; sv.borderType = .noBorder
        let cs = sv.contentSize
        let tv = NSTextView(frame: NSRect(x: 0, y: 0, width: cs.width, height: cs.height))
        tv.minSize = NSSize(width: 0, height: cs.height)
        tv.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.autoresizingMask  = .width; tv.isEditable = false; tv.isSelectable = true
        tv.backgroundColor   = .textBackgroundColor; tv.textContainerInset = NSSize(width: 14, height: 14)
        tv.isVerticallyResizable = true; tv.isHorizontallyResizable = false
        tv.textContainer?.containerSize    = NSSize(width: cs.width, height: CGFloat.greatestFiniteMagnitude)
        tv.textContainer?.widthTracksTextView = true
        sv.documentView = tv
        replayTextView  = tv

        // Toolbar
        let bar = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: barH))
        bar.autoresizingMask = [.width]
        let sep = NSBox(frame: NSRect(x: 0, y: barH - 1, width: panW, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]; bar.addSubview(sep)

        let pauseBtn = NSButton(title: "⏸ Pause", target: self, action: #selector(toggleReplayPlayback))
        pauseBtn.frame = NSRect(x: 12, y: 8, width: 90, height: 32)
        pauseBtn.bezelStyle = .rounded; bar.addSubview(pauseBtn)
        replayPauseBtn = pauseBtn

        // File selector popup — lists all trace files newest-first
        let popup = NSPopUpButton(frame: NSRect(x: 110, y: 8, width: 500, height: 28), pullsDown: false)
        popup.bezelStyle = .rounded
        popup.autoresizingMask = [.width, .minXMargin]
        for filename in sorted {
            let title = Self.replayTraceLabel(filename: filename,
                                              eventCount: filename == latest ? replayEvents.count : nil)
            popup.addItem(withTitle: title)
        }
        popup.selectItem(at: 0)
        popup.target = self
        popup.action = #selector(replayFileSelected)
        bar.addSubview(popup)
        replayTracePopup = popup

        let closeBtn = NSButton(title: "Close", target: self, action: #selector(closeReplayPanel))
        closeBtn.frame = NSRect(x: panW - 92, y: 8, width: 80, height: 32)
        closeBtn.autoresizingMask = [.minXMargin]; closeBtn.bezelStyle = .rounded; bar.addSubview(closeBtn)

        panel.contentView?.addSubview(sv)
        panel.contentView?.addSubview(bar)
        replayPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)

        // Start auto-play
        startReplay()
    }

    /// Formats a trace filename like "20260416-2139-abc.jsonl" → "Apr 16 21:39  (N events)"
    private static func replayTraceLabel(filename: String, eventCount: Int?) -> String {
        // Expected prefix: YYYYMMDD-HHMM
        let base = (filename as NSString).deletingPathExtension
        let parts = base.components(separatedBy: "-")
        var dateStr = filename
        if parts.count >= 2 {
            let datePart = parts[0] // YYYYMMDD
            let timePart = parts[1] // HHMM
            if datePart.count == 8, timePart.count == 4 {
                let year  = Int(datePart.prefix(4)) ?? 0
                let month = Int(datePart.dropFirst(4).prefix(2)) ?? 0
                let day   = Int(datePart.dropFirst(6).prefix(2)) ?? 0
                let hour  = Int(timePart.prefix(2)) ?? 0
                let min   = Int(timePart.suffix(2)) ?? 0
                var cal   = Calendar(identifier: .gregorian)
                cal.timeZone = TimeZone.current
                if let date = cal.date(from: DateComponents(year: year, month: month, day: day,
                                                            hour: hour, minute: min)) {
                    let fmt = DateFormatter()
                    fmt.dateFormat = "MMM d HH:mm"
                    dateStr = fmt.string(from: date)
                }
            }
        }
        if let n = eventCount {
            return "\(dateStr)  (\(n) events)"
        }
        return dateStr
    }

    @objc private func replayFileSelected() {
        guard let popup = replayTracePopup else { return }
        let idx = popup.indexOfSelectedItem
        guard idx >= 0, idx < replayTraceFiles.count else { return }
        let filename = replayTraceFiles[idx]
        let tracePath = tracesDir + "/" + filename
        guard let raw = try? String(contentsOfFile: tracePath, encoding: .utf8) else { return }
        replayEvents = raw.components(separatedBy: "\n").filter { !$0.isEmpty }.compactMap { line in
            guard let d = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any]
            else { return nil }
            return obj
        }
        replayIndex = 0
        replayTimer?.invalidate(); replayTimer = nil
        replayTextView?.textStorage?.setAttributedString(NSAttributedString(string: ""))
        // Update popup title to include event count now that we know it
        popup.item(at: idx)?.title = Self.replayTraceLabel(filename: filename, eventCount: replayEvents.count)
        startReplay()
    }

    private func startReplay() {
        replayTimer?.invalidate()
        replayTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.replayStep()
        }
        replayPauseBtn?.title = "⏸ Pause"
    }

    @objc private func toggleReplayPlayback() {
        if replayTimer != nil {
            replayTimer?.invalidate(); replayTimer = nil
            replayPauseBtn?.title = "▶ Play"
        } else {
            startReplay()
        }
    }

    @objc private func replayStep() {
        guard replayIndex < replayEvents.count else {
            replayTimer?.invalidate(); replayTimer = nil
            replayPauseBtn?.title = "▶ Done"
            return
        }
        let obj   = replayEvents[replayIndex]
        let phase = obj["phase"] as? String ?? "–"
        let kind  = obj["kind"]  as? String ?? "event"
        let ts    = obj["ts"]    as? String ?? ""
        let det   = obj["details"] as? String ?? ""
        replayIndex += 1

        let phaseColor: NSColor
        switch phase {
        case "sws":  phaseColor = .systemBlue
        case "rem":  phaseColor = .systemPurple
        case "wake": phaseColor = .systemGreen
        default:     phaseColor = .secondaryLabelColor
        }
        let kindColor: NSColor = kind == "cycle_complete" ? .systemGreen
                               : kind.contains("error")   ? .systemRed
                               : .labelColor

        let line = NSMutableAttributedString()
        line.append(NSAttributedString(string: "[\(phase)]  ", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
            .foregroundColor: phaseColor,
        ]))
        line.append(NSAttributedString(string: kind, attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
            .foregroundColor: kindColor,
        ]))
        if !det.isEmpty {
            let truncDet = det.count > 120 ? String(det.prefix(117)) + "…" : det
            line.append(NSAttributedString(string: "\n  \(truncDet)", attributes: [
                .font: NSFont.systemFont(ofSize: 11),
                .foregroundColor: NSColor.secondaryLabelColor,
            ]))
        }
        line.append(NSAttributedString(string: "\n  \(ts)", attributes: [
            .font: NSFont.systemFont(ofSize: 10),
            .foregroundColor: NSColor.tertiaryLabelColor,
        ]))

        // ── Payload box (LLM text output / prompt) ───────────────────────────
        if let payload = obj["payload"] as? String, !payload.isEmpty {
            let payloadKind   = obj["payload_kind"] as? String ?? ""
            let isLLMResponse = kind == "api_response" || payloadKind == "text"
            // Never truncate: api_response and journal_written are full raw outputs
            // the user needs to see. Other payloads get a generous 4000-char window.
            let displayPayload: String
            if isLLMResponse || kind == "journal_written" {
                displayPayload = payload
            } else {
                displayPayload = payload.count > 4000
                    ? String(payload.prefix(4000)) + "\n  …[\(payload.count - 4000) more chars]"
                    : payload
            }

            let indentStyle = NSMutableParagraphStyle()
            indentStyle.headIndent          = 24
            indentStyle.firstLineHeadIndent = 24
            indentStyle.paragraphSpacing    = 2

            let (payColor, bgColor): (NSColor, NSColor)
            if kind == "journal_written" {
                // Journal writes get a warmer, amber-tinted display
                payColor = NSColor.systemYellow.blended(withFraction: 0.4, of: .systemOrange) ?? .systemYellow
                bgColor  = NSColor.systemYellow.withAlphaComponent(0.08)
            } else if isLLMResponse {
                payColor = phaseColor
                bgColor  = phaseColor.withAlphaComponent(0.12)
            } else {
                payColor = NSColor.tertiaryLabelColor
                bgColor  = NSColor.white.withAlphaComponent(0.04)
            }

            line.append(NSAttributedString(string: "\n\n", attributes: [
                .font: NSFont.systemFont(ofSize: 4),
            ]))
            // Apply syntax coloring for the payload using the shared color parser
            let coloredPayload = colorizePayload(displayPayload,
                                                 baseColor: payColor,
                                                 bgColor: bgColor,
                                                 indentStyle: indentStyle)
            line.append(coloredPayload)
        }

        line.append(NSAttributedString(string: "\n\n"))

        if let tv = replayTextView {
            tv.textStorage?.append(line)
            // Auto-scroll to bottom
            tv.scrollToEndOfDocument(nil)
        }
    }

    @objc private func closeReplayPanel() {
        replayTimer?.invalidate(); replayTimer = nil
        replayPanel?.close();      replayPanel = nil
        replayTextView = nil;      replayPauseBtn = nil
    }

    // ── Ambient HUD ─────────────────────────────────────────────────────────
    // A compact semi-transparent window pinned to the bottom-right corner.
    // Shows: running status, cognitive load gauge, sparkline, last cycle.
    // Refreshes every 30s (shares the main timer tick via updateHUD).

    @objc private func toggleHUD() {
        let nowVisible = UserDefaults.standard.bool(forKey: hudVisibleKey)
        UserDefaults.standard.set(!nowVisible, forKey: hudVisibleKey)
        if !nowVisible {
            showHUD()
        } else {
            hudPanel?.orderOut(nil)
            hudPanel = nil
            hudUpdateTimer?.invalidate()
            hudUpdateTimer = nil
        }
    }

    @objc private func toggleHUDOnTop() {
        let nowOn = UserDefaults.standard.bool(forKey: hudAlwaysOnTopKey)
        let nextOn = !nowOn
        UserDefaults.standard.set(nextOn, forKey: hudAlwaysOnTopKey)
        if let panel = hudPanel {
            panel.level = nextOn ? .statusBar : .floating
        }
        hudPinBtn?.title = nextOn ? "📌" : "📍"
    }

    @objc private func cycleHUDTimeRange() {
        hudTimeRangeIndex = (hudTimeRangeIndex + 1) % 3
        let labels = ["7d", "30d", "∞"]
        hudTimeRangeBtn?.title = labels[hudTimeRangeIndex]
        // Rebuild content immediately
        if let tv = hudPanel?.contentView?.subviews.compactMap({ $0 as? NSTextView }).first {
            updateHUDContent(tv)
        }
    }

    private func showHUD() {
        hudPanel?.orderOut(nil); hudPanel = nil
        hudUpdateTimer?.invalidate(); hudUpdateTimer = nil
        hudBarChart = nil
        hudPinBtn = nil
        hudTimeRangeBtn = nil

        let w: CGFloat       = 360
        let h: CGFloat       = 290
        let cornerR: CGFloat = 12
        guard let screen = NSScreen.main else { return }
        let sv = screen.visibleFrame
        let ox = sv.maxX - w - 12
        let oy = sv.minY + 12
        let onTop = UserDefaults.standard.bool(forKey: hudAlwaysOnTopKey)

        let panel = NSPanel(
            contentRect: NSRect(x: ox, y: oy, width: w, height: h),
            styleMask:   [.nonactivatingPanel, .fullSizeContentView],
            backing: .buffered, defer: false)
        panel.level                       = onTop ? .statusBar : .floating
        panel.isMovableByWindowBackground = true
        panel.backgroundColor             = .clear
        panel.alphaValue                  = 0.94
        panel.hasShadow                   = true
        panel.isOpaque                    = false
        panel.collectionBehavior          = [.canJoinAllSpaces, .stationary]
        panel.titlebarAppearsTransparent  = true

        // ── Layer: gradient bg + rounded corners + pulsing blue border ───────
        if let cv = panel.contentView {
            cv.wantsLayer           = true
            cv.layer?.cornerRadius  = cornerR
            cv.layer?.masksToBounds = true

            let grad = CAGradientLayer()
            grad.frame = cv.bounds
            grad.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
            grad.colors = [
                NSColor(red: 0.06, green: 0.10, blue: 0.18, alpha: 0.94).cgColor,
                NSColor(red: 0.02, green: 0.04, blue: 0.09, alpha: 0.96).cgColor,
            ]
            grad.startPoint   = CGPoint(x: 0.5, y: 1.0)
            grad.endPoint     = CGPoint(x: 0.5, y: 0.0)
            grad.cornerRadius = cornerR
            cv.layer?.insertSublayer(grad, at: 0)

            let border = CALayer()
            border.frame            = cv.bounds
            border.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
            border.cornerRadius     = cornerR
            border.borderWidth      = 1.0
            border.borderColor      = NSColor.systemBlue.withAlphaComponent(0.45).cgColor
            border.backgroundColor  = .none
            cv.layer?.addSublayer(border)

            let pulse = CABasicAnimation(keyPath: "borderColor")
            pulse.fromValue      = NSColor.systemBlue.withAlphaComponent(0.30).cgColor
            pulse.toValue        = NSColor.systemCyan.withAlphaComponent(0.80).cgColor
            pulse.duration       = 2.8
            pulse.autoreverses   = true
            pulse.repeatCount    = .infinity
            pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            border.add(pulse, forKey: "borderPulse")
        }

        let btnH:   CGFloat = 22
        let chartH: CGFloat = 50
        let chartY: CGFloat = 8
        let tvY:    CGFloat = chartY + chartH + 4
        let tvH:    CGFloat = h - tvY - btnH - 6

        // Text view — stats
        let tv = NSTextView(frame: NSRect(x: 12, y: tvY, width: w - 24, height: tvH))
        tv.isEditable      = false
        tv.isSelectable    = false
        tv.backgroundColor = .clear
        tv.drawsBackground = false
        panel.contentView?.addSubview(tv)

        // Bar chart view — token history
        let chart = MiniBarChartView(frame: NSRect(x: 12, y: chartY, width: w - 24, height: chartH))
        panel.contentView?.addSubview(chart)
        hudBarChart = chart

        // ── Top toolbar buttons ───────────────────────────────────────────────
        // Close button (✕) — top-left
        let closeBtn = NSButton(frame: NSRect(x: 6, y: h - btnH, width: 22, height: btnH))
        closeBtn.bezelStyle       = .inline
        closeBtn.isBordered       = false
        closeBtn.title            = "✕"
        closeBtn.font             = NSFont.systemFont(ofSize: 12)
        closeBtn.contentTintColor = NSColor.tertiaryLabelColor
        closeBtn.target           = self
        closeBtn.action           = #selector(toggleHUD)
        panel.contentView?.addSubview(closeBtn)

        // Time range button — centre-ish top
        let timeRangeLabels = ["7d", "30d", "∞"]
        let trBtn = NSButton(frame: NSRect(x: w / 2 - 18, y: h - btnH, width: 36, height: btnH))
        trBtn.bezelStyle       = .inline
        trBtn.isBordered       = false
        trBtn.title            = timeRangeLabels[hudTimeRangeIndex]
        trBtn.font             = NSFont.monospacedSystemFont(ofSize: 11, weight: .medium)
        trBtn.contentTintColor = NSColor.secondaryLabelColor
        trBtn.target           = self
        trBtn.action           = #selector(cycleHUDTimeRange)
        panel.contentView?.addSubview(trBtn)
        hudTimeRangeBtn = trBtn

        // Pin button — top-right (stored reference so toggleHUDOnTop can update it)
        let pinBtn = NSButton(frame: NSRect(x: w - 30, y: h - btnH, width: 24, height: btnH))
        pinBtn.bezelStyle       = .inline
        pinBtn.isBordered       = false
        pinBtn.title            = onTop ? "📌" : "📍"
        pinBtn.font             = NSFont.systemFont(ofSize: 12)
        pinBtn.target           = self
        pinBtn.action           = #selector(toggleHUDOnTop)
        panel.contentView?.addSubview(pinBtn)
        hudPinBtn = pinBtn

        hudPanel = panel
        updateHUDContent(tv)
        panel.orderFront(nil)

        hudUpdateTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self, weak tv] _ in
            guard let tv = tv else { return }
            self?.updateHUDContent(tv)
        }
    }

    /// Returns the journal entries filtered to the current hudTimeRangeIndex window.
    private func hudFilteredJournal() -> [JournalEntry] {
        guard !cachedJournal.isEmpty else { return [] }
        switch hudTimeRangeIndex {
        case 0: // 7 days
            let cutoff = Date().addingTimeInterval(-7 * 86400)
            return cachedJournal.filter { isoDate($0.timestamp).map { $0 >= cutoff } ?? true }
        case 1: // 30 days
            let cutoff = Date().addingTimeInterval(-30 * 86400)
            return cachedJournal.filter { isoDate($0.timestamp).map { $0 >= cutoff } ?? true }
        default: // all
            return cachedJournal
        }
    }

    /// Returns the latest calibration score from metacog/calibration.jsonl, or nil.
    private func latestCalibrationScore() -> Double? {
        let path = subDir + "/metacog/calibration.jsonl"
        guard let content = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
        let lines = content.components(separatedBy: "\n").filter { !$0.isEmpty }
        guard let lastLine = lines.last,
              let data  = lastLine.data(using: .utf8),
              let json  = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let score = json["calibration_score"] as? Double
        else { return nil }
        return score
    }

    /// Returns the count of active (non-expired) intentions.
    private func activeIntentionsCount() -> Int {
        let path = subDir + "/intentions/registry.jsonl"
        guard let content = try? String(contentsOfFile: path, encoding: .utf8) else { return 0 }
        let now = Date()
        return content.components(separatedBy: "\n").filter { line in
            guard !line.isEmpty,
                  let data   = line.data(using: .utf8),
                  let json   = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return false }
            // An intention is active if it has no expires_at, or its expires_at is in the future
            if let exp = json["expires_at"] as? String, let expDate = isoDate(exp) {
                return expDate > now
            }
            return true
        }.count
    }

    private func updateHUDContent(_ tv: NSTextView) {
        let dot: String     = isCycling ? "◉" : cachedRunning ? "◉" : "○"
        let dotColor: NSColor = isCycling ? dreamAnimColors[animFrame % dreamAnimColors.count]
                                          : cachedRunning ? .systemGreen : .systemOrange
        let buf   = NSMutableAttributedString()
        let fSz1: CGFloat = 14   // title / status line
        let fSz2: CGFloat = 13   // primary stats
        let fSz3: CGFloat = 12   // secondary / labels

        func label(_ text: String) {
            buf.append(NSAttributedString(string: text, attributes: [
                .font:            NSFont.systemFont(ofSize: fSz3),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ]))
        }
        func value(_ text: String, color: NSColor = .labelColor, mono: Bool = false) {
            let f: NSFont = mono
                ? NSFont.monospacedSystemFont(ofSize: fSz3, weight: .medium)
                : NSFont.systemFont(ofSize: fSz3, weight: .medium)
            buf.append(NSAttributedString(string: text, attributes: [
                .font: f, .foregroundColor: color,
            ]))
        }

        // ── Line 1: status dot + name + cycle count or elapsed ───────────────
        buf.append(NSAttributedString(string: "\(dot) i-dream  ", attributes: [
            .font:            NSFont.systemFont(ofSize: fSz1, weight: .semibold),
            .foregroundColor: dotColor,
        ]))
        if isCycling, let start = cycleStartTime {
            buf.append(NSAttributedString(string: "dreaming \(fmtElapsed(Date().timeIntervalSince(start)))", attributes: [
                .font:            NSFont.systemFont(ofSize: fSz2),
                .foregroundColor: NSColor.systemCyan,
            ]))
        } else if let n = cachedState?.totalCycles {
            buf.append(NSAttributedString(string: "\(n) cycles", attributes: [
                .font:            NSFont.systemFont(ofSize: fSz2),
                .foregroundColor: NSColor.secondaryLabelColor,
            ]))
        }
        buf.append(NSAttributedString(string: "\n"))

        // ── Line 2: load gauge + sparkline (time-range filtered) ─────────────
        let filteredJournal = hudFilteredJournal()
        if !filteredJournal.isEmpty {
            let load  = cognitiveLoadScore(journal: filteredJournal)
            let gauge = fmtLoadGauge(load)
            let spark = fmtSparkline(filteredJournal.map { $0.tokensUsed }, width: 14)
            let gaugeColor: NSColor = load >= 0.7 ? .systemOrange
                                    : load >= 0.4 ? .systemYellow
                                    : .systemGreen
            buf.append(NSAttributedString(string: "\(gauge)  ", attributes: [
                .font:            NSFont.monospacedSystemFont(ofSize: fSz2, weight: .regular),
                .foregroundColor: gaugeColor,
            ]))
            buf.append(NSAttributedString(string: "\(spark)\n", attributes: [
                .font:            NSFont.monospacedSystemFont(ofSize: fSz2, weight: .regular),
                .foregroundColor: NSColor.systemCyan,
            ]))
        }

        // ── Line 3: total tokens (filtered range) + last cycle ────────────────
        if let s = cachedState {
            let filteredTok = filteredJournal.reduce(0) { $0 + $1.tokensUsed }
            let totalTok    = s.totalTokensUsed
            let showFiltered = hudTimeRangeIndex < 2 && !filteredJournal.isEmpty
            let tokStr = showFiltered
                ? "\(fmtTokens(filteredTok)) / \(fmtTokens(totalTok)) total"
                : fmtTokens(totalTok)
            label("tokens  "); value("\(tokStr)\n", mono: true)
        }

        // ── Line 4: pattern count ─────────────────────────────────────────────
        if cachedPatternCount > 0 {
            label("patterns  ")
            value("\(cachedPatternCount)")
            if cachedHighConfCount > 0 {
                value("  (\(cachedHighConfCount) high-conf)\n", color: .systemGreen)
            } else {
                buf.append(NSAttributedString(string: "\n"))
            }
        }

        // ── Line 5: last cycle time + status ─────────────────────────────────
        if let s = cachedState, let last = s.lastConsolidation {
            label("last cycle  ")
            value("\(timeAgo(last))\n")
        } else {
            label("no cycles yet\n")
        }

        // ── Line 6: metacog calibration score ────────────────────────────────
        if let score = latestCalibrationScore() {
            label("calibration  ")
            let scoreColor: NSColor = score >= 0.7 ? .systemGreen
                                    : score >= 0.3 ? .systemYellow
                                    : score >= 0.0 ? .systemOrange
                                    : .systemRed
            value(String(format: "%.2f\n", score), color: scoreColor, mono: true)
        }

        // ── Line 7: active intentions ─────────────────────────────────────────
        let intentCount = activeIntentionsCount()
        if intentCount > 0 {
            label("intentions  ")
            value("\(intentCount) active\n")
        }

        // ── Line 8: next cycle estimate ──────────────────────────────────────
        if !isCycling, let lastActivity = lastActivityDate() {
            let idleHours: Double = 4   // default threshold
            let nextCycleDate = lastActivity.addingTimeInterval(idleHours * 3600)
            if nextCycleDate > Date() {
                let remaining = nextCycleDate.timeIntervalSince(Date())
                let rmStr: String
                if remaining < 3600 {
                    rmStr = "\(Int(remaining / 60))m"
                } else {
                    rmStr = "\(Int(remaining / 3600))h \(Int((remaining.truncatingRemainder(dividingBy: 3600)) / 60))m"
                }
                label("next cycle  ")
                value("~\(rmStr)\n", color: .secondaryLabelColor)
            } else {
                label("next cycle  ")
                value("idle — ready\n", color: .systemGreen)
            }
        }

        // ── Line 9: error line (only if error is newer than last cycle) ──────
        if let err = cachedBoard?.lastError {
            buf.append(NSAttributedString(string: "⚠  \(err)", attributes: [
                .font:            NSFont.systemFont(ofSize: fSz3 - 1),
                .foregroundColor: NSColor.systemOrange,
            ]))
        }

        tv.textStorage?.setAttributedString(buf)

        // Push filtered token history to bar chart
        hudBarChart?.values = filteredJournal.map { $0.tokensUsed }
    }

    /// Format token count as e.g. "348k" or "1.2M"
    private func fmtTokens(_ n: Int) -> String {
        if n >= 1_000_000 { return String(format: "%.1fM", Double(n) / 1_000_000) }
        if n >= 1_000     { return "\(n / 1_000)k" }
        return "\(n)"
    }

    @objc private func showAssociationsDetail() {
        let assocs = allAssociations()
        guard !assocs.isEmpty else {
            alert("Associations", "No cross-pattern hypotheses have been formed yet."); return
        }
        let rt = RichText()
        rt.header("Cross-Pattern Hypotheses")
        rt.dim("\(assocs.count) total associations")
        for (i, a) in assocs.reversed().enumerated() {
            rt.spacer()
            let confPct = Int(a.confidence * 100)
            let actionTag = a.actionable ? "  · actionable" : ""
            rt.dim("[\(assocs.count - i)]  \(confPct)% confident\(actionTag)")
            // Color hypothesis by confidence: ≥80% = green, ≥60% = body, <60% = dim
            if confPct >= 80        { rt.ok(a.hypothesis) }
            else if confPct >= 60   { rt.body(a.hypothesis) }
            else                    { rt.dim(a.hypothesis) }
            if let rule = a.suggestedRule, !rule.isEmpty {
                rt.accent("  → Rule: \(rule)")
            }
            rt.divider()
        }
        showResizablePanel(title: "Associations (\(assocs.count))",
                           content: rt.build(),
                           filePath: subDir + "/dreams/associations.json")

        // Add "Network View →" button to the panel toolbar
        if let panel = detailPanel,
           let bar = panel.contentView?.subviews.first(where: { $0.frame.height == 48 && $0.frame.origin.y == 0 }) {
            let netBtn = NSButton(title: "Network View →", target: self,
                                  action: #selector(showAssociationNetwork))
            netBtn.frame = NSRect(x: 12, y: 8, width: 144, height: 32)
            netBtn.autoresizingMask = []
            netBtn.bezelStyle = .rounded
            bar.addSubview(netBtn)
        }
    }

    @objc private func showMetacogDetail() {
        let (audit, filename) = readLatestAudit()
        guard let audit = audit else {
            alert("Metacog", "No metacognition audit data found.\n\nAudit files are created during background consolidation cycles. Ensure at least one cycle has completed with the metacog module enabled."); return
        }
        // Parse date from filename like "20260412-1032-audit.json"
        var dateStr = filename ?? ""
        if let fn = filename {
            let parts = fn.components(separatedBy: "-")
            if parts.count >= 2 {
                let df = DateFormatter()
                df.dateFormat = "yyyyMMdd HHmm"
                if let d = df.date(from: "\(parts[0]) \(parts[1])") {
                    dateStr = fmtDateWithAge(ISO8601DateFormatter().string(from: d))
                }
            }
        }
        let rt = RichText()
        rt.header("Metacognition Audit")
        if !dateStr.isEmpty { rt.dim("From: \(dateStr)") }

        // ── Calibration score ──────────────────────────────────────────────
        if let score = audit.calibrationScore {
            rt.spacer()
            rt.subheader("Calibration Score")
            let scoreLabel = score >= 0.8 ? "well-calibrated"
                           : score >= 0.5 ? "moderate"
                           : score >= 0.2 ? "under-calibrated"
                           : "poor"
            rt.body(String(format: "%.2f / 1.00  (%@)", score, scoreLabel))
            rt.dim("  1.0 = predictions match outcomes perfectly")
            rt.dim("  <0.5 = systematically over- or under-confident")
        }

        // ── Sample breakdown ───────────────────────────────────────────────
        let over   = audit.overconfidentCount  ?? 0
        let under  = audit.underconfidentCount ?? 0
        let well   = audit.wellCalibratedCount ?? 0
        let total  = over + under + well
        if total > 0 {
            rt.spacer()
            rt.subheader("Sample Breakdown  (\(total) units)")
            func pct(_ n: Int) -> String { total > 0 ? String(format: "%d%%", n * 100 / total) : "–" }
            rt.body(String(format: "  ✓ Well-calibrated   %3d  (%@)", well,  pct(well)))
            rt.body(String(format: "  ↑ Overconfident     %3d  (%@)", over,  pct(over)))
            rt.body(String(format: "  ↓ Underconfident    %3d  (%@)", under, pct(under)))
        }

        // ── Biases detected ────────────────────────────────────────────────
        if let biases = audit.biasesDetected, !biases.isEmpty {
            rt.spacer()
            rt.subheader("Biases Detected  (\(biases.count))")
            biases.forEach { rt.body("  • \($0)") }
        }

        // ── Recommendations ────────────────────────────────────────────────
        if let recs = audit.recommendations, !recs.isEmpty {
            rt.spacer()
            rt.subheader("Recommendations")
            recs.enumerated().forEach { i, r in rt.body("  \(i+1). \(r)") }
        }

        // ── Historical calibration trend ───────────────────────────────────
        let calPath = subDir + "/metacog/calibration.jsonl"
        if let calContent = try? String(contentsOfFile: calPath, encoding: .utf8) {
            let scores: [Double] = calContent
                .components(separatedBy: "\n").filter { !$0.isEmpty }
                .compactMap { line -> Double? in
                    guard let d = line.data(using: .utf8),
                          let j = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
                          let s = j["calibration_score"] as? Double else { return nil }
                    return s
                }
            if scores.count >= 2 {
                rt.spacer()
                rt.subheader("Calibration Trend  (last \(min(scores.count, 10)) cycles)")
                let window = Array(scores.suffix(10))
                let sparkVals = window.map { Int($0 * 10) }
                let avg = window.reduce(0, +) / Double(window.count)
                rt.mono("  \(fmtSparkline(sparkVals, width: 10))  avg \(String(format: "%.2f", avg))")
                let trend = (scores.last ?? 0) - (scores.first ?? 0)
                let trendStr = trend > 0.05 ? "↑ improving" : trend < -0.05 ? "↓ declining" : "→ stable"
                rt.dim("  Overall trend: \(trendStr)")
            }
        }

        let auditPath = filename.map { subDir + "/metacog/audits/" + $0 }
        showResizablePanel(title: "Metacog Audit", content: rt.build(), filePath: auditPath)
    }

    @objc private func showSessionsDetail() {
        let journal = allJournal()
        guard !journal.isEmpty else {
            alert("Sessions", "No dream journal entries yet."); return
        }
        let rt = RichText()
        rt.header("Dream Journal")
        rt.dim("\(journal.count) total cycles")

        // ── Sparkline history chart ──────────────────────────────────────────
        let window = Array(journal.suffix(20))
        if window.count >= 2 {
            let tokVals = window.map { $0.tokensUsed }
            let patVals = window.map { $0.patternsExtracted }
            let avgTok = tokVals.reduce(0, +) / tokVals.count
            let avgPat = patVals.reduce(0, +) / patVals.count
            rt.spacer()
            rt.subheader("Token & Pattern Trends  (last \(window.count) cycles)")
            rt.mono("Tokens/cycle   \(fmtSparkline(tokVals, width: 20))  avg \(fmtNum(avgTok))")
            rt.mono("Patterns/cycle \(fmtSparkline(patVals, width: 20))  avg \(avgPat)")
            rt.divider()
        }

        // Compute averages for color-coding (only non-skipped entries)
        let active = journal.filter { $0.sessionsAnalyzed > 0 }
        let avgSessions = active.isEmpty ? 0.0 : Double(active.map { $0.sessionsAnalyzed }.reduce(0,+)) / Double(active.count)
        let avgPatterns = active.isEmpty ? 0.0 : Double(active.map { $0.patternsExtracted }.reduce(0,+)) / Double(active.count)
        let avgAssocs   = active.isEmpty ? 0.0 : Double(active.map { $0.associationsFound }.reduce(0,+)) / Double(active.count)
        let avgInsights = active.isEmpty ? 0.0 : Double(active.map { $0.insightsPromoted  }.reduce(0,+)) / Double(active.count)
        let avgTokens   = active.isEmpty ? 0.0 : Double(active.map { $0.tokensUsed        }.reduce(0,+)) / Double(active.count)

        // Returns green/labelColor/orange based on whether value is high/normal/low vs average
        func heatColor(_ value: Int, avg: Double) -> NSColor {
            guard avg > 0 else { return .labelColor }
            let ratio = Double(value) / avg
            if ratio >= 1.3 { return .systemGreen }
            if ratio <= 0.5 { return .systemOrange }
            return .labelColor
        }

        for entry in journal.suffix(20).reversed() {
            rt.spacer()
            // Header: clickable link → opens cycle detail panel
            let headerText = "▸ \(fmtDate(entry.timestamp))  (\(timeAgo(entry.timestamp)))"
            if entry.id != nil {
                rt.linkSubheader(headerText, linkValue: entry.timestamp)
            } else {
                rt.subheader(headerText)
            }
            if entry.sessionsAnalyzed == 0 {
                rt.dim("  Skipped — no new sessions to consolidate")
            } else {
                // Color each metric relative to the cycle average
                let fields: [(String, Int, Double)] = [
                    ("Sessions analyzed  ", entry.sessionsAnalyzed,  avgSessions),
                    ("Patterns extracted ", entry.patternsExtracted, avgPatterns),
                    ("Associations found ", entry.associationsFound, avgAssocs),
                    ("Insights promoted  ", entry.insightsPromoted,  avgInsights),
                    ("Tokens used        ", entry.tokensUsed,        avgTokens),
                ]
                for (label, val, avg) in fields {
                    guard val > 0 else { continue }
                    let color = heatColor(val, avg: avg)
                    let valStr = label.contains("Tokens") ? fmtNum(val) : "\(val)"
                    let avgStr = label.contains("Tokens") ? fmtNum(Int(avg)) : String(format: "%.1f", avg)
                    let indicator = color == .systemGreen ? " ↑" : color == .systemOrange ? " ↓" : ""
                    rt.coloredLine("  \(label)  \(valStr)\(indicator)  (avg \(avgStr))", color: color)
                }
            }
        }
        if journal.count > 20 { rt.dim("… and \(journal.count - 20) earlier entries") }
        rt.dim("\n  ▸ Click a blue header to see patterns & associations for that cycle.")

        // Build the panel with a delegate so link-clicks work
        detailPanel?.close(); detailPanel = nil; detailFilePath = subDir + "/dreams/journal.jsonl"
        let content = rt.build()
        let panW: CGFloat = 900, panH: CGFloat = 680, barH: CGFloat = 48
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.title = "Dream Journal (\(journal.count) cycles)"
        panel.isReleasedWhenClosed = false; panel.level = .floating; panel.center()
        let sv = NSScrollView(frame: NSRect(x: 0, y: barH, width: panW, height: panH - barH))
        sv.autoresizingMask = [.width, .height]; sv.hasVerticalScroller = true
        sv.autohidesScrollers = true; sv.borderType = .noBorder
        let cs = sv.contentSize
        let tv = NSTextView(frame: NSRect(x: 0, y: 0, width: cs.width, height: cs.height))
        tv.minSize = NSSize(width: 0, height: cs.height)
        tv.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.autoresizingMask = .width; tv.isEditable = false; tv.isSelectable = true
        tv.backgroundColor = .textBackgroundColor; tv.textContainerInset = NSSize(width: 14, height: 14)
        tv.isVerticallyResizable = true; tv.isHorizontallyResizable = false
        tv.textContainer?.containerSize = NSSize(width: cs.width, height: CGFloat.greatestFiniteMagnitude)
        tv.textContainer?.widthTracksTextView = true
        sv.documentView = tv
        tv.textStorage?.setAttributedString(content)

        // Wire up the delegate so link-attributed headers are clickable
        let myJournal = journal
        journalLinkDelegate = JournalLinkDelegate { [weak self] ts in
            guard let self, let entry = myJournal.first(where: { $0.timestamp == ts }) else { return }
            self.showCycleDetail(for: entry)
        }
        tv.delegate = journalLinkDelegate

        let bar = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: barH))
        bar.autoresizingMask = [.width]
        let sep = NSBox(frame: NSRect(x: 0, y: barH - 1, width: panW, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]; bar.addSubview(sep)
        let openBtn = NSButton(title: "Open File", target: self, action: #selector(openDetailFile))
        openBtn.frame = NSRect(x: panW - 184, y: 8, width: 84, height: 32)
        openBtn.autoresizingMask = [.minXMargin]; openBtn.bezelStyle = .rounded; bar.addSubview(openBtn)
        let closeBtn = NSButton(title: "Close", target: self, action: #selector(closeDetailPanel))
        closeBtn.frame = NSRect(x: panW - 92, y: 8, width: 80, height: 32)
        closeBtn.autoresizingMask = [.minXMargin]; closeBtn.bezelStyle = .rounded; bar.addSubview(closeBtn)
        panel.contentView?.addSubview(sv); panel.contentView?.addSubview(bar)
        detailPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    /// Show a floating detail panel for one journal cycle entry.
    /// Finds matching patterns (by first_seen ±30 min), associations linked to those
    /// patterns, and a trace summary (if a matching trace file exists).
    private func showCycleDetail(for entry: JournalEntry) {
        cycleDetailPanel?.close(); cycleDetailPanel = nil

        let entryDate = isoDate(entry.timestamp) ?? Date()
        let windowSecs: TimeInterval = 30 * 60   // ±30 minutes

        // ── 1. Patterns active in this cycle ────────────────────────────────
        let cyclePats = allPatterns().filter { p in
            guard let fs = p.firstSeen, let d = isoDate(fs) else { return false }
            return abs(d.timeIntervalSince(entryDate)) <= windowSecs
        }
        let cyclePatIDs = Set(cyclePats.compactMap { $0.id })

        // ── 2. Associations linked to those patterns ─────────────────────────
        let cycleAssocs = allAssociations().filter { a in
            guard let linked = a.patternsLinked, !linked.isEmpty else { return false }
            return !linked.filter { cyclePatIDs.contains($0) }.isEmpty
        }

        // ── 3. Trace file summary ────────────────────────────────────────────
        var traceLines: [String] = []
        if let idPrefix = entry.id.map({ String($0.prefix(8)) }), !idPrefix.isEmpty {
            let fm = FileManager.default
            if let files = try? fm.contentsOfDirectory(atPath: tracesDir),
               let traceFile = files.first(where: { $0.hasSuffix(".jsonl") && $0.contains(idPrefix) }) {
                let path = tracesDir + "/" + traceFile
                if let raw = try? String(contentsOfFile: path, encoding: .utf8) {
                    traceLines = raw.components(separatedBy: "\n").filter { !$0.isEmpty }
                }
            }
        }

        // ── Build rich text ──────────────────────────────────────────────────
        let rt = RichText()
        rt.header("Cycle Detail")
        rt.subheader("\(fmtDate(entry.timestamp))  ·  \(timeAgo(entry.timestamp))")
        rt.spacer()

        // Summary stats row
        rt.subheader("Cycle Summary")
        if entry.sessionsAnalyzed > 0 {
            rt.body("  Sessions analyzed  \(entry.sessionsAnalyzed)")
            rt.body("  Patterns extracted \(entry.patternsExtracted)")
            rt.body("  Associations found \(entry.associationsFound)")
            rt.body("  Insights promoted  \(entry.insightsPromoted)")
            rt.body("  Tokens used        \(fmtNum(entry.tokensUsed))")
        } else {
            rt.dim("  Skipped — no new sessions to consolidate")
        }
        rt.divider()

        // Patterns section
        rt.subheader("Patterns (\(cyclePats.count))")
        if cyclePats.isEmpty {
            rt.dim("  No patterns matched to this cycle's timestamp window.")
        } else {
            for p in cyclePats.sorted(by: { $0.confidence > $1.confidence }) {
                let pct = Int(p.confidence * 100)
                let bar = String(repeating: "▮", count: pct / 10) + String(repeating: "░", count: 10 - pct / 10)
                rt.body("  \(bar)  \(pct)%  \(p.pattern)")
                let meta = [p.category, p.valence].filter { !$0.isEmpty }.joined(separator: "  ·  ")
                if !meta.isEmpty { rt.dim("        \(meta)") }
            }
        }
        rt.divider()

        // Associations section
        rt.subheader("Associations (\(cycleAssocs.count))")
        if cycleAssocs.isEmpty {
            rt.dim("  No associations linked to this cycle's patterns.")
        } else {
            for a in cycleAssocs.sorted(by: { $0.confidence > $1.confidence }) {
                let pct = Int(a.confidence * 100)
                rt.body("  \(pct)%  \(a.hypothesis)")
                if let rule = a.suggestedRule, !rule.isEmpty {
                    rt.dim("        Rule: \(rule)")
                }
            }
        }
        rt.divider()

        // Trace phase breakdown (if available)
        if !traceLines.isEmpty {
            rt.subheader("Trace Events (\(traceLines.count))")
            // Decode and show api_call + key events concisely
            struct TraceEvent: Decodable {
                let kind:    String?
                let phase:   String?
                let model:   String?
                let tokens:  Int?
                let message: String?
                enum CodingKeys: String, CodingKey {
                    case kind, phase, model, tokens, message
                }
            }
            let events = traceLines.compactMap { line -> TraceEvent? in
                guard let d = line.data(using: .utf8) else { return nil }
                return try? JSONDecoder().decode(TraceEvent.self, from: d)
            }
            var phaseTokens: [String: Int] = [:]
            for e in events {
                if e.kind == "api_call" || e.kind == "api_response",
                   let phase = e.phase, let tok = e.tokens {
                    phaseTokens[phase, default: 0] += tok
                }
            }
            if phaseTokens.isEmpty {
                rt.dim("  \(traceLines.count) trace events recorded.")
            } else {
                for (phase, tok) in phaseTokens.sorted(by: { $0.key < $1.key }) {
                    rt.body("  \(phase.capitalized.padding(toLength: 12, withPad: " ", startingAt: 0))  \(fmtNum(tok)) tokens")
                }
            }
        }

        // ── Panel setup ──────────────────────────────────────────────────────
        let panW: CGFloat = 680, panH: CGFloat = 560, barH: CGFloat = 44
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.title = "Cycle — \(fmtDate(entry.timestamp))"
        panel.isReleasedWhenClosed = false; panel.level = .floating

        // Offset from parent so both panels are visible at once
        if let parent = detailPanel { panel.setFrameOrigin(NSPoint(x: parent.frame.origin.x + 40,
                                                                    y: parent.frame.origin.y - 40)) }
        else { panel.center() }

        let sv = NSScrollView(frame: NSRect(x: 0, y: barH, width: panW, height: panH - barH))
        sv.autoresizingMask = [.width, .height]; sv.hasVerticalScroller = true
        sv.autohidesScrollers = true; sv.borderType = .noBorder
        let cs = sv.contentSize
        let tv = NSTextView(frame: NSRect(x: 0, y: 0, width: cs.width, height: cs.height))
        tv.minSize = NSSize(width: 0, height: cs.height)
        tv.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.autoresizingMask = .width; tv.isEditable = false; tv.isSelectable = true
        tv.backgroundColor = .textBackgroundColor; tv.textContainerInset = NSSize(width: 14, height: 14)
        tv.isVerticallyResizable = true; tv.isHorizontallyResizable = false
        tv.textContainer?.containerSize = NSSize(width: cs.width, height: CGFloat.greatestFiniteMagnitude)
        tv.textContainer?.widthTracksTextView = true
        sv.documentView = tv
        tv.textStorage?.setAttributedString(rt.build())

        let bar = NSView(frame: NSRect(x: 0, y: 0, width: panW, height: barH))
        bar.autoresizingMask = [.width]
        let sep = NSBox(frame: NSRect(x: 0, y: barH - 1, width: panW, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]; bar.addSubview(sep)
        let closeBtn = NSButton(title: "Close", target: self, action: #selector(closeCycleDetailPanel))
        closeBtn.frame = NSRect(x: panW - 92, y: 6, width: 80, height: 30)
        closeBtn.autoresizingMask = [.minXMargin]; closeBtn.bezelStyle = .rounded; bar.addSubview(closeBtn)
        panel.contentView?.addSubview(sv); panel.contentView?.addSubview(bar)
        cycleDetailPanel = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    @objc private func closeCycleDetailPanel() {
        cycleDetailPanel?.close(); cycleDetailPanel = nil
    }

    @objc private func showInsightsDetail() {
        guard let raw = readAllInsights(), !raw.isEmpty else {
            alert("Insights", "No dream insights have been recorded yet."); return
        }
        let rt = RichText()
        rt.header("Dream Insights")

        // Split on Wake Cycle boundaries to preserve the date for each insight.
        // Format: "## Wake Cycle — 2026-04-14 16:12 UTC\n\n### Insight..."
        let cycleParts = raw.components(separatedBy: "\n## Wake Cycle")
        var pairs: [(date: String, block: String)] = []
        for part in cycleParts.dropFirst() {
            // First line of each part = " — 2026-04-14 16:12 UTC"
            let eol = part.firstIndex(of: "\n") ?? part.endIndex
            let dateStr = String(part[part.startIndex..<eol])
                .replacingOccurrences(of: " — ", with: "")
                .trimmingCharacters(in: .whitespaces)
            let rest = String(part[eol...])
            for block in rest.components(separatedBy: "\n### Insight").dropFirst() {
                pairs.append((date: dateStr, block: block))
            }
        }

        let total = pairs.count
        let fb = readInsightFeedback()
        let rated = fb.count
        rt.dim("\(total) insight\(total == 1 ? "" : "s") recorded\(rated > 0 ? " · \(rated) rated" : "")")
        rt.spacer()

        // Render most-recent first
        for (date, block) in pairs.reversed() {
            renderInsight(rt, block: block, date: date, feedback: fb)
        }

        showResizablePanel(title: "All Insights (\(total))",
                           content: rt.build(),
                           filePath: subDir + "/dreams/insights.md")

        // Wire up feedback link clicks on the text view inside the panel
        if let contentView = detailPanel?.contentView,
           let scrollView = contentView.subviews.first(where: { $0 is NSScrollView }) as? NSScrollView,
           let textView = scrollView.documentView as? NSTextView {
            insightFeedbackDelegate = InsightFeedbackDelegate { [weak self] insightId, rating in
                self?.recordInsightFeedback(insightId: insightId, rating: rating)
                // Refresh panel to update button colors
                DispatchQueue.main.async { self?.showInsightsDetail() }
            }
            textView.delegate = insightFeedbackDelegate
        }
    }

    /// Render one `### Insight` block into `rt`.
    private func renderInsight(_ rt: RichText, block: String, date: String,
                               feedback: [String: String] = [:]) {
        let lines = block.components(separatedBy: "\n")

        // First line is the insight header suffix, e.g. " (conf=0.87)"
        let headerLine = lines.first ?? ""
        var confLabel = ""
        if let range = headerLine.range(of: "conf=") {
            let num = headerLine[range.upperBound...].prefix(while: { $0.isNumber || $0 == "." })
            if let d = Double(num) { confLabel = "  \(Int(d * 100))% confidence" }
        }
        rt.subheader("Insight\(confLabel)")

        for line in lines.dropFirst() {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.isEmpty || t == "---" { continue }
            if t.hasPrefix(">") {
                // Hypothesis text → blue accent
                rt.accent(String(t.dropFirst()).trimmingCharacters(in: .whitespaces))
            } else if t.hasPrefix("**") {
                // **Rule:** … — strip markers, show as medium-weight subheader
                let stripped = t.replacingOccurrences(of: "**", with: "")
                rt.subheader(stripped)
            } else if t.hasPrefix("_") && t.hasSuffix("_") {
                // _Patterns: uuid1, uuid2_ — strip markers, show as muted gray
                let stripped = String(t.dropFirst().dropLast())
                rt.dim(stripped)
            } else {
                rt.body(t)
            }
        }

        // Date stamp at the bottom of each insight
        if !date.isEmpty { rt.dim("  \(date)") }

        // Feedback links (👍/👎)
        let insightId = Self.extractInsightId(from: block)
        let existing = feedback[insightId]
        let fb = NSMutableAttributedString()
        fb.append(NSAttributedString(string: "  "))
        fb.append(NSAttributedString(string: existing == "up" ? "✓ Helpful" : "👍 Helpful", attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .medium),
            .foregroundColor: existing == "up" ? NSColor.systemGreen : NSColor.tertiaryLabelColor,
            .link: "insight-up:\(insightId)" as AnyObject,
        ]))
        fb.append(NSAttributedString(string: "    "))
        fb.append(NSAttributedString(string: existing == "down" ? "✗ Not useful" : "👎 Not useful", attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .medium),
            .foregroundColor: existing == "down" ? NSColor.systemRed : NSColor.tertiaryLabelColor,
            .link: "insight-down:\(insightId)" as AnyObject,
        ]))
        fb.append(NSAttributedString(string: "\n"))
        rt.raw(fb)

        rt.divider()
    }

    /// Extract a stable identifier from an insight block (first pattern UUID, or hash fallback).
    private static func extractInsightId(from block: String) -> String {
        if let range = block.range(of: "_Patterns:") {
            let after = String(block[range.upperBound...])
            let cleaned = after.trimmingCharacters(in: .whitespacesAndNewlines)
                               .replacingOccurrences(of: "_", with: "")
            let firstUUID = cleaned.components(separatedBy: ",").first?
                .trimmingCharacters(in: .whitespaces) ?? ""
            if firstUUID.count >= 8 { return firstUUID }
        }
        // Fallback: djb2 hash of first 100 chars
        var hash: UInt64 = 5381
        for c in block.prefix(100).utf8 { hash = hash &* 33 &+ UInt64(c) }
        return String(format: "%016llx", hash)
    }

    /// Read existing insight feedback from dreams/insight-feedback.jsonl → [insightId: "up"|"down"].
    private func readInsightFeedback() -> [String: String] {
        let path = subDir + "/dreams/insight-feedback.jsonl"
        guard let raw = try? String(contentsOfFile: path, encoding: .utf8) else { return [:] }
        var result: [String: String] = [:]
        for line in raw.components(separatedBy: "\n") where !line.isEmpty {
            guard let data = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let id = obj["insight_id"] as? String,
                  let rating = obj["rating"] as? String
            else { continue }
            result[id] = rating   // last entry wins (allows changing your mind)
        }
        return result
    }

    /// Record a feedback action to dreams/insight-feedback.jsonl.
    private func recordInsightFeedback(insightId: String, rating: String) {
        let path = subDir + "/dreams/insight-feedback.jsonl"
        let ts = ISO8601DateFormatter().string(from: Date())
        let entry: [String: Any] = ["ts": ts, "insight_id": insightId, "rating": rating]
        guard let data = try? JSONSerialization.data(withJSONObject: entry),
              let line = String(data: data, encoding: .utf8)
        else { return }
        let content = line + "\n"
        if let fh = FileHandle(forWritingAtPath: path) {
            fh.seekToEndOfFile()
            fh.write(content.data(using: .utf8) ?? Data())
            fh.closeFile()
        } else {
            try? content.write(toFile: path, atomically: true, encoding: .utf8)
        }
        dlog("insight feedback: \(rating) for \(insightId.prefix(8))")
    }

    @objc private func setDreamFrequency(_ sender: NSMenuItem) {
        guard let hours = sender.representedObject as? Double else { return }
        writeDreamFrequency(hours)
        cachedFrequencyHours = hours
        dlog("dream frequency set to \(hours)h")
        // Refresh button/menu to show updated "next dream" time
        refresh()
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    @objc private func startDaemon() {
        dlog("startDaemon: trying 'i-dream service start'")
        let svc = Process()
        svc.executableURL = URL(fileURLWithPath: iDream)
        svc.arguments     = ["service", "start"]
        svc.standardOutput = FileHandle.nullDevice; svc.standardError = FileHandle.nullDevice
        do {
            try svc.run(); svc.waitUntilExit()
            dlog("service start exit=\(svc.terminationStatus)")
            if svc.terminationStatus == 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) { self.refresh() }
                return
            }
        } catch { dlog("service start threw: \(error)") }

        dlog("startDaemon: falling back to direct launch")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: iDream)
        p.arguments     = ["start", "--daemonize"]
        p.standardOutput = FileHandle.nullDevice; p.standardError = FileHandle.nullDevice
        do {
            try p.run()
            dlog("direct start launched PID=\(p.processIdentifier)")
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.5) { self.refresh() }
        } catch {
            dlog("direct start failed: \(error)")
            alert("Start Failed",
                  "Could not start i-dream.\n\nError: \(error.localizedDescription)\n\nSee: /tmp/i-dream-bar.log")
        }
    }

    @objc private func stopDaemon() {
        dlog("stopDaemon")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: iDream); p.arguments = ["stop"]
        p.standardOutput = FileHandle.nullDevice; p.standardError = FileHandle.nullDevice
        try? p.run(); p.waitUntilExit()
        dlog("stop exit=\(p.terminationStatus)")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.refresh() }
    }

    @objc private func runPrune() {
        dlog("runPrune")
        openInTerminal("\(iDream) prune")
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { self.refresh() }
    }

    /// Trigger a dream cycle, first checking usage limits.
    /// If usage is over the warn threshold, shows a confirm dialog
    /// with the current usage numbers before proceeding.
    @objc private func triggerCycleWithUsageCheck() {
        if let usage = cachedState?.usage, usage.overWarnThreshold {
            let alert = NSAlert()
            alert.messageText = "High Claude Usage — Proceed?"
            alert.informativeText = """
                Your Claude Code session usage is near its limit:

                \(usage.warningLine)

                Running a dream cycle will consume additional API tokens. \
                Automatic cycles are paused until usage resets.

                Proceed with manual trigger anyway?
                """
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Run Dream Cycle")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }
        dlog("triggerCycle (usage-checked)")
        triggerCycle()
    }

    @objc private func triggerCycle() {
        dlog("triggerCycle")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: iDream); p.arguments = ["dream"]
        p.standardOutput = FileHandle.nullDevice; p.standardError = FileHandle.nullDevice
        try? p.run()
        isCycling      = true
        cycleStartTime = Date()
        startDreamAnimation()
    }

    /// Re-run the Recent Dreams Inference (digest generation + sentiment tagging).
    /// Runs `i-dream dream wake` which re-triggers the Wake phase that synthesizes
    /// the digest from the top dream insights. Terminology: "Recent Dreams Inference"
    /// is the process of synthesizing recent high-confidence patterns into a prose summary
    /// with a sentiment tag (positive / neutral / negative).
    @objc private func triggerRecentDreamsInference() {
        dlog("triggerRecentDreamsInference")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: iDream)
        p.arguments     = ["dream", "wake"]
        p.standardOutput = FileHandle.nullDevice; p.standardError = FileHandle.nullDevice
        try? p.run()
        isCycling      = true
        cycleStartTime = Date()
        startDreamAnimation()
        // Refresh after a short delay to pick up the new digest
        DispatchQueue.main.asyncAfter(deadline: .now() + 8) { [weak self] in
            self?.refresh()
        }
    }

    @objc private func changeIcon(_ sender: NSMenuItem) {
        guard let sym = sender.representedObject as? String else { return }
        dlog("changeIcon: \(sym)")
        UserDefaults.standard.set(sym, forKey: iconDefaultsKey)
        updateButton()
    }

    @objc private func openDashboard() {
        // Regenerate the dashboard before opening so it always reflects current state.
        let p = Process()
        p.executableURL = URL(fileURLWithPath: iDream)
        p.arguments = ["dashboard"]
        p.standardOutput = FileHandle.nullDevice; p.standardError = FileHandle.nullDevice
        try? p.run(); p.waitUntilExit()
        NSWorkspace.shared.open(URL(fileURLWithPath: subDir + "/dashboard.html"))
    }

    @objc private func openLogs() {
        openInTerminal("tail -f '\(bestLogPath())'")
    }

    @objc private func openLogsInVSCode() {
        let logURL = URL(fileURLWithPath: bestLogPath())
        for bundleID in ["com.microsoft.VSCode", "com.visualstudio.code"] {
            if let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) {
                NSWorkspace.shared.open([logURL], withApplicationAt: appURL,
                                        configuration: NSWorkspace.OpenConfiguration()) { _, _ in }
                return
            }
        }
        // VS Code not found — open with default text editor
        NSWorkspace.shared.open(logURL)
    }

    @objc private func openDebugLog() {
        openInTerminal("tail -f '\(debugLog)'")
    }

    @objc private func showStatus() {
        var lines = ["Daemon:       \(cachedRunning ? "Running ◉" : "Stopped ○")"]
        if let s = cachedState {
            lines += ["", "Cycles:       \(s.totalCycles)",
                      "Tokens used:  \(fmtNum(s.totalTokensUsed))", "",
                      "Last run:     \(fmtDate(s.lastConsolidation))",
                      "              (\(timeAgo(s.lastConsolidation)))",
                      "Last active:  \(lastActivityDate().map { fmtDateDirect($0) } ?? "—")"]
        }
        if let b = cachedBoard {
            lines += ["", "Patterns:     \(b.dreamsPatterns)",
                      "Associations: \(b.associations)",
                      "Sessions:     \(b.dreamsProcessed) dreams / \(b.metacogProcessed) metacog"]
            if b.metacogAudits > 0 { lines.append("Audits:       \(b.metacogAudits)") }
            if let e = b.lastError { lines += ["", "Last error:", e] }
        }
        if !cachedJournal.isEmpty {
            lines.append(""); lines.append("Recent cycles:")
            for e in cachedJournal {
                lines.append("  \(fmtDate(e.timestamp))  →  \(e.sessionsAnalyzed) sessions, "
                    + "\(e.patternsExtracted) patterns, \(e.insightsPromoted) insights  "
                    + "(\(fmtNum(e.tokensUsed)) tkns)")
            }
        }
        let a = NSAlert()
        a.messageText = "i-dream Status"; a.informativeText = lines.joined(separator: "\n")
        a.alertStyle  = .informational; a.addButton(withTitle: "OK"); a.runModal()
    }

    @objc private func showTerminologyGlossary() {
        let rt = RichText()
        rt.header("i-dream — Terminology Glossary")
        rt.dim("Reference for all terms, phases, and concepts used in i-dream")
        rt.spacer()

        // ── Modules ────────────────────────────────────────────────────────
        rt.subheader("Modules")
        rt.body("  Dreaming      Background sleep-cycle-inspired memory consolidation.")
        rt.body("                Runs when idle 4h+. Three phases: SWS → REM → Wake.")
        rt.body("  Metacognition  Samples execution units and scores calibration quality.")
        rt.body("                Detects reasoning biases like anchoring and scope creep.")
        rt.body("  Introspection Weekly analysis of reasoning chains for depth/breadth.")
        rt.body("  Intuition      Surfaces \"gut feelings\" from valence memory at session start.")
        rt.body("  Prospective    Fires condition-action reminders when context matches.")
        rt.spacer()

        // ── Dream phases ───────────────────────────────────────────────────
        rt.subheader("Dream Phases")
        rt.body("  SWS (Slow-Wave Sleep)")
        rt.body("    Scans unprocessed session transcripts. Extracts behavioral")
        rt.body("    patterns (temp=0.3, structured output). Deduplicates by")
        rt.body("    normalized string and merges occurrence counts.")
        rt.spacer()
        rt.body("  REM (Rapid Eye Movement)")
        rt.body("    Takes top-confidence patterns across sessions. Finds creative")
        rt.body("    cross-domain associations (temp=0.9). Builds a hypothesis graph.")
        rt.spacer()
        rt.body("  Wake")
        rt.body("    Verifies insights against current filesystem state. Promotes")
        rt.body("    high-confidence patterns to dreams/insights.md. Also generates")
        rt.body("    the Recent Dreams Inference summary.")
        rt.spacer()

        // ── Key terms ──────────────────────────────────────────────────────
        rt.subheader("Key Terms")
        rt.body("  Recent Dreams Inference")
        rt.body("    A prose synthesis of the last 5 high-confidence dream insights,")
        rt.body("    with a sentiment tag (positive / neutral / negative) describing")
        rt.body("    the overall trajectory. Generated during the Wake phase and")
        rt.body("    shown in the menu under \"Recent Inferences\". Can be re-triggered")
        rt.body("    manually from the menu.")
        rt.spacer()
        rt.body("  Consolidation Cycle")
        rt.body("    One complete run of all idle-time modules (Dreaming, Metacog,")
        rt.body("    Introspection). Triggered after 4h+ of inactivity. Counted in")
        rt.body("    state.json as total_cycles.")
        rt.spacer()
        rt.body("  Valence Memory")
        rt.body("    Time-decayed pattern-outcome associations. Each entry stores a")
        rt.body("    pattern string and how previous encounters turned out (positive /")
        rt.body("    negative / neutral). Drives the Intuition module. Decays with a")
        rt.body("    30-day half-life by default.")
        rt.spacer()
        rt.body("  Execution Unit")
        rt.body("    A single bounded reasoning action sampled by Metacognition —")
        rt.body("    typically one tool call or a small group of related actions.")
        rt.body("    Scored for calibration, overconfidence, and bias markers.")
        rt.spacer()
        rt.body("  Calibration Score")
        rt.body("    A [-1.0, 1.0] value where 1.0 = predictions perfectly match")
        rt.body("    outcomes. Negative = systematically overconfident. Computed by")
        rt.body("    the Metacog module over a batch of execution units per cycle.")
        rt.spacer()
        rt.body("  Priming / Priming Decay")
        rt.body("    When an intuition fires at session start, it is \"primed\" — the")
        rt.body("    association stays active for 4h before decaying. Used to avoid")
        rt.body("    surfacing the same hint twice in quick succession.")
        rt.spacer()
        rt.body("  Intention")
        rt.body("    A condition-action pair stored by the Prospective module.")
        rt.body("    Example: if the current project touches auth code, remind to")
        rt.body("    check the rate-limiting layer. Fires once per session match.")
        rt.spacer()
        rt.body("  Insight Digest")
        rt.body("    Another name for Recent Dreams Inference — the stored file is")
        rt.body("    dreams/insight-digest.md. The sentiment is in digest-meta.json.")
        rt.spacer()
        rt.body("  Dream Replay")
        rt.body("    Step-through viewer for the JSONL event trace of a past cycle.")
        rt.body("    Shows each API call's prompt and response, phase by phase.")
        rt.spacer()
        rt.body("  Rate Insights / Insight Feedback")
        rt.body("    Thumbs-up / thumbs-down ratings on individual patterns, stored")
        rt.body("    in dreams/insight-feedback.jsonl. Future cycles use these signals")
        rt.body("    to weight pattern promotion and valence memory entries.")
        rt.spacer()
        rt.body("  Ambient HUD")
        rt.body("    The floating always-visible overlay showing live daemon status,")
        rt.body("    cognitive load gauge, sparkline, calibration score, and next")
        rt.body("    cycle estimate. Toggled via the menu or ⌘H.")
        rt.spacer()

        // ── File locations ─────────────────────────────────────────────────
        rt.subheader("Key Files")
        rt.mono("  ~/.claude/subconscious/")
        rt.mono("  ├── state.json          total_cycles, last_consolidation, tokens")
        rt.mono("  ├── dreams/")
        rt.mono("  │   ├── journal.jsonl   one entry per consolidation cycle")
        rt.mono("  │   ├── patterns.json   extracted behavioral patterns")
        rt.mono("  │   ├── insights.md     Wake-promoted high-confidence insights")
        rt.mono("  │   ├── insight-digest.md   Recent Dreams Inference prose")
        rt.mono("  │   ├── digest-meta.json    sentiment + last_run timestamp")
        rt.mono("  │   ├── associations.json   cross-pattern hypotheses (REM)")
        rt.mono("  │   └── traces/         per-cycle JSONL event logs")
        rt.mono("  ├── metacog/")
        rt.mono("  │   ├── calibration.jsonl   per-cycle calibration scores")
        rt.mono("  │   └── audits/         individual audit JSON files")
        rt.mono("  ├── valence/memory.jsonl     time-decayed pattern outcomes")
        rt.mono("  └── intentions/registry.jsonl active prospective intentions")

        showResizablePanel(title: "i-dream — Terminology Glossary", content: rt.build())
    }

    @objc private func showHowTo() {
        let rt = RichText()
        rt.header("i-dream — How To")
        rt.spacer()
        rt.subheader("What It Does")
        rt.body("i-dream processes your Claude Code sessions while you're away.")
        rt.body("Five modules run in the background, learning from every conversation:")
        rt.spacer()
        rt.body("  Dreaming        — 3-phase sleep cycle (SWS → REM → Wake). Extracts")
        rt.body("                    behavioral patterns, forms cross-session associations,")
        rt.body("                    and promotes high-confidence insights to long-term memory.")
        rt.body("  Metacognition   — samples reasoning quality at 25% of tool calls.")
        rt.body("                    Tracks calibration, bias flags, and reasoning depth.")
        rt.body("  Intuition       — maintains a valence model of Claude's confidence.")
        rt.body("                    Positive/negative signals decay over time; primes")
        rt.body("                    session starts with relevant past outcomes.")
        rt.body("  Introspection   — chains metacog audits into higher-order reports.")
        rt.body("                    Weekly reports flag persistent reasoning patterns.")
        rt.body("  Prospective     — captures intentions and follow-up items from sessions.")
        rt.body("                    Surfaces matching reminders at session start.")
        rt.spacer()
        rt.subheader("Daemon Controls")
        rt.body("  Start Daemon        — launches the background process")
        rt.body("  Stop Daemon         — gracefully stops the daemon")
        rt.body("  Trigger Dream Cycle — run one cycle immediately (daemon must be running)")
        rt.spacer()
        rt.subheader("Knowledge Base  (tap rows to explore)")
        rt.body("  Patterns        — behavioral patterns Claude has noticed about you")
        rt.body("  Associations    — cross-pattern hypotheses (if A then B)")
        rt.body("  Sessions        — dream journal: one entry per cycle")
        rt.body("  Metacog Audits  — calibration scores and detected reasoning biases")
        rt.spacer()
        rt.subheader("Hooks  (installed via  i-dream hooks install)")
        rt.body("  SessionStart     — injects primed context (valence, prospective reminders)")
        rt.body("                     and recent dream insights into the conversation.")
        rt.body("  PostToolUse      — samples metacognition at 25% of tool calls.")
        rt.body("  Stop             — triggers session consolidation when you end a session.")
        rt.body("  PreCompact       — writes a lightweight checkpoint before context compaction.")
        rt.body("  UserPromptSubmit — captures sentiment signals (ALL-CAPS, corrections,")
        rt.body("                     frustration language, positive feedback) for dreaming.")
        rt.spacer()
        rt.subheader("Shell Log Integration")
        rt.body("If diy-claude-mem is installed, SWS dream cycles automatically read")
        rt.body("today's shell command history for the session — git/npm/docker command")
        rt.body("counts feed the pattern extractor as behavioral tags.")
        rt.spacer()
        rt.subheader("Reading the Status Bar")
        rt.mono("  ◉ N       — running, N cycles completed")
        rt.mono("  ◉ 2m 15s  — dreaming right now (elapsed updates live)")
        rt.mono("  (empty)   — daemon stopped")
        rt.spacer()
        rt.subheader("Dashboard")
        rt.body("'Open Dashboard' regenerates an HTML report and opens it in your browser.")
        rt.body("Shows cycle traces, file inventory, hook events, module status,")
        rt.body("and a per-cycle 'What Claude Realized' summary table.")
        rt.spacer()
        rt.subheader("Logs")
        rt.body("  Logs → Open in Terminal  — live tail of the daemon log")
        rt.body("  Logs → Open in VS Code   — open log file in editor")
        rt.body("  Logs → Open Debug Log    — widget's own debug output (/tmp/i-dream-bar.log)")
        rt.spacer()
        rt.subheader("Data Location")
        rt.mono("  ~/.claude/subconscious/")
        rt.mono("  ├── dreams/        patterns.json, associations.json, journal.jsonl")
        rt.mono("  ├── metacog/       audits/, calibration.jsonl, introspection/")
        rt.mono("  ├── intuition/     valence.jsonl, priming.jsonl")
        rt.mono("  ├── prospective/   intentions.jsonl")
        rt.mono("  ├── logs/          i-dream.log.YYYY-MM-DD, signals.jsonl")
        rt.mono("  ├── config.toml    all module settings")
        rt.mono("  └── state.json     cycle counts, token totals")
        rt.spacer()
        rt.subheader("Build / Install")
        rt.mono("  bash tools/menubar/build.sh           # rebuild widget")
        rt.mono("  bash tools/menubar/build.sh --install # register LaunchAgent")
        rt.mono("  i-dream service install               # register daemon LaunchAgent")
        rt.mono("  i-dream hooks install                 # install all hook scripts")
        rt.spacer()
        rt.dim("Full documentation: docs/ in the project repository.")
        showResizablePanel(title: "i-dream — How To", content: rt.build())
    }

    @objc private func openGitHub() {
        NSWorkspace.shared.open(URL(string: "https://github.com/alcatraz627/i-dream")!)
    }

    /// Copy the text stored in sender.representedObject to the clipboard.
    /// Used by pattern and error menu items so their full text can be pasted into Claude.
    @objc private func copyItemText(_ sender: NSMenuItem) {
        guard let text = sender.representedObject as? String else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc private func openConfigInVSCode() {
        let configPath = home + "/.claude/subconscious/config.toml"
        // Ensure the file exists (create default if not)
        if !FileManager.default.fileExists(atPath: configPath) {
            try? "# i-dream config — edit then restart the daemon\n".write(
                toFile: configPath, atomically: true, encoding: .utf8)
        }
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        task.arguments = ["open", "-a", "Visual Studio Code", configPath]
        try? task.run()
    }

    private func alert(_ title: String, _ msg: String) {
        let a = NSAlert()
        a.messageText = title; a.informativeText = msg
        a.alertStyle  = .warning; a.addButton(withTitle: "OK"); a.runModal()
    }
}

// ─── Mini bar-chart view for HUD token history ────────────────────────────────
/// Draws a compact histogram of recent token-usage values using NSBezierPath.
/// Bars are colored cyan→yellow→orange based on relative load; newest bar is brightest.
private class MiniBarChartView: NSView {
    var values: [Int] = [] { didSet { needsDisplay = true } }

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
            let alpha     = 0.25 + recency * 0.70

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

// ─── Entry point ──────────────────────────────────────────────────────────────

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = BarDelegate()
app.delegate = delegate
app.run()
