// i-dream-bar.swift  (v4)
// Standalone macOS menu-bar widget for the i-dream consolidation daemon.
//
// Compile & run:
//   bash tools/menubar/build.sh              # build + launch
//   bash tools/menubar/build.sh --install    # build + register LaunchAgent
//   bash tools/menubar/build.sh --status     # show running state

import AppKit
import Foundation
import SwiftUI
// we deliberately do NOT import UserNotifications. The
// widget runs as an ad-hoc-signed loose binary (no .app bundle), and
// `+[UNUserNotificationCenter currentNotificationCenter]` crashes for
// unbundled processes. Falling back to `osascript display notification`
// which works for any process and gives the same end-user experience.

// ─── Paths ────────────────────────────────────────────────────────────────────

private let home      = FileManager.default.homeDirectoryForCurrentUser.path
private let subDir    = home + "/.claude/subconscious"
private let statePath = subDir + "/state.json"
private let pidPath   = subDir + "/daemon.pid"
// Resolved by probing install locations (cargo, ~/.local/bin, /usr/local,
// homebrew) so the daemon controls work wherever i-dream is installed —
// a hardcoded cargo path silently no-ops on any other install.
private let iDream    = resolveIDreamBinary()
private let debugLog  = "/tmp/i-dream-bar.log"
private let tracesDir   = subDir + "/dreams/traces"
private let activityFile = subDir + "/.last-activity"

// UN notification dedup
private let lastSeenBriefingKey = "dev.i-dream.lastSeenBriefingWeek"
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
    /// Per-occurrence timestamps (D11 v2, capped at 50) — fuels the
    /// Overview activity timelines.
    let occurrenceHistory: [String]?
    /// Stable key for selection — uses id when available, falls back to text prefix.
    var stableKey: String { id ?? String(pattern.prefix(30)) }
    enum CodingKeys: String, CodingKey {
        case id, pattern, valence, confidence, category
        case firstSeen = "first_seen"
        case occurrenceHistory = "occurrence_history"
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
    /// Both fields default-false so legacy associations.json (pre-D3 v1) decodes.
    /// Decoded for the default-summary card and for future multi-select
    /// dismiss/promote write path.
    let promoted:      Bool?
    let dismissed:     Bool?
    enum CodingKeys: String, CodingKey {
        case id, hypothesis, confidence, actionable, promoted, dismissed
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

/// A view whose background is a windowBackground/label blend resolved in
/// updateLayer — i.e. under the view's effective appearance, re-resolved on
/// theme change. Assigning `NSColor...cgColor` at build time snapshots the
/// appearance that happens to be current, which paints light chrome inside
/// dark windows (the dashboard stats-banner bug).
private final class BlendedBackgroundView: NSView {
    var blendFraction: CGFloat = 0
    var chipCornerRadius: CGFloat = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true   // updateLayer only runs on layer-backed views
    }
    required init?(coder: NSCoder) { fatalError("not used from nibs") }

    override var wantsUpdateLayer: Bool { true }
    override func updateLayer() {
        layer?.cornerRadius = chipCornerRadius
        layer?.backgroundColor = NSColor.windowBackgroundColor
            .blended(withFraction: blendFraction, of: .labelColor)?.cgColor
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

// ─── Association Network Graph ────────────────────────────────────────────────
// Interactive graph of cross-pattern hypotheses (associations).
// Nodes = associations, sized by confidence.
// Edges = shared patternsLinked IDs, thickness ∝ overlap count.
// Three concentric rings: inner ≥0.75 confidence, middle ≥0.50, outer <0.50.
// Pan / zoom / hover / click identical to PatternGraphView.

// ─── Icon choices ─────────────────────────────────────────────────────────────


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
        let idx = maxVal == 0 ? 0 : max(0, min(Int(Double(v) / Double(maxVal) * 7.0), 7))
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

    /// 2px leading accent bar layer — visible only on selection.
    /// three redundant cues per the dashboard review —
    /// accent bar (this), bg tint, semibold title. Selection becomes
    /// scannable from peripheral vision.
    private var accentBar: CALayer?

    var isSelectedTab = false {
        didSet {
            guard oldValue != isSelectedTab else { return }
            layer?.backgroundColor = isSelectedTab
                ? NSColor.controlAccentColor.withAlphaComponent(0.22).cgColor
                : nil
            self.contentTintColor = isSelectedTab
                ? _iconColor
                : _iconColor.withAlphaComponent(0.55)
            // Lazily create the accent bar; show only when selected.
            if accentBar == nil {
                let bar = CALayer()
                bar.frame = CGRect(x: 0, y: 4, width: 2.5, height: max(0, bounds.height - 8))
                bar.cornerRadius = 1
                self.layer?.addSublayer(bar)
                accentBar = bar
            }
            accentBar?.backgroundColor = isSelectedTab
                ? _iconColor.cgColor
                : NSColor.clear.cgColor
            updateAttributedTitle()
        }
    }

    override func layout() {
        super.layout()
        if let bar = accentBar {
            bar.frame = CGRect(x: 0, y: 4, width: 2.5, height: max(0, bounds.height - 8))
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
        let color: NSColor = isSelectedTab
            ? .labelColor
            : .secondaryLabelColor   // dim unselected for visible hierarchy
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13, weight: weight),
            .foregroundColor: color,
        ]
        self.attributedTitle = NSAttributedString(string: "  " + _title, attributes: attrs)
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

    /// SelectionModel — phase-1 foundation for #42 (NSTableView refactor).
    /// Lifts per-tab selection state into a single observable struct so
    /// future filter strips, sort dropdowns, hover icons, keyboard nav,
    /// and multi-select can all read/write the same source of truth.
    /// The current RichText-based list rendering already drives selection
    /// through ad-hoc closures; this struct will replace those once the
    /// list rendering switches to NSTableView in phase 2.
    ///
    /// Kept as a value type so it diffs cleanly with `oldValue` in
    /// observers (didSet on whichever container holds it).
    struct SelectionModel: Equatable {
        var primary: String? = nil       // single-select current row id
        var multi:   Set<String> = []    // multi-select set
        var sortKey: SortKey = .confidenceDesc
        var filter:  FilterModel = .init()
        // Convenience: was anything selected?
        var hasSelection: Bool { primary != nil || !multi.isEmpty }
    }
    enum SortKey: String, CaseIterable, Equatable {
        case confidenceDesc = "Confidence ↓"
        case confidenceAsc  = "Confidence ↑"
        case recentDesc     = "Recent"
        case linkedDesc     = "Linked count"
        case categoryAsc    = "Category"
        case occurrencesDesc = "Occurrences"
    }
    /// FilterModel mirrors a reviewer's recommended filter
    /// strip schema — every chip the eventual filter strip will drive.
    /// Empty defaults = "show everything." Filter eval is in-memory predicate
    /// matching against the loaded patterns/associations arrays.
    struct FilterModel: Equatable {
        var actionableOnly: Bool = false
        var minConfidence:  Double = 0.0     // [0.0, 1.0]
        var category:       Set<String> = [] // empty = all categories
        var valence:        Set<String> = [] // "positive"/"negative"/"neutral"
        var minLinked:      Int = 0          // associations only
        var sinceDays:      Int? = nil       // nil = all-time, else last N days
        var hideDismissed:  Bool = true      // default behavior
        var freeText:       String = ""      // fuzzy match against text body
        var isActive: Bool {
            actionableOnly || minConfidence > 0 || !category.isEmpty
                || !valence.isEmpty || minLinked > 0 || sinceDays != nil
                || !freeText.isEmpty || !hideDismissed
        }
    }
    /// Per-tab selection state. Phase-2 wiring (NSTableView + the
    /// existing buildPatternView / buildAssociationView paths) will
    /// observe these via didSet and drive list rendering + graph
    /// highlight + detail card from a single source of truth.
    private var patternsSelection:     SelectionModel = SelectionModel()
    private var associationsSelection: SelectionModel = SelectionModel()

    // Detail panels for selection context (Patterns + Associations tabs)
    private var patternDetailTextView:  NSTextView?
    private var assocDetailTextView:    NSTextView?

    // Insights tab state
    private var insightFeedbackDelegate: InsightFeedbackDelegate?

    // Search tab state
    private var searchField:           NSSearchField?
    private var searchResultsTextView: NSTextView?
    private var searchLinkDelegate:    JournalLinkDelegate?
    private var searchDebounceTimer:   Timer?

    // Sidebar footer state
    private var lastRefreshedLabel:    NSTextField?
    private var themePickerButtons:    [HoverButton] = []
    private let dashAlwaysOnTopKey = "dev.i-dream.dashboard.alwaysOnTop"
    private let dashThemeKey       = "dev.i-dream.dashboard.theme"  // "light"|"dark"|"system"

    /// 0=light, 1=dark, 2=system. Defaults to dark when key missing.
    private func currentThemeIndex() -> Int {
        switch UserDefaults.standard.string(forKey: dashThemeKey) {
        case "light":  return 0
        case "system": return 2
        default:       return 1   // dark default (brand identity)
        }
    }

    private func applyDashboardAppearance() {
        guard let panel = panel else { return }
        let appearance: NSAppearance? = {
            switch currentThemeIndex() {
            case 0:  return NSAppearance(named: .darkAqua) // placeholder — see note below
            case 2:  return nil  // follow system
            default: return NSAppearance(named: .darkAqua)
            }
        }()
        // Actual mapping: 0=light → .aqua, 1=dark → .darkAqua, 2=system → nil
        switch currentThemeIndex() {
        case 0:  panel.appearance = NSAppearance(named: .aqua)
        case 2:  panel.appearance = nil
        default: panel.appearance = NSAppearance(named: .darkAqua)
        }
        _ = appearance  // silence unused
    }

    @objc private func themeIconClicked(_ sender: NSButton) {
        // Tag 0=light, 1=dark, 2=system
        let v: String
        switch sender.tag {
        case 0:  v = "light"
        case 2:  v = "system"
        default: v = "dark"
        }
        UserDefaults.standard.set(v, forKey: dashThemeKey)
        applyDashboardAppearance()
        // Re-tint buttons: selected = full color, others = dim.
        let tints: [NSColor] = [
            .systemYellow,
            NSColor(red: 0.55, green: 0.41, blue: 0.85, alpha: 1),
            .systemTeal,
        ]
        for (i, btn) in themePickerButtons.enumerated() {
            btn.contentTintColor = (i == sender.tag) ? tints[i] : NSColor.tertiaryLabelColor
        }
    }

    @objc private func toggleDashboardAlwaysOnTop(_ sender: NSButton) {
        let on = sender.state == .on
        UserDefaults.standard.set(on, forKey: dashAlwaysOnTopKey)
        guard let panel = panel else {
            dlog("AOT: no panel reference (state=\(on))")
            return
        }
        // .popUpMenu (level 101) is the most reliable always-on-top level
        // on macOS — sits above .statusBar (25) and above other apps'
        // floating windows. Combined with .canJoinAllSpaces it stays
        // visible across Spaces switches.
        if on {
            panel.level = .popUpMenu
            panel.collectionBehavior.insert(.canJoinAllSpaces)
            panel.orderFrontRegardless()
        } else {
            panel.level = .floating
            panel.collectionBehavior.remove(.canJoinAllSpaces)
        }
        dlog("AOT toggled: \(on) → level=\(panel.level.rawValue)")
    }
    private var lastRefreshedDate:     Date?

    // v3 (docs/23 Stage 2): four surfaces. Browse replaces the per-type
    // Patterns/Associations/Insights/Metacog tabs with one paradigm on the
    // engine's deduped views; Help/About live in the status menu now.
    private let tabs: [(title: String, symbol: String)] = [
        ("Overview", "square.grid.2x2.fill"),
        ("Browse",   "list.bullet.rectangle"),
        ("Journal",  "book.fill"),
        ("Search",   "magnifyingglass"),
    ]

    // Data snapshots — reloaded on each showOrFront() call
    private var patterns:     [Pattern]      = []
    private var associations: [Association]  = []
    private var journal:      [JournalEntry] = []
    private var state:        DaemonState?
    private var board:        BoardData?
    private var digest:       String?
    let browseModel = BrowseModel()
    let overviewModel = OverviewModel()
    let journalModel = JournalModel()

    // ── Public interface ───────────────────────────────────────────────────────

    /// HUD task #7: open the dashboard scrolled to a specific tab index.
    /// Convenience overload of showOrFront() for HUD cells / external
    /// callers that know which tab they want.
    func showOrFront(tab: Int) {
        showOrFront()
        // Defer one tick so the panel + selectTab order is deterministic
        // on first construction.
        DispatchQueue.main.async { [weak self] in self?.selectTab(tab) }
    }

    func showOrFront() {
        dlog("dashboard: showOrFront (visible=\(panel?.isVisible == true))")
        if let p = panel, p.isVisible {
            p.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            reloadDataAsync()
            return
        }
        buildAndShow()
        reloadDataAsync()
    }

    /// Read the six data stores off the main thread, then rebuild the tab
    /// views with the fresh data. The window appears immediately with
    /// whatever data it already holds (empty states on first open) instead
    /// of freezing while multi-hundred-KB JSON parses run on the main thread.
    private func reloadDataAsync(completion: (() -> Void)? = nil) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let pat   = allPatterns()
            let assoc = allAssociations()
            let jour  = allJournal()
            let st    = readState()
            let bd    = readBoard()
            let dg    = readInsightDigest()
            let (browseRows, browseTotals) = buildBrowseRows()
            var ov = OverviewData()
            ov.reflect = readReflect()
            ov.reviewPending = readReviewPending()
            if let s = st {
                ov.state = (cycles: s.totalCycles, tokens: s.totalTokensUsed,
                            lastDream: s.lastConsolidation.map { timeAgo($0) })
            }
            ov.viz = buildOverviewViz(rows: browseRows)
            let (journalRows, heat) = buildJournalRows(journal: jour)
            DispatchQueue.main.async {
                guard let self, self.panel != nil else { return }
                self.patterns     = pat
                self.associations = assoc
                self.journal      = jour
                self.state        = st
                self.board        = bd
                self.digest       = dg
                self.browseModel.apply(rows: browseRows, totals: browseTotals)
                self.overviewModel.data = ov
                self.journalModel.rows = journalRows
                self.journalModel.heatEntries = heat
                self.rebuildContentViews()
                dlog("dashboard: async data loaded (\(pat.count)p/\(assoc.count)a/\(jour.count)j/\(browseRows.count)b)")
                completion?()
            }
        }
    }

    /// Persist an insight rating and refresh Browse so the badge updates.
    private func rateInsightFromBrowse(id: String, rating: String) {
        recordDashboardInsightFeedback(insightId: id, rating: rating)
        reloadDataAsync()
    }

    /// Help and About lost their tabs in the v3 cutover (docs/23 Stage 2);
    /// their content opens in a small panel from the status menu instead.
    private var infoPanel: NSPanel?
    func showInfoPanel(about: Bool) {
        let f = NSRect(x: 0, y: 0, width: 720, height: 640)
        let v = about ? buildAboutView(frame: f) : buildHelpView(frame: f)
        let p = NSPanel(contentRect: f, styleMask: [.titled, .closable, .resizable],
                        backing: .buffered, defer: false)
        p.title = about ? "About i-dream" : "i-dream — Help & Shortcuts"
        p.isReleasedWhenClosed = false
        p.appearance = NSAppearance(named: .darkAqua)
        p.level = .floating
        v.autoresizingMask = [.width, .height]
        p.contentView?.addSubview(v)
        p.center()
        infoPanel?.close()
        infoPanel = p
        p.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
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
        // Follow the user to the ACTIVE Space when summoned. Without this,
        // "Open Dashboard" silently fronts the panel on whatever Space it
        // was born on — the window reports isVisible while nothing appears
        // where the user is looking (field-study J2, reproduced + logged).
        p.collectionBehavior.insert(.moveToActiveSpace)
        p.minSize              = NSSize(width: 960, height: 640)
        p.center()
        // Apply user's theme choice (defaults to dark — the brand
        // identity — but light / system are available via the sidebar
        // segmented control). NB: this sets the panel-only appearance;
        // the rest of the process (HUD + menubar) stays dark via the
        // NSApp.appearance pin in applicationDidFinishLaunching.
        switch currentThemeIndex() {
        case 0:  p.appearance = NSAppearance(named: .aqua)
        case 2:  p.appearance = nil   // follow system
        default: p.appearance = NSAppearance(named: .darkAqua)
        }
        // Apply user's always-on-top preference if set. Use .popUpMenu
        // (level 101) for reliable above-other-apps behavior; .statusBar
        // sometimes loses to certain system overlays.
        if UserDefaults.standard.bool(forKey: dashAlwaysOnTopKey) {
            p.level = .popUpMenu
            p.collectionBehavior.insert(.canJoinAllSpaces)
        }
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

        // brand mark + title — 10×10 dusk-violet glyph
        // with a soft glow precedes a 15pt label-color title. Replaces the
        // 12pt tertiary-color stock label that the dashboard review flagged
        // as "the app has no face." Aligns with the project identity
        // (sleep / dreams) without abandoning the cyan/teal/orange system
        // accents.
        let brandMarkSize: CGFloat = 10
        let brandMark = NSView(frame: NSRect(x: 14, y: panH - 44 + 4,
                                              width: brandMarkSize, height: brandMarkSize))
        brandMark.wantsLayer = true
        brandMark.autoresizingMask = [.minYMargin]
        let brandColor = NSColor(red: 0.55, green: 0.41, blue: 0.85, alpha: 1.0)  // dusk violet
        let brandLayer = CALayer()
        brandLayer.frame = brandMark.bounds
        brandLayer.cornerRadius = brandMarkSize / 2
        brandLayer.backgroundColor = brandColor.cgColor
        brandLayer.shadowColor = brandColor.cgColor
        brandLayer.shadowOpacity = 0.45
        brandLayer.shadowRadius = 4
        brandLayer.shadowOffset = .zero
        brandMark.layer?.addSublayer(brandLayer)
        sidebar.addSubview(brandMark)

        let sideTitle = NSTextField(labelWithString: "i-dream")
        sideTitle.font       = .systemFont(ofSize: 15, weight: .semibold)
        sideTitle.textColor  = .labelColor
        sideTitle.frame      = NSRect(x: 14 + brandMarkSize + 8,
                                       y: panH - 44, width: sideW - 28 - brandMarkSize - 8, height: 22)
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

        // Bottom: export + refresh + theme + always-on-top + version + last-refreshed
        let exportBtn = NSButton(title: "⬇  Export JSON", target: self, action: #selector(exportDashboardData))
        exportBtn.frame            = NSRect(x: 8, y: 132, width: sideW - 16, height: 28)
        exportBtn.isBordered       = false
        exportBtn.font             = .systemFont(ofSize: 12)
        exportBtn.contentTintColor = .secondaryLabelColor
        sidebar.addSubview(exportBtn)

        let refreshBtn = NSButton(title: "↺  Refresh  (⌘R)", target: self, action: #selector(refreshDashboard))
        refreshBtn.frame            = NSRect(x: 8, y: 108, width: sideW - 16, height: 28)
        refreshBtn.isBordered       = false
        refreshBtn.font             = .systemFont(ofSize: 12)
        refreshBtn.contentTintColor = .secondaryLabelColor
        sidebar.addSubview(refreshBtn)

        // ── Theme picker — three icon-only HoverButtons ───────────────────
        // No background unless hovered (uses the same HoverButton class as
        // the floating HUD action row). Each button has a tooltip; the
        // currently-selected theme is shown by tinting its icon brighter.
        let themeRow = NSView(frame: NSRect(x: 8, y: 80, width: sideW - 16, height: 28))
        let themes: [(symbol: String, tooltip: String, value: String, tint: NSColor)] = [
            ("sun.max.fill",          "Light theme",  "light",  NSColor.systemYellow),
            ("moon.fill",             "Dark theme",   "dark",   NSColor(red: 0.55, green: 0.41, blue: 0.85, alpha: 1)),
            ("circle.lefthalf.filled","Follow system","system", NSColor.systemTeal),
        ]
        let bw  = (themeRow.bounds.width - 12) / 3   // 3 buttons + 2x6px gaps
        let cur = UserDefaults.standard.string(forKey: dashThemeKey) ?? "dark"
        themePickerButtons.removeAll()
        for (i, t) in themes.enumerated() {
            let btn = HoverButton(frame: NSRect(x: CGFloat(i) * (bw + 6), y: 0,
                                                width: bw, height: 28))
            btn.hoverLabel = t.tooltip   // also drives the HUD-style hover label if delegate set
            btn.tintColor  = t.tint
            btn.toolTip    = t.tooltip
            if let img = NSImage(systemSymbolName: t.symbol, accessibilityDescription: t.tooltip) {
                let cfg = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
                btn.image = img.withSymbolConfiguration(cfg) ?? img
                btn.imagePosition = .imageOnly
            }
            // Selected theme: full-color tint. Unselected: dim.
            btn.contentTintColor = (t.value == cur) ? t.tint : NSColor.tertiaryLabelColor
            btn.tag    = i
            btn.target = self
            btn.action = #selector(themeIconClicked(_:))
            themeRow.addSubview(btn)
            themePickerButtons.append(btn)
        }
        sidebar.addSubview(themeRow)

        // ── Always-on-top toggle ──────────────────────────────────────────
        let aotBtn = NSButton(checkboxWithTitle: "  Always on top",
                               target: self, action: #selector(toggleDashboardAlwaysOnTop(_:)))
        aotBtn.frame = NSRect(x: 8, y: 56, width: sideW - 16, height: 18)
        aotBtn.font  = .systemFont(ofSize: 11)
        aotBtn.state = UserDefaults.standard.bool(forKey: dashAlwaysOnTopKey) ? .on : .off
        if let panel = panel, aotBtn.state == .on { panel.level = .statusBar }
        sidebar.addSubview(aotBtn)

        let verLabel = NSTextField(labelWithString: "build \(BuildInfo.commitHash.prefix(7))")
        verLabel.font      = .monospacedSystemFont(ofSize: 9.5, weight: .regular)
        verLabel.textColor = .tertiaryLabelColor
        verLabel.frame     = NSRect(x: 14, y: 30, width: sideW - 28, height: 14)
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

        installKeyMonitorIfNeeded()
        NSApp.activate(ignoringOtherApps: true)
        p.makeKeyAndOrderFront(nil)
    }

    /// ⌘1–9 / ⌘R / ⌘F must work no matter which text field is focused.
    /// The panel's performKeyEquivalent override stops being reached once a
    /// field editor is active (that is how tab shortcuts died with the graph
    /// filter focused), so intercept at the event level instead.
    private var keyMonitor: Any?
    private func installKeyMonitorIfNeeded() {
        guard keyMonitor == nil else { return }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] ev in
            guard let self, let p = self.panel, ev.window === p,
                  ev.modifierFlags.intersection([.command, .option, .control, .shift]) == .command,
                  let ch = ev.charactersIgnoringModifiers?.first else { return ev }
            if ch >= "1" && ch <= "9" {
                self.selectTab(Int(ch.asciiValue! - Character("1").asciiValue!))
                return nil
            }
            if ch == "r" { self.refreshDashboard(); return nil }
            if ch == "f" {
                self.selectTab(3)
                if let sf = self.searchField { p.makeFirstResponder(sf) }
                return nil
            }
            return ev
        }
    }

    private func rebuildContentViews() {
        patternDetailTextView = nil
        assocDetailTextView = nil
        searchField             = nil
        searchResultsTextView   = nil
        for v in contentViews { v.removeFromSuperview() }
        contentViews = []
        let f = contentContainer.bounds
        dlog("dashboard: building overview")
        overviewModel.onOpenReview = {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: resolveIDreamBinary())
            p.arguments = ["review"]
            p.standardOutput = FileHandle.nullDevice
            p.standardError = FileHandle.nullDevice
            try? p.run()
        }
        overviewModel.onJumpToBrowse = { [weak self] id in
            guard let self else { return }
            self.selectTab(1)
            self.browseModel.jump(to: id)
        }
        let v0: NSView = {
            let host = NSHostingView(rootView: OverviewPane(model: overviewModel))
            host.frame = f
            host.autoresizingMask = [.width, .height]
            return host
        }()
        dlog("dashboard: building browse")
        browseModel.onRate = { [weak self] id, rating in
            self?.rateInsightFromBrowse(id: id, rating: rating)
        }
        let v1: NSView = {
            let host = NSHostingView(rootView: BrowseView(model: browseModel))
            host.frame = f
            host.autoresizingMask = [.width, .height]
            return host
        }()
        dlog("dashboard: building journal")
        journalModel.onJumpToBrowse = { [weak self] id in
            guard let self else { return }
            self.selectTab(1)
            self.browseModel.jump(to: id)
        }
        let v2: NSView = {
            let host = NSHostingView(rootView: JournalPane(model: journalModel))
            host.frame = f
            host.autoresizingMask = [.width, .height]
            return host
        }()
        dlog("dashboard: building search")
        let v3 = buildSearchView(frame: f)
        dlog("dashboard: all views built")
        contentViews = [v0, v1, v2, v3]
        for v in contentViews { contentContainer.addSubview(v) }
        let sel = navButtons.first(where: { $0.isSelectedTab })?.tag ?? 0
        for (i, v) in contentViews.enumerated() { v.isHidden = (i != sel) }

        // Update sidebar labels with data counts
        updateSidebarBadges()
    }

    private func updateSidebarBadges() {
        guard navButtons.count >= 4 else { return }
        // Browse shows deduped rows; the honest full total lives in its footer.
        navButtons[1].updateTitle("Browse (\(browseModel.rows.count))")
        navButtons[2].updateTitle("Journal (\(journal.count))")
    }

    // ── Navigation ─────────────────────────────────────────────────────────────

    @objc private func navTapped(_ sender: NSButton) { selectTab(sender.tag) }

    func selectTab(_ index: Int) {
        guard index >= 0 && index < tabs.count else { return }
        for (i, btn) in navButtons.enumerated() { btn.isSelectedTab = (i == index) }
        for (i, v) in contentViews.enumerated()  { v.isHidden        = (i != index) }
        UserDefaults.standard.set(index, forKey: "idream-dashboard-selected-tab")
    }

    @objc private func refreshDashboard() {
        guard let p = panel, p.isVisible else { showOrFront(); return }
        reloadDataAsync { [weak self] in
            self?.lastRefreshedDate = Date()
            self?.lastRefreshedLabel?.stringValue = "Refreshed just now"
        }
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

    /// Horizontal stats banner — rendered as a row of
    /// "stat chips" instead of the comma-soup that a reviewer
    /// flagged as "a paragraph to read." Each chip is a stacked
    /// caption-on-top + value-below pair, with tabular numerals so values
    /// align column-by-column across chips.
    ///
    /// Backing views resolve their layer color in updateLayer under the
    /// view's effective appearance. Converting a semantic NSColor to cgColor
    /// eagerly snapshots whatever appearance is current at build time —
    /// which is how this banner shipped as a light band inside the dark
    /// dashboard, with its caption labels drawing dark-appearance text on top.
    private func makeStatsBanner(frame: NSRect,
                                 stats: [(label: String, value: String, color: NSColor?)]) -> NSView {
        let banner = BlendedBackgroundView(frame: frame)
        banner.blendFraction = 0.04

        // Bottom separator
        let sep = NSBox(frame: NSRect(x: 0, y: 0, width: frame.width, height: 1))
        sep.boxType = .separator; sep.autoresizingMask = [.width]
        banner.addSubview(sep)

        // Lay out chips evenly across the banner width. Width adapts to the
        // banner so long captions (ASSOCIATIONS, CALIBRATION) stop truncating.
        let gap:   CGFloat = 4
        let chipW: CGFloat = max(88, min(150,
            (frame.width - 28 - CGFloat(max(stats.count - 1, 0)) * gap) / CGFloat(max(stats.count, 1))))
        let totalW = CGFloat(stats.count) * chipW + CGFloat(max(stats.count - 1, 0)) * gap
        var x: CGFloat = max(14, (frame.width - totalW) / 2)
        let chipY:    CGFloat = 4
        let chipH:    CGFloat = frame.height - 8

        for stat in stats {
            let chip = BlendedBackgroundView(frame: NSRect(x: x, y: chipY, width: chipW, height: chipH))
            chip.blendFraction = 0.06
            chip.chipCornerRadius = 4

            let lbl = NSTextField(labelWithString: stat.label.uppercased())
            lbl.font = .systemFont(ofSize: 9, weight: .semibold)
            lbl.textColor = .tertiaryLabelColor
            lbl.alignment = .center
            lbl.frame = NSRect(x: 4, y: chipH - 14, width: chipW - 8, height: 12)
            lbl.backgroundColor = .clear; lbl.drawsBackground = false; lbl.isBordered = false
            chip.addSubview(lbl)

            let val = NSTextField(labelWithString: stat.value)
            val.font = NSFont.monospacedDigitSystemFont(ofSize: 15, weight: .semibold)
            val.textColor = stat.color ?? .labelColor
            val.alignment = .center
            val.frame = NSRect(x: 4, y: 2, width: chipW - 8, height: 18)
            val.backgroundColor = .clear; val.drawsBackground = false; val.isBordered = false
            chip.addSubview(val)

            banner.addSubview(chip)
            x += chipW + gap
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

    @objc private func patternGraphSearchChanged(_ sender: NSSearchField) {
    }

    /// default summary card for the Patterns detail pane
    /// when nothing is selected. Replaces the dim "Select…" wall with a
    /// useful at-a-glance overview: counts + top 5 by confidence + keyboard
    /// hints. Reads `patterns` already in memory — no I/O.
    private func buildPatternsDefaultSummary() -> NSAttributedString {
        let rt = RichText()
        rt.header("Patterns")
        let total       = patterns.count
        let highConf    = patterns.filter { $0.confidence >= 0.8 }.count
        let categories  = Set(patterns.map { $0.category }).count
        let positive    = patterns.filter { $0.valence == "positive" }.count
        let negative    = patterns.filter { $0.valence == "negative" }.count
        rt.dim("\(total) total · \(highConf) high-confidence · \(categories) categories · \(positive)↑ \(negative)↓")
        rt.spacer()

        rt.subheader("Top by confidence")
        let topConf = patterns
            .sorted { $0.confidence > $1.confidence }
            .prefix(5)
        for p in topConf {
            let confPct = Int(p.confidence * 100)
            let glyph = p.valence == "positive" ? "▲" : p.valence == "negative" ? "▼" : "·"
            let preview = p.pattern.count > 80 ? String(p.pattern.prefix(78)) + "…" : p.pattern
            rt.body("  \(confPct)%  \(glyph)  \(preview)")
        }
        rt.spacer()

        rt.subheader("Tips")
        rt.dim("  Click a row or graph node for full detail")
        rt.dim("  Filter the list with the search field above the graph")
        rt.dim("  Categories: \(Set(patterns.map { $0.category }).sorted().joined(separator: ", "))")
        return rt.build()
    }

    /// same default summary pattern for Associations.
    private func buildAssociationsDefaultSummary() -> NSAttributedString {
        let rt = RichText()
        rt.header("Associations")
        let total       = associations.count
        let actionable  = associations.filter { $0.actionable }.count
        let promoted    = associations.filter { $0.promoted ?? false }.count
        let highConf    = associations.filter { $0.confidence >= 0.75 }.count
        rt.dim("\(total) total · \(actionable) actionable · \(promoted) promoted · \(highConf) high-confidence")
        rt.spacer()

        rt.subheader("Top actionable by confidence")
        let topActionable = associations
            .filter { $0.actionable }
            .sorted { $0.confidence > $1.confidence }
            .prefix(5)
        if topActionable.isEmpty {
            rt.dim("  (no actionable associations yet)")
        } else {
            for a in topActionable {
                let confPct = Int(a.confidence * 100)
                let preview = a.hypothesis.count > 90 ? String(a.hypothesis.prefix(88)) + "…" : a.hypothesis
                rt.body("  \(confPct)%  ◆  \(preview)")
            }
        }
        rt.spacer()

        rt.subheader("Tips")
        rt.dim("  Click a row to focus its node + linked patterns in the graph")
        rt.dim("  Default graph mode: edges show only when a node is selected")
        return rt.build()
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

    @objc private func assocGraphSearchChanged(_ sender: NSSearchField) {
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

    // ── Tab 4: Insights ────────────────────────────────────────────────────────

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

    // ── Tab 5: Metacog ─────────────────────────────────────────────────────────

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
                self?.selectTab(1) // Navigate to Browse
            } else if link.hasPrefix("assoc:") {
                self?.selectTab(1) // Navigate to Browse
            } else if link.hasPrefix("insight:") {
                self?.selectTab(1) // Navigate to Browse
            } else if link.hasPrefix("metacog:") {
                self?.selectTab(1) // Navigate to Browse
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

    // MARK: - Pattern context menu (drag-to-CLAUDE.md / drag-to-hook)

    /// Build the right-click menu for a pattern node. Two destructive-but-
    /// reversible actions: append a guideline line to a CLAUDE.md draft, or
    /// scaffold a hook. Both write to ~/.i-dream/exports/<ts>-<slug>/ and
    /// reveal in Finder so the user reviews before promoting.
    fileprivate func buildPatternContextMenu(for pat: Pattern) -> NSMenu {
        let menu = NSMenu()
        let header = NSMenuItem()
        let preview = pat.pattern.count > 60 ? String(pat.pattern.prefix(57)) + "…" : pat.pattern
        header.attributedTitle = NSAttributedString(
            string: preview,
            attributes: [.font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                         .foregroundColor: NSColor.secondaryLabelColor])
        menu.addItem(header)
        menu.addItem(NSMenuItem.separator())

        let guideItem = NSMenuItem(title: "Export as CLAUDE.md guideline…",
                                   action: #selector(exportPatternAsGuideline(_:)),
                                   keyEquivalent: "")
        guideItem.target = self
        guideItem.representedObject = pat
        menu.addItem(guideItem)

        let hookItem = NSMenuItem(title: "Export as hook scaffold…",
                                  action: #selector(exportPatternAsHook(_:)),
                                  keyEquivalent: "")
        hookItem.target = self
        hookItem.representedObject = pat
        menu.addItem(hookItem)

        menu.addItem(NSMenuItem.separator())

        let copyItem = NSMenuItem(title: "Copy pattern text",
                                  action: #selector(copyPatternText(_:)),
                                  keyEquivalent: "")
        copyItem.target = self
        copyItem.representedObject = pat
        menu.addItem(copyItem)

        return menu
    }

    @objc fileprivate func exportPatternAsGuideline(_ sender: NSMenuItem) {
        guard let pat = sender.representedObject as? Pattern else { return }
        let body = m14_renderGuidelineSnippet(for: pat)
        let url  = m14_writeExport(pat: pat, kind: "guideline", ext: "md", contents: body)
        m14_revealAndToast(url: url, kind: "Guideline")
    }

    @objc fileprivate func exportPatternAsHook(_ sender: NSMenuItem) {
        guard let pat = sender.representedObject as? Pattern else { return }
        let (script, json) = m14_renderHookScaffold(for: pat)
        let dir = m14_exportDir(pat: pat, kind: "hook")
        let scriptURL = dir.appendingPathComponent("hook.sh")
        let jsonURL   = dir.appendingPathComponent("settings-snippet.json")
        let readmeURL = dir.appendingPathComponent("README.md")
        try? script.write(to: scriptURL, atomically: true, encoding: .utf8)
        try? FileManager.default.setAttributes([.posixPermissions: 0o755],
                                               ofItemAtPath: scriptURL.path)
        try? json.write(to: jsonURL, atomically: true, encoding: .utf8)
        try? m14_renderHookReadme(for: pat).write(to: readmeURL, atomically: true, encoding: .utf8)
        m14_revealAndToast(url: dir, kind: "Hook scaffold")
    }

    @objc fileprivate func copyPatternText(_ sender: NSMenuItem) {
        guard let pat = sender.representedObject as? Pattern else { return }
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(pat.pattern, forType: .string)
    }

    // helpers — file IO

    private func m14_exportRoot() -> URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let root = home.appendingPathComponent(".i-dream/exports", isDirectory: true)
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }

    private func m14_slug(_ s: String) -> String {
        let lower = s.lowercased()
        let cleaned = lower.unicodeScalars.map { c -> String in
            if (c >= "a" && c <= "z") || (c >= "0" && c <= "9") { return String(c) }
            return "-"
        }.joined()
        // Collapse runs of dashes, trim, cap length.
        var out = ""
        var lastDash = false
        for ch in cleaned {
            if ch == "-" {
                if !lastDash { out.append(ch); lastDash = true }
            } else { out.append(ch); lastDash = false }
        }
        out = out.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return String(out.prefix(40))
    }

    private func m14_exportDir(pat: Pattern, kind: String) -> URL {
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyyMMdd-HHmmss"
        let stamp = fmt.string(from: Date())
        let slug = m14_slug(pat.pattern)
        let dir = m14_exportRoot()
            .appendingPathComponent("\(stamp)-\(kind)-\(slug)", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func m14_writeExport(pat: Pattern, kind: String, ext: String, contents: String) -> URL {
        let dir = m14_exportDir(pat: pat, kind: kind)
        let url = dir.appendingPathComponent("snippet.\(ext)")
        try? contents.write(to: url, atomically: true, encoding: .utf8)
        return url
    }

    private func m14_revealAndToast(url: URL, kind: String) {
        NSWorkspace.shared.activateFileViewerSelecting([url])
        // Use osascript notification — UN/NSUserNotification both crash on
        // unbundled binaries on this macOS version (see file-top comment).
        func esc(_ s: String) -> String {
            s.replacingOccurrences(of: "\\", with: "\\\\")
             .replacingOccurrences(of: "\"", with: "\\\"")
             .replacingOccurrences(of: "\n", with: " ")
        }
        let cmd = "display notification \"\(esc(url.lastPathComponent))\" with title \"i-dream — \(esc(kind)) exported\" sound name \"Glass\""
        DispatchQueue.global(qos: .background).async {
            let task = Process()
            task.launchPath = "/usr/bin/osascript"
            task.arguments  = ["-e", cmd]
            try? task.run()
            task.waitUntilExit()
        }
    }

    // templates — pure-string, no dependencies

    private func m14_renderGuidelineSnippet(for pat: Pattern) -> String {
        let conf = Int(pat.confidence * 100)
        return """
        <!-- Generated from i-dream pattern · \(pat.category) · \(conf)% confidence · \(pat.valence) -->
        <!-- Source pattern id: \(pat.stableKey) -->

        ## \(pat.category.capitalized)

        - \(pat.pattern)

        <!-- Review and edit before pasting into CLAUDE.md. The pattern was
             extracted automatically from session transcripts; the wording
             may need to be tightened or made more directive before it
             becomes a binding guideline. -->
        """
    }

    private func m14_renderHookScaffold(for pat: Pattern) -> (script: String, settingsJson: String) {
        // Conservative default: a UserPromptSubmit hook that injects a hint
        // line. The user can swap to PreToolUse / PostToolUse with a one-
        // line edit. We don't try to *enforce* the pattern automatically —
        // that's a power tool that needs human review.
        let escaped = pat.pattern.replacingOccurrences(of: "\"", with: "\\\"")
        let script = """
        #!/usr/bin/env bash
        # Generated from i-dream pattern · \(pat.category) · \(pat.stableKey)
        # Hook event: UserPromptSubmit (default — adjust as needed)
        #
        # This is a SCAFFOLD. Review, edit, then move into ~/.claude/scripts/
        # and reference it from settings.json. The pattern below was extracted
        # from session transcripts and may need rephrasing.

        cat <<'HINT'
        💡 Reminder from i-dream: \(escaped)
        HINT
        """

        let settings = """
        {
          "hooks": {
            "UserPromptSubmit": [
              {
                "hooks": [
                  {
                    "type": "command",
                    "command": "bash ~/.claude/scripts/i-dream-\(m14_slug(pat.pattern)).sh"
                  }
                ]
              }
            ]
          }
        }
        """
        return (script, settings)
    }

    private func m14_renderHookReadme(for pat: Pattern) -> String {
        return """
        # Hook scaffold — \(pat.category)

        Generated from i-dream pattern `\(pat.stableKey)`:

        > \(pat.pattern)

        ## Files

        - `hook.sh` — the hook script (chmod +x already applied)
        - `settings-snippet.json` — paste-ready snippet for `~/.claude/settings.json`

        ## Install

        1. Move `hook.sh` to `~/.claude/scripts/i-dream-<name>.sh`
        2. Merge `settings-snippet.json` into your `~/.claude/settings.json`
           (under the appropriate event — `UserPromptSubmit`, `PreToolUse`, etc.)
        3. Test by triggering the event and watching the hook output

        Adjust the event type and command as needed — the default is a
        `UserPromptSubmit` hint injector, which is the safest starting
        point.
        """
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

// ─── Domain registry bridge ───────────────────────────────────────────────────
// Bridges to the Rust CLI `i-dream domain list --json` (docs/14 plugin system,
// Stage 1). Used by the menu's Dream Domains submenu to enumerate every
// registered DreamDomain without baking the list into Swift.

private struct DomainEntry: Codable {
    let name: String
    let kind: String
    let description: String
    let cadence: String
}

/// Resolve the `i-dream` binary by probing common install paths. Falls back
/// to plain `i-dream` (which then relies on PATH).
private func resolveIDreamBinary() -> String {
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    let candidates = [
        "\(home)/.cargo/bin/i-dream",
        "\(home)/.local/bin/i-dream",
        "/usr/local/bin/i-dream",
        "/opt/homebrew/bin/i-dream",
    ]
    for c in candidates where FileManager.default.isExecutableFile(atPath: c) {
        return c
    }
    return "i-dream"
}

// ── Today digest counts ──────────────────────────────────────────────────────
// Parses ~/.claude/i-dream/daily/latest.md (the L2 daily digest) into
// per-section item counts for the widget Today submenu. Stateless re-read
// per menu open; if the file is missing, returns nil so the menu can
// render an actionable placeholder.

private struct TodayDigestCounts {
    let date: String
    /// Sections in declaration order, paired with item counts.
    let itemized: [(String, Int)]
}

private func loadTodayDigestCounts() -> TodayDigestCounts? {
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    let path = "\(home)/.claude/i-dream/daily/latest.md"
    guard let content = try? String(contentsOfFile: path, encoding: .utf8) else {
        return nil
    }
    // Extract date from H1: "# 2026-05-16 — i-dream daily"
    let firstLine = content.split(separator: "\n").first.map(String.init) ?? ""
    let date = firstLine
        .replacingOccurrences(of: "# ", with: "")
        .components(separatedBy: " —")
        .first ?? "?"

    // Walk sections — every "## " starts a new section; count `- ` bullets
    // until the next "## " or "---" or EOF. A section whose body is only
    // `_(italic placeholder)_` counts as 0.
    var counts: [(String, Int)] = []
    var currentSection: String? = nil
    var currentBullets = 0
    var currentHasOnlyPlaceholder = true

    func flush() {
        if let s = currentSection {
            counts.append((s, currentHasOnlyPlaceholder ? 0 : currentBullets))
        }
    }

    for line in content.split(separator: "\n", omittingEmptySubsequences: false) {
        let t = line.trimmingCharacters(in: .whitespaces)
        if t.hasPrefix("## ") {
            flush()
            currentSection = String(t.dropFirst(3))
            currentBullets = 0
            currentHasOnlyPlaceholder = true
        } else if t == "---" {
            flush()
            currentSection = nil
        } else if t.hasPrefix("- ") {
            currentBullets += 1
            currentHasOnlyPlaceholder = false
        } else if !t.isEmpty && !t.hasPrefix("_(") && !t.hasPrefix("### ") {
            // Real content (not italic placeholder, not subsection heading)
            currentHasOnlyPlaceholder = false
        }
    }
    flush()
    return TodayDigestCounts(date: date, itemized: counts)
}

/// Invoke `i-dream domain list --json` and parse the result. Returns nil if
/// the CLI is missing, exits non-zero, or emits malformed JSON. Treat nil
/// as "couldn't load" — the menu shows a placeholder.
private func loadRegisteredDomains() -> [DomainEntry]? {
    let task = Process()
    task.launchPath = resolveIDreamBinary()
    task.arguments = ["domain", "list", "--json"]
    let stdout = Pipe()
    let stderr = Pipe()
    task.standardOutput = stdout
    task.standardError = stderr
    do {
        try task.run()
    } catch {
        return nil
    }
    // Drain stdout before waiting — avoids a pipe-buffer deadlock if output ever
    // exceeds ~64KB (see readReflect).
    let data = stdout.fileHandleForReading.readDataToEndOfFile()
    task.waitUntilExit()
    guard task.terminationStatus == 0 else { return nil }
    return try? JSONDecoder().decode([DomainEntry].self, from: data)
}

// ─── Outcome readers (is the dreaming actually helping?) ───────────────────────

/// One recurring mistake pattern from `i-dream reflect --json`.
struct ReflectPattern: Decodable {
    let slug:     String
    let severity: String
    let total:    Int
    let trend:    String   // landing | worsening | persisting | dormant
}

/// Aggregate "is my Claude getting sharper?" counts across recurring patterns.
struct ReflectSummary: Decodable {
    let total:      Int
    let landing:    Int
    let worsening:  Int
    let persisting: Int
    let dormant:    Int
}

struct ReflectData: Decodable {
    let summary:  ReflectSummary
    let patterns: [ReflectPattern]
}

/// Run `i-dream reflect --json` and decode it. Callers MUST be off the main
/// thread (it spawns a subprocess). nil on any failure — the menu just omits
/// the outcome line rather than blocking or erroring.
private func readReflect() -> ReflectData? {
    let task = Process()
    task.launchPath = resolveIDreamBinary()
    task.arguments  = ["reflect", "--json"]
    let stdout = Pipe()
    let stderr = Pipe()
    task.standardOutput = stdout
    task.standardError  = stderr
    do { try task.run() } catch { return nil }
    // Drain stdout BEFORE waiting: if output ever exceeds the ~64KB pipe buffer
    // the child blocks on write while we'd block on exit — a deadlock that would
    // wedge the DataStore reload. readDataToEndOfFile returns at child EOF.
    let data = stdout.fileHandleForReading.readDataToEndOfFile()
    task.waitUntilExit()
    guard task.terminationStatus == 0 else { return nil }
    return try? JSONDecoder().decode(ReflectData.self, from: data)
}

/// Whether a weekly review is staged and waiting. The audit's non-interactive
/// path writes ~/.claude/i-dream/.review-pending (body = the audit date) when it
/// stages proposals; `i-dream review` clears it once they're handled. Returns
/// the date string (possibly empty) when pending, nil when not.
private func readReviewPending() -> String? {
    let path = home + "/.claude/i-dream/.review-pending"
    guard let body = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
    return body.trimmingCharacters(in: .whitespacesAndNewlines)
}

// ─── DataStore ─────────────────────────────────────────────────────────────────

/// Everything the menu and HUD render, gathered as one value. Built off the main
/// thread and published on main, so the menu paints from a ready snapshot
/// instead of doing ~8 disk reads + 2 subprocesses synchronously each time it
/// opens (the old cause of the slow dropdown + the menu-open freeze).
private struct DataSnapshot {
    var running         = false
    var state:          DaemonState?
    var board:          BoardData?
    var patterns:       [Pattern]      = []
    var journal:        [JournalEntry] = []
    var storeFiles:     [StoreFile]    = []
    var digest:         String?
    var frequencyHours: Double?
    var patternCount    = 0
    var highConfCount   = 0
    var signals         = 0              // user-signal count (was a sync read on menu-open)
    var todayCounts:    TodayDigestCounts?  // today's digest (was a sync read on menu-open)
    var lastActivity:   Date?            // activity-file mtime (drives "Last active" + next-dream)
    var digestSentiment = "neutral"      // colours the digest line (was a sync read on menu-open)
    var domains:        [DomainEntry]?   // was the blocking call inside the menu path
    var reflect:        ReflectData?     // outcome: is the guidance landing?
    var reviewPending:  String?          // non-nil → a weekly review is staged
}

/// Single owner of all `~/.claude/{subconscious,i-dream}` reads. One off-main
/// `reload()` replaces the two identical synchronous read blocks that used to
/// live in both `refresh` and `menuNeedsUpdate`.
private final class DataStore {
    static let shared = DataStore()
    private(set) var snapshot = DataSnapshot()
    private let queue = DispatchQueue(label: "dev.i-dream.datastore", qos: .utility)
    private var loading = false

    /// Gather everything off-main, then publish the snapshot and run `then` on
    /// the main thread. MUST be called from the main thread — `loading` is
    /// touched only here, so single-threaded access keeps it race-free.
    ///
    /// If a reload is already in flight this coalesces rather than piling up: no
    /// second load starts, and `then` runs immediately against the *current*
    /// snapshot (not after the in-flight load). Callers keeping the snapshot warm
    /// pass no `then`; refresh's `then` re-reads cached fields and tolerates the
    /// rare ≤30s-stale value the next tick corrects.
    func reload(then: (() -> Void)? = nil) {
        if loading {
            then?()
            return
        }
        loading = true
        queue.async {
            var s = DataSnapshot()
            s.running        = isDaemonRunning()
            s.state          = readState()
            s.board          = readBoard()
            s.journal        = recentJournal(limit: 20)
            s.storeFiles     = readStoreFiles()
            s.digest         = readInsightDigest()
            s.frequencyHours = readDreamFrequency()
            let all          = allPatterns()           // read+decode patterns.json once
            s.patterns       = Array(all.suffix(5))    // recent 5 (was a redundant 2nd read)
            s.patternCount   = all.count
            s.highConfCount  = all.filter { $0.confidence >= 0.8 }.count
            s.signals        = signalsCount()
            s.todayCounts    = loadTodayDigestCounts()
            s.lastActivity   = lastActivityDate()
            s.digestSentiment = readDigestSentiment()
            s.domains        = loadRegisteredDomains()
            s.reflect        = readReflect()
            s.reviewPending  = readReviewPending()
            DispatchQueue.main.async {
                self.snapshot = s
                self.loading  = false
                then?()
            }
        }
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
    /// Cached full journal for the HUD time-range view (read from disk, not the 20-entry menubar cache)
    private var hudFullJournal:    [JournalEntry] = []
    private var hudFullJournalAt:  Date           = .distantPast
    /// Cached process-resource sample (sampled every ~5s)
    private var hudProcSample:     String         = "—"
    private var hudProcSampleAt:   Date           = .distantPast
    /// Hover-label that shows the action name when the mouse is over an
    /// action button or a bar in the bar chart. Cleared on exit.
    private var hudHoverLabel:    NSTextField?

    private var insightFeedbackDelegate: InsightFeedbackDelegate?

    // Comprehensive dashboard
    private var dashboardController: DashboardWindowController?

    // Dreaming animation
    private var isCycling       = false
    private var cycleStartTime: Date?
    private var animFrame       = 0
    private var animTimer:      Timer?

    /// throttle for the briefing-state poll. refresh() ticks
    /// roughly once a minute; we only want to check briefing state every
    /// ~5 minutes (a Sunday briefing fires at most once per ISO week
    /// anyway).
    private var briefingCheckCounter = 0

    // Persistent menu instance (rebuilt in-place via NSMenuDelegate)
    private var theMenu: NSMenu!

    func applicationDidFinishLaunching(_ note: Notification) {
        // Default-pin to dark (brand identity), but the dashboard's theme
        // picker can override per-panel (light / dark / system). Process
        // default still wins for the HUD + menubar — those are always
        // dark regardless of system theme. The dashboard panel's
        // .appearance overrides the process default for that panel only.
        NSApp.appearance = NSAppearance(named: .darkAqua)

        // no permission request needed — using osascript fallback
        // (see top-of-file comment). Authorization on macOS for shell
        // notifications happens once via System Settings → Notifications
        // → Script Editor → allow notifications, but the first call will
        // also prompt the user inline.

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

        // SIGUSR1 → open dashboard (sent by `i-dream dashboard` CLI)
        signal(SIGUSR1, SIG_IGN)
        let usr1Src = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
        usr1Src.setEventHandler { [weak self] in self?.openDashboard() }
        usr1Src.resume()
    }

    // Called by AppKit right before the menu is shown. Paints immediately from
    // the latest snapshot — NO synchronous disk reads or subprocesses on the
    // main thread (that synchronous block, plus the blocking domain-list call in
    // populateMenuItems, was the slow-dropdown + menu-open-freeze cause). The 30s
    // timer keeps the snapshot fresh; we also kick a background reload here so
    // the *next* open reflects very recent changes.
    func menuNeedsUpdate(_ menu: NSMenu) {
        let s = DataStore.shared.snapshot
        cachedRunning          = s.running
        cachedState            = s.state
        cachedBoard            = s.board
        cachedPatterns         = s.patterns
        cachedJournal          = s.journal
        cachedStoreFiles       = s.storeFiles
        cachedDigest           = s.digest
        cachedFrequencyHours   = s.frequencyHours
        cachedPatternCount     = s.patternCount
        cachedHighConfCount    = s.highConfCount
        updateButton()
        menu.removeAllItems()
        populateMenuItems(menu)
        DataStore.shared.reload()
    }

    @objc func refresh() {
        DataStore.shared.reload { [weak self] in
            guard let self = self else { return }
            let s = DataStore.shared.snapshot
            self.cachedRunning        = s.running
            self.cachedState          = s.state
            self.cachedBoard          = s.board
            self.cachedPatterns       = s.patterns
            self.cachedJournal        = s.journal
            self.cachedStoreFiles     = s.storeFiles
            self.cachedDigest         = s.digest
            self.cachedFrequencyHours = s.frequencyHours
            self.cachedPatternCount   = s.patternCount
            self.cachedHighConfCount  = s.highConfCount
            dlog("refresh: running=\(s.running) cycles=\(s.state?.totalCycles ?? -1)")
            self.checkCycleCompletion()
            // Poll briefing state every ~5 refreshes (~5min): reads
            // dreams/briefings/state.json, compares last_iso_week to
            // UserDefaults; if changed → notify.
            self.briefingCheckCounter += 1
            if self.briefingCheckCounter >= 5 {
                self.briefingCheckCounter = 0
                self.checkForNewBriefing()
            }
            self.updateButton()
            // Keep HUD current if visible
            if let panel = self.hudPanel, let tv = panel.contentView?.subviews.first as? NSTextView {
                self.updateHUDContent(tv)
            }
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

    /// Word-wrap a long insight string so it spans a few lines in the menu
    /// instead of forcing the whole dropdown absurdly wide. Continuation lines
    /// get a hanging indent so they sit under the first line's text.
    private func wrapForMenu(_ text: String, width: Int = 58, indent: String = "     ") -> String {
        var lines: [String] = []
        var line = ""
        for word in text.split(separator: " ", omittingEmptySubsequences: true) {
            if line.isEmpty {
                line = String(word)
            } else if line.count + 1 + word.count <= width {
                line += " " + word
            } else {
                lines.append(line)
                line = String(word)
            }
        }
        if !line.isEmpty { lines.append(line) }
        return lines.enumerated()
            .map { idx, l in idx == 0 ? l : indent + l }
            .joined(separator: "\n")
    }

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

        // ─ Outcome: is the dreaming actually making Claude sharper? ────────────
        // The point of the whole system — surfaced first, not buried under
        // activity counts. A weekly review waiting is the one actionable thing;
        // the landing/worsening line is the at-a-glance "is it working?".
        let snap = DataStore.shared.snapshot
        if let pending = snap.reviewPending {
            let label = pending.isEmpty
                ? "Weekly review pending — open"
                : "Weekly review pending (\(pending)) — open"
            let rev = add(menu, label, #selector(openWeeklyReview), key: "")
            setIcon(rev, "tray.full.fill")
        }
        if let reflect = snap.reflect, reflect.summary.total > 0 {
            let sum = reflect.summary
            let outcomeColor: NSColor = sum.worsening > 0 ? .systemOrange : .systemGreen
            addRow(menu, "  Mistakes", "\(sum.landing) landing · \(sum.worsening) worsening",
                   valueColor: outcomeColor)
            // Name the worst recurrence so it's actionable, not just a number.
            if let worst = reflect.patterns.first(where: { $0.trend == "worsening" }) {
                addDim(menu, "  ↑ \(worst.slug)")
            }
        }

        // ─ Daemon controls ────────────────────────────────────────────────────
        if running {
            let s = add(menu, "Stop Daemon", #selector(stopDaemon), key: "s")
            setIcon(s, "stop.fill")
        } else {
            let s = add(menu, "Start Daemon", #selector(startDaemon), key: "s")
            setIcon(s, "play.fill")
        }
        let t = add(menu, "Trigger Dream Cycle", #selector(triggerCycleWithUsageCheck), key: "t")
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
            addRow(menu, "Tokens used", fmtNum(s.totalTokensUsed), valueColor: .systemBlue)
            addRow(menu, "Last run",    fmtDateWithAge(s.lastConsolidation))
            // last_activity in state.json is always null — use the activity-file
            // mtime from the snapshot instead.
            let lastAct = DataStore.shared.snapshot.lastActivity
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
            let sigs = DataStore.shared.snapshot.signals
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
        // Next dream = last activity + the frequency threshold (same activity-file
        // mtime the snapshot already holds — no extra stat on the menu path).
        let nextDream = DataStore.shared.snapshot.lastActivity?.addingTimeInterval(effectiveHz * 3600)
        let nextStr   = nextDream.map { fmtCountdown($0) } ?? "—"
        addRow(menu, "  Frequency", freqLabel, valueColor: .systemBlue)
        addRow(menu, "  Next dream", nextStr)

        // Submenu with frequency choices
        let freqMenu = NSMenu()
        // A few sensible presets, not a 12-option wall. The exact hour is still
        // settable via Edit Config for anyone who wants a non-preset value.
        var freqOptions: [(label: String, hours: Double)] = [
            ("1 hour",            1.0),
            ("2 hours",           2.0),
            ("4 hours (default)", 4.0),
            ("12 hours",         12.0),
        ]
        // Keep the current value visible/checkable even if it isn't a preset.
        let curHz = cachedFrequencyHours ?? 4.0
        if !freqOptions.contains(where: { $0.hours == curHz }) {
            let curLabel = curHz < 1.0 ? "\(Int(curHz * 60)) minutes (current)"
                                       : "\(curHz.rounded() == curHz ? String(Int(curHz)) : String(format: "%.1f", curHz)) hours (current)"
            freqOptions.append((curLabel, curHz))
        }
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

        // ─ Dream Domains ──────────────────────────────────────────────────────
        // Every registered DreamDomain (the docs/14 plugin system), read from the
        // snapshot — DataStore ran `i-dream domain list --json` off-main, so this
        // no longer blocks the menu open. Read-only listing.
        let registered = DataStore.shared.snapshot.domains
        let domainsMenu = NSMenu()
        if let domains = registered, !domains.isEmpty {
            for d in domains {
                let item = NSMenuItem()
                let pad = String(repeating: " ", count: max(0, 18 - d.name.count))
                item.attributedTitle = NSAttributedString(
                    string: "  \(d.name)\(pad) \(d.cadence)",
                    attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
                        .foregroundColor: NSColor.labelColor,
                    ])
                item.toolTip = "\(d.kind) — \(d.description)"
                domainsMenu.addItem(item)
            }
        } else {
            let err = NSMenuItem(
                title: "  (could not load — is the i-dream binary on PATH?)",
                action: nil, keyEquivalent: "")
            err.isEnabled = false
            domainsMenu.addItem(err)
        }
        let domainCount = registered?.count ?? 0
        let domainsParent = NSMenuItem(
            title: "  Dream Domains (\(domainCount)) →",
            action: nil, keyEquivalent: "")
        setIcon(domainsParent, "circle.grid.3x3.fill")
        menu.addItem(domainsParent)
        menu.setSubmenu(domainsMenu, for: domainsParent)

        // ─ Today ───────────────────────────
        // Today's digest counts (~/.claude/i-dream/daily/latest.md, 7 fixed
        // sections) + an "Open full digest" action. Read off-main by DataStore;
        // the file is written by `i-dream digest` (manual) or the daily cron.
        let todayCounts = DataStore.shared.snapshot.todayCounts
        let todayMenu = NSMenu()
        if let counts = todayCounts {
            for (section, count) in counts.itemized {
                let item = NSMenuItem()
                let pad = String(repeating: " ", count: max(0, 26 - section.count))
                item.attributedTitle = NSAttributedString(
                    string: "  \(section)\(pad) \(count)",
                    attributes: [
                        .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
                        .foregroundColor: count > 0 ? NSColor.labelColor : NSColor.tertiaryLabelColor,
                    ])
                todayMenu.addItem(item)
            }
            todayMenu.addItem(.separator())
            let open = NSMenuItem(title: "  Open full digest", action: #selector(openTodaysDigest), keyEquivalent: "")
            open.target = self
            todayMenu.addItem(open)
            let regen = NSMenuItem(title: "  Regenerate (run `i-dream digest`)", action: #selector(regenerateTodaysDigest), keyEquivalent: "")
            regen.target = self
            todayMenu.addItem(regen)
        } else {
            let err = NSMenuItem(
                title: "  (no digest yet — run `i-dream digest`)",
                action: nil, keyEquivalent: "")
            err.isEnabled = false
            todayMenu.addItem(err)
        }
        let todayDateStr = todayCounts?.date ?? "no digest"
        let todayParent = NSMenuItem(
            title: "  Today (\(todayDateStr)) →",
            action: nil, keyEquivalent: "")
        setIcon(todayParent, "calendar")
        menu.addItem(todayParent)
        menu.setSubmenu(todayMenu, for: todayParent)

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
                let sentiment = DataStore.shared.snapshot.digestSentiment
                let sentimentColor: NSColor = sentiment == "positive" ? .systemGreen
                                           : sentiment == "negative" ? .systemOrange
                                           : .labelColor
                let digestItem = NSMenuItem()
                let digestAttr = NSMutableAttributedString()
                let truncDigest = digest.count > 220 ? String(digest.prefix(217)) + "…" : digest
                digestAttr.append(NSAttributedString(string: "  \(wrapForMenu(truncDigest))\n",
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
                    let truncated = p.pattern.count > 200 ? String(p.pattern.prefix(197)) + "…" : p.pattern
                    let sym  = valenceSymbol(p.valence)
                    // Confidence colour: green ≥85%, blue ≥65%, muted <65%
                    let confColor: NSColor = p.confidence >= 0.85 ? .systemGreen
                                          : p.confidence >= 0.65 ? .systemBlue
                                          : .secondaryLabelColor
                    let confDot = p.confidence >= 0.85 ? "●" : p.confidence >= 0.65 ? "◕" : "○"
                    let dateStr = p.firstSeen != nil ? "  ·  \(fmtDateWithAge(p.firstSeen))" : ""
                    let item = NSMenuItem()
                    let full = NSMutableAttributedString()
                    full.append(NSAttributedString(string: "  \(sym) \"\(wrapForMenu(truncated))\"\n",
                                                   attributes: [.font: NSFont.systemFont(ofSize: 13)]))
                    full.append(NSAttributedString(string: "  \(confDot) \(Int(p.confidence * 100))%  ·  \(p.category)\(dateStr)",
                                                   attributes: [
                                                       .font: NSFont.systemFont(ofSize: 11),
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
            let truncErr = err.count > 200 ? String(err.prefix(197)) + "…" : err
            errFull.append(NSAttributedString(string: "  " + wrapForMenu(truncErr) + "\n",
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

        let dash = add(menu, "Open Dashboard", #selector(openDashboard), key: "d")
        setIcon(dash, "chart.bar.doc.horizontal.fill")

        // Help/About lost their dashboard tabs in the v3 cutover — they open
        // as small panels from here now.
        let helpItem = add(menu, "Help & Shortcuts", #selector(openHelpPanel))
        setIcon(helpItem, "questionmark.circle.fill")
        let aboutItem = add(menu, "About i-dream", #selector(openAboutPanel))
        setIcon(aboutItem, "info.circle.fill")

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

        let q = NSMenuItem(title: "Quit",
                           action: #selector(NSApplication.terminate(_:)),
                           keyEquivalent: "q")
        setIcon(q, "power")
        menu.addItem(q)
    }

    // ── Menu item helpers ──────────────────────────────────────────────────────

    @discardableResult
    private func add(_ menu: NSMenu, _ title: String, _ sel: Selector, key: String = "") -> NSMenuItem {
        // `key` adds a ⌘-prefixed keyboard shortcut shown in the menu
        // (mirrors the claude-instances pattern). Empty string = no
        // shortcut. Standard convention: lowercase = ⌘key, uppercase = ⌘⇧key.
        let i = NSMenuItem(title: title, action: sel, keyEquivalent: key)
        i.attributedTitle = NSAttributedString(string: title,
                                               attributes: [.font: NSFont.systemFont(ofSize: 14)])
        i.target = self; i.isEnabled = true
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
            let rateBtn = NSButton(title: "Rate Insights →", target: self,
                                   action: #selector(showInsightsFeedback))
            rateBtn.frame      = NSRect(x: 12, y: 8, width: 130, height: 32)
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




    @objc private func closeAssociationNetworkPanel() {
        associationNetworkPanel?.close()
        associationNetworkPanel = nil
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
        if let btn = hudPinBtn {
            let sym = nextOn ? "pin.fill" : "pin.slash.fill"
            if let img = NSImage(systemSymbolName: sym, accessibilityDescription: nil) {
                btn.image = img
                btn.imagePosition = .imageOnly
                btn.title = ""
            } else {
                btn.title = nextOn ? "📌" : "📍"
            }
            btn.contentTintColor = nextOn ? NSColor.systemYellow : NSColor.tertiaryLabelColor
        }
    }

    @objc private func cycleHUDTimeRange() {
        hudTimeRangeIndex = (hudTimeRangeIndex + 1) % 3
        let labels = ["7d", "30d", "∞"]
        hudTimeRangeBtn?.title = labels[hudTimeRangeIndex]
        // Force a fresh journal read so the new range actually returns different data
        // (the menubar's cachedJournal is capped at 20 entries — too small to differentiate ranges).
        hudFullJournalAt = .distantPast
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
        let h: CGFloat       = 396  // grew further for HUD task #7 quick-jump cells row
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
        // Fully opaque panel alpha: the old 0.94, stacked on the gradient's
        // own alpha, let bright text from windows underneath ghost through
        // the HUD legibly (field-study J8). The gradient below keeps a hint
        // of depth without readable bleed.
        panel.alphaValue                  = 1.0
        panel.hasShadow                   = true
        panel.isOpaque                    = false
        panel.collectionBehavior          = [.canJoinAllSpaces, .stationary]
        panel.titlebarAppearsTransparent  = true
        // Pin the HUD to dark appearance regardless of system theme. The
        // floating widget has its own brand identity (dark navy gradient,
        // cyan/purple accents, glow on hover) — letting it follow system
        // light/dark would blow out the whole palette and was reported as
        // a hard bug. NSAppearance applies to all child views.
        panel.appearance                  = NSAppearance(named: .darkAqua)

        // Replace the auto-created contentView with a custom one that catches right-click
        // and forwards it to the menubar menu (theMenu).
        let custom = HUDContentView(frame: NSRect(x: 0, y: 0, width: w, height: h))
        custom.delegate = self
        panel.contentView = custom

        // ── Layer: gradient bg + rounded corners + pulsing blue border ───────
        if let cv = panel.contentView {
            cv.wantsLayer           = true
            cv.layer?.cornerRadius  = cornerR
            cv.layer?.masksToBounds = true

            let grad = CAGradientLayer()
            grad.frame = cv.bounds
            grad.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
            grad.colors = [
                NSColor(red: 0.06, green: 0.10, blue: 0.18, alpha: 0.985).cgColor,
                NSColor(red: 0.02, green: 0.04, blue: 0.09, alpha: 1.0).cgColor,
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

        let btnH:    CGFloat = 22
        let actionH: CGFloat = 30   // action button row at very bottom
        let actionY: CGFloat = 6
        let hoverH:  CGFloat = 14   // hover-label slot above the action row
        let hoverY:  CGFloat = actionY + actionH + 2
        // HUD task #7: quick-jump row of 4 cell-buttons that open the
        // dashboard at a specific tab (Patterns/Associations/Insights/Metacog).
        let jumpH:   CGFloat = 22
        let jumpY:   CGFloat = hoverY + hoverH + 2
        let chartH:  CGFloat = 50
        let chartY:  CGFloat = jumpY + jumpH + 6
        let tvY:     CGFloat = chartY + chartH + 4
        let tvH:     CGFloat = h - tvY - btnH - 6

        // Text view — stats
        let tv = NSTextView(frame: NSRect(x: 12, y: tvY, width: w - 24, height: tvH))
        tv.isEditable      = false
        tv.isSelectable    = false
        tv.backgroundColor = .clear
        tv.drawsBackground = false
        panel.contentView?.addSubview(tv)

        // Bar chart view — token history
        let chart = MiniBarChartView(frame: NSRect(x: 12, y: chartY, width: w - 24, height: chartH))
        chart.delegate = self
        panel.contentView?.addSubview(chart)
        hudBarChart = chart

        // ── HUD task #7: quick-jump cell row ─────────────────────────────────
        // Four small icon-only HoverButtons sitting between the hover-label
        // slot and the bar chart. Each opens the dashboard scrolled to the
        // matching tab via showOrFront(tab:). Same HoverButton class as the
        // action row so they get hover-bg + tooltip + the hover-label
        // animation for free.
        let jumps: [(symbol: String, label: String, tint: NSColor, sel: Selector)] = [
            ("brain",                 "Patterns →",     NSColor.systemTeal,    #selector(openDashboardPatterns)),
            ("link",                  "Associations →", NSColor.systemOrange,  #selector(openDashboardAssociations)),
            ("sparkles",              "Insights →",     NSColor.systemYellow,  #selector(openDashboardInsights)),
            ("checkmark.seal.fill",   "Metacog →",      NSColor.systemPink,    #selector(openDashboardMetacog)),
        ]
        let jGap:    CGFloat = 6
        let jTotalGap = jGap * CGFloat(jumps.count + 1)
        let jBtnW   = (w - jTotalGap) / CGFloat(jumps.count)
        for (i, j) in jumps.enumerated() {
            let bx = jGap + CGFloat(i) * (jBtnW + jGap)
            let b = HoverButton(frame: NSRect(x: bx, y: jumpY, width: jBtnW, height: jumpH))
            b.hoverLabel = j.label
            b.delegate   = self
            b.tintColor  = j.tint
            b.toolTip    = j.label
            if let img = NSImage(systemSymbolName: j.symbol, accessibilityDescription: j.label) {
                let cfg = NSImage.SymbolConfiguration(pointSize: 12, weight: .medium)
                b.image = img.withSymbolConfiguration(cfg) ?? img
                b.imagePosition = .imageOnly
                b.contentTintColor = j.tint
            } else {
                b.title = j.label
            }
            b.target = self
            b.action = j.sel
            panel.contentView?.addSubview(b)
        }

        // ── Bottom action button row ─────────────────────────────────────────
        // 4 evenly-spaced HoverButtons with SF-symbol icons. Each carries a
        // distinct semantic tint (cyan/blue/green-or-orange/grey) and shows
        // its label in the hover-label slot above the row on mouseEnter.
        let actions: [(symbol: String, label: String, tint: NSColor, sel: Selector)] = [
            ("rectangle.stack.fill.badge.person.crop", "Open Dashboard",
             NSColor.systemCyan,
             #selector(openDashboard)),
            ("moon.stars.fill",                        "Trigger Dream Cycle",
             NSColor.systemPurple,
             #selector(triggerCycleWithUsageCheck)),
            (cachedRunning ? "stop.circle.fill" : "play.circle.fill",
             cachedRunning ? "Stop Daemon" : "Start Daemon",
             cachedRunning ? NSColor.systemOrange : NSColor.systemGreen,
             cachedRunning ? #selector(stopDaemon) : #selector(startDaemon)),
            ("ellipsis.circle.fill",                   "More… (or right-click anywhere)",
             NSColor.secondaryLabelColor,
             #selector(showHUDActionsMenu(_:))),
        ]
        let nBtns  = CGFloat(actions.count)
        let gap:   CGFloat = 8
        let totalGap = gap * (nBtns + 1)
        let btnW   = (w - totalGap) / nBtns
        for (i, a) in actions.enumerated() {
            let bx = gap + CGFloat(i) * (btnW + gap)
            let b = HoverButton(frame: NSRect(x: bx, y: actionY, width: btnW, height: actionH))
            b.hoverLabel = a.label
            b.delegate   = self
            b.tintColor  = a.tint
            b.toolTip    = a.label
            if let img = NSImage(systemSymbolName: a.symbol, accessibilityDescription: a.label) {
                let cfg = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
                b.image = img.withSymbolConfiguration(cfg) ?? img
                b.imagePosition = .imageOnly
                b.contentTintColor = a.tint
            } else {
                b.title = a.label
            }
            b.target = self
            b.action = a.sel
            panel.contentView?.addSubview(b)
        }

        // ── Hover label slot just above the action row ───────────────────────
        // Single NSTextField positioned between the bar chart and the button
        // row. HoverButton + MiniBarChartView write into it on mouseEnter and
        // clear it on mouseExit. Stays empty when the cursor is idle.
        let hoverLabel = NSTextField(labelWithString: "")
        hoverLabel.frame = NSRect(x: 12, y: hoverY, width: w - 24, height: hoverH)
        hoverLabel.font = NSFont.systemFont(ofSize: 11, weight: .medium)
        hoverLabel.textColor = NSColor.tertiaryLabelColor
        hoverLabel.alignment = .center
        hoverLabel.backgroundColor = .clear
        hoverLabel.drawsBackground = false
        hoverLabel.isBordered = false
        // Layer-backed so we can animate opacity (CALayer) and tint a
        // semi-transparent background that matches the HUD gradient.
        hoverLabel.wantsLayer = true
        hoverLabel.layer?.backgroundColor = NSColor(
            red: 0.04, green: 0.07, blue: 0.13, alpha: 0.85
        ).cgColor
        hoverLabel.layer?.cornerRadius = 5
        hoverLabel.layer?.opacity      = 0.0   // hidden until first hover
        panel.contentView?.addSubview(hoverLabel)
        hudHoverLabel = hoverLabel

        // ── Top toolbar buttons ───────────────────────────────────────────────
        // Close button — top-left, SF Symbol xmark.circle.fill
        let closeBtn = NSButton(frame: NSRect(x: 6, y: h - btnH, width: 22, height: btnH))
        closeBtn.bezelStyle       = .inline
        closeBtn.isBordered       = false
        if let img = NSImage(systemSymbolName: "xmark.circle.fill",
                             accessibilityDescription: "Close") {
            closeBtn.image = img
            closeBtn.imagePosition = .imageOnly
        } else {
            closeBtn.title = "✕"
        }
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

        // Pin button — top-right, SF Symbol pin.fill / pin.slash.fill
        let pinBtn = NSButton(frame: NSRect(x: w - 30, y: h - btnH, width: 24, height: btnH))
        pinBtn.bezelStyle       = .inline
        pinBtn.isBordered       = false
        let pinSymbol = onTop ? "pin.fill" : "pin.slash.fill"
        if let img = NSImage(systemSymbolName: pinSymbol,
                             accessibilityDescription: onTop ? "Always on top" : "Floating") {
            pinBtn.image = img
            pinBtn.imagePosition = .imageOnly
        } else {
            pinBtn.title = onTop ? "📌" : "📍"
        }
        pinBtn.contentTintColor = onTop ? NSColor.systemYellow : NSColor.tertiaryLabelColor
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
    /// Reads the FULL journal (not the menubar's 20-entry cache) so 7d/30d/∞ are
    /// actually distinguishable. Cached for 30s to avoid disk reads on every tick.
    private func hudFilteredJournal() -> [JournalEntry] {
        if Date().timeIntervalSince(hudFullJournalAt) > 30 {
            hudFullJournal   = allJournal()
            hudFullJournalAt = Date()
        }
        let source = hudFullJournal.isEmpty ? cachedJournal : hudFullJournal
        guard !source.isEmpty else { return [] }
        switch hudTimeRangeIndex {
        case 0: // 7 days
            let cutoff = Date().addingTimeInterval(-7 * 86400)
            return source.filter { isoDate($0.timestamp).map { $0 >= cutoff } ?? true }
        case 1: // 30 days
            let cutoff = Date().addingTimeInterval(-30 * 86400)
            return source.filter { isoDate($0.timestamp).map { $0 >= cutoff } ?? true }
        default: // all
            return source
        }
    }

    /// Returns a compact one-line resource readout for i-dream processes.
    /// Format: "daemon 0.4% 32M · bar 0.2% 28M". Cached for 5s.
    private func hudProcessLoad() -> String {
        if Date().timeIntervalSince(hudProcSampleAt) < 5 { return hudProcSample }
        hudProcSampleAt = Date()
        func sample(_ pgrepArgs: [String], label: String) -> String? {
            let task = Process()
            task.launchPath = "/usr/bin/pgrep"
            task.arguments  = pgrepArgs
            let pipe = Pipe(); task.standardOutput = pipe; task.standardError = Pipe()
            do { try task.run(); task.waitUntilExit() } catch { return nil }
            let pidOut = String(data: pipe.fileHandleForReading.readDataToEndOfFile(),
                                encoding: .utf8) ?? ""
            let pids = pidOut.split(separator: "\n").compactMap { Int($0) }
            guard !pids.isEmpty else { return nil }
            // ps -o %cpu,rss -p PID,PID,...
            let ps = Process()
            ps.launchPath = "/bin/ps"
            ps.arguments  = ["-o", "%cpu=,rss=", "-p", pids.map(String.init).joined(separator: ",")]
            let p2 = Pipe(); ps.standardOutput = p2; ps.standardError = Pipe()
            do { try ps.run(); ps.waitUntilExit() } catch { return nil }
            let out = String(data: p2.fileHandleForReading.readDataToEndOfFile(),
                             encoding: .utf8) ?? ""
            var totalCPU = 0.0; var totalRSS = 0  // RSS in KB
            for line in out.split(separator: "\n") {
                let parts = line.split(separator: " ", omittingEmptySubsequences: true)
                if parts.count >= 2 {
                    totalCPU += Double(parts[0]) ?? 0
                    totalRSS += Int(parts[1]) ?? 0
                }
            }
            let mb = Double(totalRSS) / 1024.0
            let mbStr = mb >= 100 ? String(format: "%.0fM", mb) : String(format: "%.1fM", mb)
            return String(format: "%@ %.1f%% %@", label, totalCPU, mbStr)
        }
        var parts: [String] = []
        if let s = sample(["-f", "i-dream daemon"], label: "daemon") { parts.append(s) }
        if let s = sample(["-x", "i-dream-bar"],    label: "bar")    { parts.append(s) }
        hudProcSample = parts.isEmpty ? "—" : parts.joined(separator: " · ")
        return hudProcSample
    }

    /// read dreams/briefings/state.json and compare its
    /// last_iso_week against the previously-seen value in UserDefaults.
    /// On change → fire a UNUserNotification linking to the new
    /// briefing file. Silently degrades on read errors / missing file
    /// (briefings may not exist yet).
    private func checkForNewBriefing() {
        let path = subDir + "/dreams/briefings/state.json"
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let week = json["last_iso_week"] as? String, !week.isEmpty
        else { return }
        let lastSeen = UserDefaults.standard.string(forKey: lastSeenBriefingKey) ?? ""
        guard week != lastSeen else { return }
        // First run after install: don't fire for the historical state.
        // Just record the current week as seen and bail out.
        if lastSeen.isEmpty {
            UserDefaults.standard.set(week, forKey: lastSeenBriefingKey)
            dlog("briefing: priming lastSeen=\(week) (no notification on first run)")
            return
        }
        // New briefing — fire via osascript (works for unbundled processes).
        UserDefaults.standard.set(week, forKey: lastSeenBriefingKey)
        let briefingPath = subDir + "/dreams/briefings/\(week).md"
        let body: String = (try? String(contentsOfFile: briefingPath, encoding: .utf8))
            .map { String($0.prefix(120)).trimmingCharacters(in: .whitespacesAndNewlines) }
            ?? "Tap to read this week's briefing."
        // Sanitize — osascript double-quote strings can't contain unescaped
        // quotes, newlines, or backslashes.
        func escape(_ s: String) -> String {
            s.replacingOccurrences(of: "\\", with: "\\\\")
             .replacingOccurrences(of: "\"", with: "\\\"")
             .replacingOccurrences(of: "\n", with: " ")
             .replacingOccurrences(of: "\r", with: " ")
        }
        let title = "i-dream — \(week) briefing"
        let cmd = "display notification \"\(escape(body))\" with title \"\(escape(title))\" sound name \"Glass\""
        DispatchQueue.global(qos: .background).async {
            let task = Process()
            task.launchPath = "/usr/bin/osascript"
            task.arguments  = ["-e", cmd]
            do {
                try task.run()
                task.waitUntilExit()
                dlog("briefing notification fired for week \(week)")
            } catch {
                dlog("briefing notification osascript failed: \(error)")
            }
        }
    }

    /// Set the HUD hover-label text + colour. Called by HoverButton on
    /// mouseEnter/mouseExit and by MiniBarChartView during bar hover.
    /// The label has a CALayer-backed background tinted to match the HUD
    /// gradient; opacity is animated 0↔1 over 120ms so labels appear and
    /// vanish softly instead of snapping.
    func setHUDHoverLabel(_ text: String, color: NSColor) {
        guard let label = hudHoverLabel else { return }
        label.stringValue = text
        label.textColor   = color
        guard let layer = label.layer else { return }
        let target: Float = text.isEmpty ? 0.0 : 1.0
        // Avoid restarting an animation that's already at the target value
        // (mouseMoved fires many times per second on bar hover).
        if abs(layer.opacity - target) < 0.01 { return }
        let anim = CABasicAnimation(keyPath: "opacity")
        anim.fromValue      = layer.opacity
        anim.toValue        = target
        anim.duration       = 0.12
        anim.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        anim.fillMode       = .forwards
        anim.isRemovedOnCompletion = false
        layer.add(anim, forKey: "fade")
        layer.opacity = target
    }

    /// Bar-chart click → open the dashboard. The bar index is recorded in
    /// case a future iteration wants to pass `--cycle <index>` to the
    /// dashboard for deep-linking; for now we just bring the panel up.
    fileprivate func barChartClicked(at index: Int, entry: JournalEntry?) {
        if let e = entry {
            dlog("HUD bar-chart click: index=\(index) timestamp=\(e.timestamp) tokens=\(e.tokensUsed)")
        }
        openDashboard()
    }

    /// Exposes the menubar menu so the floating HUD can show it on right-click.
    @objc func popUpHUDContextMenu(with event: NSEvent, from view: NSView) {
        // theMenu has an NSMenuDelegate (menuNeedsUpdate) that rebuilds it on open,
        // so the right-click menu always shows current state.
        NSMenu.popUpContextMenu(theMenu, with: event, for: view)
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

    /// D8 polish — count intentions auto-promoted in the last 7 days.
    /// Detected by `action.source` starting with "D8 auto-promote" (the
    /// label set in ProspectiveModule::auto_promote_associations) and
    /// `created` within the past week. Lets the widget surface
    /// "12 auto-promoted this week" so the user sees the daemon's
    /// background work without opening the dashboard.
    private func recentlyAutoPromotedCount() -> Int {
        let path = subDir + "/intentions/registry.jsonl"
        guard let content = try? String(contentsOfFile: path, encoding: .utf8) else { return 0 }
        let weekAgo = Date().addingTimeInterval(-7 * 24 * 3600)
        return content.components(separatedBy: "\n").filter { line in
            guard !line.isEmpty,
                  let data   = line.data(using: .utf8),
                  let json   = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return false }
            // action.source is the provenance label from D8.
            guard let action = json["action"] as? [String: Any],
                  let source = action["source"] as? String,
                  source.hasPrefix("D8 auto-promote")
            else { return false }
            // Field is `created` in the on-disk schema (not `created_at`).
            guard let created = json["created"] as? String,
                  let createdDate = isoDate(created) else { return false }
            return createdDate >= weekAgo
        }.count
    }

    private func updateHUDContent(_ tv: NSTextView) {
        let dot: String     = isCycling ? "◉" : cachedRunning ? "◉" : "○"
        let dotColor: NSColor = isCycling ? dreamAnimColors[animFrame % dreamAnimColors.count]
                                          : cachedRunning ? .systemGreen : .systemOrange
        let buf   = NSMutableAttributedString()
        // Type scale collapsed to two sizes (was 3): TITLE 14sb, BODY 12m.
        // Tabular numerals for all numeric values so columns line up.
        // Status colors (green/orange) reserved for status meaning ONLY —
        // counts and KPIs use semantic .labelColor / .secondaryLabelColor.
        let fTitle: CGFloat = 14
        let fBody:  CGFloat = 12

        func label(_ text: String) {
            buf.append(NSAttributedString(string: text, attributes: [
                .font:            NSFont.systemFont(ofSize: fBody),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ]))
        }
        func value(_ text: String, color: NSColor = .labelColor, mono: Bool = true) {
            // Default mono-digit so all numeric values align in columns.
            let f: NSFont = mono
                ? NSFont.monospacedDigitSystemFont(ofSize: fBody, weight: .medium)
                : NSFont.systemFont(ofSize: fBody, weight: .medium)
            buf.append(NSAttributedString(string: text, attributes: [
                .font: f, .foregroundColor: color,
            ]))
        }

        // ── Line 1: status dot + name + cycle count or elapsed ───────────────
        buf.append(NSAttributedString(string: "\(dot) i-dream  ", attributes: [
            .font:            NSFont.systemFont(ofSize: fTitle, weight: .semibold),
            .foregroundColor: dotColor,
        ]))
        if isCycling, let start = cycleStartTime {
            buf.append(NSAttributedString(string: "dreaming \(fmtElapsed(Date().timeIntervalSince(start)))", attributes: [
                .font:            NSFont.monospacedDigitSystemFont(ofSize: fBody, weight: .regular),
                .foregroundColor: NSColor.systemCyan,
            ]))
        } else if let n = cachedState?.totalCycles {
            buf.append(NSAttributedString(string: "\(n) cycles", attributes: [
                .font:            NSFont.monospacedDigitSystemFont(ofSize: fBody, weight: .regular),
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
                .font:            NSFont.monospacedDigitSystemFont(ofSize: fBody, weight: .regular),
                .foregroundColor: gaugeColor,
            ]))
            buf.append(NSAttributedString(string: "\(spark)\n", attributes: [
                .font:            NSFont.monospacedDigitSystemFont(ofSize: fBody, weight: .regular),
                .foregroundColor: NSColor.secondaryLabelColor,
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
                // De-greenified per dashboard review — green is reserved for status
                // signaling, not category counts. High-conf goes in dim secondary.
                value("  (\(cachedHighConfCount) high-conf)\n", color: .secondaryLabelColor)
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
            // D8 polish — surface auto-promoted-this-week count next to
            // active total so the user sees the daemon's background work.
            let autoCount = recentlyAutoPromotedCount()
            if autoCount > 0 {
                value("\(intentCount) active  ")
                value("(+\(autoCount) auto/wk)\n", color: .systemGreen)
            } else {
                value("\(intentCount) active\n")
            }
        }

        // ── Line 7b: dreams today + avg tokens / cycle (filtered range) ───────
        // Two cheap stats derived from filteredJournal: count of cycles whose
        // timestamp is today (calendar day) and the mean token count across
        // the filtered window. Skipped silently when the journal is empty.
        if !filteredJournal.isEmpty {
            let cal = Calendar.current
            let todayStart = cal.startOfDay(for: Date())
            let dreamsToday = filteredJournal.filter { e in
                guard let d = isoDate(e.timestamp) else { return false }
                return d >= todayStart
            }.count
            let totalTokRange = filteredJournal.reduce(0) { $0 + $1.tokensUsed }
            let avgTok = totalTokRange / max(1, filteredJournal.count)
            let avgStr = avgTok >= 1000 ? "\(avgTok / 1000)k" : "\(avgTok)"
            label("today  "); value("\(dreamsToday) cycle\(dreamsToday == 1 ? "" : "s")    ")
            label("avg/cycle  "); value("\(avgStr)\n", mono: true)
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

        // ── Line 9: process resource load (i-dream daemon + bar widget) ──────
        let proc = hudProcessLoad()
        if proc != "—" {
            label("processes  ")
            value("\(proc)\n", color: .secondaryLabelColor, mono: true)
        }

        // ── Line 10: range hint — clarifies which stats are window-filtered ──
        let rangeLabels = ["7d", "30d", "all-time"]
        buf.append(NSAttributedString(string: "  load·spark·tokens: \(rangeLabels[hudTimeRangeIndex])\n",
            attributes: [
                .font:            NSFont.systemFont(ofSize: fBody - 1),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ]))

        // ── Line 11: error line (only if error is newer than last cycle) ─────
        if let err = cachedBoard?.lastError {
            buf.append(NSAttributedString(string: "⚠  \(err)", attributes: [
                .font:            NSFont.systemFont(ofSize: fBody - 1),
                .foregroundColor: NSColor.systemOrange,
            ]))
        }

        tv.textStorage?.setAttributedString(buf)

        // Push filtered token history to bar chart, including the entries
        // themselves so hover labels can read timestamp + tokens per bar.
        hudBarChart?.values  = filteredJournal.map { $0.tokensUsed }
        hudBarChart?.entries = filteredJournal
    }

    /// Wired to the HUD's "More…" button — pops up the same menubar menu next to the button.
    @objc private func showHUDActionsMenu(_ sender: NSButton) {
        let event = NSEvent.mouseEvent(
            with: .leftMouseDown,
            location: NSPoint(x: sender.bounds.minX, y: sender.bounds.minY),
            modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: sender.window?.windowNumber ?? 0,
            context: nil, eventNumber: 0, clickCount: 1, pressure: 1.0)
            ?? NSApp.currentEvent
        if let ev = event {
            NSMenu.popUpContextMenu(theMenu, with: ev, for: sender)
        }
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

    /// Open the weekly review — `i-dream review` spawns its own Ghostty + claude
    /// session seeded with the staged proposals, so just launch and return.
    @objc private func openWeeklyReview() {
        dlog("openWeeklyReview")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: resolveIDreamBinary())
        p.arguments = ["review"]
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        try? p.run()
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

    @objc private func openDashboard() {
        if dashboardController == nil {
            dashboardController = DashboardWindowController()
        }
        dashboardController!.showOrFront()
    }

    @objc private func openHelpPanel() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showInfoPanel(about: false)
    }
    @objc private func openAboutPanel() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showInfoPanel(about: true)
    }

    /// HUD task #7: openDashboard variants for the HUD cells. Each opens
    /// the dashboard at the matching v3 surface. Tab index map (v3):
    /// 0=Overview · 1=Browse · 2=Journal · 3=Search.
    @objc private func openDashboardPatterns() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showOrFront(tab: 1)
    }
    @objc private func openDashboardAssociations() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showOrFront(tab: 1)
    }
    @objc private func openDashboardInsights() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showOrFront(tab: 1)
    }
    @objc private func openDashboardMetacog() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showOrFront(tab: 1)
    }

    @objc private func openLogs() {
        openInTerminal("tail -f '\(bestLogPath())'")
    }

    @objc private func openTodaysDigest() {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let path = "\(home)/.claude/i-dream/daily/latest.md"
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    @objc private func regenerateTodaysDigest() {
        // Shell out to `i-dream digest` (writes the file; we then open it).
        let task = Process()
        task.launchPath = resolveIDreamBinary()
        task.arguments = ["digest"]
        task.standardOutput = Pipe()
        task.standardError = Pipe()
        do {
            try task.run()
            task.waitUntilExit()
        } catch {
            return
        }
        openTodaysDigest()
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

// — Wire format of a derived view file (mirrors src/consolidation/views.rs;
//   decoded with convertFromSnakeCase) —
private struct DerivedViewFile: Codable {
    let kind: String
    let total: Int
    let clusterCount: Int
    let truncatedAt: Int?
    let hasMore: Bool
    let items: [DerivedViewItem]
}

private struct DerivedViewItem: Codable {
    let stableId: String
    let id: String
    let text: String
    let category: String?
    let valence: String?
    let confidence: Double
    let occurrences: Int?
    let daysSinceFirstSeen: Int?
    let daysSinceLastSeen: Int?
    let clusterId: String
    let clusterSize: Int
    let isRepresentative: Bool
    let actionable: Bool?
    let promoted: Bool?
    let dismissed: Bool?
    let patternsLinked: [String]?
}

private func readDerivedView(_ name: String) -> DerivedViewFile? {
    let path = home + "/.claude/i-dream/derived/views/\(name).json"
    guard let data = FileManager.default.contents(atPath: path) else { return nil }
    let dec = JSONDecoder()
    dec.keyDecodingStrategy = .convertFromSnakeCase
    return try? dec.decode(DerivedViewFile.self, from: data)
}

/// Stable id for an insight block header — must produce the same ids as the
/// historical ratings already recorded in insight-feedback.jsonl.
private func stableInsightHash(_ header: String) -> String {
    let hash = header.utf8.reduce(0) { ($0 &* 31) &+ UInt64($1) }
    return String(format: "%016llx", hash)
}

private func insightFeedbackMap() -> [String: String] {
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

// — Browse row model —

struct BrowseRow: Identifiable {
    enum Kind: String, CaseIterable, Identifiable {
        case pattern = "Patterns"
        case association = "Associations"
        case insight = "Insights"
        case metacog = "Metacog"
        var id: String { rawValue }
        var tint: Color {
            switch self {
            case .pattern: return .cyan
            case .association: return .orange
            case .insight: return .yellow
            case .metacog: return .pink
            }
        }
        var symbol: String {
            switch self {
            case .pattern: return "brain.head.profile"
            case .association: return "link"
            case .insight: return "sparkles"
            case .metacog: return "checkmark.seal.fill"
            }
        }
    }

    let id: String
    let kind: Kind
    let title: String
    let detail: String
    let confidence: Double?
    let clusterSize: Int
    let ageDays: Int?
    let category: String?
    var rating: String?
    /// Insights only: the id ratings are recorded under. Content-hashed —
    /// the legacy header-only hash collides ("### Insight (conf=0.82)"
    /// repeats 13× in the live store), which both dropped SwiftUI ForEach
    /// rows and mis-attributed legacy ratings across header-twins.
    var ratingId: String = ""
    /// Second row of the two-line layout: valence dot + occurrences.
    var valence: String? = nil
    var occurrences: Int = 0
    /// Cross-navigation: related rows (label, target row id). An
    /// association chips to the pattern clusters it links; a pattern chips
    /// to the associations built on it — the old graph's one real job.
    var linkedChips: [(label: String, target: String)] = []
}

struct BrowseTotals {
    var totalItems = 0
    var clusters = 0
    var note = ""
}

/// Assemble Browse rows from every knowledge store. Pure file reads —
/// safe to call off the main thread (reloadDataAsync does).
func buildBrowseRows() -> ([BrowseRow], BrowseTotals) {
    var rows: [BrowseRow] = []
    var totals = BrowseTotals()

    let pv = readDerivedView("patterns")
    let av = readDerivedView("associations")

    // Cross-linking maps. Browse shows cluster representatives, so chip
    // targets are cluster ids (== the representative's stable id).
    var patClusterByUuid: [String: String] = [:]
    var patTitleByCluster: [String: String] = [:]
    if let v = pv {
        for it in v.items { patClusterByUuid[it.id] = it.clusterId }
        for it in v.items where it.isRepresentative { patTitleByCluster[it.clusterId] = it.text }
    }
    // Pattern cluster -> associations built on any of its members.
    var assocChipsForPatCluster: [String: [(label: String, target: String)]] = [:]
    if let v = av {
        for it in v.items where it.isRepresentative && !(it.dismissed ?? false) {
            var seen = Set<String>()
            for uuid in it.patternsLinked ?? [] {
                guard let cluster = patClusterByUuid[uuid], seen.insert(cluster).inserted else { continue }
                assocChipsForPatCluster[cluster, default: []].append(
                    (label: String(it.text.prefix(60)), target: it.stableId))
            }
        }
    }

    if let v = pv {
        totals.totalItems += v.total
        totals.clusters += v.clusterCount
        for it in v.items where it.isRepresentative {
            rows.append(BrowseRow(
                id: it.stableId, kind: .pattern, title: it.text, detail: it.text,
                confidence: it.confidence, clusterSize: it.clusterSize,
                ageDays: it.daysSinceLastSeen, category: it.category, rating: nil,
                valence: it.valence, occurrences: it.occurrences ?? 0,
                linkedChips: Array((assocChipsForPatCluster[it.clusterId] ?? []).prefix(4))))
        }
    }
    if let v = av {
        totals.totalItems += v.total
        totals.clusters += v.clusterCount
        for it in v.items where it.isRepresentative && !(it.dismissed ?? false) {
            var chips: [(label: String, target: String)] = []
            var seen = Set<String>()
            for uuid in it.patternsLinked ?? [] {
                guard let cluster = patClusterByUuid[uuid], seen.insert(cluster).inserted,
                      let title = patTitleByCluster[cluster] else { continue }
                chips.append((label: String(title.prefix(60)), target: cluster))
            }
            rows.append(BrowseRow(
                id: it.stableId, kind: .association, title: it.text, detail: it.text,
                confidence: it.confidence, clusterSize: it.clusterSize,
                ageDays: it.daysSinceLastSeen,
                category: (it.actionable ?? false) ? "actionable" : nil, rating: nil,
                linkedChips: Array(chips.prefix(4))))
        }
    }

    let feedback = insightFeedbackMap()
    if let raw = readAllInsights() {
        let blocks = raw.components(separatedBy: "\n").reduce(into: [(header: String, lines: [String])]()) { acc, line in
            if line.hasPrefix("### Insight") { acc.append((header: line, lines: [])) }
            else if !acc.isEmpty { acc[acc.count - 1].lines.append(line) }
        }
        totals.totalItems += blocks.count
        for b in blocks.reversed() {
            let legacyId = stableInsightHash(b.header)
            let body = b.lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            let contentId = stableInsightHash(b.header + body)
            var conf: Double? = nil
            if let r = b.header.range(of: #"conf=([0-9.]+)"#, options: .regularExpression) {
                conf = Double(b.header[r].replacingOccurrences(of: "conf=", with: ""))
            }
            let title = body.components(separatedBy: "\n").first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? b.header
            rows.append(BrowseRow(
                id: contentId, kind: .insight, title: title, detail: body,
                confidence: conf, clusterSize: 1, ageDays: nil,
                category: nil,
                rating: feedback[contentId] ?? feedback[legacyId],
                ratingId: contentId))
        }
    }

    let (audit, filename) = readLatestAudit()
    if let a = audit {
        let cal = a.calibrationScore.map { String(format: "%.2f", $0) } ?? "—"
        rows.append(BrowseRow(
            id: "metacog-latest", kind: .metacog,
            title: "Latest metacog audit — calibration \(cal)",
            detail: "File: \(filename ?? "?")\nCalibration: \(cal)\nSee daemon metacog audits dir for history.",
            confidence: a.calibrationScore, clusterSize: 1, ageDays: nil,
            category: nil, rating: nil))
        totals.totalItems += 1
    }

    // Freshest first; unknown ages sink; confidence breaks ties.
    rows.sort {
        let a = $0.ageDays ?? Int.max
        let b = $1.ageDays ?? Int.max
        if a != b { return a < b }
        return ($0.confidence ?? 0) > ($1.confidence ?? 0)
    }
    totals.note = "showing \(rows.count) rows · \(totals.totalItems) items in stores"
    return (rows, totals)
}

// — Overview data (felt-value first, then honest viz) —

struct OverviewViz {
    /// Top repeated lessons by cluster size: (row id, title, size, tint kind).
    var topLessons: [(id: String, title: String, size: Int, kind: BrowseRow.Kind)] = []
    /// Weekly activity buckets for the biggest clusters, oldest → newest.
    var timelines: [(title: String, weekly: [Int], total: Int)] = []
    var kindCounts: [(String, Int, Int)] = []   // (label, raw total, clusters)
    var valence: (pos: Int, neu: Int, neg: Int) = (0, 0, 0)
}

struct OverviewData {
    var reflect: ReflectData?
    var reviewPending: String?
    var state: (cycles: Int, tokens: Int, lastDream: String?)?
    var viz = OverviewViz()
}

/// Weekly ISO buckets covering the trailing `weeks`; index 0 = oldest.
private func weekIndex(_ date: Date, now: Date, weeks: Int) -> Int? {
    let secondsPerWeek = 7.0 * 86400.0
    let delta = now.timeIntervalSince(date)
    guard delta >= 0 else { return nil }
    let idx = weeks - 1 - Int(delta / secondsPerWeek)
    return (0..<weeks).contains(idx) ? idx : nil
}

/// Build the Overview's viz block from the derived views + raw pattern
/// occurrence history. Pure file reads — off-main safe.
func buildOverviewViz(rows: [BrowseRow]) -> OverviewViz {
    var viz = OverviewViz()
    let weeks = 12

    viz.topLessons = rows
        .filter { $0.clusterSize > 1 }
        .sorted { $0.clusterSize > $1.clusterSize }
        .prefix(10)
        .map { (id: $0.id, title: $0.title, size: $0.clusterSize, kind: $0.kind) }

    if let pv = readDerivedView("patterns") {
        var clusterOfUuid: [String: String] = [:]
        var titleOfCluster: [String: String] = [:]
        for it in pv.items {
            clusterOfUuid[it.id] = it.clusterId
            if it.isRepresentative { titleOfCluster[it.clusterId] = it.text }
        }
        // Occurrence timestamps per cluster, bucketed by week.
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let isoPlain = ISO8601DateFormatter()
        let now = Date()
        var buckets: [String: [Int]] = [:]
        var totals: [String: Int] = [:]
        for p in allPatterns() {
            guard let uuid = p.id, let cluster = clusterOfUuid[uuid] else { continue }
            let stamps = (p.occurrenceHistory ?? []) + [p.firstSeen].compactMap { $0 }
            for s in stamps {
                guard let d = iso.date(from: s) ?? isoPlain.date(from: s) else { continue }
                totals[cluster, default: 0] += 1
                if let w = weekIndex(d, now: now, weeks: weeks) {
                    buckets[cluster, default: Array(repeating: 0, count: weeks)][w] += 1
                }
            }
        }
        viz.timelines = totals.sorted { $0.value > $1.value }.prefix(5).compactMap { cluster, total in
            guard let title = titleOfCluster[cluster] else { return nil }
            return (title: title, weekly: buckets[cluster] ?? Array(repeating: 0, count: weeks), total: total)
        }

        var pos = 0, neu = 0, neg = 0
        for it in pv.items {
            switch it.valence {
            case "positive": pos += 1
            case "negative": neg += 1
            default: neu += 1
            }
        }
        viz.valence = (pos, neu, neg)
        viz.kindCounts.append(("Patterns", pv.total, pv.clusterCount))
    }
    if let av = readDerivedView("associations") {
        viz.kindCounts.append(("Associations", av.total, av.clusterCount))
    }
    return viz
}

final class OverviewModel: ObservableObject {
    @Published var data = OverviewData()
    var onOpenReview: (() -> Void)?
    var onJumpToBrowse: ((String) -> Void)?
}

// — Model + view —

final class BrowseModel: ObservableObject {
    @Published var rows: [BrowseRow] = []
    @Published var totals = BrowseTotals()
    @Published var filter: BrowseRow.Kind? = nil
    @Published var expandedId: String? = nil
    /// Writes the rating and refreshes; wired by the dashboard controller.
    var onRate: ((String, String) -> Void)?

    /// Set when a linked chip is clicked; the view scrolls there and expands.
    @Published var jumpTarget: String? = nil

    func apply(rows: [BrowseRow], totals: BrowseTotals) {
        self.rows = rows
        self.totals = totals
        if let e = expandedId, !rows.contains(where: { $0.id == e }) {
            expandedId = nil   // selection never outlives a shrunk refresh
        }
    }

    func jump(to id: String) {
        guard rows.contains(where: { $0.id == id }) else { return }
        filter = nil          // target may live under another type filter
        expandedId = id
        jumpTarget = id
    }
    var filtered: [BrowseRow] {
        guard let f = filter else { return rows }
        return rows.filter { $0.kind == f }
    }
    func count(of kind: BrowseRow.Kind) -> Int {
        rows.filter { $0.kind == kind }.count
    }
}

struct BrowseView: View {
    @ObservedObject var model: BrowseModel
    /// Hard exact row height — hover/expansion must never shift layout
    /// (sibling anti-idea A-7: minHeight lets hover change the box).
    /// Two-line layout: title + metadata line.
    private let rowH: CGFloat = 42

    /// On-demand cluster map overlay (user-requested viz): bubbles sized by
    /// cluster membership, filterable by typing, click = jump to the row.
    @State private var showClusterMap = false
    @State private var clusterQuery = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            chipBar
            Divider()
            if showClusterMap {
                clusterMap
                Divider()
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(model.filtered) { row in
                            rowView(row)
                                .id(row.id)
                            if model.expandedId == row.id {
                                detailView(row)
                            }
                        }
                    }
                }
                .onChange(of: model.jumpTarget) { target in
                    guard let target else { return }
                    withAnimation { proxy.scrollTo(target, anchor: .center) }
                    model.jumpTarget = nil
                }
            }
            Divider()
            footer
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var chipBar: some View {
        HStack(spacing: 6) {
            chip(nil, label: "All (\(model.rows.count))")
            ForEach(BrowseRow.Kind.allCases) { k in
                chip(k, label: "\(k.rawValue) (\(model.count(of: k)))")
            }
            Spacer()
            Button(action: { showClusterMap.toggle() }) {
                Label("Clusters", systemImage: "circle.hexagongrid")
                    .font(.system(size: 11))
                    .foregroundColor(showClusterMap ? .primary : .secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    /// The cluster map: every multi-member lesson as a bubble, area ∝
    /// cluster size. Typing dims non-matching bubbles instead of hiding
    /// them (selective highlight, as requested); clicking jumps to the row.
    private var clusterMap: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Highlight clusters…", text: $clusterQuery)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11))
                .frame(maxWidth: 320)
            let clusters = model.rows
                .filter { $0.clusterSize > 1 }
                .sorted { $0.clusterSize > $1.clusterSize }
            ScrollView {
                let cols = [GridItem(.adaptive(minimum: 92, maximum: 130), spacing: 8)]
                LazyVGrid(columns: cols, spacing: 8) {
                    ForEach(clusters) { c in
                        clusterBubble(c)
                    }
                }
            }
            .frame(maxHeight: 260)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color.primary.opacity(0.03))
    }

    private func clusterBubble(_ c: BrowseRow) -> some View {
        let maxSize = model.rows.map(\.clusterSize).max() ?? 1
        // Area-proportional radius so a ×28 doesn't dwarf everything linearly.
        let d = 34 + 56 * sqrt(CGFloat(c.clusterSize) / CGFloat(maxSize))
        let q = clusterQuery.trimmingCharacters(in: .whitespaces).lowercased()
        let words = q.split(separator: " ").map(String.init)
        let matches = q.isEmpty || words.allSatisfy { c.title.lowercased().contains($0) }
        return Button(action: {
            showClusterMap = false
            model.jump(to: c.id)
        }) {
            VStack(spacing: 3) {
                ZStack {
                    Circle()
                        .fill(c.kind.tint.opacity(matches ? 0.35 : 0.06))
                    Circle()
                        .strokeBorder(c.kind.tint.opacity(matches ? 0.9 : 0.15), lineWidth: 1.5)
                    Text("×\(c.clusterSize)")
                        .font(.system(size: 11, weight: .bold).monospacedDigit())
                        .foregroundColor(matches ? .primary : .secondary.opacity(0.4))
                }
                .frame(width: d, height: d)
                Text(c.title)
                    .font(.system(size: 9))
                    .lineLimit(2)
                    .multilineTextAlignment(.center)
                    .foregroundColor(matches ? .secondary : .secondary.opacity(0.3))
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(c.title)
    }

    private func chip(_ kind: BrowseRow.Kind?, label: String) -> some View {
        let active = model.filter == kind
        return Button(action: { model.filter = kind }) {
            Text(label)
                .font(.system(size: 11, weight: active ? .semibold : .regular))
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background((kind?.tint ?? .secondary).opacity(active ? 0.28 : 0.10))
                .foregroundColor(active ? .primary : .secondary)
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
    }

    private func rowView(_ row: BrowseRow) -> some View {
        Button(action: {
            model.expandedId = (model.expandedId == row.id) ? nil : row.id
        }) {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: row.kind.symbol)
                    .font(.system(size: 11))
                    .foregroundColor(row.kind.tint)
                    .frame(width: 16)
                    .padding(.top, 2)
                VStack(alignment: .leading, spacing: 2) {
                    Text(cleanTitle(row.title))
                        .font(.system(size: 12, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundColor(.primary)
                    Text(metaLine(row))
                        .font(.system(size: 10))
                        .lineLimit(1)
                        .foregroundColor(.secondary)
                }
                Spacer(minLength: 8)
                if row.clusterSize > 1 {
                    Text("×\(row.clusterSize)")
                        .font(.system(size: 10, weight: .semibold).monospacedDigit())
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(row.kind.tint.opacity(0.18))
                        .clipShape(Capsule())
                }
                if let r = row.rating {
                    Text(r == "up" ? "👍" : "👎").font(.system(size: 10))
                }
                if let c = row.confidence {
                    Text("\(Int(c * 100))%")
                        .font(.system(size: 10).monospacedDigit())
                        .foregroundColor(.secondary)
                        .frame(width: 34, alignment: .trailing)
                }
                Text(ageLabel(row.ageDays))
                    .font(.system(size: 10).monospacedDigit())
                    .foregroundColor(ageColor(row.ageDays))
                    .frame(width: 38, alignment: .trailing)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .frame(height: rowH)   // exact, not min
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(model.expandedId == row.id ? Color.primary.opacity(0.06) : Color.clear)
    }

    /// Second line: kind · category · valence dot · occurrences · links.
    private func metaLine(_ row: BrowseRow) -> AttributedString {
        var parts: [String] = [String(row.kind.rawValue.dropLast())]  // singular-ish
        if let c = row.category { parts.append(c) }
        if let v = row.valence { parts.append(valenceGlyph(v)) }
        if row.occurrences > 1 { parts.append("\(row.occurrences) occurrences") }
        if !row.linkedChips.isEmpty { parts.append("\(row.linkedChips.count) linked") }
        return AttributedString(parts.joined(separator: " · "))
    }

    private func valenceGlyph(_ v: String) -> String {
        switch v {
        case "positive": return "●pos"
        case "negative": return "●neg"
        default: return "●neu"
        }
    }

    /// Insight titles arrive as blockquote lines ("> ..."); strip the syntax.
    private func cleanTitle(_ t: String) -> String {
        var s = t.trimmingCharacters(in: .whitespaces)
        while s.hasPrefix(">") { s = String(s.dropFirst()).trimmingCharacters(in: .whitespaces) }
        return s
    }

    private func detailView(_ row: BrowseRow) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ScrollView {
                Text(richDetail(row.detail))
                    .font(.system(size: 12))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 240)
            if !row.linkedChips.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text(row.kind == .association ? "LINKED PATTERNS" : "BUILT-ON ASSOCIATIONS")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundColor(.secondary)
                    ForEach(Array(row.linkedChips.enumerated()), id: \.offset) { _, chip in
                        Button(action: { model.jump(to: chip.target) }) {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.right.circle")
                                    .font(.system(size: 9))
                                Text(chip.label + "…")
                                    .font(.system(size: 10))
                                    .lineLimit(1)
                            }
                            .foregroundColor(.cyan)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            HStack(spacing: 10) {
                if let cat = row.category {
                    Text(cat).font(.system(size: 10))
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(Color.secondary.opacity(0.15))
                        .clipShape(Capsule())
                }
                if row.clusterSize > 1 {
                    Text("\(row.clusterSize) rewordings of this lesson collapsed into one row")
                        .font(.system(size: 10)).foregroundColor(.secondary)
                }
                Spacer()
                if row.kind == .insight {
                    Button("👍") { model.onRate?(row.ratingId, "up") }.buttonStyle(.plain)
                    Button("👎") { model.onRate?(row.ratingId, "down") }.buttonStyle(.plain)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color.primary.opacity(0.04))
    }

    private var footer: some View {
        HStack {
            Text(model.totals.note + " · \(model.totals.clusters) deduped clusters")
                .font(.system(size: 10))
                .foregroundColor(.secondary)
            Spacer()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    /// Markdown-aware detail rendering (bold, code, italics) that keeps the
    /// store's line breaks. Falls back to the raw string on parse failure —
    /// showing syntax beats showing nothing.
    private func richDetail(_ raw: String) -> AttributedString {
        let cleaned = raw
            .components(separatedBy: "\n")
            .map { line -> String in
                var s = line
                while s.hasPrefix("> ") { s = String(s.dropFirst(2)) }
                return s
            }
            .joined(separator: "\n")
        if let parsed = try? AttributedString(
            markdown: cleaned,
            options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)) {
            return parsed
        }
        return AttributedString(cleaned)
    }

    private func ageLabel(_ d: Int?) -> String {
        guard let d else { return "—" }
        if d == 0 { return "today" }
        if d < 30 { return "\(d)d" }
        if d < 365 { return "\(d / 30)mo" }
        return "\(d / 365)y"
    }
    private func ageColor(_ d: Int?) -> Color {
        guard let d else { return .secondary }
        return d >= 30 ? .orange : .secondary
    }
}

// — Journal: cycle history with exact numbers + cross-nav into Browse —

struct JournalRowVM: Identifiable {
    let id: String
    let dateLabel: String
    let agoLabel: String
    let counts: String
    let tokens: Int
    /// Patterns whose first_seen falls in this cycle's window, as
    /// (label, Browse row id) chips — approximate but honest join.
    let chips: [(label: String, target: String)]
}

final class JournalModel: ObservableObject {
    @Published var rows: [JournalRowVM] = []
    @Published var heatEntries: [(date: Date, tokens: Int)] = []
    var onJumpToBrowse: ((String) -> Void)?
}

/// Join cycles to the patterns first seen inside each cycle's window.
/// Off-main safe (pure computation over already-loaded data).
private func buildJournalRows(journal: [JournalEntry]) -> ([JournalRowVM], [(date: Date, tokens: Int)]) {
    var clusterOfUuid: [String: String] = [:]
    var titleOfCluster: [String: String] = [:]
    if let pv = readDerivedView("patterns") {
        for it in pv.items {
            clusterOfUuid[it.id] = it.clusterId
            if it.isRepresentative { titleOfCluster[it.clusterId] = it.text }
        }
    }
    // Parse every date exactly once — the naive per-(cycle × pattern) parse
    // was ~185k ISO8601 formatter calls and held the pane blank for ~10s.
    let firstSeens: [(date: Date, cluster: String, title: String)] = allPatterns()
        .compactMap { p in
            guard let fs = p.firstSeen, let d = isoDate(fs),
                  let uuid = p.id, let cluster = clusterOfUuid[uuid],
                  let title = titleOfCluster[cluster] else { return nil }
            return (date: d, cluster: cluster, title: title)
        }
        .sorted { $0.date < $1.date }
    let sorted = journal.compactMap { e -> (JournalEntry, Date)? in
        guard let d = isoDate(e.timestamp) else { return nil }
        return (e, d)
    }.sorted { $0.1 < $1.1 }

    var rows: [JournalRowVM] = []
    var cursor = 0   // advances monotonically through firstSeens
    for (i, (entry, ts)) in sorted.enumerated() {
        let windowStart = i > 0 ? sorted[i - 1].1 : ts.addingTimeInterval(-6 * 3600)
        var chips: [(String, String)] = []
        var seen = Set<String>()
        while cursor < firstSeens.count, firstSeens[cursor].date <= ts {
            let f = firstSeens[cursor]
            cursor += 1
            guard entry.patternsExtracted > 0, f.date > windowStart,
                  chips.count < 3, seen.insert(f.cluster).inserted else { continue }
            chips.append((String(f.title.prefix(70)), f.cluster))
        }
        let counts = [
            entry.sessionsAnalyzed > 0 ? "\(entry.sessionsAnalyzed) sessions" : nil,
            entry.patternsExtracted > 0 ? "\(entry.patternsExtracted) patterns" : nil,
            entry.associationsFound > 0 ? "\(entry.associationsFound) assoc" : nil,
            entry.insightsPromoted > 0 ? "\(entry.insightsPromoted) insights" : nil,
        ].compactMap { $0 }.joined(separator: " · ")
        rows.append(JournalRowVM(
            id: entry.timestamp,
            dateLabel: fmtDate(entry.timestamp),
            agoLabel: timeAgo(entry.timestamp),
            counts: counts.isEmpty ? "skipped — no sessions to analyze" : counts,
            tokens: entry.tokensUsed,
            chips: chips))
    }
    let heat = sorted.map { (date: $0.1, tokens: $0.0.tokensUsed) }
    return (rows.reversed(), heat)
}

/// The existing AppKit heat map, hosted in SwiftUI — same visual, new pane.
private struct HeatMapWrapper: NSViewRepresentable {
    let entries: [(date: Date, tokens: Int)]
    func makeNSView(context: Context) -> CalendarHeatMapView {
        CalendarHeatMapView(frame: .zero)
    }
    func updateNSView(_ v: CalendarHeatMapView, context: Context) {
        v.entries = entries
        v.needsDisplay = true
    }
}

struct JournalPane: View {
    @ObservedObject var model: JournalModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !model.heatEntries.isEmpty {
                HeatMapWrapper(entries: model.heatEntries)
                    .frame(height: 120)
                    .padding(.horizontal, 16)
                    .padding(.top, 10)
            }
            Divider().padding(.top, 8)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.rows) { row in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 8) {
                                Text(row.dateLabel)
                                    .font(.system(size: 12, weight: .semibold))
                                Text("·  \(row.agoLabel)")
                                    .font(.system(size: 11))
                                    .foregroundColor(.secondary)
                                Spacer()
                                Text("\(row.tokens.formatted()) tokens")
                                    .font(.system(size: 11).monospacedDigit())
                                    .foregroundColor(.secondary)
                            }
                            Text(row.counts)
                                .font(.system(size: 11))
                                .foregroundColor(.secondary)
                            if !row.chips.isEmpty {
                                VStack(alignment: .leading, spacing: 2) {
                                    ForEach(Array(row.chips.enumerated()), id: \.offset) { _, chip in
                                        Button(action: { model.onJumpToBrowse?(chip.target) }) {
                                            HStack(spacing: 4) {
                                                Image(systemName: "arrow.right.circle")
                                                    .font(.system(size: 9))
                                                Text(chip.label + "…")
                                                    .font(.system(size: 10))
                                                    .lineLimit(1)
                                            }
                                            .foregroundColor(.cyan)
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                                .padding(.leading, 2)
                            }
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        Divider().padding(.horizontal, 16)
                    }
                }
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

// — Overview: felt-value first, then honest visualization —

struct OverviewPane: View {
    @ObservedObject var model: OverviewModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                feltValueCard
                if let s = model.data.state { statusLine(s) }
                if !model.data.viz.topLessons.isEmpty { topLessons }
                if !model.data.viz.kindCounts.isEmpty { distribution }
                if !model.data.viz.timelines.isEmpty { timelines }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    /// The dream→behavior loop, first. Same content as the menu's top block.
    private var feltValueCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let pending = model.data.reviewPending {
                Button(action: { model.onOpenReview?() }) {
                    Label("Weekly review pending (\(pending)) — open", systemImage: "checklist")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundColor(.orange)
                }
                .buttonStyle(.plain)
            }
            if let r = model.data.reflect {
                HStack(spacing: 6) {
                    Text("Mistakes:")
                        .font(.system(size: 13, weight: .semibold))
                    Text("\(r.summary.landing) landing")
                        .font(.system(size: 13))
                        .foregroundColor(.green)
                    Text("·").foregroundColor(.secondary)
                    Text("\(r.summary.worsening) worsening")
                        .font(.system(size: 13))
                        .foregroundColor(r.summary.worsening > 0 ? .orange : .secondary)
                }
                if let worst = r.patterns.first(where: { $0.trend == "worsening" }) {
                    Text("↑ \(worst.slug)")
                        .font(.system(size: 11))
                        .foregroundColor(.orange)
                }
            } else if model.data.reviewPending == nil {
                Text("No reflect data yet — run a few cycles.")
                    .font(.system(size: 12)).foregroundColor(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.05))
        .cornerRadius(8)
    }

    private func statusLine(_ s: (cycles: Int, tokens: Int, lastDream: String?)) -> some View {
        HStack(spacing: 14) {
            Label("\(s.cycles) cycles", systemImage: "moon.zzz.fill")
            Label(fmtNum(s.tokens) + " tokens", systemImage: "circle.hexagongrid.fill")
            if let last = s.lastDream {
                Label("last dream \(last)", systemImage: "clock")
            }
            Spacer()
        }
        .font(.system(size: 11))
        .foregroundColor(.secondary)
    }

    /// "Your biggest repeated lessons" — cluster sizes as honest bars,
    /// clicking a bar opens the lesson in Browse.
    private var topLessons: some View {
        VStack(alignment: .leading, spacing: 6) {
            sectionHeader("TOP REPEATED LESSONS")
            let maxSize = model.data.viz.topLessons.map(\.size).max() ?? 1
            ForEach(Array(model.data.viz.topLessons.enumerated()), id: \.offset) { _, lesson in
                Button(action: { model.onJumpToBrowse?(lesson.id) }) {
                    HStack(spacing: 8) {
                        Text(lesson.title)
                            .font(.system(size: 11))
                            .lineLimit(1)
                            .frame(width: 340, alignment: .leading)
                        GeometryReader { geo in
                            RoundedRectangle(cornerRadius: 2)
                                .fill(lesson.kind.tint.opacity(0.65))
                                .frame(width: max(4, geo.size.width * CGFloat(lesson.size) / CGFloat(maxSize)))
                        }
                        .frame(height: 12)
                        Text("\(lesson.size)")
                            .font(.system(size: 11, weight: .semibold).monospacedDigit())
                            .frame(width: 26, alignment: .trailing)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var distribution: some View {
        VStack(alignment: .leading, spacing: 6) {
            sectionHeader("STORES")
            ForEach(Array(model.data.viz.kindCounts.enumerated()), id: \.offset) { _, kc in
                HStack(spacing: 8) {
                    Text(kc.0).font(.system(size: 11)).frame(width: 100, alignment: .leading)
                    Text("\(kc.1) items → \(kc.2) lessons")
                        .font(.system(size: 11).monospacedDigit())
                        .foregroundColor(.secondary)
                }
            }
            let v = model.data.viz.valence
            let total = max(1, v.pos + v.neu + v.neg)
            HStack(spacing: 8) {
                Text("Valence").font(.system(size: 11)).frame(width: 100, alignment: .leading)
                GeometryReader { geo in
                    HStack(spacing: 1) {
                        Rectangle().fill(Color.green.opacity(0.7))
                            .frame(width: geo.size.width * CGFloat(v.pos) / CGFloat(total))
                        Rectangle().fill(Color.gray.opacity(0.5))
                            .frame(width: geo.size.width * CGFloat(v.neu) / CGFloat(total))
                        Rectangle().fill(Color.orange.opacity(0.7))
                            .frame(width: geo.size.width * CGFloat(v.neg) / CGFloat(total))
                    }
                }
                .frame(height: 10)
                Text("\(v.pos)+ · \(v.neu)○ · \(v.neg)−")
                    .font(.system(size: 10).monospacedDigit())
                    .foregroundColor(.secondary)
            }
        }
    }

    /// Per-lesson weekly activity, trailing 12 weeks. Neutral bars — the
    /// value's meaning lives in the label, not trace color (sibling #6).
    private var timelines: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionHeader("ACTIVITY — TRAILING 12 WEEKS")
            ForEach(Array(model.data.viz.timelines.enumerated()), id: \.offset) { _, t in
                HStack(spacing: 8) {
                    Text(t.title)
                        .font(.system(size: 11))
                        .lineLimit(1)
                        .frame(width: 340, alignment: .leading)
                    let peak = max(1, t.weekly.max() ?? 1)
                    HStack(alignment: .bottom, spacing: 2) {
                        ForEach(Array(t.weekly.enumerated()), id: \.offset) { _, count in
                            RoundedRectangle(cornerRadius: 1)
                                .fill(Color.secondary.opacity(count == 0 ? 0.15 : 0.7))
                                .frame(width: 8, height: max(2, 22 * CGFloat(count) / CGFloat(peak)))
                        }
                    }
                    .frame(height: 22, alignment: .bottom)
                    Text("\(t.total) hits")
                        .font(.system(size: 10).monospacedDigit())
                        .foregroundColor(.secondary)
                }
            }
        }
    }

    private func sectionHeader(_ s: String) -> some View {
        Text(s)
            .font(.system(size: 9, weight: .semibold))
            .foregroundColor(.secondary)
            .kerning(0.5)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = BarDelegate()
app.delegate = delegate
app.run()
