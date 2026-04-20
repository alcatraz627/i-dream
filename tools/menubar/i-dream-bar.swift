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
    /// Stable key for selection — uses id when available, falls back to text prefix.
    var stableKey: String { id ?? String(pattern.prefix(30)) }
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
        let ps = NSMutableParagraphStyle(); ps.paragraphSpacing = 2; ps.paragraphSpacingBefore = 4
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 14, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: ps,
        ])); return self
    }
    @discardableResult func body(_ text: String) -> RichText {
        let ps = NSMutableParagraphStyle(); ps.paragraphSpacing = 2
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: ps,
        ])); return self
    }
    @discardableResult func dim(_ text: String) -> RichText {
        let ps = NSMutableParagraphStyle(); ps.paragraphSpacing = 2
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: NSColor.secondaryLabelColor,
            .paragraphStyle: ps,
        ])); return self
    }
    @discardableResult func mono(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    /// Clickable monospaced path — link value is passed to the panel's link delegate on click.
    @discardableResult func monoLink(_ text: String, linkValue: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font:            NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: NSColor.systemBlue,
            .link:            linkValue as AnyObject,
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

// ─── Key-aware panel ─────────────────────────────────────────────────────────
// NSPanel with .nonactivatingPanel doesn't become key window by default,
// which breaks Cmd+A/Cmd+C in text fields. This subclass fixes that.

private class KeyablePanel: NSPanel {
    override var canBecomeKey: Bool { true }

    /// Route Cmd+1…9 to tab selection, Cmd+R to refresh.
    /// The `tabHandler` closure is set by DashboardWindowController after panel creation.
    var tabHandler: ((Int) -> Void)?
    var refreshHandler: (() -> Void)?

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.modifierFlags.contains(.command) else {
            return super.performKeyEquivalent(with: event)
        }
        if let chars = event.charactersIgnoringModifiers, chars.count == 1 {
            let ch = chars.first!
            // Cmd+1 through Cmd+9
            if ch >= "1" && ch <= "9" {
                let idx = Int(ch.asciiValue! - Character("1").asciiValue!)
                tabHandler?(idx)
                return true
            }
            // Cmd+R → refresh
            if ch == "r" || ch == "R" {
                refreshHandler?()
                return true
            }
        }
        return super.performKeyEquivalent(with: event)
    }
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
    var insightTexts: [String: String] = [:]   // insightId → full text for clipboard
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
        if linkStr.hasPrefix("insight-copy:") {
            let id = String(linkStr.dropFirst("insight-copy:".count))
            if let text = insightTexts[id] {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
            }
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

    /// External selection from a list view — draws an accent glow ring on the matching node.
    var highlightedId: String? = nil { didSet { needsDisplay = true } }
    /// Called when the user clicks a node, passing the pattern's id (or nil on deselect).
    var onNodeSelected: ((String?) -> Void)? = nil
    /// Whether a search query is actively typed (even if zero results).
    var isSearchActive: Bool = false { didSet { needsDisplay = true } }
    /// Search filter: indices of nodes matching a search query.
    var searchMatchedIndices: Set<Int> = [] { didSet { needsDisplay = true } }

    /// Set search filter by matching node pattern text against query words.
    func applySearch(_ query: String) {
        guard !query.isEmpty else { isSearchActive = false; searchMatchedIndices = []; return }
        isSearchActive = true
        let words = query.lowercased().components(separatedBy: " ").filter { !$0.isEmpty }
        var matched = Set<Int>()
        for (i, node) in nodes.enumerated() {
            let text = (node.pattern.pattern + " " + node.pattern.category + " " + node.pattern.valence).lowercased()
            if words.allSatisfy({ text.contains($0) }) { matched.insert(i) }
        }
        searchMatchedIndices = matched
    }

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
        let gp = viewToGraph(convert(event.locationInWindow, from: nil))
        if let hit = hitNode(at: gp) {
            if selectedIdx == hit {
                // Toggle off — click same node again deselects
                popover?.close(); popover = nil
                selectedIdx = nil; needsDisplay = true
                onNodeSelected?(nil)
            } else {
                selectedIdx = hit; needsDisplay = true
                showPopover(for: hit)
                onNodeSelected?(nodes[hit].pattern.stableKey)
            }
        } else {
            popover?.close(); popover = nil
            selectedIdx = nil; needsDisplay = true
            onNodeSelected?(nil)
        }
    }

    // MARK: – Coordinate helpers

    private func viewToGraph(_ pt: CGPoint) -> CGPoint {
        let cx = bounds.midX, cy = bounds.midY
        return CGPoint(
            x: (pt.x - panOffset.x - cx) / zoomScale + cx,
            y: (pt.y - panOffset.y - cy) / zoomScale + cy)
    }

    private func hitNode(at graphPt: CGPoint) -> Int? {
        for (i, node) in nodes.enumerated() {
            if hypot(graphPt.x - node.position.x, graphPt.y - node.position.y) <= node.radius + 6 {
                return i
            }
        }
        return nil
    }

    /// Indices of nodes "linked" to the focused node (same category + nearest 15).
    private func linkedIndices(for idx: Int) -> Set<Int> {
        let pos = nodes[idx].position
        let cat = nodes[idx].pattern.category
        let nearest = (0..<nodes.count)
            .filter { $0 != idx }
            .sorted { hypot(nodes[$0].position.x - pos.x, nodes[$0].position.y - pos.y)
                    < hypot(nodes[$1].position.x - pos.x, nodes[$1].position.y - pos.y) }
            .prefix(15)
            .filter { nodes[$0].pattern.category == cat }
        return Set(nearest)
    }

    /// The "active focus" index — either selectedIdx or the index matching highlightedId.
    private var focusIdx: Int? {
        if let sel = selectedIdx { return sel }
        guard let hid = highlightedId else { return nil }
        return nodes.firstIndex { $0.pattern.stableKey == hid }
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

        // ── Focus-based linked set ──────────────────────────────────────────
        let fIdx = focusIdx
        let linked: Set<Int> = fIdx.map { linkedIndices(for: $0) } ?? []
        let hasFocus = fIdx != nil

        // ── Connection lines ──────────────────────────────────────────────────
        // When a node is focused (selected or highlighted), draw lines to linked nodes.
        // On hover (without focus), show nearest 15 as before.
        if let fi = fIdx {
            let fiPos = nodes[fi].position
            let fiColor = nodeColor(nodes[fi].pattern)
            for j in linked {
                let c2 = nodeColor(nodes[j].pattern)
                let blended = fiColor.blended(withFraction: 0.5, of: c2) ?? fiColor
                ctx.setStrokeColor(blended.withAlphaComponent(0.45).cgColor)
                ctx.setLineWidth(1.4)
                ctx.move(to: fiPos)
                ctx.addLine(to: nodes[j].position)
                ctx.strokePath()
            }
        } else if let hov = hoveredIdx {
            let hovPos = nodes[hov].position
            let hovCat = nodes[hov].pattern.category
            let nearest = (0 ..< nodes.count)
                .filter { $0 != hov }
                .sorted { hypot(nodes[$0].position.x - hovPos.x, nodes[$0].position.y - hovPos.y)
                        < hypot(nodes[$1].position.x - hovPos.x, nodes[$1].position.y - hovPos.y) }
                .prefix(15)
            for j in nearest {
                let sameCategory = nodes[j].pattern.category == hovCat
                let alpha: CGFloat = sameCategory ? 0.38 : 0.16
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
            let isFocused  = idx == fIdx
            let isLinked   = linked.contains(idx)
            let isSearchMatch = isSearchActive && searchMatchedIndices.contains(idx)
            // Dim nodes that aren't part of the focus group or search results
            let dimmed     = (hasFocus && !isFocused && !isLinked) || (isSearchActive && !isSearchMatch)
            let fillAlpha: CGFloat = dimmed ? 0.15 : isSelected ? 1.0 : isHovered ? 0.92 : 0.72
            let r          = node.radius

            // Glow ring for hovered / selected / focused nodes
            if isFocused || isHovered || isSelected {
                ctx.setStrokeColor(baseColor.withAlphaComponent(isSelected || isFocused ? 0.55 : 0.28).cgColor)
                ctx.setLineWidth(5.5)
                let gr = r + 4
                ctx.strokeEllipse(in: CGRect(x: node.position.x - gr, y: node.position.y - gr,
                                             width: gr * 2, height: gr * 2))
            }
            // External highlight from list selection — accent-colour double ring
            if let hid = highlightedId, p.stableKey == hid {
                let accentColor = NSColor.controlAccentColor
                ctx.setStrokeColor(accentColor.withAlphaComponent(0.55).cgColor)
                ctx.setLineWidth(3.0)
                let er = r + 7
                ctx.strokeEllipse(in: CGRect(x: node.position.x - er, y: node.position.y - er,
                                             width: er * 2, height: er * 2))
                ctx.setStrokeColor(accentColor.withAlphaComponent(0.22).cgColor)
                ctx.setLineWidth(8.0)
                let er2 = r + 12
                ctx.strokeEllipse(in: CGRect(x: node.position.x - er2, y: node.position.y - er2,
                                             width: er2 * 2, height: er2 * 2))
            }

            let rect = CGRect(x: node.position.x - r, y: node.position.y - r,
                              width: r * 2, height: r * 2)
            if !isHovered && !isSelected && !dimmed {
                ctx.setStrokeColor(NSColor.white.withAlphaComponent(0.25).cgColor)
                ctx.setLineWidth(1.5)
                let hr = r + 1.0
                ctx.strokeEllipse(in: CGRect(x: node.position.x - hr, y: node.position.y - hr,
                                             width: hr * 2, height: hr * 2))
            }
            ctx.setFillColor(baseColor.withAlphaComponent(fillAlpha).cgColor)
            ctx.setStrokeColor(baseColor.withAlphaComponent(dimmed ? 0.25 : 1.0).cgColor)
            ctx.setLineWidth(isSelected ? 2.5 : isHovered ? 2.0 : 1.2)
            ctx.fillEllipse(in: rect)
            ctx.strokeEllipse(in: rect)

        }

        // ── Node labels — two-pass with overlap culling ───────────────────────
        // Pass 1: hovered / selected nodes always render (highest priority).
        // Pass 2: zoom-triggered labels skip any whose rect intersects a used slot.
        // This prevents the "text blob" that appeared when all nodes rendered
        // labels simultaneously above the zoom threshold.
        var usedLabelRects: [CGRect] = []
        func drawNodeLabel(_ node: Node, isHovered: Bool, isSelected: Bool) {
            let p = node.pattern
            let fontSize: CGFloat = isHovered ? 10 : 9
            let color: NSColor    = isHovered || isSelected ? .labelColor : .secondaryLabelColor
            let label = p.pattern.components(separatedBy: " ").prefix(3).joined(separator: " ")
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: fontSize),
                .foregroundColor: color,
            ]
            let str    = NSAttributedString(string: label, attributes: attrs)
            let sz     = str.size()
            let origin = CGPoint(x: node.position.x - sz.width / 2,
                                 y: node.position.y + node.radius + 3)
            let slot   = CGRect(origin: origin, size: sz).insetBy(dx: -8, dy: -3)
            guard !usedLabelRects.contains(where: { $0.intersects(slot) }) else { return }
            str.draw(at: origin)
            usedLabelRects.append(slot)
        }
        // Pass 1 — hovered / selected
        for (idx, node) in nodes.enumerated() {
            let isHov = idx == hoveredIdx; let isSel = idx == selectedIdx
            guard isHov || isSel else { continue }
            drawNodeLabel(node, isHovered: isHov, isSelected: isSel)
        }
        // Pass 2 — zoom-triggered (only if zoomed in enough; cull overlaps)
        if zoomScale > 2.5 {
            for (idx, node) in nodes.enumerated() {
                if idx == hoveredIdx || idx == selectedIdx { continue }
                drawNodeLabel(node, isHovered: false, isSelected: false)
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

        let popW: CGFloat = 420
        let vc = NSViewController()
        let tv = NSTextView(frame: NSRect(x: 14, y: 10, width: popW - 28, height: 200))
        tv.isEditable = false; tv.backgroundColor = .clear
        tv.textContainerInset = NSSize(width: 0, height: 4)
        tv.isVerticallyResizable = true
        tv.textContainer?.widthTracksTextView = true

        let rt = RichText()
        rt.subheader(p.pattern)
        rt.spacer()

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
            rt.spacer()
            let names = siblings.prefix(3)
                .map { $0.pattern.pattern.components(separatedBy: " ").prefix(4).joined(separator: " ") }
                .joined(separator: " · ")
            rt.dim("related: \(names)\(siblings.count > 3 ? " + \(siblings.count - 3) more" : "")")
        }

        tv.textStorage?.setAttributedString(rt.build())

        // Measure actual content height and size popover to fit
        tv.layoutManager?.ensureLayout(for: tv.textContainer!)
        let contentH = tv.layoutManager?.usedRect(for: tv.textContainer!).height ?? 170
        let popH = min(max(contentH + 28, 100), 400)  // clamp 100–400
        tv.frame = NSRect(x: 14, y: 10, width: popW - 28, height: popH - 20)
        let vw = NSView(frame: NSRect(x: 0, y: 0, width: popW, height: popH))
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

    /// External selection from a list view — accent glow ring on the matching node.
    var highlightedId: String? = nil { didSet { needsDisplay = true } }
    var onNodeSelected: ((String?) -> Void)? = nil
    /// Whether a search query is actively typed (even if zero results).
    var isSearchActive: Bool = false { didSet { needsDisplay = true } }
    /// Search filter: indices of nodes matching a search query.
    var searchMatchedIndices: Set<Int> = [] { didSet { needsDisplay = true } }

    /// Set search filter by matching node hypothesis text against query words.
    func applySearch(_ query: String) {
        guard !query.isEmpty else { isSearchActive = false; searchMatchedIndices = []; return }
        isSearchActive = true
        let words = query.lowercased().components(separatedBy: " ").filter { !$0.isEmpty }
        var matched = Set<Int>()
        for (i, node) in nodes.enumerated() {
            let text = node.assoc.hypothesis.lowercased()
            let rule = (node.assoc.suggestedRule ?? "").lowercased()
            let combined = text + " " + rule
            if words.allSatisfy({ combined.contains($0) }) { matched.insert(i) }
        }
        searchMatchedIndices = matched
    }

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
            if selectedIdx == hit {
                // Toggle off — deselect
                popover?.close(); popover = nil
                selectedIdx = nil; needsDisplay = true
                onNodeSelected?(nil)
            } else {
                selectedIdx = hit; needsDisplay = true; showPopover(for: hit)
                onNodeSelected?(nodes[hit].assoc.id)
            }
        } else {
            popover?.close(); popover = nil; selectedIdx = nil; needsDisplay = true
            onNodeSelected?(nil)
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

    /// Indices of nodes connected to the given node via edges.
    private func linkedIndices(for idx: Int) -> Set<Int> {
        var result = Set<Int>()
        for edge in edges {
            if edge.a == idx { result.insert(edge.b) }
            if edge.b == idx { result.insert(edge.a) }
        }
        // Also include same-ring neighbours (±3 positions) when no edges exist
        if result.isEmpty {
            let ring = confidenceRing(for: nodes[idx].assoc)
            for (i, n) in nodes.enumerated() where i != idx {
                if confidenceRing(for: n.assoc) == ring {
                    let dist = hypot(n.position.x - nodes[idx].position.x,
                                     n.position.y - nodes[idx].position.y)
                    if dist < min(bounds.width, bounds.height) * 0.25 {
                        result.insert(i)
                    }
                }
            }
        }
        return result
    }

    private func confidenceRing(for a: Association) -> Int {
        a.confidence >= 0.75 ? 0 : a.confidence >= 0.50 ? 1 : 2
    }

    /// The "active focus" index — either selectedIdx or the index matching highlightedId.
    private var focusIdx: Int? {
        if let sel = selectedIdx { return sel }
        guard let hid = highlightedId else { return nil }
        return nodes.firstIndex { $0.assoc.id == hid }
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

        // ── Focus-based linked set ──────────────────────────────────────────
        let fIdx = focusIdx
        let linked: Set<Int> = fIdx.map { linkedIndices(for: $0) } ?? []
        let hasFocus = fIdx != nil

        // ── Edges ─────────────────────────────────────────────────────────────
        // When focused, show ALL edges involving the focused node + linked nodes.
        // On hover without focus, show edges connecting the hovered node.
        if let fi = fIdx {
            let maxWeight = edges.map { $0.weight }.max() ?? 1
            let focusEdges = edges.filter { $0.a == fi || $0.b == fi || linked.contains($0.a) || linked.contains($0.b) }
            for edge in focusEdges {
                let ca = nodeColor(nodes[edge.a].assoc)
                let cb = nodeColor(nodes[edge.b].assoc)
                let blended = ca.blended(withFraction: 0.5, of: cb) ?? ca
                let w = 0.8 + 1.2 * CGFloat(edge.weight) / CGFloat(max(maxWeight, 1))
                ctx.setStrokeColor(blended.withAlphaComponent(0.50).cgColor)
                ctx.setLineWidth(w)
                ctx.move(to: nodes[edge.a].position)
                ctx.addLine(to: nodes[edge.b].position)
                ctx.strokePath()
            }
            // Draw connector lines from focus to linked even without edges
            let fiPos = nodes[fi].position
            for j in linked {
                let hasEdge = edges.contains { ($0.a == fi && $0.b == j) || ($0.a == j && $0.b == fi) }
                if !hasEdge {
                    let c2 = nodeColor(nodes[j].assoc)
                    ctx.setStrokeColor(c2.withAlphaComponent(0.25).cgColor)
                    ctx.setLineWidth(0.8)
                    ctx.setLineDash(phase: 0, lengths: [4, 3])
                    ctx.move(to: fiPos)
                    ctx.addLine(to: nodes[j].position)
                    ctx.strokePath()
                    ctx.setLineDash(phase: 0, lengths: [])
                }
            }
        } else if let hov = hoveredIdx {
            let maxWeight = edges.map { $0.weight }.max() ?? 1
            let hovEdges  = edges
                .filter { $0.a == hov || $0.b == hov }
                .sorted { $0.weight > $1.weight }
                .prefix(5)
            for edge in hovEdges {
                let ca = nodeColor(nodes[edge.a].assoc)
                let cb = nodeColor(nodes[edge.b].assoc)
                let blended = ca.blended(withFraction: 0.5, of: cb) ?? ca
                ctx.setStrokeColor(blended.withAlphaComponent(0.30).cgColor)
                let w = 0.8 + 1.2 * CGFloat(edge.weight) / CGFloat(max(maxWeight, 1))
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
            let isFocused  = idx == fIdx
            let isLinked   = linked.contains(idx)
            let isSearchMatch = isSearchActive && searchMatchedIndices.contains(idx)
            let dimmed     = (hasFocus && !isFocused && !isLinked) || (isSearchActive && !isSearchMatch)
            let r          = node.radius

            if isFocused || isHovered || isSelected {
                ctx.setStrokeColor(baseColor.withAlphaComponent(isSelected || isFocused ? 0.55 : 0.28).cgColor)
                ctx.setLineWidth(5.5)
                let gr = r + 4
                ctx.strokeEllipse(in: CGRect(x: node.position.x - gr, y: node.position.y - gr,
                                              width: gr * 2, height: gr * 2))
            }
            // External highlight from list selection
            if let hid = highlightedId, a.id == hid {
                let accentColor = NSColor.controlAccentColor
                ctx.setStrokeColor(accentColor.withAlphaComponent(0.55).cgColor)
                ctx.setLineWidth(3.0)
                let er = r + 7
                ctx.strokeEllipse(in: CGRect(x: node.position.x - er, y: node.position.y - er,
                                             width: er * 2, height: er * 2))
                ctx.setStrokeColor(accentColor.withAlphaComponent(0.22).cgColor)
                ctx.setLineWidth(8.0)
                let er2 = r + 12
                ctx.strokeEllipse(in: CGRect(x: node.position.x - er2, y: node.position.y - er2,
                                             width: er2 * 2, height: er2 * 2))
            }

            let fillAlpha: CGFloat = dimmed ? 0.15 : isSelected ? 1.0 : isHovered ? 0.92 : 0.70
            ctx.setFillColor(baseColor.withAlphaComponent(fillAlpha).cgColor)
            ctx.setStrokeColor(baseColor.withAlphaComponent(dimmed ? 0.25 : 1.0).cgColor)
            ctx.setLineWidth(isSelected ? 2.5 : isHovered ? 2.0 : 1.0)
            let rect = CGRect(x: node.position.x - r, y: node.position.y - r,
                               width: r * 2, height: r * 2)
            ctx.fillEllipse(in: rect)
            ctx.strokeEllipse(in: rect)

            // Diamond marker for actionable nodes (dim if not in focus group)
            if a.actionable {
                let dm: CGFloat = 4
                let dp = node.position
                let diamondAlpha: CGFloat = dimmed ? 0.2 : isHovered ? 0.9 : 0.7
                ctx.setFillColor(NSColor.white.withAlphaComponent(diamondAlpha).cgColor)
                ctx.move(to: CGPoint(x: dp.x, y: dp.y + dm))
                ctx.addLine(to: CGPoint(x: dp.x + dm, y: dp.y))
                ctx.addLine(to: CGPoint(x: dp.x, y: dp.y - dm))
                ctx.addLine(to: CGPoint(x: dp.x - dm, y: dp.y))
                ctx.closePath()
                ctx.fillPath()
            }

        }

        // ── Association node labels — two-pass with overlap culling ───────────
        var usedAssocLabelRects: [CGRect] = []
        func drawAssocLabel(_ node: Node, isHovered: Bool, isSelected: Bool) {
            let a = node.assoc
            let fontSize: CGFloat = isHovered ? 10 : 9
            let color: NSColor    = isHovered || isSelected ? .labelColor : .secondaryLabelColor
            let label = a.hypothesis.components(separatedBy: " ").prefix(4).joined(separator: " ")
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: fontSize),
                .foregroundColor: color,
            ]
            let str    = NSAttributedString(string: label, attributes: attrs)
            let sz     = str.size()
            let origin = CGPoint(x: node.position.x - sz.width / 2,
                                 y: node.position.y + node.radius + 3)
            let slot   = CGRect(origin: origin, size: sz).insetBy(dx: -8, dy: -3)
            guard !usedAssocLabelRects.contains(where: { $0.intersects(slot) }) else { return }
            str.draw(at: origin)
            usedAssocLabelRects.append(slot)
        }
        for (idx, node) in nodes.enumerated() {
            let isHov = idx == hoveredIdx; let isSel = idx == selectedIdx
            guard isHov || isSel else { continue }
            drawAssocLabel(node, isHovered: isHov, isSelected: isSel)
        }
        if zoomScale > 2.5 {
            for (idx, node) in nodes.enumerated() {
                if idx == hoveredIdx || idx == selectedIdx { continue }
                drawAssocLabel(node, isHovered: false, isSelected: false)
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

        let popW: CGFloat = 460
        let vc = NSViewController()
        let tv = NSTextView(frame: NSRect(x: 14, y: 10, width: popW - 28, height: 200))
        tv.isEditable = false; tv.backgroundColor = .clear
        tv.textContainerInset = NSSize(width: 0, height: 4)
        tv.isVerticallyResizable = true
        tv.textContainer?.widthTracksTextView = true

        let rt = RichText()
        rt.subheader(a.hypothesis)
        rt.spacer()

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
            rt.spacer()
            let names = neighbours.prefix(3).map { e -> String in
                let other = e.a == idx ? e.b : e.a
                let h = nodes[other].assoc.hypothesis
                return h.components(separatedBy: " ").prefix(4).joined(separator: " ")
            }.joined(separator: " · ")
            rt.dim("connected: \(names)\(neighbours.count > 3 ? " + \(neighbours.count - 3) more" : "")")
        }

        tv.textStorage?.setAttributedString(rt.build())

        // Measure actual content height and size popover to fit
        tv.layoutManager?.ensureLayout(for: tv.textContainer!)
        let contentH = tv.layoutManager?.usedRect(for: tv.textContainer!).height ?? 180
        let popH = min(max(contentH + 28, 100), 400)
        tv.frame = NSRect(x: 14, y: 10, width: popW - 28, height: popH - 20)
        let vw = NSView(frame: NSRect(x: 0, y: 0, width: popW, height: popH))
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

// ─── Comprehensive Dashboard ──────────────────────────────────────────────────

/// Sidebar nav button — flat label+icon button with a coloured background when selected.
/// Does NOT override draw() to avoid the infinite-redraw trap of mutating self.font inside draw().
final class NavSidebarButton: NSButton {
    private var _title  = ""
    private var _symbol = ""
    private var _iconColor: NSColor = .labelColor

    /// Per-tab icon colours (indexed by tab position).
    private static let iconColors: [NSColor] = [
        .systemPurple,   // Overview
        .systemTeal,     // Patterns
        .systemOrange,   // Associations
        .systemIndigo,   // Journal
        .systemYellow,   // Insights
        .systemPink,     // Metacog
        .systemGreen,    // Search
        .secondaryLabelColor, // Help
        .secondaryLabelColor, // About
    ]

    var isSelectedTab = false {
        didSet {
            guard oldValue != isSelectedTab else { return }
            layer?.backgroundColor = isSelectedTab
                ? NSColor.controlAccentColor.withAlphaComponent(0.18).cgColor
                : nil
            self.contentTintColor = isSelectedTab ? _iconColor : _iconColor.withAlphaComponent(0.7)
            updateAttributedTitle()
        }
    }

    private static let tabTooltips: [String] = [
        "System overview — stats, digest, valence (⌘1)",
        "Behavioral patterns extracted from sessions (⌘2)",
        "Cross-pattern associations and hypotheses (⌘3)",
        "Consolidation cycle history and token usage (⌘4)",
        "Promoted insights with confidence ratings (⌘5)",
        "Metacognitive audits and calibration (⌘6)",
        "Search across all knowledge base data (⌘7)",
        "Keyboard shortcuts and feature reference (⌘8)",
        "Build info, daemon status, data paths (⌘9)",
    ]

    func configure(title: String, symbol: String, index: Int) {
        _title  = title
        _symbol = symbol
        _iconColor = index < NavSidebarButton.iconColors.count
            ? NavSidebarButton.iconColors[index] : .labelColor
        self.tag            = index
        self.isBordered     = false
        self.imagePosition  = .imageLeading
        self.alignment      = .left
        self.wantsLayer     = true
        self.layer?.cornerRadius = 6
        if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: title) {
            self.image = img
        }
        self.contentTintColor = _iconColor.withAlphaComponent(0.7)
        if index < NavSidebarButton.tabTooltips.count {
            self.toolTip = NavSidebarButton.tabTooltips[index]
        }
        updateAttributedTitle()
    }

    /// Update the displayed title (e.g. to add a count badge).
    func updateTitle(_ newTitle: String) {
        _title = newTitle
        updateAttributedTitle()
    }

    private func updateAttributedTitle() {
        let weight: NSFont.Weight = isSelectedTab ? .semibold : .regular
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13, weight: weight),
            .foregroundColor: NSColor.labelColor,
        ]
        self.attributedTitle = NSAttributedString(string: " " + _title, attributes: attrs)
    }
}

/// Manages the comprehensive dashboard panel — a full split-view window
/// with sidebar navigation and embedded graph/text views for all i-dream data.
final class DashboardWindowController: NSObject {
    private var panel: NSPanel?
    private var navButtons:       [NavSidebarButton] = []
    private var contentContainer: NSView!
    private var contentViews:     [NSView]           = []

    // Cross-linking strong refs — prevent delegate/graph from deallocating
    private var patternGraphView:    PatternGraphView?
    private var patternListDelegate: JournalLinkDelegate?
    private var assocGraphView:      AssociationGraphView?
    private var assocListDelegate:   JournalLinkDelegate?
    private var overviewLinkDelegate: JournalLinkDelegate?

    // Detail panels for selection context (Patterns + Associations tabs)
    private var patternDetailTextView:  NSTextView?
    private var assocDetailTextView:    NSTextView?

    // Insights tab state
    private var insightFeedbackDelegate: InsightFeedbackDelegate?
    private var insightsTextView: NSTextView?

    // Search tab state
    private var searchField:           NSSearchField?
    private var searchResultsTextView: NSTextView?
    private var searchLinkDelegate:    JournalLinkDelegate?
    private var searchDebounceTimer:   Timer?

    // Sidebar footer state
    private var lastRefreshedLabel:    NSTextField?
    private var lastRefreshedDate:     Date?

    private let tabs: [(title: String, symbol: String)] = [
        ("Overview",     "square.grid.2x2.fill"),
        ("Patterns",     "brain.head.profile"),
        ("Associations", "link"),
        ("Journal",      "book.fill"),
        ("Insights",     "sparkles"),
        ("Metacog",      "checkmark.seal.fill"),
        ("Search",       "magnifyingglass"),
        ("Help",         "questionmark.circle.fill"),
        ("About",        "info.circle.fill"),
    ]

    // Data snapshots — reloaded on each showOrFront() call
    private var patterns:     [Pattern]      = []
    private var associations: [Association]  = []
    private var journal:      [JournalEntry] = []
    private var state:        DaemonState?
    private var board:        BoardData?
    private var digest:       String?

    // ── Public interface ───────────────────────────────────────────────────────

    func showOrFront() {
        patterns     = allPatterns()
        associations = allAssociations()
        journal      = allJournal()
        state        = readState()
        board        = readBoard()
        digest       = readInsightDigest()

        if let p = panel, p.isVisible {
            rebuildContentViews()
            p.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        buildAndShow()
    }

    // ── Panel construction ─────────────────────────────────────────────────────

    private func buildAndShow() {
        panel?.close()

        let panW: CGFloat = 1240, panH: CGFloat = 840
        let sideW: CGFloat = 200

        let p = KeyablePanel(
            contentRect: NSRect(x: 0, y: 0, width: panW, height: panH),
            styleMask: [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing: .buffered, defer: false)
        p.title                = "i-dream — Dashboard"
        p.isReleasedWhenClosed = false
        p.level                = .floating
        p.minSize              = NSSize(width: 960, height: 640)
        p.center()
        self.panel = p

        // Wire keyboard shortcuts (Cmd+1-9, Cmd+R)
        p.tabHandler     = { [weak self] idx in self?.selectTab(idx) }
        p.refreshHandler = { [weak self] in self?.refreshDashboard() }

        let cv = p.contentView!

        // ── Sidebar ────────────────────────────────────────────────────────────
        let sidebar = NSVisualEffectView(frame: NSRect(x: 0, y: 0, width: sideW, height: panH))
        sidebar.autoresizingMask = [.height]
        sidebar.material = .sidebar
        sidebar.blendingMode = .behindWindow
        sidebar.state = .active

        let sideTitle = NSTextField(labelWithString: "i-dream")
        sideTitle.font       = .systemFont(ofSize: 12, weight: .semibold)
        sideTitle.textColor  = .tertiaryLabelColor
        sideTitle.frame      = NSRect(x: 14, y: panH - 44, width: sideW - 28, height: 18)
        sideTitle.autoresizingMask = [.minYMargin]
        sidebar.addSubview(sideTitle)

        navButtons = []
        for (i, tab) in tabs.enumerated() {
            let btn = NavSidebarButton(frame: NSRect(
                x: 14, y: panH - 80 - CGFloat(i) * 44,
                width: sideW - 22, height: 36))
            btn.autoresizingMask = [.minYMargin]
            btn.configure(title: tab.title, symbol: tab.symbol, index: i)
            btn.target = self
            btn.action = #selector(navTapped(_:))
            sidebar.addSubview(btn)
            navButtons.append(btn)
        }

        // Bottom: export + refresh + version + last-refreshed
        let exportBtn = NSButton(title: "⬇  Export JSON", target: self, action: #selector(exportDashboardData))
        exportBtn.frame            = NSRect(x: 8, y: 72, width: sideW - 16, height: 28)
        exportBtn.isBordered       = false
        exportBtn.font             = .systemFont(ofSize: 12)
        exportBtn.contentTintColor = .secondaryLabelColor
        sidebar.addSubview(exportBtn)

        let refreshBtn = NSButton(title: "↺  Refresh  (⌘R)", target: self, action: #selector(refreshDashboard))
        refreshBtn.frame            = NSRect(x: 8, y: 48, width: sideW - 16, height: 28)
        refreshBtn.isBordered       = false
        refreshBtn.font             = .systemFont(ofSize: 12)
        refreshBtn.contentTintColor = .secondaryLabelColor
        sidebar.addSubview(refreshBtn)

        let verLabel = NSTextField(labelWithString: "build \(BuildInfo.commitHash.prefix(7))")
        verLabel.font      = .monospacedSystemFont(ofSize: 9.5, weight: .regular)
        verLabel.textColor = .tertiaryLabelColor
        verLabel.frame     = NSRect(x: 14, y: 28, width: sideW - 28, height: 14)
        sidebar.addSubview(verLabel)

        let refreshedLbl = NSTextField(labelWithString: "Refreshed just now")
        refreshedLbl.font      = .systemFont(ofSize: 9.5)
        refreshedLbl.textColor = .tertiaryLabelColor
        refreshedLbl.frame     = NSRect(x: 14, y: 10, width: sideW - 28, height: 14)
        sidebar.addSubview(refreshedLbl)
        lastRefreshedLabel = refreshedLbl
        lastRefreshedDate  = Date()

        // Vertical divider
        let sideSep = NSBox(frame: NSRect(x: sideW - 1, y: 0, width: 1, height: panH))
        sideSep.boxType          = .separator
        sideSep.autoresizingMask = [.height]
        sidebar.addSubview(sideSep)
        cv.addSubview(sidebar)

        // ── Content container ──────────────────────────────────────────────────
        contentContainer = NSView(frame: NSRect(x: sideW, y: 0,
                                                 width: panW - sideW, height: panH))
        contentContainer.autoresizingMask = [.width, .height]
        cv.addSubview(contentContainer)

        rebuildContentViews()
        let restored = UserDefaults.standard.integer(forKey: "idream-dashboard-selected-tab")
        selectTab(restored < tabs.count ? restored : 0)

        NSApp.activate(ignoringOtherApps: true)
        p.makeKeyAndOrderFront(nil)
    }

    private func rebuildContentViews() {
        patternGraphView    = nil
        patternListDelegate = nil
        patternDetailTextView = nil
        assocGraphView      = nil
        assocListDelegate   = nil
        assocDetailTextView = nil
        searchField             = nil
        searchResultsTextView   = nil
        for v in contentViews { v.removeFromSuperview() }
        contentViews = []
        let f = contentContainer.bounds
        contentViews = [
            buildOverviewView(frame: f),
            buildPatternView(frame: f),
            buildAssociationView(frame: f),
            buildJournalView(frame: f),
            buildInsightsView(frame: f),
            buildMetacogView(frame: f),
            buildSearchView(frame: f),
            buildHelpView(frame: f),
            buildAboutView(frame: f),
        ]
        for v in contentViews { contentContainer.addSubview(v) }
        let sel = navButtons.first(where: { $0.isSelectedTab })?.tag ?? 0
        for (i, v) in contentViews.enumerated() { v.isHidden = (i != sel) }

        // Update sidebar labels with data counts
        updateSidebarBadges()
    }

    private func updateSidebarBadges() {
        guard navButtons.count >= 9 else { return }
        // 0: Overview — no count
        // 1: Patterns
        navButtons[1].updateTitle("Patterns (\(patterns.count))")
        // 2: Associations
        navButtons[2].updateTitle("Associations (\(associations.count))")
        // 3: Journal
        navButtons[3].updateTitle("Journal (\(journal.count))")
        // 4: Insights — count insight blocks from raw markdown
        let insightBlockCount: Int = {
            guard let raw = readAllInsights() else { return 0 }
            return raw.components(separatedBy: "\n").filter { $0.hasPrefix("### Insight") }.count
        }()
        navButtons[4].updateTitle("Insights (\(insightBlockCount))")
        // 5: Metacog — show calibration score
        let (audit, _) = readLatestAudit()
        let calStr = audit?.calibrationScore.map { String(format: "%.2f", $0) } ?? "—"
        navButtons[5].updateTitle("Metacog (\(calStr))")
        // 6: Search, 7: Help, 8: About — no counts
    }

    // ── Navigation ─────────────────────────────────────────────────────────────

    @objc private func navTapped(_ sender: NSButton) { selectTab(sender.tag) }

    private func selectTab(_ index: Int) {
        guard index >= 0 && index < tabs.count else { return }
        for (i, btn) in navButtons.enumerated() { btn.isSelectedTab = (i == index) }
        for (i, v) in contentViews.enumerated()  { v.isHidden        = (i != index) }
        UserDefaults.standard.set(index, forKey: "idream-dashboard-selected-tab")
    }

    @objc private func refreshDashboard() {
        showOrFront()
        lastRefreshedDate = Date()
        lastRefreshedLabel?.stringValue = "Refreshed just now"
    }

    @objc private func exportDashboardData() {
        let sp = NSSavePanel()
        sp.title          = "Export i-dream Data"
        sp.nameFieldStringValue = "i-dream-export-\(ISO8601DateFormatter().string(from: Date()).prefix(10)).json"
        sp.allowedContentTypes  = [.json]
        sp.canCreateDirectories = true

        guard sp.runModal() == .OK, let url = sp.url else { return }

        let patternsArr = patterns.map { p -> [String: Any] in
            ["pattern": p.pattern, "category": p.category, "confidence": p.confidence,
             "valence": p.valence, "firstSeen": p.firstSeen ?? ""]
        }
        let assocsArr = associations.map { a -> [String: Any] in
            ["hypothesis": a.hypothesis, "confidence": a.confidence,
             "actionable": a.actionable, "suggestedRule": a.suggestedRule ?? ""]
        }
        let journalArr = journal.map { j -> [String: Any] in
            ["timestamp": j.timestamp, "tokensUsed": j.tokensUsed,
             "sessionsAnalyzed": j.sessionsAnalyzed, "patternsExtracted": j.patternsExtracted,
             "associationsFound": j.associationsFound, "insightsPromoted": j.insightsPromoted]
        }
        let exportData: [String: Any] = [
            "exportedAt": ISO8601DateFormatter().string(from: Date()),
            "build": "\(BuildInfo.commitHash)/\(BuildInfo.sourceHash)",
            "totalCycles": state?.totalCycles ?? 0,
            "totalTokensUsed": state?.totalTokensUsed ?? 0,
            "patterns": patternsArr,
            "associations": assocsArr,
            "journal": journalArr,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: exportData,
                                                      options: [.prettyPrinted, .sortedKeys])
        else { return }
        try? data.write(to: url)
    }

    // ── Shared helpers ─────────────────────────────────────────────────────────

    private func makeScrollableTextView(frame: NSRect) -> (NSScrollView, NSTextView) {
        let sv = NSScrollView(frame: frame)
        sv.autoresizingMask    = [.width, .height]
        sv.hasVerticalScroller = true
        sv.autohidesScrollers  = true
        sv.borderType          = .noBorder

        let cs = sv.contentSize
        let tv = NSTextView(frame: NSRect(x: 0, y: 0, width: cs.width, height: cs.height))
        tv.minSize                            = NSSize(width: 0, height: cs.height)
        tv.maxSize                            = NSSize(width: CGFloat.greatestFiniteMagnitude,
                                                       height: CGFloat.greatestFiniteMagnitude)
        tv.autoresizingMask                   = .width
        tv.isEditable                         = false
        tv.isSelectable                       = true
        tv.backgroundColor                    = .clear
        tv.drawsBackground                    = false
        tv.textContainerInset                 = NSSize(width: 24, height: 20)
        tv.isVerticallyResizable              = true
        tv.isHorizontallyResizable            = false
        tv.textContainer?.widthTracksTextView = true
        tv.textContainer?.containerSize       = NSSize(width: cs.width,
                                                       height: CGFloat.greatestFiniteMagnitude)
        sv.documentView = tv
        return (sv, tv)
    }

    /// Horizontal stats banner — 40px strip with key metrics.
    /// `stats` items are displayed as   Label: Value  │  Label: Value  │ …
    private func makeStatsBanner(frame: NSRect,
                                 stats: [(label: String, value: String, color: NSColor?)]) -> NSView {
        let banner = NSView(frame: frame)
        banner.wantsLayer = true
        banner.layer?.backgroundColor = NSColor.windowBackgroundColor
            .blended(withFraction: 0.04, of: .labelColor)?.cgColor

        // Bottom separator
        let sep = NSBox(frame: NSRect(x: 0, y: 0, width: frame.width, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]
        banner.addSubview(sep)

        var x: CGFloat = 18
        let midY = frame.height / 2

        for (i, stat) in stats.enumerated() {
            if i > 0 {
                let pipe = NSTextField(labelWithString: "│")
                pipe.font = .systemFont(ofSize: 11); pipe.textColor = .separatorColor
                pipe.sizeToFit()
                pipe.setFrameOrigin(CGPoint(x: x, y: midY - pipe.frame.height / 2))
                banner.addSubview(pipe)
                x += pipe.frame.width + 10
            }
            let lbl = NSTextField(labelWithString: stat.label + ": ")
            lbl.font = .systemFont(ofSize: 11); lbl.textColor = .secondaryLabelColor
            lbl.sizeToFit()
            lbl.setFrameOrigin(CGPoint(x: x, y: midY - lbl.frame.height / 2))
            banner.addSubview(lbl); x += lbl.frame.width + 1

            let val = NSTextField(labelWithString: stat.value)
            val.font = .systemFont(ofSize: 11, weight: .semibold)
            val.textColor = stat.color ?? .labelColor
            val.sizeToFit()
            val.setFrameOrigin(CGPoint(x: x, y: midY - val.frame.height / 2))
            banner.addSubview(val); x += val.frame.width + 14
        }
        return banner
    }

    /// Parse recent ERROR lines from today's daemon log.
    private func recentLogErrors(limit: Int = 3) -> [String] {
        let logPath = bestLogPath()
        guard let content = try? String(contentsOfFile: logPath, encoding: .utf8) else { return [] }
        let lines = content.components(separatedBy: "\n")
        let errors = lines.filter { $0.contains(" ERROR ") }
        return Array(errors.suffix(limit).map { line -> String in
            // Extract just the message part after the log level
            if let range = line.range(of: " ERROR ") {
                return String(line[range.upperBound...]).trimmingCharacters(in: .whitespaces)
            }
            return line
        })
    }

    // ── Tab 0: Overview ────────────────────────────────────────────────────────

    private func buildOverviewView(frame: NSRect) -> NSView {
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        let (sv, tv) = makeScrollableTextView(frame: frame)
        sv.autoresizingMask = [.width, .height]
        container.addSubview(sv)

        let rt = RichText()
        let hiConf   = patterns.filter { $0.confidence >= 0.8 }.count
        let totalTok = journal.reduce(0) { $0 + $1.tokensUsed }
        let avgConf  = patterns.isEmpty ? 0.0
            : patterns.reduce(0.0) { $0 + $1.confidence } / Double(patterns.count)
        let actionable = associations.filter { $0.actionable }.count
        let posCnt = patterns.filter { $0.valence == "positive" }.count
        let negCnt = patterns.filter { $0.valence == "negative" }.count
        let neuCnt = patterns.count - posCnt - negCnt

        // ── Header ──────────────────────────────────────────────────────────
        rt.header("Dashboard Overview")
        if let s = state {
            let statusIcon = s.totalCycles > 0 ? "●" : "○"
            let statusColor: NSColor = s.totalCycles > 0 ? .systemGreen : .systemOrange
            rt.raw(NSAttributedString(string: "  \(statusIcon) ", attributes: [
                .font: NSFont.systemFont(ofSize: 13), .foregroundColor: statusColor]))
            rt.raw(NSAttributedString(string: "Daemon running  ·  Last dream \(timeAgo(s.lastConsolidation))  ·  \(s.totalCycles) cycles\n", attributes: [
                .font: NSFont.systemFont(ofSize: 13), .foregroundColor: NSColor.secondaryLabelColor]))
        }
        rt.spacer()

        // ── Error/Alert Banner ──────────────────────────────────────────────
        let logErrors = recentLogErrors()
        if !logErrors.isEmpty {
            let bannerLine = NSMutableAttributedString()
            bannerLine.append(NSAttributedString(string: "  ⚠ Recent Errors (\(logErrors.count))\n", attributes: [
                .font: NSFont.systemFont(ofSize: 13, weight: .semibold),
                .foregroundColor: NSColor.systemOrange,
                .backgroundColor: NSColor.systemOrange.withAlphaComponent(0.08)]))
            for err in logErrors {
                let truncated = err.count > 120 ? String(err.prefix(120)) + "…" : err
                bannerLine.append(NSAttributedString(string: "  · \(truncated)\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 10.5, weight: .regular),
                    .foregroundColor: NSColor.systemOrange.withAlphaComponent(0.9)]))
            }
            bannerLine.append(NSAttributedString(string: "\n", attributes: [:]))
            rt.raw(bannerLine)
        }

        // ── Stat Cards Row 1 ────────────────────────────────────────────────
        rt.raw(statCardsRow([
            ("Patterns",      "\(patterns.count)",      .systemTeal,    "\(hiConf) high conf"),
            ("Associations",  "\(associations.count)",   .systemOrange,  "\(actionable) actionable"),
            ("Dream Cycles",  "\(journal.count)",        .systemIndigo,  fmtNum(totalTok) + " tokens"),
        ]))
        rt.spacer()

        // ── Stat Cards Row 2 ────────────────────────────────────────────────
        let (audit, _) = readLatestAudit()
        let calScore = audit?.calibrationScore.map { String(format: "%.0f%%", $0 * 100) } ?? "—"
        let biasCount = audit?.biasesDetected?.count ?? 0
        rt.raw(statCardsRow([
            ("Avg Confidence", String(format: "%.0f%%", avgConf * 100),  avgConf >= 0.7 ? .systemGreen : .systemBlue, "\(hiConf) patterns ≥80%"),
            ("Calibration",    calScore,                                 .systemPink,    "\(biasCount) biases detected"),
            ("Valence",        "\(posCnt)↑  \(negCnt)↓  \(neuCnt)·",    .labelColor,    "sentiment distribution"),
        ]))
        rt.spacer()

        // ── Insight Digest ──────────────────────────────────────────────────
        if let d = digest, !d.isEmpty {
            let sentiment = readDigestSentiment()
            let sentColor: NSColor = sentiment == "positive" ? .systemGreen
                                   : sentiment == "negative" ? .systemOrange : .labelColor
            rt.raw(NSAttributedString(string: "  ┌─ Latest Insight Digest ", attributes: [
                .font: NSFont.systemFont(ofSize: 13, weight: .semibold), .foregroundColor: NSColor.labelColor]))
            rt.raw(NSAttributedString(string: String(repeating: "─", count: 35) + "┐\n", attributes: [
                .font: NSFont.systemFont(ofSize: 10), .foregroundColor: NSColor.separatorColor]))
            let digestLines = d.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: "\n")
            for line in digestLines.prefix(8) {
                rt.raw(NSAttributedString(string: "  │  " + line + "\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 13), .foregroundColor: sentColor]))
            }
            if digestLines.count > 8 {
                rt.raw(NSAttributedString(string: "  │  … (\(digestLines.count - 8) more lines)\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.tertiaryLabelColor]))
            }
            rt.raw(NSAttributedString(string: "  └" + String(repeating: "─", count: 62) + "┘\n", attributes: [
                .font: NSFont.systemFont(ofSize: 10), .foregroundColor: NSColor.separatorColor]))
            rt.spacer()
        }

        // ── Pattern Categories Chart ────────────────────────────────────────
        if !patterns.isEmpty {
            rt.subheader("Pattern Categories")
            let cats = Dictionary(grouping: patterns, by: { $0.category })
                .sorted { $0.value.count > $1.value.count }
            let maxCount = cats.first?.value.count ?? 1
            let catColors: [NSColor] = [.systemTeal, .systemOrange, .systemIndigo, .systemPink,
                                         .systemGreen, .systemBlue, .systemPurple, .systemYellow,
                                         .systemRed, .systemBrown]
            for (i, (cat, pats)) in cats.prefix(10).enumerated() {
                let barLen = max(1, pats.count * 30 / maxCount)
                let bar    = String(repeating: "█", count: barLen)
                let empty  = String(repeating: " ", count: max(0, 30 - barLen))
                let catPadded = cat.padding(toLength: 16, withPad: " ", startingAt: 0)
                let avgC = pats.reduce(0.0) { $0 + $1.confidence } / Double(pats.count)
                let color = catColors[i % catColors.count]
                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  \(catPadded) ", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
                    .foregroundColor: NSColor.labelColor]))
                line.append(NSAttributedString(string: bar, attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
                    .foregroundColor: color]))
                line.append(NSAttributedString(string: empty, attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)]))
                line.append(NSAttributedString(string: " \(String(format: "%3d", pats.count))", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .bold),
                    .foregroundColor: NSColor.labelColor]))
                line.append(NSAttributedString(string: "  avg \(String(format: "%.0f%%", avgC * 100))\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                    .foregroundColor: NSColor.secondaryLabelColor]))
                rt.raw(line)
            }
            if cats.count > 10 {
                rt.dim("    … and \(cats.count - 10) more categories")
            }
            rt.spacer()
        }

        // ── Valence Distribution Bar ────────────────────────────────────────
        if !patterns.isEmpty {
            rt.subheader("Valence Distribution")
            let total = max(1, patterns.count)
            let posBar = max(0, posCnt * 40 / total)
            let negBar = max(0, negCnt * 40 / total)
            let neuBar = max(0, 40 - posBar - negBar)
            let dist = NSMutableAttributedString()
            dist.append(NSAttributedString(string: "  ", attributes: [:]))
            if posBar > 0 {
                dist.append(NSAttributedString(string: String(repeating: "▓", count: posBar), attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                    .foregroundColor: NSColor.systemGreen]))
            }
            if neuBar > 0 {
                dist.append(NSAttributedString(string: String(repeating: "░", count: neuBar), attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                    .foregroundColor: NSColor.secondaryLabelColor]))
            }
            if negBar > 0 {
                dist.append(NSAttributedString(string: String(repeating: "▓", count: negBar), attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                    .foregroundColor: NSColor.systemRed]))
            }
            dist.append(NSAttributedString(string: "\n", attributes: [:]))
            rt.raw(dist)
            let legend = NSMutableAttributedString(string: "  ")
            legend.append(NSAttributedString(string: "▲ \(posCnt) positive", attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.systemGreen]))
            legend.append(NSAttributedString(string: "    ● \(neuCnt) neutral", attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.secondaryLabelColor]))
            legend.append(NSAttributedString(string: "    ▼ \(negCnt) negative\n", attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.systemRed]))
            rt.raw(legend)
            rt.spacer()
        }

        // ── Token Usage Sparkline ───────────────────────────────────────────
        if !journal.isEmpty {
            rt.subheader("Token Usage  (per cycle)")
            let allTok = journal.map { $0.tokensUsed }
            let spark  = fmtSparkline(allTok, width: 40)
            if !spark.isEmpty {
                rt.raw(NSAttributedString(string: "  \(spark)\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 14, weight: .regular),
                    .foregroundColor: NSColor.systemIndigo]))
                let minTok = allTok.min() ?? 0
                let maxTok = allTok.max() ?? 0
                let avgTok = allTok.isEmpty ? 0 : allTok.reduce(0, +) / allTok.count
                rt.dim("  min \(fmtNum(minTok))  ·  avg \(fmtNum(avgTok))  ·  max \(fmtNum(maxTok))  ·  total \(fmtNum(totalTok))")
            }
            rt.spacer()
        }

        // ── Confidence Histogram ────────────────────────────────────────────
        if !patterns.isEmpty {
            rt.subheader("Confidence Distribution")
            let buckets = stride(from: 0.0, to: 1.01, by: 0.1).map { threshold -> Int in
                patterns.filter { $0.confidence >= threshold && $0.confidence < threshold + 0.1 }.count
            }
            let maxBucket = max(1, buckets.max() ?? 1)
            for (i, count) in buckets.enumerated() {
                let label = String(format: "%3.0f%%", Double(i) * 10)
                let barLen = max(0, count * 28 / maxBucket)
                let color: NSColor = i >= 8 ? .systemGreen : i >= 6 ? .systemBlue : i >= 4 ? .systemYellow : .systemRed
                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  \(label) ", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                    .foregroundColor: NSColor.secondaryLabelColor]))
                if barLen > 0 {
                    line.append(NSAttributedString(string: String(repeating: "▮", count: barLen), attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                        .foregroundColor: color]))
                }
                if count > 0 {
                    line.append(NSAttributedString(string: " \(count)", attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .medium),
                        .foregroundColor: NSColor.labelColor]))
                }
                line.append(NSAttributedString(string: "\n", attributes: [:]))
                rt.raw(line)
            }
            rt.spacer()
        }

        // ── Recent Cycles ───────────────────────────────────────��───────────
        if !journal.isEmpty {
            rt.subheader("Recent Cycles")
            for entry in journal.suffix(5).reversed() {
                let parts = [
                    entry.sessionsAnalyzed  > 0 ? "\(entry.sessionsAnalyzed) sessions"  : nil,
                    entry.patternsExtracted > 0 ? "\(entry.patternsExtracted) patterns"  : nil,
                    entry.associationsFound > 0 ? "\(entry.associationsFound) assoc"     : nil,
                    entry.insightsPromoted  > 0 ? "\(entry.insightsPromoted) insights"   : nil,
                ].compactMap { $0 }.joined(separator: "  ·  ")
                rt.raw(NSAttributedString(string: "  ◆ ", attributes: [
                    .font: NSFont.systemFont(ofSize: 12, weight: .bold),
                    .foregroundColor: NSColor.systemIndigo]))
                rt.raw(NSAttributedString(string: "\(fmtDate(entry.timestamp))  (\(timeAgo(entry.timestamp)))", attributes: [
                    .font: NSFont.systemFont(ofSize: 13, weight: .medium),
                    .foregroundColor: NSColor.systemIndigo,
                    .link: "overview-cycle:\(entry.timestamp)" as NSString,
                    .cursor: NSCursor.pointingHand]))
                rt.raw(NSAttributedString(string: "\n", attributes: [:]))
                rt.raw(NSAttributedString(string: "    \(parts.isEmpty ? "skipped — no sessions" : parts)  ·  \(fmtNum(entry.tokensUsed)) tokens\n\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 12),
                    .foregroundColor: NSColor.secondaryLabelColor]))
            }
        }

        // Wire link delegate for cycle navigation
        overviewLinkDelegate = JournalLinkDelegate { [weak self] linkStr in
            guard let self = self else { return }
            if linkStr.hasPrefix("overview-cycle:") {
                self.selectTab(3)  // Journal tab
            }
        }
        tv.delegate = overviewLinkDelegate
        tv.isAutomaticLinkDetectionEnabled = false

        tv.textStorage?.setAttributedString(rt.build())
        return container
    }

    /// Render a row of stat cards as styled attributed text.
    /// Each card: value (bold colored), label (medium dim), detail (tertiary).
    /// Cards are separated by thin vertical pipes with even spacing.
    private func statCardsRow(_ cards: [(label: String, value: String, color: NSColor, detail: String)]) -> NSAttributedString {
        let result = NSMutableAttributedString()
        // Use tab stops to create even columns
        let colW: CGFloat = 200
        let style = NSMutableParagraphStyle()
        style.tabStops = (0..<cards.count).map { NSTextTab(textAlignment: .left, location: CGFloat($0) * colW + 16) }
        style.lineSpacing = 2

        // Row 1: values
        result.append(NSAttributedString(string: "\t"))
        for (i, card) in cards.enumerated() {
            if i > 0 {
                result.append(NSAttributedString(string: "\t", attributes: [.paragraphStyle: style]))
            }
            result.append(NSAttributedString(string: card.value, attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 17, weight: .bold),
                .foregroundColor: card.color,
                .paragraphStyle: style]))
        }
        result.append(NSAttributedString(string: "\n"))

        // Row 2: labels
        result.append(NSAttributedString(string: "\t"))
        for (i, card) in cards.enumerated() {
            if i > 0 {
                result.append(NSAttributedString(string: "\t", attributes: [.paragraphStyle: style]))
            }
            result.append(NSAttributedString(string: card.label, attributes: [
                .font: NSFont.systemFont(ofSize: 11, weight: .medium),
                .foregroundColor: NSColor.secondaryLabelColor,
                .paragraphStyle: style]))
        }
        result.append(NSAttributedString(string: "\n"))

        // Row 3: details
        result.append(NSAttributedString(string: "\t"))
        for (i, card) in cards.enumerated() {
            if i > 0 {
                result.append(NSAttributedString(string: "\t", attributes: [.paragraphStyle: style]))
            }
            result.append(NSAttributedString(string: card.detail, attributes: [
                .font: NSFont.systemFont(ofSize: 10),
                .foregroundColor: NSColor.tertiaryLabelColor,
                .paragraphStyle: style]))
        }
        result.append(NSAttributedString(string: "\n"))
        return result
    }

    // ── Tab 1: Pattern Network ─────────────────────────────────────────────────

    private func buildPatternView(frame: NSRect) -> NSView {
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        let bannerH: CGFloat = 40
        let hintH:   CGFloat = 36
        let contentH = frame.height - bannerH - hintH

        // Stats banner (top)
        let hiConf  = patterns.filter { $0.confidence >= 0.8 }.count
        let avgConf = patterns.isEmpty ? 0.0 : patterns.reduce(0.0) { $0 + $1.confidence } / Double(patterns.count)
        let catCnt  = Set(patterns.map { $0.category }).count
        let posCnt  = patterns.filter { $0.valence == "positive" }.count
        let negCnt  = patterns.filter { $0.valence == "negative" }.count
        let banner  = makeStatsBanner(
            frame: NSRect(x: 0, y: frame.height - bannerH, width: frame.width, height: bannerH),
            stats: [
                ("Total",       "\(patterns.count)", nil),
                ("High conf",   "\(hiConf)",          hiConf > 0 ? .systemGreen : nil),
                ("Avg conf",    String(format: "%.0f%%", avgConf * 100), nil),
                ("Categories",  "\(catCnt)",           nil),
                ("Positive",    "\(posCnt)",            posCnt > 0 ? .systemGreen : nil),
                ("Negative",    "\(negCnt)",            negCnt > 0 ? .systemOrange : nil),
            ])
        banner.autoresizingMask = [.width, .minYMargin]
        container.addSubview(banner)

        // Hint bar (bottom)
        let hintBar = NSView(frame: NSRect(x: 0, y: 0, width: frame.width, height: hintH))
        hintBar.autoresizingMask = [.width, .maxYMargin]
        let hintSep = NSBox(frame: NSRect(x: 0, y: hintH - 1, width: frame.width, height: 1))
        hintSep.boxType = .separator; hintSep.autoresizingMask = [.width]
        hintBar.addSubview(hintSep)
        let hintLbl = NSTextField(labelWithString:
            "Click list item or graph node to see details  ·  Drag to pan  ·  Scroll/pinch to zoom  ·  Dbl-click graph to reset")
        hintLbl.font = .systemFont(ofSize: 10.5); hintLbl.textColor = .tertiaryLabelColor
        hintLbl.frame = NSRect(x: 14, y: 10, width: frame.width - 28, height: 16)
        hintLbl.autoresizingMask = [.width]
        hintBar.addSubview(hintLbl)
        container.addSubview(hintBar)

        guard !patterns.isEmpty else {
            let lbl = NSTextField(labelWithString: "No patterns yet — run a few dream cycles.")
            lbl.font = .systemFont(ofSize: 14); lbl.textColor = .secondaryLabelColor
            lbl.frame = NSRect(x: 20, y: frame.height / 2 - 12, width: frame.width - 40, height: 24)
            lbl.autoresizingMask = [.width, .minYMargin]
            container.addSubview(lbl)
            return container
        }

        // Main horizontal split: left panel (list + detail) | right (graph)
        let listW: CGFloat = 310
        let splitFrame = NSRect(x: 0, y: hintH, width: frame.width, height: contentH)
        let split = NSSplitView(frame: splitFrame)
        split.isVertical       = true
        split.dividerStyle     = .thin
        split.autoresizingMask = [.width, .height]

        // Left panel: vertical split — grouped list (top) + detail pane (bottom)
        let leftPanel = NSSplitView(frame: NSRect(x: 0, y: 0, width: listW, height: contentH))
        leftPanel.isVertical       = false
        leftPanel.dividerStyle     = .thin
        leftPanel.autoresizingMask = [.width, .height]

        // --- Grouped pattern list (top of left panel) ---
        let listH = contentH * 0.6
        let (listSV, listTV) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: listW, height: listH))
        listTV.textContainerInset = NSSize(width: 10, height: 10)
        listTV.backgroundColor = .clear; listTV.drawsBackground = false

        // Group patterns by category, sorted by count descending
        let byCategory = Dictionary(grouping: patterns, by: { $0.category })
        let sortedCats = byCategory.keys.sorted { byCategory[$0]!.count > byCategory[$1]!.count }
        let catColors: [String: NSColor] = {
            let palette: [NSColor] = [.systemTeal, .systemPurple, .systemOrange, .systemIndigo, .systemPink,
                                      .systemGreen, .systemBlue, .systemYellow, .systemRed, .systemBrown]
            var m: [String: NSColor] = [:]
            for (i, cat) in sortedCats.enumerated() { m[cat] = palette[i % palette.count] }
            return m
        }()

        let lrt = RichText()
        let itemSpacing = NSMutableParagraphStyle()
        itemSpacing.lineSpacing = 3
        itemSpacing.paragraphSpacingBefore = 2

        for cat in sortedCats {
            let pats = byCategory[cat]!.sorted { $0.confidence > $1.confidence }
            let catColor = catColors[cat] ?? .secondaryLabelColor

            // Category header with colored dot and count
            let catHeaderStyle = NSMutableParagraphStyle()
            catHeaderStyle.paragraphSpacingBefore = 8
            let catHeader = NSMutableAttributedString()
            catHeader.append(NSAttributedString(string: "●  ", attributes: [
                .font: NSFont.systemFont(ofSize: 11),
                .foregroundColor: catColor,
                .paragraphStyle: catHeaderStyle]))
            catHeader.append(NSAttributedString(string: cat.uppercased(), attributes: [
                .font: NSFont.systemFont(ofSize: 11.5, weight: .bold),
                .foregroundColor: catColor]))
            catHeader.append(NSAttributedString(string: "  (\(pats.count))\n", attributes: [
                .font: NSFont.systemFont(ofSize: 10.5, weight: .medium),
                .foregroundColor: NSColor.tertiaryLabelColor]))
            lrt.raw(catHeader)

            for pat in pats {
                let pid    = pat.stableKey
                let conf   = Int(pat.confidence * 100)
                let cCol: NSColor = pat.confidence >= 0.8 ? .systemGreen
                                  : pat.confidence >= 0.6 ? .systemBlue : .secondaryLabelColor
                let badge  = String(format: "%3d%%", conf)
                let text   = pat.pattern.count > 52 ? String(pat.pattern.prefix(49)) + "…" : pat.pattern
                let valDot: String = pat.valence == "positive" ? "▲" : pat.valence == "negative" ? "▼" : "·"
                let valCol: NSColor = pat.valence == "positive" ? .systemGreen
                                    : pat.valence == "negative" ? .systemOrange : .tertiaryLabelColor

                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  " + badge + " ", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .semibold),
                    .foregroundColor: cCol,
                    .paragraphStyle: itemSpacing]))
                line.append(NSAttributedString(string: valDot + " ", attributes: [
                    .font: NSFont.systemFont(ofSize: 11),
                    .foregroundColor: valCol]))
                line.append(NSAttributedString(string: text + "\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 13),
                    .foregroundColor: NSColor.labelColor,
                    .link: "pattern:\(pid)" as NSString,
                    .cursor: NSCursor.pointingHand]))
                lrt.raw(line)
            }
            // Spacer between categories
            lrt.raw(NSAttributedString(string: "\n", attributes: [.font: NSFont.systemFont(ofSize: 4)]))
        }
        listTV.textStorage?.setAttributedString(lrt.build())
        listTV.linkTextAttributes = [.foregroundColor: NSColor.labelColor, .underlineStyle: 0]

        // --- Detail pane (bottom of left panel) ---
        let detailH = contentH * 0.4
        let (detailSV, detailTV) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: listW, height: detailH))
        detailTV.textContainerInset = NSSize(width: 10, height: 10)
        detailTV.backgroundColor = .clear; detailTV.drawsBackground = false
        patternDetailTextView = detailTV

        // Initial placeholder for detail pane
        let placeholderRT = RichText()
        placeholderRT.dim("Select a pattern from the list or graph to see details.")
        detailTV.textStorage?.setAttributedString(placeholderRT.build())

        // Detail card wrapper — rounded border with subtle background
        let detailWrap = NSView(frame: NSRect(x: 0, y: 0, width: listW, height: detailH))
        detailWrap.autoresizingMask = [.width, .height]
        detailWrap.wantsLayer = true

        let cardInset: CGFloat = 6
        let cardView = NSView(frame: detailWrap.bounds.insetBy(dx: cardInset, dy: cardInset))
        cardView.autoresizingMask = [.width, .height]
        cardView.wantsLayer = true
        cardView.layer?.cornerRadius = 8
        cardView.layer?.borderWidth  = 1
        cardView.layer?.borderColor  = NSColor.separatorColor.cgColor
        cardView.layer?.backgroundColor = NSColor.controlBackgroundColor.withAlphaComponent(0.3).cgColor

        detailSV.frame = cardView.bounds.insetBy(dx: 2, dy: 2)
        detailSV.autoresizingMask = [.width, .height]
        cardView.addSubview(detailSV)
        detailWrap.addSubview(cardView)

        leftPanel.addArrangedSubview(listSV)
        leftPanel.addArrangedSubview(detailWrap)
        DispatchQueue.main.async { leftPanel.setPosition(listH, ofDividerAt: 0) }

        // --- Graph (right panel) ---
        let gv = PatternGraphView(
            frame: NSRect(x: 0, y: 0, width: frame.width - listW - 1, height: contentH),
            patterns: patterns)
        gv.autoresizingMask = [.width, .height]

        // Shared selection handler — updates detail pane, graph highlight, and list scroll
        let selectPattern: (String?) -> Void = { [weak self, weak gv, weak listTV, weak detailTV] selectedId in
            gv?.highlightedId = selectedId
            // Clear previous highlight and apply new one
            if let tv = listTV, let ts = tv.textStorage {
                let full = NSRange(location: 0, length: ts.length)
                ts.removeAttribute(.backgroundColor, range: full)
                if let id = selectedId {
                    var found: NSRange?
                    ts.enumerateAttribute(.link, in: full) { val, range, stop in
                        if (val as? NSString) == "pattern:\(id)" as NSString {
                            found = range; stop.pointee = true
                        }
                    }
                    if let r = found {
                        // Extend highlight to cover the full line (including badge before the link)
                        let lineRange = (ts.string as NSString).lineRange(for: r)
                        let hlColor = NSColor.controlAccentColor.withAlphaComponent(0.15)
                        ts.addAttribute(.backgroundColor, value: hlColor, range: lineRange)
                        tv.scrollRangeToVisible(r)
                    }
                }
            }
            // Update detail pane
            guard let self = self, let dtv = detailTV else { return }
            guard let id = selectedId,
                  let pat = self.patterns.first(where: { $0.stableKey == id }) else {
                let rt = RichText()
                rt.dim("Select a pattern from the list or graph to see details.")
                dtv.textStorage?.setAttributedString(rt.build())
                return
            }
            self.renderPatternDetail(pat, into: dtv)
        }

        // List + Detail → Graph + Detail (handles both pattern: and assoc: links)
        let ld = JournalLinkDelegate { [weak self] link in
            if link.hasPrefix("pattern:") {
                selectPattern(String(link.dropFirst("pattern:".count)))
            } else if link.hasPrefix("assoc:") {
                // Navigate to Associations tab and select the association
                self?.selectTab(2)
            }
        }
        patternListDelegate = ld
        listTV.delegate     = ld
        detailTV.delegate   = ld
        detailTV.linkTextAttributes = [.foregroundColor: NSColor.labelColor, .underlineStyle: 0]

        // Graph → List + Detail
        gv.onNodeSelected = { selectedId in
            selectPattern(selectedId)
        }
        patternGraphView = gv

        // Graph wrapper with search field overlay
        let graphWrap = NSView(frame: NSRect(x: 0, y: 0, width: frame.width - listW - 1, height: contentH))
        graphWrap.autoresizingMask = [.width, .height]

        // Search field at top of graph
        let searchH: CGFloat = 32
        let searchBar = NSView(frame: NSRect(x: 0, y: contentH - searchH, width: graphWrap.frame.width, height: searchH))
        searchBar.autoresizingMask = [.width, .minYMargin]
        searchBar.wantsLayer = true
        searchBar.layer?.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.85).cgColor

        let graphSearch = NSSearchField(frame: NSRect(x: 8, y: 4, width: graphWrap.frame.width - 16, height: 24))
        graphSearch.placeholderString = "Filter graph nodes…"
        graphSearch.font = .systemFont(ofSize: 11)
        graphSearch.autoresizingMask = [.width]
        graphSearch.target = self
        graphSearch.action = #selector(patternGraphSearchChanged(_:))
        searchBar.addSubview(graphSearch)

        gv.frame = NSRect(x: 0, y: 0, width: graphWrap.frame.width, height: contentH - searchH)
        gv.autoresizingMask = [.width, .height]
        graphWrap.addSubview(gv)
        graphWrap.addSubview(searchBar)

        split.addArrangedSubview(leftPanel)
        split.addArrangedSubview(graphWrap)
        container.addSubview(split)
        DispatchQueue.main.async { split.setPosition(listW, ofDividerAt: 0) }
        return container
    }

    @objc private func patternGraphSearchChanged(_ sender: NSSearchField) {
        patternGraphView?.applySearch(sender.stringValue)
    }

    /// Render pattern detail into a text view — full text, metadata, and linked associations.
    private func renderPatternDetail(_ pat: Pattern, into tv: NSTextView) {
        let rt = RichText()

        // Pattern header
        rt.subheader(pat.pattern)
        rt.spacer()

        // Metadata row: category, valence, confidence bar
        let confPct = Int(pat.confidence * 100)
        let filled  = String(repeating: "▮", count: confPct / 10)
        let empty   = String(repeating: "░", count: 10 - confPct / 10)
        let valColor: NSColor = pat.valence == "positive" ? .systemGreen
                              : pat.valence == "negative" ? .systemOrange
                              : .secondaryLabelColor
        let metaStr = NSMutableAttributedString()
        metaStr.append(NSAttributedString(string: pat.category, attributes: [
            .font: NSFont.systemFont(ofSize: 11, weight: .medium),
            .foregroundColor: NSColor.secondaryLabelColor]))
        metaStr.append(NSAttributedString(string: "  ·  ", attributes: [
            .font: NSFont.systemFont(ofSize: 11),
            .foregroundColor: NSColor.tertiaryLabelColor]))
        metaStr.append(NSAttributedString(string: pat.valence, attributes: [
            .font: NSFont.systemFont(ofSize: 11, weight: .medium),
            .foregroundColor: valColor]))
        metaStr.append(NSAttributedString(string: "  ·  ", attributes: [
            .font: NSFont.systemFont(ofSize: 11),
            .foregroundColor: NSColor.tertiaryLabelColor]))
        metaStr.append(NSAttributedString(string: "\(filled)\(empty) \(confPct)%\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor]))
        rt.raw(metaStr)

        if let first = pat.firstSeen {
            rt.dim("first seen: \(fmtDate(first))")
        }

        // Linked associations — find associations whose patternsLinked contains this pattern's ID
        if let pid = pat.id {
            let linked = associations.filter { ($0.patternsLinked ?? []).contains(pid) }
            if !linked.isEmpty {
                rt.spacer()
                let hdrStr = NSMutableAttributedString()
                hdrStr.append(NSAttributedString(string: "⚡ Linked Associations", attributes: [
                    .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                    .foregroundColor: NSColor.systemOrange]))
                hdrStr.append(NSAttributedString(string: "  (\(linked.count))\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: NSColor.tertiaryLabelColor]))
                rt.raw(hdrStr)

                for assoc in linked.sorted(by: { $0.confidence > $1.confidence }).prefix(8) {
                    let conf = Int(assoc.confidence * 100)
                    let aCol: NSColor = assoc.actionable ? .systemGreen : .systemBlue
                    let marker = assoc.actionable ? "◆" : "○"
                    let text = assoc.hypothesis.count > 60
                        ? String(assoc.hypothesis.prefix(57)) + "…"
                        : assoc.hypothesis

                    let aLine = NSMutableAttributedString()
                    aLine.append(NSAttributedString(string: "  \(marker) ", attributes: [
                        .font: NSFont.systemFont(ofSize: 10),
                        .foregroundColor: aCol]))
                    aLine.append(NSAttributedString(string: "\(conf)%  ", attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: 9, weight: .medium),
                        .foregroundColor: aCol.withAlphaComponent(0.7)]))
                    aLine.append(NSAttributedString(string: text + "\n", attributes: [
                        .font: NSFont.systemFont(ofSize: 11),
                        .foregroundColor: NSColor.labelColor,
                        .link: "assoc:\(assoc.id)" as NSString,
                        .cursor: NSCursor.pointingHand]))
                    rt.raw(aLine)
                }
                if linked.count > 8 {
                    rt.dim("  … and \(linked.count - 8) more")
                }
            } else {
                rt.spacer()
                rt.dim("No linked associations yet.")
            }

            // Same-category siblings
            let siblings = patterns.filter { $0.category == pat.category && $0.id != pid }
                .sorted { $0.confidence > $1.confidence }
            if !siblings.isEmpty {
                rt.spacer()
                let sibHdr = NSMutableAttributedString()
                sibHdr.append(NSAttributedString(string: "⟁ Same Category", attributes: [
                    .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                    .foregroundColor: NSColor.systemTeal]))
                sibHdr.append(NSAttributedString(string: "  (\(siblings.count) in \(pat.category))\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: NSColor.tertiaryLabelColor]))
                rt.raw(sibHdr)
                for sib in siblings.prefix(5) {
                    let sConf = Int(sib.confidence * 100)
                    let sText = sib.pattern.count > 55 ? String(sib.pattern.prefix(52)) + "…" : sib.pattern
                    rt.dim("  \(sConf)%  \(sText)")
                }
                if siblings.count > 5 {
                    rt.dim("  … and \(siblings.count - 5) more")
                }
            }
        }

        tv.textStorage?.setAttributedString(rt.build())
        tv.scrollToBeginningOfDocument(nil)
    }

    // ── Tab 2: Association Network ─────────────────────────────────────────────

    private func buildAssociationView(frame: NSRect) -> NSView {
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        let bannerH: CGFloat = 40
        let hintH:   CGFloat = 36
        let contentH = frame.height - bannerH - hintH

        let actionCnt  = associations.filter { $0.actionable }.count
        let hiConfA    = associations.filter { $0.confidence >= 0.8 }.count
        let avgConfA   = associations.isEmpty ? 0.0
            : associations.reduce(0.0) { $0 + $1.confidence } / Double(associations.count)
        let linkedPats = associations.flatMap { $0.patternsLinked ?? [] }
        let uniquePats = Set(linkedPats).count

        let banner = makeStatsBanner(
            frame: NSRect(x: 0, y: frame.height - bannerH, width: frame.width, height: bannerH),
            stats: [
                ("Total",       "\(associations.count)", nil),
                ("Actionable",  "\(actionCnt)",           actionCnt > 0 ? .systemGreen : nil),
                ("High conf",   "\(hiConfA)",             hiConfA > 0 ? .systemGreen : nil),
                ("Avg conf",    String(format: "%.0f%%", avgConfA * 100), nil),
                ("Linked pats", "\(uniquePats)",          uniquePats > 0 ? .systemTeal : nil),
            ])
        banner.autoresizingMask = [.width, .minYMargin]
        container.addSubview(banner)

        // Hint bar (bottom)
        let hintBar = NSView(frame: NSRect(x: 0, y: 0, width: frame.width, height: hintH))
        hintBar.autoresizingMask = [.width, .maxYMargin]
        let hintSep = NSBox(frame: NSRect(x: 0, y: hintH - 1, width: frame.width, height: 1))
        hintSep.boxType = .separator; hintSep.autoresizingMask = [.width]
        hintBar.addSubview(hintSep)
        let hintLbl = NSTextField(labelWithString:
            "◆ = actionable  ·  Rings: inner ≥75% · mid ≥50% · outer <50%  ·  Click to see linked patterns")
        hintLbl.font = .systemFont(ofSize: 10.5); hintLbl.textColor = .tertiaryLabelColor
        hintLbl.frame = NSRect(x: 14, y: 10, width: frame.width - 28, height: 16)
        hintLbl.autoresizingMask = [.width]
        hintBar.addSubview(hintLbl)
        container.addSubview(hintBar)

        guard !associations.isEmpty else {
            let lbl = NSTextField(labelWithString: "No associations yet — run a few dream cycles.")
            lbl.font = .systemFont(ofSize: 14); lbl.textColor = .secondaryLabelColor
            lbl.frame = NSRect(x: 20, y: frame.height / 2 - 12, width: frame.width - 40, height: 24)
            lbl.autoresizingMask = [.width, .minYMargin]
            container.addSubview(lbl)
            return container
        }

        // Main horizontal split: left panel (list + detail) | right (graph)
        let listW: CGFloat = 320
        let splitFrame = NSRect(x: 0, y: hintH, width: frame.width, height: contentH)
        let split = NSSplitView(frame: splitFrame)
        split.isVertical       = true
        split.dividerStyle     = .thin
        split.autoresizingMask = [.width, .height]

        // Left panel: vertical split — list (top) + detail (bottom)
        let leftPanel = NSSplitView(frame: NSRect(x: 0, y: 0, width: listW, height: contentH))
        leftPanel.isVertical       = false
        leftPanel.dividerStyle     = .thin
        leftPanel.autoresizingMask = [.width, .height]

        // --- Association list (top) grouped by confidence tier ---
        let listH = contentH * 0.55
        let (listSV, listTV) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: listW, height: listH))
        listTV.textContainerInset = NSSize(width: 10, height: 10)
        listTV.backgroundColor = .clear; listTV.drawsBackground = false

        // Group by confidence tier (using comparisons to avoid floating-point gaps)
        let tiers: [(name: String, test: (Double) -> Bool, color: NSColor)] = [
            ("High Confidence  (≥75%)", { $0 >= 0.75 }, .systemGreen),
            ("Medium Confidence  (50–74%)", { $0 >= 0.50 && $0 < 0.75 }, .systemBlue),
            ("Low Confidence  (<50%)", { $0 < 0.50 }, .secondaryLabelColor),
        ]

        let art = RichText()
        let aItemSpacing = NSMutableParagraphStyle()
        aItemSpacing.lineSpacing = 3
        aItemSpacing.paragraphSpacingBefore = 2

        for tier in tiers {
            let group = associations
                .filter { tier.test($0.confidence) }
                .sorted { $0.confidence > $1.confidence }
            guard !group.isEmpty else { continue }

            // Tier header
            let tierHdrStyle = NSMutableParagraphStyle()
            tierHdrStyle.paragraphSpacingBefore = 8
            let tierHdr = NSMutableAttributedString()
            tierHdr.append(NSAttributedString(string: "●  ", attributes: [
                .font: NSFont.systemFont(ofSize: 11),
                .foregroundColor: tier.color,
                .paragraphStyle: tierHdrStyle]))
            tierHdr.append(NSAttributedString(string: tier.name, attributes: [
                .font: NSFont.systemFont(ofSize: 11.5, weight: .bold),
                .foregroundColor: tier.color]))
            tierHdr.append(NSAttributedString(string: "  (\(group.count))\n", attributes: [
                .font: NSFont.systemFont(ofSize: 10.5),
                .foregroundColor: NSColor.tertiaryLabelColor]))
            art.raw(tierHdr)

            for assoc in group {
                let conf  = Int(assoc.confidence * 100)
                let badge = String(format: "%3d%%", conf)
                let text  = assoc.hypothesis.count > 50 ? String(assoc.hypothesis.prefix(47)) + "…" : assoc.hypothesis
                let marker = assoc.actionable ? "◆" : "○"
                let mCol: NSColor = assoc.actionable ? .systemGreen : .tertiaryLabelColor
                let linkedCnt = assoc.patternsLinked?.count ?? 0

                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  " + badge + " ", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .semibold),
                    .foregroundColor: tier.color,
                    .paragraphStyle: aItemSpacing]))
                line.append(NSAttributedString(string: marker + " ", attributes: [
                    .font: NSFont.systemFont(ofSize: 11),
                    .foregroundColor: mCol]))
                line.append(NSAttributedString(string: text + "\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 13),
                    .foregroundColor: NSColor.labelColor,
                    .link: "assoc:\(assoc.id)" as NSString,
                    .cursor: NSCursor.pointingHand]))
                if linkedCnt > 0 {
                    line.append(NSAttributedString(string: "        \(linkedCnt) linked patterns\n", attributes: [
                        .font: NSFont.systemFont(ofSize: 9.5),
                        .foregroundColor: NSColor.tertiaryLabelColor]))
                }
                art.raw(line)
            }
            art.raw(NSAttributedString(string: "\n", attributes: [.font: NSFont.systemFont(ofSize: 4)]))
        }
        listTV.textStorage?.setAttributedString(art.build())
        listTV.linkTextAttributes = [.foregroundColor: NSColor.labelColor, .underlineStyle: 0]

        // --- Detail pane (bottom of left panel) ---
        let detailH = contentH * 0.45
        let (detailSV, detailTV) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: listW, height: detailH))
        detailTV.textContainerInset = NSSize(width: 10, height: 10)
        detailTV.backgroundColor = .clear; detailTV.drawsBackground = false
        assocDetailTextView = detailTV

        let placeholderRT = RichText()
        placeholderRT.dim("Select an association to see linked patterns and details.")
        detailTV.textStorage?.setAttributedString(placeholderRT.build())

        // Detail card wrapper — rounded border with subtle background
        let detailWrap = NSView(frame: NSRect(x: 0, y: 0, width: listW, height: detailH))
        detailWrap.autoresizingMask = [.width, .height]
        detailWrap.wantsLayer = true

        let aCardInset: CGFloat = 6
        let aCardView = NSView(frame: detailWrap.bounds.insetBy(dx: aCardInset, dy: aCardInset))
        aCardView.autoresizingMask = [.width, .height]
        aCardView.wantsLayer = true
        aCardView.layer?.cornerRadius = 8
        aCardView.layer?.borderWidth  = 1
        aCardView.layer?.borderColor  = NSColor.separatorColor.cgColor
        aCardView.layer?.backgroundColor = NSColor.controlBackgroundColor.withAlphaComponent(0.3).cgColor

        detailSV.frame = aCardView.bounds.insetBy(dx: 2, dy: 2)
        detailSV.autoresizingMask = [.width, .height]
        aCardView.addSubview(detailSV)
        detailWrap.addSubview(aCardView)

        leftPanel.addArrangedSubview(listSV)
        leftPanel.addArrangedSubview(detailWrap)
        DispatchQueue.main.async { leftPanel.setPosition(listH, ofDividerAt: 0) }

        // --- Graph (right panel) ---
        let av = AssociationGraphView(
            frame: NSRect(x: 0, y: 0, width: frame.width - listW - 1, height: contentH),
            associations: associations)
        av.autoresizingMask = [.width, .height]

        // Shared selection handler
        let selectAssociation: (String?) -> Void = { [weak self, weak av, weak listTV, weak detailTV] selectedId in
            av?.highlightedId = selectedId
            // Clear previous highlight and apply new one
            if let tv = listTV, let ts = tv.textStorage {
                let full = NSRange(location: 0, length: ts.length)
                ts.removeAttribute(.backgroundColor, range: full)
                if let id = selectedId {
                    var found: NSRange?
                    ts.enumerateAttribute(.link, in: full) { val, range, stop in
                        if (val as? NSString) == "assoc:\(id)" as NSString {
                            found = range; stop.pointee = true
                        }
                    }
                    if let r = found {
                        let lineRange = (ts.string as NSString).lineRange(for: r)
                        let hlColor = NSColor.controlAccentColor.withAlphaComponent(0.15)
                        ts.addAttribute(.backgroundColor, value: hlColor, range: lineRange)
                        tv.scrollRangeToVisible(r)
                    }
                }
            }
            // Update detail pane
            guard let self = self, let dtv = detailTV else { return }
            guard let id = selectedId,
                  let assoc = self.associations.first(where: { $0.id == id }) else {
                let rt = RichText()
                rt.dim("Select an association to see linked patterns and details.")
                dtv.textStorage?.setAttributedString(rt.build())
                return
            }
            self.renderAssociationDetail(assoc, into: dtv)
        }

        // List + Detail → Graph + Detail (handles both assoc: and pattern: links)
        let ald = JournalLinkDelegate { [weak self] link in
            if link.hasPrefix("assoc:") {
                selectAssociation(String(link.dropFirst("assoc:".count)))
            } else if link.hasPrefix("pattern:") {
                // Navigate to Patterns tab
                self?.selectTab(1)
            }
        }
        assocListDelegate = ald
        listTV.delegate   = ald
        detailTV.delegate = ald
        detailTV.linkTextAttributes = [.foregroundColor: NSColor.labelColor, .underlineStyle: 0]

        // Graph → List + Detail
        av.onNodeSelected = { selectedId in
            selectAssociation(selectedId)
        }
        assocGraphView = av

        // Graph wrapper with search field overlay
        let graphWrap = NSView(frame: NSRect(x: 0, y: 0, width: frame.width - listW - 1, height: contentH))
        graphWrap.autoresizingMask = [.width, .height]

        // Search field at top of graph
        let searchH: CGFloat = 32
        let searchBar = NSView(frame: NSRect(x: 0, y: contentH - searchH, width: graphWrap.frame.width, height: searchH))
        searchBar.autoresizingMask = [.width, .minYMargin]
        searchBar.wantsLayer = true
        searchBar.layer?.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.85).cgColor

        let graphSearch = NSSearchField(frame: NSRect(x: 8, y: 4, width: graphWrap.frame.width - 16, height: 24))
        graphSearch.placeholderString = "Filter graph nodes…"
        graphSearch.font = .systemFont(ofSize: 11)
        graphSearch.autoresizingMask = [.width]
        graphSearch.target = self
        graphSearch.action = #selector(assocGraphSearchChanged(_:))
        searchBar.addSubview(graphSearch)

        av.frame = NSRect(x: 0, y: 0, width: graphWrap.frame.width, height: contentH - searchH)
        av.autoresizingMask = [.width, .height]
        graphWrap.addSubview(av)
        graphWrap.addSubview(searchBar)

        split.addArrangedSubview(leftPanel)
        split.addArrangedSubview(graphWrap)
        container.addSubview(split)
        DispatchQueue.main.async { split.setPosition(listW, ofDividerAt: 0) }
        return container
    }

    @objc private func assocGraphSearchChanged(_ sender: NSSearchField) {
        assocGraphView?.applySearch(sender.stringValue)
    }

    /// Render association detail into a text view — full hypothesis, metadata, linked patterns, and suggested rule.
    private func renderAssociationDetail(_ assoc: Association, into tv: NSTextView) {
        let rt = RichText()

        // Hypothesis header
        rt.subheader(assoc.hypothesis)
        rt.spacer()

        // Metadata
        let confPct = Int(assoc.confidence * 100)
        let filled  = String(repeating: "▮", count: confPct / 10)
        let empty   = String(repeating: "░", count: 10 - confPct / 10)
        let metaStr = NSMutableAttributedString()
        metaStr.append(NSAttributedString(string: "\(filled)\(empty) \(confPct)%", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor]))
        if assoc.actionable {
            metaStr.append(NSAttributedString(string: "  ·  ◆ actionable", attributes: [
                .font: NSFont.systemFont(ofSize: 11, weight: .medium),
                .foregroundColor: NSColor.systemGreen]))
        }
        metaStr.append(NSAttributedString(string: "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 11)]))
        rt.raw(metaStr)

        // Suggested rule
        if let rule = assoc.suggestedRule, !rule.isEmpty {
            rt.spacer()
            let ruleStr = NSMutableAttributedString()
            ruleStr.append(NSAttributedString(string: "→ Rule: ", attributes: [
                .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                .foregroundColor: NSColor.systemYellow]))
            ruleStr.append(NSAttributedString(string: rule + "\n", attributes: [
                .font: NSFont.systemFont(ofSize: 11),
                .foregroundColor: NSColor.labelColor]))
            rt.raw(ruleStr)
        }

        // Linked patterns — resolve IDs to actual pattern objects
        let linkedIds = assoc.patternsLinked ?? []
        if !linkedIds.isEmpty {
            let resolvedPatterns = linkedIds.compactMap { id in
                patterns.first { $0.id == id }
            }
            rt.spacer()
            let hdrStr = NSMutableAttributedString()
            hdrStr.append(NSAttributedString(string: "🔗 Linked Patterns", attributes: [
                .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                .foregroundColor: NSColor.systemTeal]))
            hdrStr.append(NSAttributedString(string: "  (\(resolvedPatterns.count) of \(linkedIds.count) resolved)\n", attributes: [
                .font: NSFont.systemFont(ofSize: 10),
                .foregroundColor: NSColor.tertiaryLabelColor]))
            rt.raw(hdrStr)

            for pat in resolvedPatterns.sorted(by: { $0.confidence > $1.confidence }) {
                let pConf = Int(pat.confidence * 100)
                let valDot: String = pat.valence == "positive" ? "▲" : pat.valence == "negative" ? "▼" : "·"
                let valCol: NSColor = pat.valence == "positive" ? .systemGreen
                                    : pat.valence == "negative" ? .systemOrange : .tertiaryLabelColor
                let catColor: NSColor = .secondaryLabelColor
                let pText = pat.pattern.count > 55 ? String(pat.pattern.prefix(52)) + "…" : pat.pattern

                let pLine = NSMutableAttributedString()
                pLine.append(NSAttributedString(string: "  \(valDot) ", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: valCol]))
                pLine.append(NSAttributedString(string: "\(pConf)%  ", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 9, weight: .medium),
                    .foregroundColor: catColor]))
                pLine.append(NSAttributedString(string: pText + "\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 11),
                    .foregroundColor: NSColor.labelColor,
                    .link: "pattern:\(pat.stableKey)" as NSString,
                    .cursor: NSCursor.pointingHand]))
                pLine.append(NSAttributedString(string: "        \(pat.category)  ·  \(pat.valence)\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 9.5),
                    .foregroundColor: NSColor.tertiaryLabelColor]))
                rt.raw(pLine)
            }

            // Show unresolved IDs if any
            let unresolvedCount = linkedIds.count - resolvedPatterns.count
            if unresolvedCount > 0 {
                rt.dim("  + \(unresolvedCount) pattern(s) not found in current data")
            }
        } else {
            rt.spacer()
            rt.dim("No linked patterns.")
        }

        tv.textStorage?.setAttributedString(rt.build())
        tv.scrollToBeginningOfDocument(nil)
    }

    // ── Tab 3: Journal ─────────────────────────────────────────────────────────

    private func buildJournalView(frame: NSRect) -> NSView {
        let bannerH: CGFloat = 40
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        let allTok   = journal.map { $0.tokensUsed }
        let totalTok = allTok.reduce(0, +)
        let maxTok   = allTok.max() ?? 0
        let avgTok   = journal.isEmpty ? 0 : totalTok / journal.count
        let skipped  = journal.filter { $0.sessionsAnalyzed == 0 }.count

        let banner = makeStatsBanner(
            frame: NSRect(x: 0, y: frame.height - bannerH, width: frame.width, height: bannerH),
            stats: [
                ("Cycles",  "\(journal.count)",  nil),
                ("Skipped", "\(skipped)",         skipped > 0 ? .systemOrange : nil),
                ("Total tok", fmtNum(totalTok),  nil),
                ("Avg tok",   fmtNum(avgTok),    nil),
                ("Peak tok",  fmtNum(maxTok),    nil),
            ])
        banner.autoresizingMask = [.width, .minYMargin]
        container.addSubview(banner)

        // Calendar heat map — sits between banner and scroll view
        let heatMapH: CGFloat = journal.isEmpty ? 0 : 130
        let heatMap = CalendarHeatMapView(frame: NSRect(
            x: 24, y: frame.height - bannerH - heatMapH,
            width: frame.width - 48, height: heatMapH))
        heatMap.autoresizingMask = [.width, .minYMargin]
        if !journal.isEmpty {
            heatMap.entries = journal.compactMap { entry -> (date: Date, tokens: Int)? in
                guard let d = isoDate(entry.timestamp) else { return nil }
                return (date: d, tokens: entry.tokensUsed)
            }
            container.addSubview(heatMap)
        }

        let (sv, tv) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: frame.width, height: frame.height - bannerH - heatMapH))
        sv.autoresizingMask = [.width, .height]
        container.addSubview(sv)

        let rt = RichText()
        rt.header("Dream Journal  (\(journal.count) cycles)")
        rt.spacer()

        if journal.isEmpty {
            rt.dim("No journal entries yet.")
            tv.textStorage?.setAttributedString(rt.build())
            return container
        }

        let spark = fmtSparkline(allTok)
        if !spark.isEmpty {
            rt.raw(NSAttributedString(string: "Token usage:  \(spark)  (oldest → newest)\n", attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                .foregroundColor: NSColor.secondaryLabelColor]))
            rt.spacer()
        }

        let maxTokD = Double(maxTok > 0 ? maxTok : 1)
        for entry in journal.reversed() {
            rt.divider()
            rt.subheader("\(fmtDate(entry.timestamp))  ·  \(timeAgo(entry.timestamp))")
            let parts = [
                entry.sessionsAnalyzed  > 0 ? "\(entry.sessionsAnalyzed) sessions"  : nil,
                entry.patternsExtracted > 0 ? "\(entry.patternsExtracted) patterns"  : nil,
                entry.associationsFound > 0 ? "\(entry.associationsFound) assoc"     : nil,
                entry.insightsPromoted  > 0 ? "\(entry.insightsPromoted) insights"   : nil,
            ].compactMap { $0 }
            rt.body("  " + (parts.isEmpty ? "skipped — no sessions to analyze" : parts.joined(separator: "  ·  ")))
            let frac   = Double(entry.tokensUsed) / maxTokD
            let barLen = max(1, Int(frac * 36))
            let barStr = String(repeating: "█", count: barLen) + String(repeating: "░", count: 36 - barLen)
            let tokCol: NSColor = frac > 0.8 ? .systemOrange : frac > 0.4 ? .systemBlue : .secondaryLabelColor
            rt.raw(NSAttributedString(string: "  \(barStr)  \(fmtNum(entry.tokensUsed)) tokens\n", attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 10.5, weight: .regular),
                .foregroundColor: tokCol]))
            rt.spacer()
        }

        tv.textStorage?.setAttributedString(rt.build())
        return container
    }

    // ── Tab 4: Insights ────────────────────────────────────────────────────────

    private func buildInsightsView(frame: NSRect) -> NSView {
        let bannerH: CGFloat = 40
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        let raw = readAllInsights() ?? ""
        let blocks = raw.components(separatedBy: "\n").reduce(into: [(header: String, lines: [String])]()) { acc, line in
            if line.hasPrefix("### Insight") { acc.append((header: line, lines: [])) }
            else if !acc.isEmpty { acc[acc.count - 1].lines.append(line) }
        }
        let hiBlock  = blocks.filter { b -> Bool in
            guard let r = b.header.range(of: #"conf=([0-9.]+)"#, options: .regularExpression) else { return false }
            return (Double(b.header[r].replacingOccurrences(of: "conf=", with: "")) ?? 0) >= 0.8
        }.count
        let avgConfI: Double = {
            let vals = blocks.compactMap { b -> Double? in
                guard let r = b.header.range(of: #"conf=([0-9.]+)"#, options: .regularExpression) else { return nil }
                return Double(b.header[r].replacingOccurrences(of: "conf=", with: ""))
            }
            return vals.isEmpty ? 0.0 : vals.reduce(0, +) / Double(vals.count)
        }()

        // Read existing feedback ratings
        let feedback = readDashboardInsightFeedback()
        let ratedCount = feedback.count

        let banner = makeStatsBanner(
            frame: NSRect(x: 0, y: frame.height - bannerH, width: frame.width, height: bannerH),
            stats: [
                ("Insights",  "\(blocks.count)",  nil),
                ("High conf", "\(hiBlock)",        hiBlock > 0 ? .systemGreen : nil),
                ("Avg conf",  String(format: "%.0f%%", avgConfI * 100), nil),
                ("Rated",     "\(ratedCount)",     ratedCount > 0 ? .systemBlue : nil),
            ])
        banner.autoresizingMask = [.width, .minYMargin]
        container.addSubview(banner)

        let (sv, tv) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: frame.width, height: frame.height - bannerH))
        sv.autoresizingMask = [.width, .height]
        container.addSubview(sv)
        insightsTextView = tv

        let rt = RichText()
        rt.header("Insights")
        rt.spacer()

        guard !raw.isEmpty else {
            rt.dim("No insights yet — run a few dream cycles first.")
            tv.textStorage?.setAttributedString(rt.build())
            return container
        }

        rt.dim("\(blocks.count) insight\(blocks.count == 1 ? "" : "s") on record  ·  \(hiBlock) high-confidence (≥80%)\(ratedCount > 0 ? "  ·  \(ratedCount) rated" : "")")
        rt.spacer()

        var insightTextsMap: [String: String] = [:]
        let reversed = blocks.reversed()
        for (displayIdx, block) in reversed.enumerated() {
            // Derive a stable insight ID from header content (hash of the header text)
            let insightId = stableInsightId(block.header)
            let existingRating = feedback[insightId]
            insightTextsMap[insightId] = ([block.header] + block.lines).joined(separator: "\n")

            var conf = 0.70
            if let r = block.header.range(of: #"conf=([0-9.]+)"#, options: .regularExpression) {
                conf = Double(block.header[r].replacingOccurrences(of: "conf=", with: "")) ?? 0.70
            }
            let color: NSColor = conf >= 0.85 ? .systemGreen : conf >= 0.65 ? .systemBlue : .secondaryLabelColor
            let confPctI = Int(conf * 100)
            let confFilled = String(repeating: "▮", count: confPctI / 10)
            let confEmpty  = String(repeating: "░", count: 10 - confPctI / 10)

            // Index number in light yellow + confidence bar
            let indexNum = displayIdx + 1
            let headerLine = NSMutableAttributedString()
            headerLine.append(NSAttributedString(string: "#\(indexNum) ", attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .bold),
                .foregroundColor: NSColor.systemYellow.withAlphaComponent(0.85)]))
            headerLine.append(NSAttributedString(string: "\(confFilled)\(confEmpty) \(confPctI)%", attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .semibold),
                .foregroundColor: color]))

            // Rate insight: thumbs up/down buttons
            let upColor: NSColor = existingRating == "up" ? .systemGreen : .tertiaryLabelColor
            let downColor: NSColor = existingRating == "down" ? .systemRed : .tertiaryLabelColor
            headerLine.append(NSAttributedString(string: "    ", attributes: [
                .font: NSFont.systemFont(ofSize: 11)]))
            headerLine.append(NSAttributedString(string: "👍", attributes: [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: upColor,
                .link: "insight-up:\(insightId)" as NSString,
                .cursor: NSCursor.pointingHand]))
            headerLine.append(NSAttributedString(string: " ", attributes: [
                .font: NSFont.systemFont(ofSize: 11)]))
            headerLine.append(NSAttributedString(string: "👎", attributes: [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: downColor,
                .link: "insight-down:\(insightId)" as NSString,
                .cursor: NSCursor.pointingHand]))
            headerLine.append(NSAttributedString(string: "  ", attributes: [
                .font: NSFont.systemFont(ofSize: 11)]))
            headerLine.append(NSAttributedString(string: "📋", attributes: [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: NSColor.tertiaryLabelColor,
                .link: "insight-copy:\(insightId)" as NSString,
                .cursor: NSCursor.pointingHand]))
            headerLine.append(NSAttributedString(string: "\n", attributes: [
                .font: NSFont.systemFont(ofSize: 11)]))
            rt.raw(headerLine)

            for line in block.lines {
                if line == "---" || line.isEmpty { continue }
                // Strip blockquote prefix for body text
                let stripped = line.hasPrefix("> ") ? String(line.dropFirst(2)) : line
                // Determine role-based styling
                let isRule    = stripped.hasPrefix("**Rule:")
                let isPattern = stripped.hasPrefix("_Patterns:") || stripped.hasPrefix("_Pattern:")
                let baseFont: NSFont
                let baseColor: NSColor
                if isPattern {
                    baseFont  = NSFont.monospacedSystemFont(ofSize: 10, weight: .regular)
                    baseColor = NSColor.tertiaryLabelColor
                } else if isRule {
                    baseFont  = NSFont.systemFont(ofSize: 13, weight: .medium)
                    baseColor = color
                } else {
                    baseFont  = NSFont.systemFont(ofSize: 13)
                    baseColor = color
                }
                // Parse inline markdown: **bold** and _italic_
                let result = NSMutableAttributedString()
                var cursor = stripped.startIndex
                while cursor < stripped.endIndex {
                    // Check for **bold**
                    if stripped[cursor...].hasPrefix("**") {
                        let afterOpen = stripped.index(cursor, offsetBy: 2)
                        if let closeRange = stripped.range(of: "**", range: afterOpen..<stripped.endIndex) {
                            let boldText = String(stripped[afterOpen..<closeRange.lowerBound])
                            let bFont = NSFont.systemFont(ofSize: baseFont.pointSize, weight: .bold)
                            result.append(NSAttributedString(string: boldText, attributes: [
                                .font: bFont, .foregroundColor: baseColor]))
                            cursor = closeRange.upperBound
                            continue
                        }
                    }
                    // Check for _italic_ (but not inside identifiers/paths)
                    if stripped[cursor] == "_" && (cursor == stripped.startIndex || stripped[stripped.index(before: cursor)] == " ") {
                        let afterOpen = stripped.index(after: cursor)
                        if afterOpen < stripped.endIndex,
                           let closeIdx = stripped[afterOpen...].firstIndex(of: "_"),
                           closeIdx > afterOpen {
                            let italicText = String(stripped[afterOpen..<closeIdx])
                            let desc = baseFont.fontDescriptor.withSymbolicTraits(.italic)
                            let iFont = NSFont(descriptor: desc, size: baseFont.pointSize) ?? baseFont
                            result.append(NSAttributedString(string: italicText, attributes: [
                                .font: iFont, .foregroundColor: baseColor]))
                            cursor = stripped.index(after: closeIdx)
                            continue
                        }
                    }
                    // Plain character
                    result.append(NSAttributedString(string: String(stripped[cursor]), attributes: [
                        .font: baseFont, .foregroundColor: baseColor]))
                    cursor = stripped.index(after: cursor)
                }
                result.append(NSAttributedString(string: "\n", attributes: [
                    .font: baseFont, .foregroundColor: baseColor]))
                rt.raw(result)
            }
            rt.spacer()
        }

        tv.textStorage?.setAttributedString(rt.build())

        // Wire up feedback delegate for rating clicks + clipboard
        insightFeedbackDelegate = InsightFeedbackDelegate { [weak self] insightId, rating in
            self?.recordDashboardInsightFeedback(insightId: insightId, rating: rating)
            // Rebuild the insights view to reflect updated rating state
            DispatchQueue.main.async {
                self?.refreshInsightsView()
            }
        }
        insightFeedbackDelegate?.insightTexts = insightTextsMap
        tv.delegate = insightFeedbackDelegate
        tv.linkTextAttributes = [.foregroundColor: NSColor.labelColor, .underlineStyle: 0]

        return container
    }

    /// Generate a stable ID for an insight from its header line.
    private func stableInsightId(_ header: String) -> String {
        // Use a simple hash of the header content for stability across rebuilds
        let hash = header.utf8.reduce(0) { ($0 &* 31) &+ UInt64($1) }
        return String(format: "%016llx", hash)
    }

    /// Read insight feedback from dreams/insight-feedback.jsonl.
    private func readDashboardInsightFeedback() -> [String: String] {
        let path = subDir + "/dreams/insight-feedback.jsonl"
        guard let raw = try? String(contentsOfFile: path, encoding: .utf8) else { return [:] }
        var result: [String: String] = [:]
        for line in raw.components(separatedBy: "\n") where !line.isEmpty {
            guard let data = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let id = obj["insight_id"] as? String,
                  let rating = obj["rating"] as? String
            else { continue }
            result[id] = rating
        }
        return result
    }

    /// Record a feedback action to dreams/insight-feedback.jsonl.
    private func recordDashboardInsightFeedback(insightId: String, rating: String) {
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
    }

    /// Refresh the insights tab content after a feedback action.
    private func refreshInsightsView() {
        guard insightsTextView != nil else { return }
        // Find the insights tab index (tab 4)
        if contentViews.count > 4 {
            let parent = contentViews[4].superview ?? contentViews[4]
            let newView = buildInsightsView(frame: parent.bounds)
            newView.frame = contentViews[4].frame
            newView.autoresizingMask = contentViews[4].autoresizingMask
            contentViews[4].superview?.replaceSubview(contentViews[4], with: newView)
            contentViews[4] = newView
        }
    }

    // ── Tab 5: Metacog ─────────────────────────────────────────────────────────

    private func buildMetacogView(frame: NSRect) -> NSView {
        let bannerH: CGFloat = 40
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        let (audit, filename) = readLatestAudit()
        let latestRaw = readLatestAuditRaw()
        let calScore  = audit?.calibrationScore ?? 0.0
        let over      = audit?.overconfidentCount  ?? 0
        let under     = audit?.underconfidentCount ?? 0
        let well      = audit?.wellCalibratedCount ?? 0
        let total     = over + under + well
        let biasCount = audit?.biasesDetected?.count ?? 0

        let calColor: NSColor = calScore >= 0.8 ? .systemGreen : calScore >= 0.5 ? .systemOrange : .systemRed
        let banner = makeStatsBanner(
            frame: NSRect(x: 0, y: frame.height - bannerH, width: frame.width, height: bannerH),
            stats: [
                ("Calibration", audit != nil ? String(format: "%.2f", calScore) : "—", audit != nil ? calColor : nil),
                ("Samples",     "\(total)",    nil),
                ("Well-cal",    "\(well)",     well > 0 ? .systemGreen : nil),
                ("Overconf",    "\(over)",     over > 0 ? .systemOrange : nil),
                ("Biases",      "\(biasCount)", biasCount > 0 ? .systemOrange : nil),
            ])
        banner.autoresizingMask = [.width, .minYMargin]
        container.addSubview(banner)

        let (sv, tv) = makeScrollableTextView(
            frame: NSRect(x: 0, y: 0, width: frame.width, height: frame.height - bannerH))
        sv.autoresizingMask = [.width, .height]
        container.addSubview(sv)

        let rt = RichText()
        rt.header("Metacognition")
        rt.spacer()

        // ── How It Works explainer ──
        rt.subheader("How Metacognition Works")
        rt.spacer()
        let diagramColor = NSColor.systemCyan.withAlphaComponent(0.7)
        let diagram = """
          ┌─────────────────┐     ┌──────────────┐     ┌──────────────────┐
          │  Claude Session │────▶│  PostToolUse  │────▶│  Activity Store  │
          │  (your work)    │     │  Hook (25%)   │     │  activity.jsonl  │
          └─────────────────┘     └──────────────┘     └────────┬─────────┘
                                                                │
                  ┌─────────────────────────────────────────────┘
                  ▼
          ┌──────────────────┐     ┌──────────────┐     ┌──────────────────┐
          │  Consolidation   │────▶│  Metacog LLM  │────▶│  Audit Report   │
          │  Cycle (idle 4h) │     │  Analysis     │     │  calibration,   │
          └──────────────────┘     └──────────────┘     │  biases, recs   │
                                                         └──────────────────┘
        """
        rt.raw(NSAttributedString(string: diagram + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
            .foregroundColor: diagramColor]))
        rt.dim("  Every 4th tool call is sampled. During idle consolidation, an LLM")
        rt.dim("  audits recent samples for calibration quality, biases, and patterns.")
        rt.dim("  The calibration score measures how well Claude's confidence predictions")
        rt.dim("  match actual outcomes (1.0 = perfect, <0.5 = significant miscalibration).")
        rt.spacer()

        if let audit = audit {
            var dateStr = "—"
            if let fn = filename {
                let parts = fn.components(separatedBy: "-")
                if parts.count >= 2 {
                    let df = DateFormatter(); df.dateFormat = "yyyyMMdd HHmm"
                    if let d = df.date(from: "\(parts[0]) \(parts[1])") {
                        dateStr = fmtDateWithAge(ISO8601DateFormatter().string(from: d))
                    }
                }
            }
            rt.subheader("Latest Audit")
            rt.dim("  From: \(dateStr)")

            // Show extended audit metadata when available
            if let raw = latestRaw {
                var metaDetails: [String] = []
                if let units = raw["units_analyzed"] as? Int, let unitTotal = raw["units_total"] as? Int {
                    metaDetails.append("  Analyzed \(units) of \(unitTotal) sampled units")
                }
                if let tokens = raw["tokens_used"] as? Int {
                    metaDetails.append("  Tokens used: \(tokens)")
                }
                if let sessions = raw["sessions"] as? [[Any]] {
                    metaDetails.append("  Sessions covered: \(sessions.count)")
                }
                for detail in metaDetails { rt.dim(detail) }
            }
            rt.spacer()

            if let score = audit.calibrationScore {
                let label = score >= 0.8 ? "well-calibrated" : score >= 0.5 ? "moderate"
                          : score >= 0.2 ? "under-calibrated" : "poor"
                rt.raw(NSAttributedString(
                    string: String(format: "  Calibration   %.2f / 1.00  (%@)\n", score, label),
                    attributes: [.font: NSFont.systemFont(ofSize: 13), .foregroundColor: calColor]))
                rt.dim("  1.0 = predictions match outcomes perfectly")
                rt.spacer()
            }

            if total > 0 {
                rt.subheader("Sample Breakdown  (\(total) samples)")
                func pct(_ n: Int) -> String { String(format: "%d%%", n * 100 / max(1, total)) }
                // Visual bar for each category
                let barWidth = 20
                let wellBar  = String(repeating: "▮", count: well * barWidth / max(1, total))
                let overBar  = String(repeating: "▮", count: over * barWidth / max(1, total))
                let underBar = String(repeating: "▮", count: under * barWidth / max(1, total))
                rt.raw(NSAttributedString(string: "  ✓ Well-calibrated   \(wellBar) \(well)  (\(pct(well)))\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
                    .foregroundColor: NSColor.systemGreen]))
                rt.raw(NSAttributedString(string: "  ↑ Overconfident     \(overBar) \(over)  (\(pct(over)))\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
                    .foregroundColor: NSColor.systemOrange]))
                rt.raw(NSAttributedString(string: "  ↓ Underconfident    \(underBar) \(under)  (\(pct(under)))\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
                    .foregroundColor: NSColor.systemBlue]))
                rt.spacer()
            }

            if let biases = audit.biasesDetected, !biases.isEmpty {
                rt.subheader("Biases Detected  (\(biases.count))")
                for bias in biases {
                    // Format bias names: replace underscores with spaces, title case
                    let formatted = bias.replacingOccurrences(of: "_", with: " ")
                    rt.raw(NSAttributedString(string: "  ⚠ \(formatted)\n", attributes: [
                        .font: NSFont.systemFont(ofSize: 12, weight: .medium),
                        .foregroundColor: NSColor.systemOrange]))
                }
                rt.spacer()
            }

            if let recs = audit.recommendations, !recs.isEmpty {
                rt.subheader("Recommendations")
                recs.enumerated().forEach { i, r in rt.body("  \(i + 1). \(r)") }
                rt.spacer()
            }
        } else {
            rt.dim("No metacog audit data found yet.")
            rt.body("Audits are created during background consolidation cycles.")
            rt.spacer()
        }

        // ── Calibration trend ──
        let calPath = subDir + "/metacog/calibration.jsonl"
        if let calContent = try? String(contentsOfFile: calPath, encoding: .utf8) {
            let scores: [Double] = calContent.components(separatedBy: "\n").filter { !$0.isEmpty }
                .compactMap { line -> Double? in
                    guard let d = line.data(using: .utf8),
                          let j = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
                          let s = j["calibration_score"] as? Double else { return nil }
                    return s
                }
            if scores.count >= 2 {
                rt.subheader("Calibration Trend  (last \(min(scores.count, 10)) cycles)")
                let window    = Array(scores.suffix(10))
                let sparkVals = window.map { Int($0 * 10) }
                let avg       = window.reduce(0, +) / Double(window.count)
                rt.mono("  \(fmtSparkline(sparkVals, width: 10))  avg \(String(format: "%.2f", avg))")
                let trend    = (scores.last ?? 0) - (scores.first ?? 0)
                let trendStr = trend > 0.05 ? "↑ improving" : trend < -0.05 ? "↓ declining" : "→ stable"
                rt.dim("  Overall trend: \(trendStr)")
                rt.spacer()
            }
        }

        // ── Audit history ──
        let auditHistory = readAuditHistory(limit: 6)
        if auditHistory.count >= 2 {
            rt.subheader("Audit History  (last \(auditHistory.count))")
            for entry in auditHistory {
                let scoreStr = String(format: "%.2f", entry.score)
                let hColor: NSColor = entry.score >= 0.8 ? .systemGreen
                                    : entry.score >= 0.5 ? .systemOrange : .systemRed
                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  \(entry.dateLabel)  ", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                    .foregroundColor: NSColor.secondaryLabelColor]))
                line.append(NSAttributedString(string: scoreStr, attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .semibold),
                    .foregroundColor: hColor]))
                line.append(NSAttributedString(string: "  \(entry.biases)b  \(entry.samples)s\n", attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                    .foregroundColor: NSColor.tertiaryLabelColor]))
                rt.raw(line)
            }
            rt.spacer()
        }

        // ── Activity stats ──
        let actPath = subDir + "/metacog/activity.jsonl"
        if let actContent = try? String(contentsOfFile: actPath, encoding: .utf8) {
            let lines = actContent.components(separatedBy: "\n").filter { !$0.isEmpty }
            let cnt = lines.count
            rt.subheader("Activity Samples")
            rt.body("  \(cnt) tool-call samples recorded")
            rt.dim("  Sampled at 25% of PostToolUse events")

            // Show top tools if parseable
            var toolCounts: [String: Int] = [:]
            for line in lines.suffix(500) {
                guard let d = line.data(using: .utf8),
                      let j = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
                      let tool = j["tool"] as? String else { continue }
                toolCounts[tool, default: 0] += 1
            }
            if !toolCounts.isEmpty {
                let top = toolCounts.sorted { $0.value > $1.value }.prefix(5)
                rt.dim("  Top tools (recent 500 samples):")
                for (tool, count) in top {
                    rt.dim("    \(count)×  \(tool)")
                }
            }
        }

        tv.textStorage?.setAttributedString(rt.build())
        return container
    }

    /// Read raw JSON of the latest audit for extended metadata.
    private func readLatestAuditRaw() -> [String: Any]? {
        let auditsDir = subDir + "/metacog/audits"
        guard let files = try? FileManager.default.contentsOfDirectory(atPath: auditsDir),
              let latest = files.filter({ $0.hasSuffix(".json") }).sorted().last,
              let data = try? Data(contentsOf: URL(fileURLWithPath: auditsDir + "/" + latest)),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return obj
    }

    private struct AuditHistoryEntry {
        let dateLabel: String
        let score: Double
        let biases: Int
        let samples: Int
    }

    /// Read recent audit files and extract summary info for the history view.
    private func readAuditHistory(limit: Int) -> [AuditHistoryEntry] {
        let auditsDir = subDir + "/metacog/audits"
        guard let files = try? FileManager.default.contentsOfDirectory(atPath: auditsDir) else { return [] }
        let sorted = files.filter { $0.hasSuffix(".json") }.sorted().suffix(limit)
        return sorted.compactMap { fn -> AuditHistoryEntry? in
            let path = auditsDir + "/" + fn
            guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return nil }

            // Parse the response JSON to get the audit fields
            var score = 0.0
            var biases = 0
            var samples = 0
            if let response = obj["response"] as? String {
                let stripped = response
                    .replacingOccurrences(of: "```json\n", with: "")
                    .replacingOccurrences(of: "```json",   with: "")
                    .replacingOccurrences(of: "\n```",     with: "")
                    .replacingOccurrences(of: "```",       with: "")
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                if let inner = stripped.data(using: .utf8),
                   let audit = try? JSONSerialization.jsonObject(with: inner) as? [String: Any] {
                    score = audit["calibration_score"] as? Double ?? 0
                    biases = (audit["biases_detected"] as? [Any])?.count ?? 0
                    let w = audit["well_calibrated_count"] as? Int ?? 0
                    let o = audit["overconfident_count"] as? Int ?? 0
                    let u = audit["underconfident_count"] as? Int ?? 0
                    samples = w + o + u
                }
            }

            // Extract date from filename: "20260419-0310-audit.json"
            let parts = fn.components(separatedBy: "-")
            var dateLabel = fn
            if parts.count >= 2 {
                let df = DateFormatter(); df.dateFormat = "yyyyMMdd HHmm"
                let outFmt = DateFormatter(); outFmt.dateFormat = "MMM dd HH:mm"
                if let d = df.date(from: "\(parts[0]) \(parts[1])") {
                    dateLabel = outFmt.string(from: d)
                }
            }
            return AuditHistoryEntry(dateLabel: dateLabel, score: score, biases: biases, samples: samples)
        }
    }

    // ── Tab 6: Search ──────────────────────────────────────────────────────────

    private func buildSearchView(frame: NSRect) -> NSView {
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]

        // Search field at top
        let fieldH: CGFloat = 32
        let pad: CGFloat = 16
        let tagBarH: CGFloat = 30
        let sf = NSSearchField(frame: NSRect(x: pad, y: frame.height - fieldH - pad,
                                              width: frame.width - pad * 2, height: fieldH))
        sf.autoresizingMask = [.width, .minYMargin]
        sf.placeholderString = "Search — supports multiple words (fuzzy), e.g. \"retry tool\""
        sf.font = .systemFont(ofSize: 13)
        sf.target = self
        sf.action = #selector(searchChanged(_:))
        sf.sendsSearchStringImmediately = true
        container.addSubview(sf)
        searchField = sf

        // Quick-filter tag bar
        let tagBar = NSView(frame: NSRect(x: 0, y: frame.height - fieldH - pad - tagBarH - 4,
                                           width: frame.width, height: tagBarH))
        tagBar.autoresizingMask = [.width, .minYMargin]
        let categories = Set(patterns.map { $0.category }).sorted()
        var tagX: CGFloat = pad
        for cat in categories.prefix(12) {
            let tag = NSButton(frame: NSRect(x: tagX, y: 4, width: 0, height: 22))
            tag.title = cat
            tag.bezelStyle = .inline
            tag.font = .systemFont(ofSize: 10, weight: .medium)
            tag.contentTintColor = .systemTeal
            tag.target = self
            tag.action = #selector(searchTagClicked(_:))
            tag.sizeToFit()
            tag.frame.size.width += 12
            tagBar.addSubview(tag)
            tagX += tag.frame.width + 6
        }
        container.addSubview(tagBar)

        // Results area below
        let topUsed = fieldH + pad + tagBarH + 8
        let resultsFrame = NSRect(x: 0, y: 0,
                                   width: frame.width,
                                   height: frame.height - topUsed)
        let (sv, tv) = makeScrollableTextView(frame: resultsFrame)
        sv.autoresizingMask = [.width, .height]
        container.addSubview(sv)
        searchResultsTextView = tv

        // Wire up link delegate for clickable search results
        searchLinkDelegate = JournalLinkDelegate { [weak self] link in
            if link.hasPrefix("pattern:") {
                self?.selectTab(1) // Navigate to Patterns tab
            } else if link.hasPrefix("assoc:") {
                self?.selectTab(2) // Navigate to Associations tab
            } else if link.hasPrefix("insight:") {
                self?.selectTab(4) // Navigate to Insights tab
            } else if link.hasPrefix("metacog:") {
                self?.selectTab(5) // Navigate to Metacog tab
            }
        }
        tv.delegate = searchLinkDelegate
        tv.linkTextAttributes = [.foregroundColor: NSColor.labelColor, .underlineStyle: 0]

        // Show initial placeholder with stats
        renderSearchPlaceholder(tv)
        return container
    }

    private func renderSearchPlaceholder(_ tv: NSTextView) {
        let rt = RichText()
        rt.spacer()
        rt.subheader("  Search i-dream Knowledge Base")
        rt.spacer()
        rt.body("  Type to search across all data. Multiple words are matched independently")
        rt.body("  (fuzzy): \"retry tool\" matches items containing both \"retry\" AND \"tool\".")
        rt.spacer()
        rt.dim("  ┌─ Data Sources ────────────────────────────────────────────┐")
        rt.dim("  │  Patterns        \(patterns.count) items    pattern text, category, valence  │")
        rt.dim("  │  Associations    \(associations.count) items    hypotheses, suggested rules     │")
        rt.dim("  │  Insights        full text     insight blocks with context     │")
        rt.dim("  │  Metacog         latest audit  biases and recommendations      │")
        rt.dim("  └──────────────────────────────────────────────────────────────┘")
        rt.spacer()
        rt.dim("  Click a category tag above for quick filtering.")
        rt.dim("  Use quotes for exact phrases (not yet supported — planned for V2).")
        tv.textStorage?.setAttributedString(rt.build())
    }

    @objc private func searchTagClicked(_ sender: NSButton) {
        searchField?.stringValue = sender.title
        if let sf = searchField { searchChanged(sf) }
    }

    /// Fuzzy match: returns true if ALL words in `queryWords` appear in `text`.
    private func fuzzyMatch(_ text: String, queryWords: [String]) -> Bool {
        let lower = text.lowercased()
        return queryWords.allSatisfy { lower.contains($0) }
    }

    /// Compute a relevance score: higher = better match. Rewards exact substring,
    /// word-boundary matches, and early position.
    private func relevanceScore(_ text: String, queryWords: [String], fullQuery: String) -> Int {
        let lower = text.lowercased()
        var score = 0
        // Exact full query match bonus
        if lower.contains(fullQuery) { score += 100 }
        // Word-boundary bonus for each word
        for w in queryWords {
            if lower.hasPrefix(w) { score += 20 }
            if lower.contains(" \(w)") { score += 10 }
            if lower.contains(w) { score += 5 }
        }
        return score
    }

    @objc private func searchChanged(_ sender: NSSearchField) {
        let rawQuery = sender.stringValue.trimmingCharacters(in: .whitespaces)
        guard let tv = searchResultsTextView else { return }

        if rawQuery.isEmpty {
            searchDebounceTimer?.invalidate()
            renderSearchPlaceholder(tv)
            return
        }

        // Debounce: wait 150ms after last keystroke before executing search
        searchDebounceTimer?.invalidate()
        searchDebounceTimer = Timer.scheduledTimer(withTimeInterval: 0.15, repeats: false) { [weak self] _ in
            self?.performSearch(rawQuery)
        }
    }

    private func performSearch(_ rawQuery: String) {
        guard let tv = searchResultsTextView else { return }
        let query = rawQuery.lowercased()
        let queryWords = query.components(separatedBy: .whitespaces).filter { !$0.isEmpty }
        let rt = RichText()
        var totalHits = 0

        // ── Patterns ────────────────────────────────────────────────────────
        let matchedPatterns = patterns.enumerated().filter { (_, p) in
            let searchable = "\(p.pattern) \(p.category) \(p.valence) \(p.id ?? "")"
            return fuzzyMatch(searchable, queryWords: queryWords)
        }.sorted { (a, b) in
            let scoreA = relevanceScore(a.element.pattern, queryWords: queryWords, fullQuery: query)
            let scoreB = relevanceScore(b.element.pattern, queryWords: queryWords, fullQuery: query)
            return scoreA > scoreB
        }
        if !matchedPatterns.isEmpty {
            rt.raw(sectionHeader("Patterns", count: matchedPatterns.count, icon: "◆", color: .systemTeal))
            for (_, p) in matchedPatterns.prefix(30) {
                let confPct = Int(p.confidence * 100)
                let valColor: NSColor = p.valence == "positive" ? .systemGreen
                    : p.valence == "negative" ? .systemRed : .secondaryLabelColor
                let valIcon = p.valence == "positive" ? "▲" : p.valence == "negative" ? "▼" : "●"
                // Tag pills: [category] [valence] [confidence]
                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  \(valIcon) ", attributes: [
                    .font: NSFont.systemFont(ofSize: 12, weight: .bold), .foregroundColor: valColor]))
                line.append(tagPill(p.category, color: .systemTeal))
                line.append(NSAttributedString(string: " "))
                line.append(tagPill(p.valence, color: valColor))
                line.append(NSAttributedString(string: " "))
                line.append(tagPill("\(confPct)%", color: confPct >= 80 ? .systemGreen : confPct >= 60 ? .systemBlue : .secondaryLabelColor))
                line.append(NSAttributedString(string: "\n"))
                rt.raw(line)
                // Full pattern text with highlight — clickable link to Patterns tab
                let patKey = p.stableKey
                let highlighted = highlightQuery(in: p.pattern, queryWords: queryWords,
                                                  baseFont: .systemFont(ofSize: 13),
                                                  baseColor: .labelColor)
                let indented = NSMutableAttributedString(string: "     ")
                indented.append(highlighted)
                // Add link attribute across the whole text range (excluding indent)
                indented.addAttributes([
                    .link: "pattern:\(patKey)" as NSString,
                    .cursor: NSCursor.pointingHand,
                ], range: NSRange(location: 5, length: indented.length - 5))
                indented.append(NSAttributedString(string: "  → ", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: NSColor.systemTeal.withAlphaComponent(0.6)]))
                indented.append(NSAttributedString(string: "view", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: NSColor.systemTeal.withAlphaComponent(0.6),
                    .link: "pattern:\(patKey)" as NSString,
                    .cursor: NSCursor.pointingHand]))
                indented.append(NSAttributedString(string: "\n"))
                rt.raw(indented)
                // Date if available
                if let fs = p.firstSeen, !fs.isEmpty {
                    rt.raw(NSAttributedString(string: "     First seen: \(fmtDate(fs))\n", attributes: [
                        .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.tertiaryLabelColor]))
                }
                rt.raw(NSAttributedString(string: "\n"))
            }
            if matchedPatterns.count > 30 {
                rt.dim("    … and \(matchedPatterns.count - 30) more patterns")
            }
            totalHits += matchedPatterns.count
            rt.spacer()
        }

        // ── Associations ────────────────────────────────────────────────────
        let matchedAssocs = associations.enumerated().filter { (_, a) in
            let searchable = "\(a.hypothesis) \(a.suggestedRule ?? "") \(a.id)"
            return fuzzyMatch(searchable, queryWords: queryWords)
        }.sorted { (a, b) in
            let scoreA = relevanceScore(a.element.hypothesis, queryWords: queryWords, fullQuery: query)
            let scoreB = relevanceScore(b.element.hypothesis, queryWords: queryWords, fullQuery: query)
            return scoreA > scoreB
        }
        if !matchedAssocs.isEmpty {
            rt.raw(sectionHeader("Associations", count: matchedAssocs.count, icon: "◇", color: .systemOrange))
            for (_, a) in matchedAssocs.prefix(30) {
                let confPct = Int(a.confidence * 100)
                let line = NSMutableAttributedString()
                line.append(NSAttributedString(string: "  ", attributes: [:]))
                if a.actionable {
                    line.append(tagPill("actionable", color: .systemYellow))
                    line.append(NSAttributedString(string: " "))
                }
                line.append(tagPill("\(confPct)%", color: confPct >= 80 ? .systemGreen : confPct >= 60 ? .systemBlue : .secondaryLabelColor))
                line.append(NSAttributedString(string: "\n"))
                rt.raw(line)
                // Hypothesis with highlight — clickable link to Associations tab
                let highlighted = highlightQuery(in: a.hypothesis, queryWords: queryWords,
                                                  baseFont: .systemFont(ofSize: 13),
                                                  baseColor: .labelColor)
                let indented = NSMutableAttributedString(string: "     ")
                indented.append(highlighted)
                indented.addAttributes([
                    .link: "assoc:\(a.id)" as NSString,
                    .cursor: NSCursor.pointingHand,
                ], range: NSRange(location: 5, length: indented.length - 5))
                indented.append(NSAttributedString(string: "  ��� ", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: NSColor.systemOrange.withAlphaComponent(0.6)]))
                indented.append(NSAttributedString(string: "view", attributes: [
                    .font: NSFont.systemFont(ofSize: 10),
                    .foregroundColor: NSColor.systemOrange.withAlphaComponent(0.6),
                    .link: "assoc:\(a.id)" as NSString,
                    .cursor: NSCursor.pointingHand]))
                indented.append(NSAttributedString(string: "\n"))
                rt.raw(indented)
                // Suggested rule if present
                if let rule = a.suggestedRule, !rule.isEmpty {
                    let ruleHl = highlightQuery(in: rule, queryWords: queryWords,
                                                 baseFont: .systemFont(ofSize: 12),
                                                 baseColor: .secondaryLabelColor)
                    let ruleLine = NSMutableAttributedString(string: "     Rule: ", attributes: [
                        .font: NSFont.systemFont(ofSize: 12, weight: .medium),
                        .foregroundColor: NSColor.secondaryLabelColor])
                    ruleLine.append(ruleHl)
                    ruleLine.append(NSAttributedString(string: "\n"))
                    rt.raw(ruleLine)
                }
                rt.raw(NSAttributedString(string: "\n"))
            }
            if matchedAssocs.count > 30 {
                rt.dim("    … and \(matchedAssocs.count - 30) more associations")
            }
            totalHits += matchedAssocs.count
            rt.spacer()
        }

        // ── Insights ────────────────────────────────────────────────────────
        if let raw = readAllInsights() {
            let lines = raw.components(separatedBy: "\n")
            var matchedLines: [(lineNum: Int, text: String)] = []
            for (i, line) in lines.enumerated() {
                if fuzzyMatch(line, queryWords: queryWords) {
                    matchedLines.append((i + 1, line))
                }
            }
            if !matchedLines.isEmpty {
                rt.raw(sectionHeader("Insights", count: matchedLines.count, icon: "✦", color: .systemYellow))
                for hit in matchedLines.prefix(25) {
                    let trimmed = hit.text.trimmingCharacters(in: .whitespaces)
                    if trimmed.isEmpty { continue }
                    let insightLine = NSMutableAttributedString()
                    insightLine.append(NSAttributedString(string: "  L\(hit.lineNum)  ", attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                        .foregroundColor: NSColor.tertiaryLabelColor,
                    ]))
                    let highlighted = highlightQuery(in: trimmed, queryWords: queryWords,
                                                      baseFont: .systemFont(ofSize: 12),
                                                      baseColor: .labelColor)
                    insightLine.append(highlighted)
                    // Make the entire line clickable to navigate to Insights tab
                    insightLine.addAttributes([
                        .link: "insight:L\(hit.lineNum)" as NSString,
                        .cursor: NSCursor.pointingHand,
                    ], range: NSRange(location: 0, length: insightLine.length))
                    insightLine.append(NSAttributedString(string: "\n"))
                    rt.raw(insightLine)
                }
                if matchedLines.count > 25 {
                    rt.dim("    … and \(matchedLines.count - 25) more lines")
                }
                totalHits += matchedLines.count
                rt.spacer()
            }
        }

        // ── Metacog ─────────────────────────────────────────────────────────
        let (audit, auditFile) = readLatestAudit()
        if let audit = audit {
            var metacogHits: [(kind: String, text: String)] = []
            for b in (audit.biasesDetected ?? []) where fuzzyMatch(b, queryWords: queryWords) {
                metacogHits.append(("bias", b))
            }
            for r in (audit.recommendations ?? []) where fuzzyMatch(r, queryWords: queryWords) {
                metacogHits.append(("rec", r))
            }
            if !metacogHits.isEmpty {
                let auditLabel: String = {
                    guard let f = auditFile else { return "latest" }
                    let name = (f as NSString).lastPathComponent
                    return name.replacingOccurrences(of: "-audit.json", with: "")
                }()
                rt.raw(sectionHeader("Metacog (\(auditLabel))", count: metacogHits.count, icon: "⬡", color: .systemPink))
                for hit in metacogHits.prefix(15) {
                    let kindColor: NSColor = hit.kind == "bias" ? .systemOrange : .systemBlue
                    let line = NSMutableAttributedString(string: "  ")
                    line.append(tagPill(hit.kind, color: kindColor))
                    line.append(NSAttributedString(string: " "))
                    let highlighted = highlightQuery(in: hit.text, queryWords: queryWords,
                                                      baseFont: .systemFont(ofSize: 12),
                                                      baseColor: .labelColor)
                    line.append(highlighted)
                    // Make clickable to navigate to Metacog tab
                    line.addAttributes([
                        .link: "metacog:\(hit.kind)" as NSString,
                        .cursor: NSCursor.pointingHand,
                    ], range: NSRange(location: 0, length: line.length))
                    line.append(NSAttributedString(string: "\n\n"))
                    rt.raw(line)
                }
                totalHits += metacogHits.count
                rt.spacer()
            }
        }

        // ── Summary / no results ────────────────────────────────────────────
        if totalHits == 0 {
            rt.spacer()
            rt.dim("  No results for \"\(rawQuery)\"")
            rt.spacer()
            rt.dim("  Tips:")
            rt.dim("    • Try fewer or shorter words")
            rt.dim("    • Click a category tag above to browse by topic")
            rt.dim("    • All words must match (AND logic)")
        } else {
            rt.divider()
            rt.dim("  \(totalHits) result(s) across all categories for \"\(rawQuery)\"")
        }

        tv.textStorage?.setAttributedString(rt.build())
        tv.scrollToBeginningOfDocument(nil)
    }

    /// Styled section header with icon and count.
    private func sectionHeader(_ title: String, count: Int, icon: String, color: NSColor) -> NSAttributedString {
        let result = NSMutableAttributedString()
        result.append(NSAttributedString(string: "  \(icon) ", attributes: [
            .font: NSFont.systemFont(ofSize: 15, weight: .bold), .foregroundColor: color]))
        result.append(NSAttributedString(string: "\(title)  ", attributes: [
            .font: NSFont.systemFont(ofSize: 15, weight: .bold), .foregroundColor: NSColor.labelColor]))
        result.append(NSAttributedString(string: "\(count)\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .medium), .foregroundColor: color]))
        result.append(NSAttributedString(string: "  " + String(repeating: "─", count: 50) + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 10), .foregroundColor: NSColor.separatorColor]))
        return result
    }

    /// Render a small inline tag pill: ┃category┃
    private func tagPill(_ text: String, color: NSColor) -> NSAttributedString {
        NSAttributedString(string: " \(text) ", attributes: [
            .font: NSFont.systemFont(ofSize: 10, weight: .semibold),
            .foregroundColor: color,
            .backgroundColor: color.withAlphaComponent(0.12),
        ])
    }

    /// Highlight occurrences of all `queryWords` in `text` with a yellow background.
    private func highlightQuery(in text: String, queryWords: [String],
                                 baseFont: NSFont, baseColor: NSColor) -> NSAttributedString {
        let result = NSMutableAttributedString(string: text, attributes: [
            .font: baseFont, .foregroundColor: baseColor,
        ])
        let lower = text.lowercased() as NSString
        for word in queryWords {
            var searchStart = 0
            while searchStart < lower.length {
                let range = lower.range(of: word, range: NSRange(location: searchStart,
                                                                   length: lower.length - searchStart))
                if range.location == NSNotFound { break }
                result.addAttribute(.backgroundColor, value: NSColor.systemYellow.withAlphaComponent(0.3),
                                    range: range)
                result.addAttribute(.foregroundColor, value: NSColor.labelColor, range: range)
                searchStart = range.location + range.length
            }
        }
        return result
    }

    // ── Tab 7: Help ────────────────────────────────────────────────────────────

    private func buildHelpView(frame: NSRect) -> NSView {
        let container = NSView(frame: frame)
        container.autoresizingMask = [.width, .height]
        let (sv, tv) = makeScrollableTextView(frame: NSRect(origin: .zero, size: frame.size))
        sv.autoresizingMask = [.width, .height]
        let rt = RichText()

        rt.header("Help & Reference Guide")
        rt.spacer()

        // --- Getting Started ---
        rt.raw(helpSection("Getting Started", icon: "▸", color: .systemGreen))
        rt.body("  i-dream monitors your Claude Code sessions, extracts behavioural")
        rt.body("  patterns, finds cross-session associations, and surfaces insights.")
        rt.body("  The daemon runs automatically in the background. This dashboard")
        rt.body("  gives you a window into what it has learned.")
        rt.spacer()

        // --- Navigation ---
        rt.raw(helpSection("Navigation", icon: "◧", color: .systemPurple))
        rt.raw(helpRow("Sidebar",     "Click any tab to switch views"))
        rt.raw(helpRow("↺ Refresh",   "Reload all data from disk"))
        rt.raw(helpRow("Search tab",  "Full-text fuzzy search across all data"))
        rt.raw(helpRow("Detail pane", "Click a pattern or association to see details below the list"))
        rt.spacer()

        // --- Graph Interactions ---
        rt.raw(helpSection("Graph Interactions", icon: "⬡", color: .systemTeal))
        rt.raw(helpShortcut("Click node",      "Select node, show details in sidebar + popover"))
        rt.raw(helpShortcut("Click list item",  "Cross-highlight the matching graph node"))
        rt.raw(helpShortcut("Drag",             "Pan the graph"))
        rt.raw(helpShortcut("Scroll / Pinch",   "Zoom in and out"))
        rt.raw(helpShortcut("Hover",            "Preview connected edges"))
        rt.raw(helpShortcut("Double-click",     "Reset zoom and pan to default"))
        rt.raw(helpShortcut("Filter field",     "Type to dim non-matching nodes"))
        rt.spacer()

        // --- Pattern Network Legend ---
        rt.raw(helpSection("Pattern Network", icon: "●", color: .systemTeal))
        rt.raw(helpLegend("●", .systemGreen,         "High confidence (≥85%)"))
        rt.raw(helpLegend("●", .systemBlue,           "Medium confidence (≥65%)"))
        rt.raw(helpLegend("●", .secondaryLabelColor,  "Lower confidence (<65%)"))
        rt.raw(helpLegend("▲", .systemGreen,          "Positive valence"))
        rt.raw(helpLegend("▼", .systemOrange,         "Negative valence"))
        rt.body("  Node size scales with confidence. Category labels orbit the ring.")
        rt.body("  Selecting a node dims unrelated nodes and draws connection lines.")
        rt.spacer()

        // --- Association Network Legend ---
        rt.raw(helpSection("Association Network", icon: "◆", color: .systemOrange))
        rt.raw(helpLegend("◆", .systemGreen,         "Actionable, high confidence"))
        rt.raw(helpLegend("◆", .systemBlue,           "Actionable"))
        rt.raw(helpLegend("○", .secondaryLabelColor,  "Non-actionable"))
        rt.body("  Three concentric rings: inner ≥75%, middle ≥50%, outer <50% confidence.")
        rt.body("  Edges connect associations sharing linked patterns; thicker = more overlap.")
        rt.spacer()

        // --- Tab Reference ---
        rt.raw(helpSection("Tab Reference", icon: "▤", color: .systemIndigo))
        rt.raw(helpRow("Overview",     "Dashboard with stats cards, charts, sparklines"))
        rt.raw(helpRow("Patterns",     "Grouped by category, detail pane with linked associations"))
        rt.raw(helpRow("Associations", "Grouped by confidence tier, shows linked pattern text"))
        rt.raw(helpRow("Journal",      "Dream cycle history with token usage bars"))
        rt.raw(helpRow("Insights",     "Full markdown-rendered insight blocks"))
        rt.raw(helpRow("Metacog",      "Calibration scores, biases, recommendations"))
        rt.raw(helpRow("Search",       "Fuzzy multi-word search across all data"))
        rt.spacer()

        // --- Data & CLI ---
        rt.raw(helpSection("Data & Commands", icon: "⌘", color: .systemYellow))
        rt.dim("  Data directory:")
        rt.mono("    ~/.claude/subconscious/")
        rt.spacer()
        rt.dim("  Useful commands:")
        rt.mono("    cargo run -- daemon start     # start the dream daemon")
        rt.mono("    cargo run -- daemon stop      # stop the daemon")
        rt.mono("    cargo run -- daemon status    # check daemon state")
        rt.mono("    cargo run -- dream            # trigger a dream cycle now")
        rt.spacer()
        rt.dim("  Dashboard build:")
        rt.mono("    bash tools/menubar/build.sh            # compile + launch")
        rt.mono("    bash tools/menubar/build.sh --install  # auto-start on login")
        rt.mono("    bash tools/menubar/build.sh --status   # check build staleness")
        rt.spacer()

        tv.textStorage?.setAttributedString(rt.build())
        container.addSubview(sv)
        return container
    }

    // --- Help page rendering helpers ---

    /// Renders a colored section header with icon.
    private func helpSection(_ title: String, icon: String, color: NSColor) -> NSAttributedString {
        let str = NSMutableAttributedString()
        str.append(NSAttributedString(string: "  \(icon) ", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: color]))
        str.append(NSAttributedString(string: title.uppercased(), attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .bold),
            .foregroundColor: color]))
        str.append(NSAttributedString(string: "\n  ", attributes: [
            .font: NSFont.systemFont(ofSize: 4)]))
        // Divider line
        let divLen = max(title.count + 4, 20)
        str.append(NSAttributedString(string: String(repeating: "─", count: divLen) + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 9, weight: .regular),
            .foregroundColor: color.withAlphaComponent(0.3)]))
        return str
    }

    /// Renders a key-value help row: label left-aligned, description right.
    private func helpRow(_ label: String, _ desc: String) -> NSAttributedString {
        let str = NSMutableAttributedString()
        let padded = label.padding(toLength: 16, withPad: " ", startingAt: 0)
        str.append(NSAttributedString(string: "  \(padded)", attributes: [
            .font: NSFont.systemFont(ofSize: 11.5, weight: .medium),
            .foregroundColor: NSColor.labelColor]))
        str.append(NSAttributedString(string: desc + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 11.5),
            .foregroundColor: NSColor.secondaryLabelColor]))
        return str
    }

    /// Renders a keyboard shortcut row.
    private func helpShortcut(_ key: String, _ desc: String) -> NSAttributedString {
        let str = NSMutableAttributedString()
        let padded = key.padding(toLength: 18, withPad: " ", startingAt: 0)
        str.append(NSAttributedString(string: "  \(padded)", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
            .foregroundColor: NSColor.systemTeal]))
        str.append(NSAttributedString(string: desc + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 11),
            .foregroundColor: NSColor.secondaryLabelColor]))
        return str
    }

    /// Renders a colored legend item: symbol in color + description.
    private func helpLegend(_ symbol: String, _ color: NSColor, _ desc: String) -> NSAttributedString {
        let str = NSMutableAttributedString()
        str.append(NSAttributedString(string: "  \(symbol) ", attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: color]))
        str.append(NSAttributedString(string: desc + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 11.5),
            .foregroundColor: NSColor.secondaryLabelColor]))
        return str
    }

    // ── Tab 8: About ───────────────────────────────────────────────────────────

    private func buildAboutView(frame: NSRect) -> NSView {
        let (sv, tv) = makeScrollableTextView(frame: frame)
        let rt = RichText()

        rt.header("About i-dream")
        rt.spacer()
        rt.body("  i-dream is a background cognitive reflection system that analyses your")
        rt.body("  Claude Code sessions overnight, extracts patterns, associations, and insights,")
        rt.body("  and surfaces them here so future sessions can benefit.")
        rt.spacer()

        rt.subheader("Version")
        rt.body("  i-dream            v0.1.0")
        rt.body("  Dashboard widget   v1.0.0")
        // Compute "last updated" from the most recent data file modification
        let dataFiles = [
            subDir + "/dreams/patterns.json",
            subDir + "/dreams/associations.json",
            subDir + "/dreams/journal.json",
            subDir + "/dreams/insights.md",
            subDir + "/dreams/insight-digest.md",
        ]
        let fm = FileManager.default
        let dateFmt = DateFormatter()
        dateFmt.dateFormat = "yyyy-MM-dd HH:mm"
        var latestDate: Date?
        for path in dataFiles {
            if let attrs = try? fm.attributesOfItem(atPath: path),
               let mod = attrs[.modificationDate] as? Date {
                if latestDate == nil || mod > latestDate! { latestDate = mod }
            }
        }
        if let d = latestDate {
            let elapsed = Date().timeIntervalSince(d)
            let ago: String = elapsed < 60 ? "just now"
                : elapsed < 3600 ? "\(Int(elapsed / 60))m ago"
                : elapsed < 86400 ? "\(Int(elapsed / 3600))h ago"
                : "\(Int(elapsed / 86400))d ago"
            rt.body("  Data last updated  \(dateFmt.string(from: d))  (\(ago))")
        } else {
            rt.dim("  Data last updated  —")
        }
        rt.spacer()

        rt.subheader("Build Info")
        rt.body(String(format: "  Commit hash      %@", BuildInfo.commitHash))
        rt.body(String(format: "  Source hash      %@", BuildInfo.sourceHash))
        rt.body(String(format: "  Built at         %@", BuildInfo.builtAt))
        rt.spacer()

        rt.subheader("Daemon Status")
        if let s = state {
            let statusStr = s.totalCycles > 0 ? "running  ·  \(s.totalCycles) cycles completed" : "started  ·  no cycles yet"
            rt.body("  Status           \(statusStr)")
            rt.body("  Last dream       \(fmtDate(s.lastConsolidation))  (\(timeAgo(s.lastConsolidation)))")
            rt.body("  Total tokens     \(fmtNum(s.totalTokensUsed))")
        } else {
            rt.dim("  Daemon state not found — run i-dream daemon to start.")
        }
        rt.spacer()

        rt.subheader("Data Paths")
        let paths: [(String, String)] = [
            ("Root",        subDir),
            ("Patterns",    subDir + "/dreams/patterns.json"),
            ("Associations",subDir + "/dreams/associations.json"),
            ("Journal",     subDir + "/dreams/journal.json"),
            ("Insights",    subDir + "/dreams/insights.md"),
            ("Digest",      subDir + "/dreams/insight-digest.md"),
            ("Metacog",     subDir + "/metacog/"),
        ]
        for (label, path) in paths {
            let exists = FileManager.default.fileExists(atPath: path)
            let size: String = {
                if let attrs = try? FileManager.default.attributesOfItem(atPath: path),
                   let bytes = attrs[.size] as? Int, bytes > 0 {
                    return bytes < 1024 ? "\(bytes) B"
                         : bytes < 1_048_576 ? String(format: "%.1f KB", Double(bytes) / 1024)
                         : String(format: "%.1f MB", Double(bytes) / 1_048_576)
                }
                return exists ? "dir" : "—"
            }()
            let indicator = exists ? "✓" : "✗"
            let labelPadded = (label + ":").padding(toLength: 14, withPad: " ", startingAt: 0)
            rt.raw(NSAttributedString(
                string: "  \(indicator)  \(labelPadded)  \(size)  \(path)\n",
                attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
                    .foregroundColor: exists ? NSColor.labelColor : NSColor.secondaryLabelColor,
                ]))
        }
        rt.spacer()

        rt.subheader("Knowledge Base Summary")
        if let b = board {
            rt.body("  Patterns         \(b.dreamsPatterns)")
            rt.body("  Associations     \(b.associations)")
            rt.body("  Sessions proc.   \(b.dreamsProcessed) dreams  ·  \(b.metacogProcessed) metacog")
            if b.metacogAudits > 0 { rt.body("  Metacog audits   \(b.metacogAudits)") }
        } else {
            rt.dim("  Board data not available.")
        }

        tv.textStorage?.setAttributedString(rt.build())
        return sv
    }
}

// ─── Crash Reporter ───────────────────────────────────────────────────────────
//
// Two-layer strategy:
//   1. NSSetUncaughtExceptionHandler — catches ObjC/Swift bridged exceptions in
//      normal execution context; can safely show NSAlert + write log.
//   2. SIGABRT / SIGSEGV / SIGILL / SIGBUS / SIGFPE signal handlers — write a
//      crash-sentinel file via POSIX write() (async-signal-safe), then re-raise
//      to let the OS generate the standard crash report.
//   On next launch: if a sentinel exists, show a "previous crash" alert and
//   offer to copy the details so the user can paste them for investigation.

private let crashReportDir  = home + "/.claude/subconscious/crash-reports"
private let crashSentinelPath = crashReportDir + "/i-dream-bar-latest.crashlog"

enum CrashReporter {

    static func install() {
        try? FileManager.default.createDirectory(
            atPath: crashReportDir, withIntermediateDirectories: true)

        // ── Layer 1: uncaught ObjC/Swift-bridged exceptions ────────────────────
        NSSetUncaughtExceptionHandler { exception in
            let trace = exception.callStackSymbols.prefix(30).joined(separator: "\n")
            let body  = """
                === i-dream-bar Crash Report ===
                Date:   \(ISO8601DateFormatter().string(from: Date()))
                Build:  \(BuildInfo.commitHash)/\(BuildInfo.sourceHash) built \(BuildInfo.builtAt)
                Type:   Uncaught Exception
                Name:   \(exception.name.rawValue)
                Reason: \(exception.reason ?? "(none)")

                Stack Trace:
                \(trace)
                """
            try? body.write(toFile: crashSentinelPath, atomically: true, encoding: .utf8)

            // We're still in normal context — can safely show UI.
            DispatchQueue.main.async {
                CrashReporter.showCrashAlert(title: "i-dream crashed (exception)",
                    reason: "\(exception.name.rawValue): \(exception.reason ?? "(no reason)")",
                    traceLines: exception.callStackSymbols.prefix(20).map { $0 })
            }
        }

        // ── Layer 2: fatal signals (SIGSEGV / SIGABRT etc.) ───────────────────
        // Write a minimal sentinel file using only async-signal-safe syscalls,
        // then re-raise so the OS generates its normal crash report.
        func installSignalHandler(_ sig: Int32) {
            signal(sig) { signum in
                // Minimal async-signal-safe write — no Swift runtime, no malloc
                let fd = Darwin.open(crashSentinelPath,
                                     O_WRONLY | O_CREAT | O_TRUNC, 0o644)
                if fd >= 0 {
                    let msg = "SIGNAL \(signum) — see ~/Library/Logs/DiagnosticReports/ for full trace\n"
                    _ = msg.withCString { Darwin.write(fd, $0, strlen($0)) }
                    Darwin.close(fd)
                }
                // Re-raise with default handler so the OS crash report is created.
                signal(signum, SIG_DFL)
                Darwin.raise(signum)
            }
        }
        for sig in [SIGSEGV, SIGABRT, SIGILL, SIGBUS, SIGFPE] { installSignalHandler(sig) }
    }

    /// Called at startup — if a previous crash sentinel exists, show it once then delete it.
    static func checkForPreviousCrash() {
        guard let body = try? String(contentsOfFile: crashSentinelPath, encoding: .utf8),
              !body.isEmpty else { return }
        // Delete sentinel before showing alert (prevent loop if alert itself crashes)
        try? FileManager.default.removeItem(atPath: crashSentinelPath)

        let isSignal = body.hasPrefix("SIGNAL")
        let title    = isSignal ? "i-dream crashed (signal)" : "i-dream crashed"
        let lines    = body.components(separatedBy: "\n")
        let reason   = lines.first(where: { $0.hasPrefix("Reason:") || $0.hasPrefix("SIGNAL") }) ?? lines.first ?? "(unknown)"
        let trace    = isSignal ? [] : Array(lines.drop(while: { !$0.hasPrefix("Stack") }).dropFirst().prefix(20))

        showCrashAlert(title: title, reason: reason, traceLines: trace, isPreviousCrash: true)
    }

    /// Display a crash alert with reason + truncated stack trace.
    /// - `isPreviousCrash`: true when shown on next launch (not immediately after crash).
    static func showCrashAlert(title: String, reason: String,
                               traceLines: some Collection<String>,
                               isPreviousCrash: Bool = false) {
        let alert         = NSAlert()
        alert.alertStyle  = .critical
        alert.messageText = title
        let intro = isPreviousCrash
            ? "i-dream detected a crash from the previous session and restarted successfully."
            : "i-dream encountered a fatal error and needs to quit."
        let traceText = traceLines.isEmpty
            ? "(see ~/Library/Logs/DiagnosticReports/ for full trace)"
            : traceLines.joined(separator: "\n")
        alert.informativeText = "\(intro)\n\nReason: \(reason)\n\nStack trace (top 20 frames):\n\(traceText)"
        alert.addButton(withTitle: "Copy Details")
        alert.addButton(withTitle: isPreviousCrash ? "Dismiss" : "Quit")

        // Show the alert as a floating panel so it appears above everything
        NSApp.activate(ignoringOtherApps: true)
        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            let full = "[\(title)]\nReason: \(reason)\n\nStack:\n\(traceText)\n\nBuild: \(BuildInfo.commitHash)/\(BuildInfo.sourceHash) built \(BuildInfo.builtAt)"
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(full, forType: .string)
        }
        if !isPreviousCrash { exit(1) }
    }
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
    private var panelLinkDelegate:    JournalLinkDelegate?   // generic link handler for resizable panels
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

    // Comprehensive dashboard
    private var dashboardController: DashboardWindowController?

    // Dreaming animation
    private var isCycling       = false
    private var cycleStartTime: Date?
    private var animFrame       = 0
    private var animTimer:      Timer?

    // Persistent menu instance (rebuilt in-place via NSMenuDelegate)
    private var theMenu: NSMenu!

    func applicationDidFinishLaunching(_ note: Notification) {
        CrashReporter.install()
        CrashReporter.checkForPreviousCrash()
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
    /// "Open File" button is shown in the toolbar. If `linkHandler` is given,
    /// `.link`-attributed runs in the text view call the handler with the link value.
    private func showResizablePanel(title: String, content: NSAttributedString,
                                     filePath: String? = nil,
                                     linkHandler: ((String) -> Void)? = nil) {
        // Close and release any existing detail panel
        detailPanel?.close()
        detailPanel    = nil
        detailFilePath = filePath
        panelLinkDelegate = nil

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

        if let handler = linkHandler {
            let delegate = JournalLinkDelegate(handler)
            panelLinkDelegate = delegate   // strong ref — NSTextView.delegate is weak
            tv.delegate = delegate
        }

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
        if dashboardController == nil {
            dashboardController = DashboardWindowController()
        }
        dashboardController!.showOrFront()
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

        // ── File locations (click to open in VS Code) ──────────────────────
        rt.subheader("Key Files")
        rt.dim("  Click any path to open in VS Code.")
        rt.spacer()
        let base = NSHomeDirectory() + "/.claude/subconscious"
        rt.monoLink("  \(base)/",
                    linkValue: base)
        rt.monoLink("  ├── state.json              total_cycles, last_consolidation, tokens",
                    linkValue: base + "/state.json")
        rt.monoLink("  ├── dreams/",
                    linkValue: base + "/dreams")
        rt.monoLink("  │   ├── journal.jsonl        one entry per consolidation cycle",
                    linkValue: base + "/dreams/journal.jsonl")
        rt.monoLink("  │   ├── patterns.json        extracted behavioral patterns",
                    linkValue: base + "/dreams/patterns.json")
        rt.monoLink("  │   ├── insights.md          Wake-promoted high-confidence insights",
                    linkValue: base + "/dreams/insights.md")
        rt.monoLink("  │   ├── insight-digest.md    Recent Dreams Inference prose",
                    linkValue: base + "/dreams/insight-digest.md")
        rt.monoLink("  │   ├── digest-meta.json     sentiment + last_run timestamp",
                    linkValue: base + "/dreams/digest-meta.json")
        rt.monoLink("  │   ├── associations.json    cross-pattern hypotheses (REM)",
                    linkValue: base + "/dreams/associations.json")
        rt.monoLink("  │   └── traces/              per-cycle JSONL event logs",
                    linkValue: base + "/dreams/traces")
        rt.monoLink("  ├── metacog/",
                    linkValue: base + "/metacog")
        rt.monoLink("  │   ├── calibration.jsonl    per-cycle calibration scores",
                    linkValue: base + "/metacog/calibration.jsonl")
        rt.monoLink("  │   └── audits/              individual audit JSON files",
                    linkValue: base + "/metacog/audits")
        rt.monoLink("  ├── valence/memory.jsonl      time-decayed pattern outcomes",
                    linkValue: base + "/valence/memory.jsonl")
        rt.monoLink("  └── intentions/registry.jsonl active prospective intentions",
                    linkValue: base + "/intentions/registry.jsonl")

        showResizablePanel(
            title: "i-dream — Terminology Glossary",
            content: rt.build(),
            linkHandler: { path in
                // Open the tapped path in VS Code. Works for both files and folders.
                if let url = URL(string: "vscode://file\(path)") {
                    NSWorkspace.shared.open(url)
                }
            }
        )
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

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = BarDelegate()
app.delegate = delegate
app.run()
