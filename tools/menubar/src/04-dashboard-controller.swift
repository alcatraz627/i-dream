
/// Sidebar nav button — flat label+icon button with a coloured background when selected.
/// Does NOT override draw() to avoid the infinite-redraw trap of mutating self.font inside draw().
final class NavSidebarButton: NSButton {
    private var _title  = ""
    private var _symbol = ""
    private var _iconColor: NSColor = .labelColor

    /// Per-tab accent, shown ONLY on the selected tab (v3 4-tab order).
    /// Unselected icons stay monochrome — one loud hue at a time. Journal
    /// deliberately avoids orange: that hue means "association" in content.
    private static let iconColors: [NSColor] = [
        .systemPurple,   // Overview
        .systemCyan,     // Browse — echoes the pattern/content tint
        .systemIndigo,   // Journal
        .systemGreen,    // Search
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
                : .secondaryLabelColor
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
        "System overview — outcomes, stats, digest (⌘1)",
        "Browse all lessons — patterns, associations, insights (⌘2)",
        "Consolidation cycle history and token usage (⌘3)",
        "Search across all knowledge base data (⌘4)",
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
        self.contentTintColor = .secondaryLabelColor
        if index < NavSidebarButton.tabTooltips.count {
            self.toolTip = NavSidebarButton.tabTooltips[index]
        }
        updateAttributedTitle()
    }

    /// Trailing item count, rendered right-aligned in mono — replaces the
    /// old "(463)" parenthetical baked into the title string.
    private var _count: Int?
    func updateCount(_ n: Int?) {
        _count = n
        updateAttributedTitle()
    }

    private func updateAttributedTitle() {
        let weight: NSFont.Weight = isSelectedTab ? .semibold : .regular
        let color: NSColor = isSelectedTab
            ? .labelColor
            : .secondaryLabelColor   // dim unselected for visible hierarchy
        let m = NSMutableAttributedString(string: "  " + _title, attributes: [
            .font: NSFont.systemFont(ofSize: 13, weight: weight),
            .foregroundColor: color,
        ])
        // Count sits at a right-aligned tab stop in mono, so the label never
        // shifts width and the numbers land in one column across tabs.
        if let n = _count {
            let ps = NSMutableParagraphStyle()
            ps.tabStops = [NSTextTab(textAlignment: .right, location: max(40, bounds.width - 34))]
            m.append(NSAttributedString(string: "\t\(n)", attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
                // Tertiary washes out over the selection tint — step up when selected.
                .foregroundColor: isSelectedTab ? NSColor.secondaryLabelColor
                                                : NSColor.tertiaryLabelColor,
            ]))
            m.addAttribute(.paragraphStyle, value: ps, range: NSRange(location: 0, length: m.length))
        }
        self.attributedTitle = m
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
    let searchModel = SearchModel()
    /// Which surface is showing — the key monitor needs it for the
    /// Search tab's arrow-key flow.
    private var currentTab = 0

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
                if let usage = s.usage, usage.limit5h > 0 || usage.limit7d > 0 {
                    ov.usage = (line: usage.warningLine, warn: usage.overWarnThreshold)
                }
            }
            ov.lastActive = lastActivityDate().map { d in
                let secs = Date().timeIntervalSince(d)
                switch secs {
                case ..<60:    return "just now"
                case ..<3600:  return "\(Int(secs / 60))m ago"
                case ..<86400: return "\(Int(secs / 3600))h ago"
                default:       return "\(Int(secs / 86400))d ago"
                }
            }
            ov.signals = signalsCount()
            if let text = dg {
                ov.digest = (text: text, sentiment: readDigestSentiment())
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
                self.searchModel.allRows = browseRows
                self.rebuildContentViews()
                dlog("dashboard: async data loaded (\(pat.count)p/\(assoc.count)a/\(jour.count)j/\(browseRows.count)b)")
                completion?()
            }
        }
    }

    /// Render the panel's view tree to /tmp as PNG — the display-independent
    /// verification affordance (works while the physical display sleeps).
    /// Verification-protocol appearance override — panel-only, transient.
    /// nil follows the system; the durable pref stays with the theme picker.
    func applyTransientAppearance(_ name: NSAppearance.Name?) {
        panel?.appearance = name.flatMap { NSAppearance(named: $0) }
    }

    /// Data-level assertions for the --smoke harness; empty = healthy.
    /// Runs after the async load has had time to land.
    func smokeDataChecks() -> [String] {
        var f: [String] = []
        if browseModel.rows.isEmpty { f.append("browse: 0 rows — stores unreadable?") }
        if browseModel.clusterRows.isEmpty { f.append("browse: 0 repeated-lesson clusters") }
        if journalModel.rows.isEmpty { f.append("journal: 0 cycles") }
        if overviewModel.data.reflect == nil && overviewModel.data.state == nil {
            f.append("overview: neither reflect nor state loaded")
        }
        return f
    }

    func dumpSnapshot() {
        guard let v = panel?.contentView else {
            dlog("dashboard: snapshot requested but no panel")
            return
        }
        guard let rep = v.bitmapImageRepForCachingDisplay(in: v.bounds) else { return }
        v.cacheDisplay(in: v.bounds, to: rep)
        if let data = rep.representation(using: .png, properties: [:]) {
            try? data.write(to: URL(fileURLWithPath: "/tmp/i-dream-dashboard-snap.png"))
            dlog("dashboard: snapshot dumped (\(data.count) bytes)")
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

        // ── Footer — three grouped blocks on an 8px rhythm above a hairline:
        //    actions (export/refresh) · prefs (theme, always-on-top) ·
        //    metadata (build, refreshed). Everything aligns at x=14 with the
        //    nav column; type sits on the DS scale (11 controls / 10 meta).
        let footX: CGFloat = 14
        let footW: CGFloat = sideW - 28

        let footSep = NSBox(frame: NSRect(x: footX, y: 176, width: footW, height: 1))
        footSep.boxType = .separator
        sidebar.addSubview(footSep)

        // HoverButton gives the two actions the same hover wash + hand
        // cursor the theme icons have — one affordance grammar in the column.
        let exportBtn = HoverButton(frame: NSRect(x: footX - 4, y: 144, width: footW + 8, height: 24))
        exportBtn.title            = "⬇  Export JSON"
        exportBtn.target           = self
        exportBtn.action           = #selector(exportDashboardData)
        exportBtn.alignment        = .left
        exportBtn.font             = .systemFont(ofSize: 11)
        exportBtn.tintColor        = .secondaryLabelColor
        exportBtn.contentTintColor = .secondaryLabelColor
        sidebar.addSubview(exportBtn)

        let refreshBtn = HoverButton(frame: NSRect(x: footX - 4, y: 112, width: footW + 8, height: 24))
        refreshBtn.title            = "↺  Refresh  (⌘R)"
        refreshBtn.target           = self
        refreshBtn.action           = #selector(refreshDashboard)
        refreshBtn.alignment        = .left
        refreshBtn.font             = .systemFont(ofSize: 11)
        refreshBtn.tintColor        = .secondaryLabelColor
        refreshBtn.contentTintColor = .secondaryLabelColor
        sidebar.addSubview(refreshBtn)

        // ── Theme picker — three icon-only HoverButtons ───────────────────
        // No background unless hovered. Only the SELECTED theme wears its
        // hue; unselected icons stay monochrome (quiet tier — the old
        // always-tinted yellow/violet/teal trio was three decorative hues).
        let themeRow = NSView(frame: NSRect(x: footX, y: 76, width: footW, height: 28))
        let themes: [(symbol: String, tooltip: String, value: String, tint: NSColor)] = [
            ("sun.max.fill",          "Light theme",  "light",  NSColor.systemYellow),
            ("moon.fill",             "Dark theme",   "dark",   NSColor(red: 0.55, green: 0.41, blue: 0.85, alpha: 1)),
            ("circle.lefthalf.filled","Follow system","system", NSColor.systemTeal),
        ]
        let bw  = (themeRow.bounds.width - 16) / 3   // 3 buttons + 2×8px gaps
        let cur = UserDefaults.standard.string(forKey: dashThemeKey) ?? "dark"
        themePickerButtons.removeAll()
        for (i, t) in themes.enumerated() {
            let btn = HoverButton(frame: NSRect(x: CGFloat(i) * (bw + 8), y: 0,
                                                width: bw, height: 28))
            btn.hoverLabel = t.tooltip   // also drives the HUD-style hover label if delegate set
            btn.tintColor  = t.tint
            btn.toolTip    = t.tooltip
            if let img = NSImage(systemSymbolName: t.symbol, accessibilityDescription: t.tooltip) {
                let cfg = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
                btn.image = img.withSymbolConfiguration(cfg) ?? img
                btn.imagePosition = .imageOnly
            }
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
        aotBtn.frame = NSRect(x: footX, y: 50, width: footW, height: 18)
        aotBtn.font  = .systemFont(ofSize: 11)
        aotBtn.state = UserDefaults.standard.bool(forKey: dashAlwaysOnTopKey) ? .on : .off
        if let panel = panel, aotBtn.state == .on { panel.level = .statusBar }
        sidebar.addSubview(aotBtn)

        // Metadata block: two 16px-pitch lines, 10pt (no more 9.5 strays).
        let verLabel = NSTextField(labelWithString: "build \(BuildInfo.commitHash.prefix(7))")
        verLabel.font      = .monospacedSystemFont(ofSize: 10, weight: .regular)
        verLabel.textColor = .tertiaryLabelColor
        verLabel.frame     = NSRect(x: footX, y: 28, width: footW, height: 14)
        sidebar.addSubview(verLabel)

        let refreshedLbl = NSTextField(labelWithString: "Refreshed just now")
        refreshedLbl.font      = .systemFont(ofSize: 10)
        refreshedLbl.textColor = .tertiaryLabelColor
        refreshedLbl.frame     = NSRect(x: footX, y: 12, width: footW, height: 14)
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
            guard let self, let p = self.panel, ev.window === p else { return ev }
            let mods = ev.modifierFlags.intersection([.command, .option, .control, .shift])
            // Search-tab keyboard flow: ↑/↓ move the selection while the
            // field keeps focus; ⏎ is handled by the field's onSubmit.
            if mods.isEmpty, self.currentTab == 3 {
                if ev.keyCode == 125 { self.searchModel.moveSelection(+1); return nil }
                if ev.keyCode == 126 { self.searchModel.moveSelection(-1); return nil }
            }
            guard mods == .command,
                  let ch = ev.charactersIgnoringModifiers?.first else { return ev }
            if ch >= "1" && ch <= "9" {
                self.selectTab(Int(ch.asciiValue! - Character("1").asciiValue!))
                return nil
            }
            if ch == "r" { self.refreshDashboard(); return nil }
            if ch == "f" || ch == "k" {
                self.selectTab(3)
                self.searchModel.focusToken += 1
                return nil
            }
            return ev
        }
    }

    private func rebuildContentViews() {
        patternDetailTextView = nil
        assocDetailTextView = nil
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
        overviewModel.onRerunInference = { [weak self] in
            let p = Process()
            p.executableURL = URL(fileURLWithPath: resolveIDreamBinary())
            p.arguments = ["dream", "wake"]
            p.standardOutput = FileHandle.nullDevice
            p.standardError = FileHandle.nullDevice
            try? p.run()
            // The digest lands asynchronously; refresh once it has had a chance.
            DispatchQueue.main.asyncAfter(deadline: .now() + 8) { self?.reloadDataAsync() }
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
        journalModel.lookupRow = { [weak self] id in
            self?.browseModel.rows.first(where: { $0.id == id })
        }
        let v2: NSView = {
            let host = NSHostingView(rootView: JournalPane(model: journalModel))
            host.frame = f
            host.autoresizingMask = [.width, .height]
            return host
        }()
        dlog("dashboard: building search")
        searchModel.onOpen = { [weak self] id in
            guard let self else { return }
            self.selectTab(1)
            self.browseModel.jump(to: id)
        }
        let v3: NSView = {
            let host = NSHostingView(rootView: SearchPane(model: searchModel))
            host.frame = f
            host.autoresizingMask = [.width, .height]
            return host
        }()
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
        navButtons[1].updateCount(browseModel.rows.count)
        navButtons[2].updateCount(journal.count)
    }

    // ── Navigation ─────────────────────────────────────────────────────────────

    @objc private func navTapped(_ sender: NSButton) { selectTab(sender.tag) }

    func selectTab(_ index: Int) {
        guard index >= 0 && index < tabs.count else { return }
        currentTab = index
        for (i, btn) in navButtons.enumerated() { btn.isSelectedTab = (i == index) }
        for (i, v) in contentViews.enumerated()  { v.isHidden        = (i != index) }
        UserDefaults.standard.set(index, forKey: "idream-dashboard-selected-tab")
        if index == 3 { searchModel.focusToken += 1 }
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




    /// Fuzzy match: returns true if ALL words in `queryWords` appear in `text`.

    /// Compute a relevance score: higher = better match. Rewards exact substring,
    /// word-boundary matches, and early position.



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
