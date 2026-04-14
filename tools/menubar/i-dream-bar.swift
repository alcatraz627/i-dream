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
private let tracesDir = subDir + "/dreams/traces"

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

private func fmtElapsed(_ secs: TimeInterval) -> String {
    let s = Int(secs)
    return s < 60 ? "\(s)s" : "\(s / 60)m \(s % 60)s"
}

// ─── Data models ─────────────────────────────────────────────────────────────

private struct DaemonState: Codable {
    let lastActivity:      String?
    let lastConsolidation: String?
    let totalCycles:       Int
    let totalTokensUsed:   Int
    enum CodingKeys: String, CodingKey {
        case lastActivity      = "last_activity"
        case lastConsolidation = "last_consolidation"
        case totalCycles       = "total_cycles"
        case totalTokensUsed   = "total_tokens_used"
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
    let pattern:    String
    let valence:    String
    let confidence: Double
    let category:   String
    let firstSeen:  String?
    enum CodingKeys: String, CodingKey {
        case pattern, valence, confidence, category
        case firstSeen = "first_seen"
    }
}

private struct JournalEntry: Codable {
    let timestamp:         String
    let sessionsAnalyzed:  Int
    let patternsExtracted: Int
    let associationsFound: Int
    let insightsPromoted:  Int
    let tokensUsed:        Int
    enum CodingKeys: String, CodingKey {
        case timestamp
        case sessionsAnalyzed  = "sessions_analyzed"
        case patternsExtracted = "patterns_extracted"
        case associationsFound = "associations_found"
        case insightsPromoted  = "insights_promoted"
        case tokensUsed        = "tokens_used"
    }
}

private struct Association: Codable {
    let id:           String
    let hypothesis:   String
    let confidence:   Double
    let actionable:   Bool
    let suggestedRule: String?
    enum CodingKeys: String, CodingKey {
        case id, hypothesis, confidence, actionable
        case suggestedRule = "suggested_rule"
    }
}

private struct MetacogAudit: Codable {
    let calibrationScore: Double?
    let biasesDetected:   [String]?
    let recommendations:  [String]?
    enum CodingKeys: String, CodingKey {
        case calibrationScore = "calibration_score"
        case biasesDetected   = "biases_detected"
        case recommendations
    }
}

// ─── Rich text builder ────────────────────────────────────────────────────────

/// Fluent builder for NSAttributedString with semantic styling methods.
/// Converts plain-string content from detail views into visually structured text.
final class RichText {
    private let buf = NSMutableAttributedString()

    @discardableResult func header(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 14, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func subheader(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .medium),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func body(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func dim(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 11),
            .foregroundColor: NSColor.secondaryLabelColor,
        ])); return self
    }
    @discardableResult func mono(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
            .foregroundColor: NSColor.labelColor,
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
    func build() -> NSAttributedString { buf }
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

private func lastDaemonError() -> String? {
    guard let content = try? String(contentsOfFile: bestLogPath(), encoding: .utf8) else { return nil }
    for line in content.components(separatedBy: "\n").reversed() {
        guard line.contains(" ERROR "), let r = line.range(of: " ERROR ") else { continue }
        let msg = String(line[r.upperBound...])
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

private func readLatestAudit() -> (audit: MetacogAudit?, filename: String?) {
    let auditsDir = subDir + "/metacog/audits"
    guard let files = try? FileManager.default.contentsOfDirectory(atPath: auditsDir) else {
        return (nil, nil)
    }
    guard let latest = files.filter({ $0.hasSuffix(".json") }).sorted().last else {
        return (nil, nil)
    }
    let path  = auditsDir + "/" + latest
    guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)) else { return (nil, nil) }
    let audit = try? JSONDecoder().decode(MetacogAudit.self, from: data)
    return (audit, latest)
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

    private var cachedRunning  = false
    private var cachedState:   DaemonState?
    private var cachedBoard:   BoardData?
    private var cachedPatterns: [Pattern]      = []
    private var cachedJournal:  [JournalEntry] = []

    // Persistent resizable detail panel (replaces NSAlert popups)
    private var detailPanel:    NSPanel?
    private var detailFilePath: String?

    // Dreaming animation
    private var isCycling       = false
    private var cycleStartTime: Date?
    private var animFrame       = 0
    private var animTimer:      Timer?

    // Persistent menu instance (rebuilt in-place via NSMenuDelegate)
    private var theMenu: NSMenu!

    func applicationDidFinishLaunching(_ note: Notification) {
        dlog("launched PID=\(ProcessInfo.processInfo.processIdentifier)")
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)

        theMenu                  = NSMenu()
        theMenu.autoenablesItems = false
        theMenu.delegate         = self
        statusItem.menu          = theMenu

        refresh()
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
        cachedRunning  = isDaemonRunning()
        cachedState    = readState()
        cachedBoard    = readBoard()
        cachedPatterns = recentPatterns()
        cachedJournal  = recentJournal()
        updateButton()
        menu.removeAllItems()
        populateMenuItems(menu)
    }

    @objc func refresh() {
        cachedRunning  = isDaemonRunning()
        cachedState    = readState()
        cachedBoard    = readBoard()
        cachedPatterns = recentPatterns()
        cachedJournal  = recentJournal()
        dlog("refresh: running=\(cachedRunning) cycles=\(cachedState?.totalCycles ?? -1)")
        checkCycleCompletion()
        updateButton()
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

    private func checkCycleCompletion() {
        guard isCycling, let start = cycleStartTime else { return }
        // Safety timeout: 3 minutes
        if Date().timeIntervalSince(start) > 180 {
            dlog("cycle animation timeout"); stopDreamAnimation(); return
        }
        let progress = detectDreamProgress(since: start)
        if progress.isDone {
            dlog("cycle complete — trace detected"); stopDreamAnimation(); refresh()
        }
    }

    // ── Status bar button ──────────────────────────────────────────────────────

    private func updateButton() {
        guard let btn = statusItem.button else { return }
        let sym = currentIconSymbol()
        if let img = NSImage(systemSymbolName: sym, accessibilityDescription: "i-dream") {
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
            btn.title   = " \(n)"
            btn.toolTip = "i-dream: running · \(n) cycles"
        } else {
            btn.title   = ""
            btn.toolTip = "i-dream: stopped — click to manage"
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
        menu.addItem(.separator())

        // ─ Activity ───────────────────────────────────────────────────────────
        addSection(menu, "Activity")
        if let s = s {
            addRow(menu, "Cycles",      "\(s.totalCycles)",        valueColor: .systemBlue)
            addRow(menu, "Tokens used", fmtNum(s.totalTokensUsed), valueColor: .systemBlue)
            addRow(menu, "Last run",    fmtDateWithAge(s.lastConsolidation))
            addRow(menu, "Last active", s.lastActivity != nil ? fmtDateWithAge(s.lastActivity) : "—")
        } else {
            addDim(menu, "  state.json not found")
        }

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
        if !cachedJournal.isEmpty || !cachedPatterns.isEmpty {
            menu.addItem(.separator())
            addSection(menu, "Recent Inferences")
            // Show last cycle summary
            if let latest = cachedJournal.last {
                let parts = [
                    latest.sessionsAnalyzed > 0 ? "\(latest.sessionsAnalyzed) sess" : nil,
                    latest.patternsExtracted > 0 ? "\(latest.patternsExtracted) pat" : nil,
                    latest.associationsFound > 0 ? "\(latest.associationsFound) assoc" : nil,
                    latest.insightsPromoted  > 0 ? "\(latest.insightsPromoted) ins" : nil,
                ].compactMap { $0 }.joined(separator: "  ·  ")
                let summary = parts.isEmpty ? "skipped — no sessions" : parts
                addTwoLine(menu,
                           top:    "  Last cycle  \(fmtDate(latest.timestamp))",
                           bottom: "  \(summary)  ·  \(fmtNum(latest.tokensUsed)) tkns")
            }
            // Show recent pattern learnings (actual text)
            if !cachedPatterns.isEmpty {
                for p in cachedPatterns {
                    let text = p.pattern.count > 82 ? String(p.pattern.prefix(79)) + "…" : p.pattern
                    let sym  = valenceSymbol(p.valence)
                    addTwoLine(menu,
                               top:    "  \(sym) \"\(text)\"",
                               bottom: "  \(p.category)  ·  \(Int(p.confidence * 100))% confident")
                }
            }
        }

        // ─ Last error ─────────────────────────────────────────────────────────
        if let err = b?.lastError {
            menu.addItem(.separator())
            addSection(menu, "⚠  Last Error  (today)")
            addDim(menu, "  " + err)
        }

        menu.addItem(.separator())

        // ─ Daemon controls ────────────────────────────────────────────────────
        addSection(menu, "Daemon")
        if running {
            let s = add(menu, "Stop Daemon", #selector(stopDaemon))
            setIcon(s, "stop.fill")
        } else {
            let s = add(menu, "Start Daemon", #selector(startDaemon))
            setIcon(s, "play.fill")
        }
        let t = add(menu, "Trigger Dream Cycle", #selector(triggerCycle))
        setIcon(t, "arrow.triangle.2.circlepath")
        t.isEnabled = running && !isCycling

        menu.addItem(.separator())

        // ─ Tools ──────────────────────────────────────────────────────────────
        let dash = add(menu, "Open Dashboard", #selector(openDashboard))
        setIcon(dash, "chart.bar.doc.horizontal.fill")

        let howTo = add(menu, "Show How-To…", #selector(showHowTo))
        setIcon(howTo, "questionmark.circle.fill")

        let gh = add(menu, "View on GitHub", #selector(openGitHub))
        setIcon(gh, "arrow.up.right.square")

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
        let i = NSMenuItem(title: title, action: sel, keyEquivalent: "")
        i.target = self; i.isEnabled = true
        menu.addItem(i); return i
    }

    private func addSection(_ menu: NSMenu, _ title: String) {
        let i = NSMenuItem()
        i.attributedTitle = NSAttributedString(string: title.uppercased(), attributes: [
            .font: NSFont.systemFont(ofSize: 10, weight: .medium),
            .foregroundColor: NSColor.secondaryLabelColor,
        ])
        i.isEnabled = false; menu.addItem(i)
    }

    private func addColored(_ menu: NSMenu, _ title: String,
                            color: NSColor, font: NSFont = .systemFont(ofSize: 13)) {
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
                                       attributes: [.font: NSFont.systemFont(ofSize: 12)]))
        full.append(NSAttributedString(string: value, attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
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
                                       attributes: [.font: NSFont.systemFont(ofSize: 12)]))
        full.append(NSAttributedString(string: value, attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: valueColor ?? NSColor.labelColor,
        ]))
        full.append(NSAttributedString(string: "  ›", attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: NSColor.tertiaryLabelColor,
        ]))
        i.attributedTitle = full; i.action = action; i.target = self; i.isEnabled = true
        menu.addItem(i); return i
    }

    private func addTwoLine(_ menu: NSMenu, top: String, bottom: String) {
        let i    = NSMenuItem()
        let full = NSMutableAttributedString()
        full.append(NSAttributedString(string: top + "\n",
                                       attributes: [.font: NSFont.systemFont(ofSize: 11)]))
        full.append(NSAttributedString(string: bottom, attributes: [
            .font: NSFont.systemFont(ofSize: 10),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]))
        i.attributedTitle = full; i.isEnabled = false; menu.addItem(i)
    }

    private func addDim(_ menu: NSMenu, _ title: String) {
        let i = NSMenuItem()
        i.attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: NSColor.secondaryLabelColor,
            .font: NSFont.systemFont(ofSize: 11),
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

    // ── SF Symbol icon helper ──────────────────────────────────────────────────

    private func setIcon(_ item: NSMenuItem, _ symbol: String) {
        if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) {
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
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 520),
            styleMask:   [.titled, .closable, .resizable, .miniaturizable, .nonactivatingPanel],
            backing:     .buffered,
            defer:       false
        )
        panel.title                = title
        panel.isReleasedWhenClosed = false
        panel.level                = .floating
        panel.center()

        // ── Layout: scroll view above a thin toolbar ────────────────────────
        let root = NSView()
        panel.contentView = root

        let sv = NSScrollView()
        sv.translatesAutoresizingMaskIntoConstraints = false
        sv.hasVerticalScroller   = true
        sv.hasHorizontalScroller = false
        sv.autohidesScrollers    = true
        sv.borderType            = .noBorder
        root.addSubview(sv)

        let tv = NSTextView()
        tv.isEditable   = false
        tv.isSelectable = true
        tv.backgroundColor         = .textBackgroundColor
        tv.textContainerInset      = NSSize(width: 14, height: 14)
        tv.isVerticallyResizable   = true
        tv.isHorizontallyResizable = false
        tv.textContainer?.widthTracksTextView = true
        tv.textStorage?.setAttributedString(content)
        sv.documentView = tv

        let bar = NSView()
        bar.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(bar)

        // Subtle separator line above the toolbar
        let sep = NSBox()
        sep.translatesAutoresizingMaskIntoConstraints = false
        sep.boxType = .separator
        bar.addSubview(sep)

        let closeBtn = NSButton(title: "Close", target: self,
                                action: #selector(closeDetailPanel))
        closeBtn.translatesAutoresizingMaskIntoConstraints = false
        closeBtn.bezelStyle = .rounded
        bar.addSubview(closeBtn)

        var barConstraints: [NSLayoutConstraint] = [
            sep.leadingAnchor.constraint(equalTo: bar.leadingAnchor),
            sep.trailingAnchor.constraint(equalTo: bar.trailingAnchor),
            sep.topAnchor.constraint(equalTo: bar.topAnchor),
            sep.heightAnchor.constraint(equalToConstant: 1),
            closeBtn.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: -12),
            closeBtn.centerYAnchor.constraint(equalTo: bar.centerYAnchor, constant: 4),
        ]

        if filePath != nil {
            let openBtn = NSButton(title: "Open File", target: self,
                                   action: #selector(openDetailFile))
            openBtn.translatesAutoresizingMaskIntoConstraints = false
            openBtn.bezelStyle = .rounded
            bar.addSubview(openBtn)
            barConstraints += [
                openBtn.trailingAnchor.constraint(equalTo: closeBtn.leadingAnchor, constant: -8),
                openBtn.centerYAnchor.constraint(equalTo: bar.centerYAnchor, constant: 4),
            ]
        }

        NSLayoutConstraint.activate(barConstraints + [
            bar.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            bar.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            bar.heightAnchor.constraint(equalToConstant: 48),
            sv.topAnchor.constraint(equalTo: root.topAnchor),
            sv.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            sv.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            sv.bottomAnchor.constraint(equalTo: bar.topAnchor),
        ])

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
            rt.subheader("\(val)  \(p.pattern)")
            rt.dim("  \(p.category)  ·  \(Int(p.confidence * 100))% confident\(since)")
            rt.spacer()
        }
        if patterns.count > 15 { rt.dim("… and \(patterns.count - 15) earlier patterns") }
        showResizablePanel(title: "Patterns (\(patterns.count))",
                           content: rt.build(),
                           filePath: subDir + "/dreams/patterns.json")
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
            rt.dim("[\(assocs.count - i)]  \(Int(a.confidence * 100))% confident\(a.actionable ? "  · actionable" : "")")
            rt.body(a.hypothesis)
            if let rule = a.suggestedRule, !rule.isEmpty {
                rt.dim("  → Rule: \(rule)")
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
            alert("Metacog", "No metacognition audit data found."); return
        }
        // Parse date from filename like "20260412-1032-audit.json"
        var dateStr = filename ?? ""
        if let fn = filename {
            let parts = fn.components(separatedBy: "-")
            if parts.count >= 2 {
                let df = DateFormatter()
                df.dateFormat = "yyyyMMdd HHmm"
                if let d = df.date(from: "\(parts[0]) \(parts[1])") {
                    dateStr = fmtDateDirect(d)
                }
            }
        }
        let rt = RichText()
        rt.header("Metacognition Audit")
        if !dateStr.isEmpty { rt.dim("From: \(dateStr)") }
        rt.spacer()
        if let score = audit.calibrationScore {
            rt.subheader("Calibration Score")
            rt.body(String(format: "%.2f  (1.0 = perfectly calibrated)", score))
            rt.spacer()
        }
        if let biases = audit.biasesDetected, !biases.isEmpty {
            rt.subheader("Biases Detected")
            biases.forEach { rt.body("  • \($0)") }
            rt.spacer()
        }
        if let recs = audit.recommendations, !recs.isEmpty {
            rt.subheader("Recommendations")
            recs.forEach { rt.body("  • \($0)") }
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
        for entry in journal.suffix(20).reversed() {
            rt.spacer()
            rt.subheader(fmtDate(entry.timestamp))
            if entry.sessionsAnalyzed == 0 {
                rt.dim("  Skipped — no new sessions to consolidate")
            } else {
                rt.body("  Sessions analyzed:   \(entry.sessionsAnalyzed)")
                if entry.patternsExtracted > 0 { rt.body("  Patterns extracted:  \(entry.patternsExtracted)") }
                if entry.associationsFound  > 0 { rt.body("  Associations found:  \(entry.associationsFound)") }
                if entry.insightsPromoted   > 0 { rt.body("  Insights promoted:   \(entry.insightsPromoted)") }
                rt.dim("  Tokens used:         \(fmtNum(entry.tokensUsed))")
            }
        }
        if journal.count > 20 { rt.dim("… and \(journal.count - 20) earlier entries") }
        showResizablePanel(title: "Dream Journal (\(journal.count) cycles)",
                           content: rt.build(),
                           filePath: subDir + "/dreams/journal.jsonl")
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
                      "Last active:  \(fmtDate(s.lastActivity))"]
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

    @objc private func showHowTo() {
        let rt = RichText()
        rt.header("i-dream — How To")
        rt.spacer()
        rt.subheader("What It Does")
        rt.body("i-dream processes your Claude Code sessions while you sleep.")
        rt.body("It runs 3-phase dream cycles (SWS → REM → Wake) to extract patterns,")
        rt.body("form associations, and promote long-term insights.")
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
        rt.subheader("Conversational Hooks")
        rt.body("  UserPromptSubmit hook captures sentiment in each message:")
        rt.body("  ALL-CAPS words, frustration/swear words, correction language,")
        rt.body("  and positive feedback — stored in logs/signals.jsonl for the")
        rt.body("  dreaming module to incorporate into session analysis.")
        rt.spacer()
        rt.subheader("Reading the Status Bar")
        rt.mono("  ◉ N       — running, N cycles completed")
        rt.mono("  ◉ 2m 15s  — dreaming right now (elapsed updates live)")
        rt.mono("  (empty)   — daemon stopped")
        rt.spacer()
        rt.subheader("Dashboard")
        rt.body("'Open Dashboard' regenerates an HTML report and opens it in your browser.")
        rt.body("Shows cycle traces, file inventory, hook events, and module status.")
        rt.spacer()
        rt.subheader("Logs")
        rt.body("  Logs → Open in Terminal  — live tail of the daemon log")
        rt.body("  Logs → Open in VS Code   — open log file in editor")
        rt.body("  Logs → Open Debug Log    — widget's own debug output (/tmp/i-dream-bar.log)")
        rt.spacer()
        rt.subheader("Data Location")
        rt.mono("  ~/.claude/subconscious/")
        rt.mono("  ├── dreams/      patterns.json, associations.json, journal.jsonl")
        rt.mono("  ├── metacog/     audits/, calibration.jsonl")
        rt.mono("  ├── logs/        i-dream.log.YYYY-MM-DD, signals.jsonl")
        rt.mono("  └── state.json   cycle counts, token totals")
        rt.spacer()
        rt.subheader("Build / Install")
        rt.mono("  bash tools/menubar/build.sh           # rebuild widget")
        rt.mono("  bash tools/menubar/build.sh --install # register LaunchAgent")
        rt.mono("  i-dream service install               # register daemon LaunchAgent")
        rt.mono("  i-dream hooks install                 # install all hook scripts")
        rt.spacer()
        rt.dim("Full documentation: docs/05-how-to.md in the project repository.")
        showResizablePanel(title: "i-dream — How To", content: rt.build())
    }

    @objc private func openGitHub() {
        NSWorkspace.shared.open(URL(string: "https://github.com/alcatraz627/i-dream")!)
    }

    private func alert(_ title: String, _ msg: String) {
        let a = NSAlert()
        a.messageText = title; a.informativeText = msg
        a.alertStyle  = .warning; a.addButton(withTitle: "OK"); a.runModal()
    }
}

// ─── Entry point ──────────────────────────────────────────────────────────────

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = BarDelegate()
app.delegate = delegate
app.run()
