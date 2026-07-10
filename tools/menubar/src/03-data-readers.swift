
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

// ─── Lane health ────────────────────────────────────────────────────────────
// A dead input lane (a queue nobody drains, a decay that never runs) is
// invisible until it is named. The engine writes a red/yellow/green verdict
// per cycle; the menu reads the latest line and surfaces anything not green.

private struct LaneHealthLane: Codable {
    let lane:   String
    let status: String   // "green" | "yellow" | "red"
    let reason: String
}

private struct LaneHealthReading: Codable {
    let red:    Int
    let yellow: Int
    let green:  Int
    let lanes:  [LaneHealthLane]
}

/// Read the latest lane-health reading (last line of the per-cycle JSONL).
/// Returns nil when the engine hasn't emitted one yet.
private func readLaneHealth() -> LaneHealthReading? {
    let path = subDir + "/dreams/lane-health.jsonl"
    guard let content = try? String(contentsOfFile: path, encoding: .utf8),
          let last = content.components(separatedBy: "\n").filter({ !$0.isEmpty }).last
    else { return nil }
    return try? JSONDecoder().decode(LaneHealthReading.self, from: Data(last.utf8))
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
