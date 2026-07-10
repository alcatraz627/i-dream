
// ─── App delegate ─────────────────────────────────────────────────────────────

final class BarDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    var statusItem: NSStatusItem!
    var timer: Timer?

    private var cachedRunning        = false
    private var cachedState:         DaemonState?
    private var cachedBoard:         BoardData?
    private var cachedJournal:       [JournalEntry] = []
    private var cachedStoreFiles:    [StoreFile]    = []
    private var cachedLaneHealth:    LaneHealthReading? = nil
    private var cachedDigest:        String?
    private var cachedFrequencyHours: Double?
    private var cachedPatternCount:  Int = 0
    private var cachedHighConfCount: Int = 0

    // Persistent resizable detail panel (replaces NSAlert popups)
    private var detailPanel:          NSPanel?
    private var detailFilePath:       String?
    private var panelLinkDelegate:    JournalLinkDelegate?   // generic link handler for resizable panels

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

        // --smoke: verification harness. Real boot, real data, no status
        // item; renders every dashboard tab, asserts, exits. See runSmoke().
        if CommandLine.arguments.contains("--smoke") {
            runSmoke()
            return
        }

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

        // SIGUSR1 → open dashboard (sent by `i-dream dashboard` CLI).
        // The sources MUST be retained: a resumed DispatchSource stored in a
        // local is released when this method returns and its handler never
        // fires — which is why the CLI summon had been a silent no-op since
        // the feature shipped.
        signal(SIGUSR1, SIG_IGN)
        let usr1Src = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
        usr1Src.setEventHandler { [weak self] in self?.openDashboard() }
        usr1Src.resume()
        signalSources.append(usr1Src)

        // SIGUSR2 → dump a self-rendered PNG of the dashboard window to
        // /tmp/i-dream-dashboard-snap.png. Screen capture reads the display
        // framebuffer, which is black when the physical display sleeps
        // (remote-control sessions) — the app rendering its own view tree
        // is display-independent, and needs no screen-recording permission.
        signal(SIGUSR2, SIG_IGN)
        let usr2Src = DispatchSource.makeSignalSource(signal: SIGUSR2, queue: .main)
        usr2Src.setEventHandler { [weak self] in
            guard let self, let dc = self.dashboardController else { return }
            // Optional control files drive the capture (signals carry no
            // args): tab index, transient theme, cluster panel, hover row.
            // Theme applies to the panel only and is never persisted — the
            // sidebar picker still owns the durable preference.
            if let s = try? String(contentsOfFile: "/tmp/i-dream-snap-tab", encoding: .utf8),
               let idx = Int(s.trimmingCharacters(in: .whitespacesAndNewlines)) {
                dc.selectTab(idx)
            }
            if let s = try? String(contentsOfFile: "/tmp/i-dream-snap-theme", encoding: .utf8) {
                switch s.trimmingCharacters(in: .whitespacesAndNewlines) {
                case "light":  dc.applyTransientAppearance(.aqua)
                case "dark":   dc.applyTransientAppearance(.darkAqua)
                case "system": dc.applyTransientAppearance(nil)
                default: break
                }
            }
            if let s = try? String(contentsOfFile: "/tmp/i-dream-snap-cluster", encoding: .utf8) {
                dc.browseModel.showClusterMap = s.trimmingCharacters(in: .whitespacesAndNewlines) != "0"
            }
            if let s = try? String(contentsOfFile: "/tmp/i-dream-snap-hover", encoding: .utf8) {
                let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
                dc.browseModel.hoverClusterId =
                    t == "top" ? dc.browseModel.clusterRows.first?.id
                               : (t.isEmpty ? nil : t)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { dc.dumpSnapshot() }
        }
        usr2Src.resume()
        signalSources.append(usr2Src)
    }

    /// Keeps the signal dispatch sources alive for the app's lifetime.
    private var signalSources: [DispatchSourceSignal] = []

    /// The `--smoke` verification harness: opens the dashboard on the real
    /// data stores, walks all four tabs dumping self-rendered PNGs to
    /// /tmp/i-dream-smoke/, runs the controller's data assertions, prints a
    /// report, and exits 0/1. One command proves a build actually renders —
    /// nothing here touches the running widget, prefs, or the stores.
    private func runSmoke() {
        dlog("smoke: start")
        let dir = "/tmp/i-dream-smoke"
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        dashboardController = DashboardWindowController()
        let dc = dashboardController!
        dc.showOrFront()
        var failures: [String] = []

        func capture(_ tab: Int) {
            if tab >= 4 {
                failures += dc.smokeDataChecks()
                for i in 0..<4 {
                    let p = "\(dir)/tab\(i).png"
                    let sz = ((try? FileManager.default.attributesOfItem(atPath: p))?[.size] as? Int) ?? 0
                    if sz < 5_000 { failures.append("tab\(i).png missing or tiny (\(sz)B)") }
                }
                if failures.isEmpty {
                    print("SMOKE PASS — 4 tabs rendered to \(dir), data checks clean")
                } else {
                    print("SMOKE FAIL:\n  " + failures.joined(separator: "\n  "))
                }
                exit(failures.isEmpty ? 0 : 1)
            }
            dc.selectTab(tab)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                dc.dumpSnapshot()
                try? FileManager.default.removeItem(atPath: "\(dir)/tab\(tab).png")
                try? FileManager.default.copyItem(
                    atPath: "/tmp/i-dream-dashboard-snap.png",
                    toPath: "\(dir)/tab\(tab).png")
                capture(tab + 1)
            }
        }

        // Give the async data load a beat, then walk the tabs; a hang is a
        // failure, not a wait.
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { capture(0) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 25.0) {
            print("SMOKE FAIL: timed out")
            exit(1)
        }
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
        cachedJournal          = s.journal
        cachedStoreFiles       = s.storeFiles
        cachedLaneHealth       = s.laneHealth
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
            self.cachedJournal        = s.journal
            self.cachedStoreFiles     = s.storeFiles
            self.cachedLaneHealth     = s.laneHealth
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

    private func populateMenuItems(_ menu: NSMenu) {
        let running = cachedRunning
        let s       = cachedState
        let b       = cachedBoard

        // ─ Dreaming indicator ─────────────────────────────────────────────────
        if isCycling, let start = cycleStartTime {
            let progress = detectDreamProgress(since: start)
            let color    = dreamAnimColors[animFrame % dreamAnimColors.count]
            addColored(menu, "◉  Dreaming   \(fmtElapsed(progress.elapsed))", color: color)
            addDim(menu, "  Phase: \(progress.phase)")
            menu.addItem(.separator())
        }

        // ─ Status header ──────────────────────────────────────────────────────
        let statusColor: NSColor = running ? .systemGreen : .systemOrange
        let statusText  = running ? "◉  i-dream  —  Running" : "○  i-dream  —  Stopped"
        addColored(menu, statusText, color: statusColor)

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

        // Flagship launcher — promoted from the old footer, where it sat at
        // row ~46 below the inference wall (field-study J1).
        let dash = add(menu, "Open Dashboard", #selector(openDashboard), key: "d")
        setIcon(dash, "chart.bar.doc.horizontal.fill")

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

        // ─ Rhythm — when it last ran, when it thinks next ─────────────────────
        // The old ACTIVITY block (cycles/usage/tokens/last-active/signals)
        // lives on the dashboard Overview now; only glance values stay inline.
        if let s = s {
            addRow(menu, "Last run", fmtDateWithAge(s.lastConsolidation))
        } else {
            addDim(menu, "  state.json not found")
        }
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
        addRow(menu, "Next dream", "\(nextStr)  ·  every \(freqLabel)")

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
        let freqParent = NSMenuItem(title: "Change Frequency →", action: nil, keyEquivalent: "")
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
                item.attributedTitle = columned([
                    seg("  \(d.name)", BarFont.monoSecondary, .labelColor),
                    seg(d.cadence, BarFont.monoSecondary, .labelColor),
                ], stops: [150])
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
            title: "Dream Domains (\(domainCount)) →",
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
                let item  = NSMenuItem()
                let color: NSColor = count > 0 ? .labelColor : .tertiaryLabelColor
                item.attributedTitle = columned([
                    seg("  \(section)", BarFont.monoSecondary, color),
                    seg("\(count)", BarFont.monoSecondary, color),
                ], stops: [190])
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
            title: "Today (\(todayDateStr)) →",
            action: nil, keyEquivalent: "")
        setIcon(todayParent, "calendar")
        menu.addItem(todayParent)
        menu.setSubmenu(todayMenu, for: todayParent)

        // ─ Knowledge launchers — each opens the dashboard Browse filtered ─────
        menu.addItem(.separator())
        if let b = b {
            // Quiet chroma tier: counts on launcher rows are secondary — only
            // identity (status header) and severity (warnings) stay loud.
            let pi = addClickable(menu, "Patterns",    "\(b.dreamsPatterns)",
                                  valueColor: .secondaryLabelColor, action: #selector(showPatternsDetail))
            setIcon(pi, "brain")
            let ai = addClickable(menu, "Associations", "\(b.associations)",
                                  valueColor: .secondaryLabelColor, action: #selector(showAssociationsDetail))
            setIcon(ai, "link")
            let si = addClickable(menu, "Sessions",
                                  "\(b.dreamsProcessed) dreams  ·  \(b.metacogProcessed) metacog",
                                  valueColor: .secondaryLabelColor, action: #selector(showSessionsDetail))
            setIcon(si, "book.fill")
            if b.metacogAudits > 0 {
                let mi = addClickable(menu, "Metacog audits", "\(b.metacogAudits)",
                                      valueColor: .secondaryLabelColor, action: #selector(showMetacogDetail))
                setIcon(mi, "checkmark.seal.fill")
            }
        }
        // One row replaces the old RECENT INFERENCES wall (digest prose, last
        // cycle, five pattern quotes). The digest + re-run live on the
        // dashboard Overview; the quotes live in Browse where age is visible.
        let ins = addClickable(menu, "Insights", "", action: #selector(showInsightsDetail))
        setIcon(ins, "sparkles")

        // ─ Last error — one line, click to copy; hover for the full text ──────
        if let err = b?.lastError {
            menu.addItem(.separator())
            let one = err.replacingOccurrences(of: "\n", with: "  ")
            let errItem = NSMenuItem()
            errItem.attributedTitle = seg("⚠ \(tailTruncate(one, 64))", BarFont.secondary, .systemRed)
            errItem.toolTip = err
            errItem.action = #selector(copyItemText(_:))
            errItem.target = self
            errItem.isEnabled = true
            errItem.representedObject = err
            setIcon(errItem, "doc.on.clipboard")
            menu.addItem(errItem)
        }

        // ─ Store & lane health — one conditional row; a dead lane names itself ─
        let largeStores = cachedStoreFiles.filter { $0.isLarge }
        let redLanes    = cachedLaneHealth?.lanes.filter { $0.status == "red" }    ?? []
        let yellowLanes = cachedLaneHealth?.lanes.filter { $0.status == "yellow" } ?? []
        if !largeStores.isEmpty || !redLanes.isEmpty || !yellowLanes.isEmpty {
            let healthMenu = NSMenu()

            // Failing lanes first — red, then yellow — each naming its bad fact.
            func laneRow(_ l: LaneHealthLane, _ color: NSColor) {
                let item = NSMenuItem()
                item.attributedTitle = columned([
                    seg("  ● \(l.lane)", BarFont.monoSecondary, color),
                    seg(l.reason, BarFont.monoSecondary, .secondaryLabelColor),
                ], stops: [200])
                item.toolTip = "\(l.lane): \(l.reason)"
                healthMenu.addItem(item)
            }
            for l in redLanes    { laneRow(l, .systemRed) }
            for l in yellowLanes { laneRow(l, .systemYellow) }
            if !redLanes.isEmpty || !yellowLanes.isEmpty { healthMenu.addItem(.separator()) }

            // Then the store-size rows (existing behavior).
            for f in cachedStoreFiles {
                let item = NSMenuItem()
                item.attributedTitle = columned([
                    seg("  \(f.isLarge ? "⚠" : "✓") \(f.label)", BarFont.monoSecondary,
                        f.isLarge ? .labelColor : .secondaryLabelColor),
                    seg("\(f.entries) entries · \(fmtBytes(f.sizeBytes))", BarFont.monoSecondary,
                        f.isLarge ? .systemOrange : .secondaryLabelColor),
                ], stops: [200])
                healthMenu.addItem(item)
            }
            healthMenu.addItem(.separator())
            let pruneItem = NSMenuItem(title: "  Run Prune in Terminal…",
                                       action: #selector(runPrune), keyEquivalent: "")
            pruneItem.target = self
            setIcon(pruneItem, "arrow.3.trianglepath")
            healthMenu.addItem(pruneItem)

            // Parent row leads with dead lanes when present, else the size warning.
            let largeSuffix = largeStores.isEmpty ? "" : " · \(largeStores.count) large"
            let title: String
            let color: NSColor
            if !redLanes.isEmpty {
                title = "⚠ Health — \(redLanes.count) lane\(redLanes.count == 1 ? "" : "s") down\(largeSuffix)"
                color = .systemRed
            } else if !yellowLanes.isEmpty {
                title = "⚠ Health — \(yellowLanes.count) aging\(largeSuffix)"
                color = .systemOrange
            } else {
                title = "⚠ Store Health (\(largeStores.count) large)"
                color = .systemOrange
            }
            let healthParent = NSMenuItem()
            healthParent.attributedTitle = seg(title, BarFont.body, color)
            menu.addItem(healthParent)
            menu.setSubmenu(healthMenu, for: healthParent)
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

        // Low-frequency footer actions collapse into one row (menu diet).
        // Help/About open as small panels (they lost their tabs in the v3
        // cutover); config opens in the editor.
        let moreMenu = NSMenu()
        let helpItem = NSMenuItem(title: "Help & Shortcuts", action: #selector(openHelpPanel), keyEquivalent: "")
        helpItem.target = self
        setIcon(helpItem, "questionmark.circle.fill")
        moreMenu.addItem(helpItem)
        let aboutItem = NSMenuItem(title: "About i-dream", action: #selector(openAboutPanel), keyEquivalent: "")
        aboutItem.target = self
        setIcon(aboutItem, "info.circle.fill")
        moreMenu.addItem(aboutItem)
        let cfg = NSMenuItem(title: "Edit Config in VS Code", action: #selector(openConfigInVSCode), keyEquivalent: "")
        cfg.target = self
        setIcon(cfg, "gearshape.fill")
        moreMenu.addItem(cfg)
        let moreParent = NSMenuItem(title: "More", action: nil, keyEquivalent: "")
        setIcon(moreParent, "ellipsis.circle")
        menu.addItem(moreParent)
        menu.setSubmenu(moreMenu, for: moreParent)

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
        i.attributedTitle = seg(title, BarFont.body, .labelColor)
        i.target = self; i.isEnabled = true
        menu.addItem(i); return i
    }

    private func addSection(_ menu: NSMenu, _ title: String) {
        let i = NSMenuItem()
        i.attributedTitle = seg(title.uppercased(), BarFont.sectionLabel,
                                NSColor.labelColor.withAlphaComponent(0.7))
        i.isEnabled = false; menu.addItem(i)
    }

    private func addColored(_ menu: NSMenu, _ title: String,
                            color: NSColor, font: NSFont = BarFont.title) {
        let i = NSMenuItem()
        i.attributedTitle = seg(title, font, color)
        i.isEnabled = false; menu.addItem(i)
    }

    /// Label + value column at a shared tab stop (real alignment, not pad).
    private static let rowValueStop: CGFloat = 170

    private func addRow(_ menu: NSMenu, _ label: String, _ value: String,
                        valueColor: NSColor? = nil) {
        let i = NSMenuItem()
        i.attributedTitle = columned([
            seg("  \(label)", BarFont.body, .labelColor),
            seg(value, BarFont.monoBody, valueColor ?? .labelColor),
        ], stops: [Self.rowValueStop])
        i.isEnabled = false; menu.addItem(i)
    }

    /// Like addRow but clickable — shows a subtle › arrow and has an action.
    @discardableResult
    private func addClickable(_ menu: NSMenu, _ label: String, _ value: String,
                               valueColor: NSColor? = nil, action: Selector) -> NSMenuItem {
        let i     = NSMenuItem()
        let cell2 = NSMutableAttributedString()
        cell2.append(seg(value, BarFont.monoBody, valueColor ?? .labelColor))
        cell2.append(seg("  ›", BarFont.body, .tertiaryLabelColor))
        i.attributedTitle = columned([
            seg(label, BarFont.body, .labelColor),
            cell2,
        ], stops: [Self.rowValueStop])
        i.action = action; i.target = self; i.isEnabled = true
        menu.addItem(i); return i
    }

    private func addTwoLine(_ menu: NSMenu, top: String, bottom: String) {
        let i    = NSMenuItem()
        let full = NSMutableAttributedString()
        full.append(seg(top + "\n", BarFont.body, .labelColor))
        full.append(seg(bottom, BarFont.secondary, NSColor.labelColor.withAlphaComponent(0.6)))
        i.attributedTitle = full; i.isEnabled = false; menu.addItem(i)
    }

    private func addDim(_ menu: NSMenu, _ title: String) {
        let i = NSMenuItem()
        i.attributedTitle = seg(title, BarFont.secondary,
                                NSColor.labelColor.withAlphaComponent(0.6))
        i.isEnabled = false; menu.addItem(i)
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



    /// v3: Sessions opens the dashboard Journal — the canonical cycle-history
    /// surface — replacing the legacy floating RichText journal panel
    /// (topbar-review.md "Load-bearing coupling to fix"; docs/23 Stage 4).
    @objc private func showSessionsDetail() {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showOrFront(tab: 2)
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
        // Real click-to-run with a confirm (docs/23 Stage 4) — pruning rewrites
        // store files, so it never fires off a stray menu click.
        let a = NSAlert()
        a.messageText = "Prune the knowledge stores?"
        a.informativeText = "Opens Terminal and runs `i-dream prune` to compact the large store files."
        a.addButton(withTitle: "Run in Terminal")
        a.addButton(withTitle: "Cancel")
        NSApp.activate(ignoringOtherApps: true)
        guard a.runModal() == .alertFirstButtonReturn else { return }
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

    /// v3: the Knowledge/Insights menu launchers open the dashboard's
    /// Browse surface filtered by type. Their legacy floating RichText
    /// panels were the "fifth paradigm" the redesign deletes
    /// (topbar-review.md "Load-bearing coupling to fix"; docs/23 Stage 4).
    private func openDashboardBrowse(_ filter: BrowseRow.Kind?) {
        if dashboardController == nil { dashboardController = DashboardWindowController() }
        dashboardController!.showOrFront(tab: 1)
        dashboardController!.browseModel.filter = filter
    }
    @objc private func showPatternsDetail()     { openDashboardBrowse(.pattern) }
    @objc private func showAssociationsDetail() { openDashboardBrowse(.association) }
    @objc private func showInsightsDetail()     { openDashboardBrowse(.insight) }
    @objc private func showMetacogDetail()      { openDashboardBrowse(.metacog) }

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
