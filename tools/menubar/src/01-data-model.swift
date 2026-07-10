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
