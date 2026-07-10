
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
    var laneHealth:     LaneHealthReading? = nil
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
            s.laneHealth     = readLaneHealth()
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
