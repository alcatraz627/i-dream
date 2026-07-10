
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
    // The dropdown's old ACTIVITY block lives here since the Stage-4 menu
    // diet — the menu keeps only glance values, the Overview keeps the stats.
    var usage: (line: String, warn: Bool)?
    var lastActive: String?
    var signals = 0
    var digest: (text: String, sentiment: String)?
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
    var onRerunInference: (() -> Void)?
}

// — DesignKit (AppKit) — ported from claude-instances/native/DesignKit.swift.
//   The dropdown's shared design system: one type scale, segment + columned
//   text builders, one truncation rule per field kind. Menu helpers feed these
//   instead of hand-rolling NSAttributedString math and left-pad alignment. —

enum BarFont {
    /// User font-size multiplier (`ui.fontScale`, default 1.0). Read at render
    /// time and clamped to a sane range. No settings control ships until a
    /// live reader exists; the default keeps rendering unchanged.
    static var scale: CGFloat {
        let s = UserDefaults.standard.double(forKey: "ui.fontScale")
        return s > 0 ? min(1.6, max(0.7, CGFloat(s))) : 1.0
    }
    static var title:         NSFont { .systemFont(ofSize: 13 * scale, weight: .semibold) }  // identity/status
    static var body:          NSFont { .systemFont(ofSize: 14 * scale, weight: .regular) }   // rows, prose
    static var secondary:     NSFont { .systemFont(ofSize: 13 * scale, weight: .regular) }   // dim/detail rows
    static var caption:       NSFont { .systemFont(ofSize: 11 * scale, weight: .regular) }   // footnotes
    static var monoBody:      NSFont { .monospacedSystemFont(ofSize: 14 * scale, weight: .regular) }
    static var monoSecondary: NSFont { .monospacedSystemFont(ofSize: 13 * scale, weight: .regular) }
    static var sectionLabel:  NSFont { .systemFont(ofSize: 12 * scale, weight: .semibold) }
}

/// One styled text segment; `columned` and the menu helpers concatenate them.
func seg(_ text: String, _ font: NSFont, _ color: NSColor) -> NSAttributedString {
    NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: color])
}

/// Real column alignment via tab stops — replaces hand-padded spaces, which
/// break whenever content width changes. Cell 0 sits at the row origin,
/// cells 1..n at their stop (pt).
func columned(_ cells: [NSAttributedString], stops: [CGFloat]) -> NSAttributedString {
    let ps = NSMutableParagraphStyle()
    ps.tabStops = stops.map { NSTextTab(textAlignment: .left, location: $0) }
    ps.defaultTabInterval = 0
    let m = NSMutableAttributedString()
    for (i, cell) in cells.enumerated() {
        if i > 0 { m.append(NSAttributedString(string: "\t")) }
        m.append(cell)
    }
    m.addAttribute(.paragraphStyle, value: ps, range: NSRange(location: 0, length: m.length))
    return m
}

/// Identifiers and one-line prose tail-truncate; keep the head.
func tailTruncate(_ s: String, _ maxChars: Int) -> String {
    s.count <= maxChars ? s : String(s.prefix(max(0, maxChars - 1))) + "…"
}

// — Shared design tokens + row primitives (visual-design.md: one type
//   scale, one spacing rhythm, quiet ambient color, explicit affordances) —

enum DS {
    // Spacing rhythm: everything sits on 4/8/12/16. No 5s, 6s, 9s, 10s.
    static let half: CGFloat = 4
    static let unit: CGFloat = 8
    static let pad: CGFloat = 12
    static let padWide: CGFloat = 16
    // Type scale — six roles, one home for every SwiftUI pane (the AppKit
    // menu has its BarFont mirror). Sizes outside this scale are a defect.
    static let display = Font.system(size: 15, weight: .semibold)  // window identity
    static let title = Font.system(size: 13, weight: .semibold)    // pane headings, key values
    static let body = Font.system(size: 12)                        // prose, primary content
    static let label = Font.system(size: 11)                       // row titles, controls
    static let caption = Font.system(size: 10)                     // footnotes, hints
    static let micro = Font.system(size: 9, weight: .semibold)     // tile labels, tracked caps
    /// Ambient (quiet) tier: recognizable hue, de-chromaed so it recedes.
    static func quiet(_ c: Color) -> Color { c.opacity(0.55) }
    static let surface = Color.primary.opacity(0.05)
    static let surfaceHover = Color.primary.opacity(0.09)
}

func cleanRowTitle(_ t: String) -> String {
    var s = t.trimmingCharacters(in: .whitespaces)
    while s.hasPrefix(">") { s = String(s.dropFirst()).trimmingCharacters(in: .whitespaces) }
    return s
}

/// Markdown-aware detail rendering (bold/code kept, blockquote syntax
/// stripped); falls back to the raw string — syntax beats nothing.
func richDetailText(_ raw: String) -> AttributedString {
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

func rowAgeLabel(_ d: Int?) -> String {
    guard let d else { return "—" }
    if d == 0 { return "today" }
    if d < 30 { return "\(d)d" }
    if d < 365 { return "\(d / 30)mo" }
    return "\(d / 365)y"
}

func rowAgeColor(_ d: Int?) -> Color {
    guard let d else { return .secondary }
    return d >= 30 ? .orange : .secondary
}

/// The in-pane preview shown on the FIRST click of any cross-entity
/// reference. Reviewing happens in place; jumping tabs is the explicit
/// second action ("Open in Browse ↗") — this kills the click→teleport
/// whiplash the user flagged.
struct RowPreviewPanel: View {
    let row: BrowseRow
    var onOpenInBrowse: (String) -> Void
    var onPreview: (String) -> Void
    var onClose: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: DS.unit) {
            HStack(spacing: 6) {
                Image(systemName: row.kind.symbol)
                    .foregroundColor(row.kind.tint)
                Text(row.kind.rawValue.dropLast())
                    .font(DS.caption).foregroundColor(.secondary)
                if row.clusterSize > 1 {
                    Text("×\(row.clusterSize)")
                        .font(DS.caption.weight(.semibold).monospacedDigit())
                        .padding(.horizontal, DS.half).padding(.vertical, 1)
                        .background(DS.quiet(row.kind.tint).opacity(0.25))
                        .clipShape(Capsule())
                }
                Spacer()
                if let c = row.confidence {
                    Text("\(Int(c * 100))%").font(DS.caption.monospacedDigit())
                        .foregroundColor(.secondary)
                }
                Text(rowAgeLabel(row.ageDays))
                    .font(DS.caption.monospacedDigit())
                    .foregroundColor(rowAgeColor(row.ageDays))
                Button(action: onClose) {
                    Image(systemName: "xmark.circle.fill").foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
                .help("Close preview")
            }
            Text(cleanRowTitle(row.title))
                .font(DS.title)
                .lineLimit(3)
                .textSelection(.enabled)
            Divider()
            ScrollView {
                Text(richDetailText(row.detail))
                    .font(DS.body)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if !row.linkedChips.isEmpty {
                Divider()
                VStack(alignment: .leading, spacing: 3) {
                    Text(row.kind == .association ? "LINKED PATTERNS" : "BUILT-ON ASSOCIATIONS")
                        .font(DS.micro)
                        .foregroundColor(.secondary)
                    ForEach(Array(row.linkedChips.enumerated()), id: \.offset) { _, chip in
                        Button(action: { onPreview(chip.target) }) {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.right.circle").font(DS.micro)
                                Text(chip.label + "…").font(DS.caption).lineLimit(1)
                            }
                            .foregroundColor(.accentColor)
                        }
                        .buttonStyle(.plain)
                        .onHover { inside in
                            if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                        }
                    }
                }
            }
            HStack {
                Button(action: { onOpenInBrowse(row.id) }) {
                    Label("Open in Browse", systemImage: "arrow.up.forward.square")
                        .font(DS.caption)
                }
                .buttonStyle(.bordered)
                Spacer()
            }
        }
        .padding(12)
        .frame(width: 360, alignment: .topLeading)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(DS.surface)
    }
}

// — Model + view —

final class BrowseModel: ObservableObject {
    @Published var rows: [BrowseRow] = []
    @Published var totals = BrowseTotals()
    @Published var filter: BrowseRow.Kind? = nil
    @Published var expandedId: String? = nil
    // Cluster-panel state lives on the model (not view @State) so the
    // verification protocol can drive it and it survives tab switches.
    @Published var showClusterMap = false
    @Published var hoverClusterId: String? = nil
    /// Writes the rating and refreshes; wired by the dashboard controller.
    var onRate: ((String, String) -> Void)?

    /// Set when a linked chip is clicked; the view scrolls there and expands.
    @Published var jumpTarget: String? = nil

    /// Repeated lessons for the cluster panel, sorted once per data refresh.
    /// The panel re-renders on every hover event, so it must never re-filter
    /// and re-sort the full row set per mouse move.
    private(set) var clusterRows: [BrowseRow] = []

    func apply(rows: [BrowseRow], totals: BrowseTotals) {
        self.rows = rows
        self.totals = totals
        clusterRows = rows.filter { $0.clusterSize > 1 }
                          .sorted { $0.clusterSize > $1.clusterSize }
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

    /// First-click preview: show the referenced row in the side panel
    /// without moving the list or switching tabs.
    @Published var previewRow: BrowseRow? = nil
    func preview(id: String) {
        previewRow = rows.first(where: { $0.id == id })
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

    @State private var clusterQuery = ""

    var body: some View {
        HStack(spacing: 0) {
            listColumn
            if let preview = model.previewRow {
                Divider()
                RowPreviewPanel(
                    row: preview,
                    onOpenInBrowse: { id in
                        model.previewRow = nil
                        model.jump(to: id)
                    },
                    onPreview: { id in model.preview(id: id) },
                    onClose: { model.previewRow = nil })
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var listColumn: some View {
        VStack(alignment: .leading, spacing: 0) {
            chipBar
            Divider()
            if model.showClusterMap {
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
    }

    private var chipBar: some View {
        HStack(spacing: DS.unit) {
            chip(nil, label: "All (\(model.rows.count))")
            ForEach(BrowseRow.Kind.allCases) { k in
                chip(k, label: "\(k.rawValue) (\(model.count(of: k)))")
            }
            Spacer()
            Button(action: { model.showClusterMap.toggle() }) {
                Label("Clusters", systemImage: "circle.hexagongrid.fill")
                    .font(DS.label.weight(.medium))
            }
            .buttonStyle(.bordered)
            .tint(model.showClusterMap ? Color.accentColor : Color.secondary)
            .help("Cluster map — repeated lessons ranked by recurrence")
        }
        .padding(.horizontal, DS.pad)
        .padding(.vertical, DS.unit)
    }

    /// The cluster panel: repeated lessons as a ranked thin-bar list — the
    /// Overview's TOP REPEATED LESSONS idiom, because bar length reads
    /// magnitude honestly where bubble area does not. Hovering a row lights
    /// up the clusters it cross-links to (pattern ↔ association) and recedes
    /// the rest; typing dims non-matches; clicking jumps to the row.
    private var clusterMap: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Highlight clusters…", text: $clusterQuery)
                .textFieldStyle(.roundedBorder)
                .font(DS.label)
                .frame(maxWidth: 320)
            let clusters = model.clusterRows
            let maxSize = clusters.first?.clusterSize ?? 1   // sorted desc
            // The hovered cluster plus everything it links to. Chips are
            // written on both sides of a link, but scan the reverse direction
            // too so emphasis never looks one-directional.
            let linked: Set<String> = {
                guard let h = model.hoverClusterId else { return [] }
                var s: Set<String> = [h]
                if let row = clusters.first(where: { $0.id == h }) {
                    for chip in row.linkedChips { s.insert(chip.target) }
                }
                for r in clusters where r.linkedChips.contains(where: { $0.target == h }) {
                    s.insert(r.id)
                }
                return s
            }()
            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(clusters) { c in
                        clusterBar(c, maxSize: maxSize, linked: linked)
                    }
                }
            }
            .frame(maxHeight: 320)
            // The fixed-height footer doubles as the partner readout: partners
            // of big clusters usually rank below the fold, so emphasis alone
            // cannot show them — print them here while a linked row is hovered.
            Group {
                if let h = model.hoverClusterId,
                   let row = clusters.first(where: { $0.id == h }),
                   !row.linkedChips.isEmpty {
                    Text("⛓ \(row.linkedChips.count) linked:  " +
                         row.linkedChips.map { $0.label }.joined(separator: "  ·  "))
                } else {
                    Text("\(clusters.count) repeated lessons · hover a row to see its cross-links · click to jump")
                }
            }
            .font(DS.caption)
            .foregroundColor(.secondary)
            .lineLimit(1)
        }
        .padding(.horizontal, DS.unit)
        .padding(.vertical, DS.unit)
        .background(Color.primary.opacity(0.03))
    }

    private func clusterBar(_ c: BrowseRow, maxSize: Int, linked: Set<String>) -> some View {
        let q = clusterQuery.trimmingCharacters(in: .whitespaces).lowercased()
        let words = q.split(separator: " ").map(String.init)
        let matchesQuery = q.isEmpty || words.allSatisfy { c.title.lowercased().contains($0) }
        // Emphasis: with no hover every matching row is full strength; while
        // hovering, only the linked set stays loud. Opacity-only, fixed row
        // height — hover must never shift layout (sibling anti-idea A-7).
        let emphasized = matchesQuery && (model.hoverClusterId == nil || linked.contains(c.id))
        return Button(action: {
            model.showClusterMap = false
            model.jump(to: c.id)
        }) {
            HStack(spacing: 8) {
                Image(systemName: c.kind.symbol)
                    .font(DS.caption)
                    .foregroundColor(emphasized ? c.kind.tint : .secondary.opacity(0.3))
                    .frame(width: 14)
                Text(c.title)
                    .font(DS.label)
                    .lineLimit(1)
                    .foregroundColor(emphasized ? .primary : .secondary.opacity(0.35))
                    .frame(width: 320, alignment: .leading)
                GeometryReader { geo in
                    RoundedRectangle(cornerRadius: 2)
                        .fill(c.kind.tint.opacity(emphasized ? 0.65 : 0.10))
                        .frame(width: max(3, geo.size.width * CGFloat(c.clusterSize) / CGFloat(maxSize)))
                        .frame(maxHeight: .infinity, alignment: .center)
                }
                .frame(height: 10)
                Text("×\(c.clusterSize)")
                    .font(DS.caption.weight(.semibold).monospacedDigit())
                    .foregroundColor(emphasized ? .primary : .secondary.opacity(0.35))
                    .frame(width: 30, alignment: .trailing)
                HStack(spacing: 2) {
                    Image(systemName: "link").font(DS.micro)
                    Text("\(c.linkedChips.count)").font(DS.caption.monospacedDigit())
                }
                .foregroundColor(c.linkedChips.isEmpty ? .clear
                                 : emphasized ? .secondary : .secondary.opacity(0.25))
                .frame(width: 26, alignment: .trailing)
            }
            .frame(height: 22)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { inside in
            if inside {
                model.hoverClusterId = c.id
                NSCursor.pointingHand.push()
            } else {
                if model.hoverClusterId == c.id { model.hoverClusterId = nil }
                NSCursor.pop()
            }
        }
        .help(c.linkedChips.isEmpty ? "\(c.title) — ×\(c.clusterSize)"
              : "\(c.title) — ×\(c.clusterSize) · linked: \(c.linkedChips.map { $0.label }.joined(separator: " · "))")
    }

    private func chip(_ kind: BrowseRow.Kind?, label: String) -> some View {
        let active = model.filter == kind
        return Button(action: { model.filter = kind }) {
            Text(label)
                .font(DS.label.weight(active ? .semibold : .regular))
                .padding(.horizontal, DS.unit)
                .padding(.vertical, 4)
                .background((kind?.tint ?? .secondary).opacity(active ? 0.30 : 0.12))
                .overlay(Capsule().strokeBorder(
                    (kind?.tint ?? .secondary).opacity(active ? 0.55 : 0.0), lineWidth: 1))
                .foregroundColor(active ? .primary : .secondary)
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .onHover { inside in
            if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
        }
    }

    private func rowView(_ row: BrowseRow) -> some View {
        Button(action: {
            model.expandedId = (model.expandedId == row.id) ? nil : row.id
        }) {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: row.kind.symbol)
                    .font(DS.label)
                    .foregroundColor(row.kind.tint)
                    .frame(width: 16)
                    .padding(.top, 2)
                VStack(alignment: .leading, spacing: 2) {
                    Text(cleanRowTitle(row.title))
                        .font(DS.body.weight(.medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundColor(.primary)
                    Text(metaLine(row))
                        .font(DS.caption)
                        .lineLimit(1)
                        .foregroundColor(.secondary)
                }
                Spacer(minLength: 8)
                if row.clusterSize > 1 {
                    Text("×\(row.clusterSize)")
                        .font(DS.caption.weight(.semibold).monospacedDigit())
                        .padding(.horizontal, DS.half).padding(.vertical, 1)
                        .background(row.kind.tint.opacity(0.18))
                        .clipShape(Capsule())
                }
                if let r = row.rating {
                    Text(r == "up" ? "👍" : "👎").font(DS.caption)
                }
                if let c = row.confidence {
                    Text("\(Int(c * 100))%")
                        .font(DS.caption.monospacedDigit())
                        .foregroundColor(.secondary)
                        .frame(width: 34, alignment: .trailing)
                }
                Text(rowAgeLabel(row.ageDays))
                    .font(DS.caption.monospacedDigit())
                    .foregroundColor(rowAgeColor(row.ageDays))
                    .frame(width: 38, alignment: .trailing)
            }
            .padding(.horizontal, DS.unit)
            .padding(.vertical, DS.half)
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

    private func detailView(_ row: BrowseRow) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ScrollView {
                Text(richDetailText(row.detail))
                    .font(DS.body)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 240)
            if !row.linkedChips.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text(row.kind == .association ? "LINKED PATTERNS" : "BUILT-ON ASSOCIATIONS")
                        .font(DS.micro)
                        .foregroundColor(.secondary)
                    ForEach(Array(row.linkedChips.enumerated()), id: \.offset) { _, chip in
                        Button(action: { model.preview(id: chip.target) }) {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.right.circle")
                                    .font(DS.caption)
                                Text(chip.label + "…")
                                    .font(DS.caption)
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
                    Text(cat).font(DS.caption)
                        .padding(.horizontal, DS.half).padding(.vertical, 2)
                        .background(Color.secondary.opacity(0.15))
                        .clipShape(Capsule())
                }
                if row.clusterSize > 1 {
                    Text("\(row.clusterSize) rewordings of this lesson collapsed into one row")
                        .font(DS.caption).foregroundColor(.secondary)
                }
                Spacer()
                if row.kind == .insight {
                    Button("👍") { model.onRate?(row.ratingId, "up") }.buttonStyle(.plain)
                    Button("👎") { model.onRate?(row.ratingId, "down") }.buttonStyle(.plain)
                }
            }
        }
        .padding(.horizontal, DS.pad)
        .padding(.vertical, DS.unit)
        .background(Color.primary.opacity(0.04))
    }

    private var footer: some View {
        HStack {
            Text(model.totals.note + " · \(model.totals.clusters) deduped clusters")
                .font(DS.caption)
                .foregroundColor(.secondary)
            Spacer()
        }
        .padding(.horizontal, DS.unit)
        .padding(.vertical, DS.half)
    }

    /// Markdown-aware detail rendering (bold, code, italics) that keeps the
    /// store's line breaks. Falls back to the raw string on parse failure —
    /// showing syntax beats showing nothing.

}

// — Search: the query spine over the deduped knowledge base —
// Results are Browse rows (cluster representatives), so grouping-by-lesson
// is inherent: the push-approval query returns ONE ×22 row, not ten
// rewordings. Quoted "exact phrases" are honored — the old pane's
// "planned for V2" confession, shipped.

final class SearchModel: ObservableObject {
    @Published var query = ""
    @Published var selected = 0
    /// Incremented by the controller when ⌘F/tab-switch wants the field focused.
    @Published var focusToken = 0
    /// Snapshot of Browse rows; refreshed by every dashboard data load.
    var allRows: [BrowseRow] = []
    var onOpen: ((String) -> Void)?

    struct Hit: Identifiable {
        let row: BrowseRow
        let score: Int
        var id: String { row.id }
    }

    var hits: [Hit] {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard q.count >= 2 else { return [] }
        var phrases: [String] = []
        var rest = q.lowercased()
        while let a = rest.firstIndex(of: "\"") {
            let after = rest.index(after: a)
            guard let b = rest[after...].firstIndex(of: "\"") else { break }
            let phrase = String(rest[after..<b]).trimmingCharacters(in: .whitespaces)
            if !phrase.isEmpty { phrases.append(phrase) }
            rest.removeSubrange(a...b)
        }
        let words = rest.split(separator: " ").map(String.init).filter { !$0.isEmpty }
        guard !(phrases.isEmpty && words.isEmpty) else { return [] }

        var out: [Hit] = []
        for row in allRows {
            let title = row.title.lowercased()
            let body = row.detail.lowercased()
            var ok = true
            for p in phrases where !title.contains(p) && !body.contains(p) { ok = false; break }
            if !ok { continue }
            var score = phrases.count * 4
            for w in words {
                if title.contains(w) { score += 3 }
                else if body.contains(w) { score += 1 }
                else { ok = false; break }
            }
            if !ok { continue }
            if let age = row.ageDays, age <= 14 { score += 1 }
            if row.clusterSize > 1 { score += 1 }
            out.append(Hit(row: row, score: score))
        }
        return out.sorted {
            if $0.score != $1.score { return $0.score > $1.score }
            return ($0.row.ageDays ?? .max) < ($1.row.ageDays ?? .max)
        }
    }

    func moveSelection(_ delta: Int) {
        let n = hits.count
        guard n > 0 else { return }
        selected = min(max(0, selected + delta), n - 1)
    }

    func openSelected() {
        let h = hits
        guard !h.isEmpty else { return }
        previewRow = h[min(selected, h.count - 1)].row
    }

    /// First click (and ⏎) preview in place; tab-jump only from the
    /// preview's explicit "Open in Browse".
    @Published var previewRow: BrowseRow? = nil
}

struct SearchPane: View {
    @ObservedObject var model: SearchModel
    @FocusState private var fieldFocused: Bool

    var body: some View {
        HStack(spacing: 0) {
            searchColumn
            if let preview = model.previewRow {
                Divider()
                RowPreviewPanel(
                    row: preview,
                    onOpenInBrowse: { id in
                        model.previewRow = nil
                        model.onOpen?(id)
                    },
                    onPreview: { id in
                        if let r = model.allRows.first(where: { $0.id == id }) {
                            model.previewRow = r
                        }
                    },
                    onClose: { model.previewRow = nil })
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var searchColumn: some View {
        VStack(alignment: .leading, spacing: 0) {
            TextField("Search all lessons — words match independently, \"quotes\" match exactly", text: $model.query)
                .textFieldStyle(.roundedBorder)
                .font(DS.body)
                .focused($fieldFocused)
                .onSubmit { model.openSelected() }
                .padding(12)
                .onChange(of: model.query) { _ in model.selected = 0 }
                .onChange(of: model.focusToken) { _ in fieldFocused = true }
                .onAppear { fieldFocused = true }
            Divider()
            if model.query.trimmingCharacters(in: .whitespaces).count < 2 {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Type to search patterns, associations, insights and metacog — one row per lesson (rewordings are collapsed).")
                    Text("↑↓ select · ⏎ open in Browse · \"quoted phrases\" match exactly")
                        .foregroundColor(.secondary)
                }
                .font(DS.body)
                .padding(16)
                Spacer()
            } else {
                let hits = model.hits
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(Array(hits.enumerated()), id: \.element.id) { i, hit in
                                hitRow(hit.row, isSelected: i == model.selected)
                                    .id(hit.id)
                                    .onTapGesture {
                                        model.selected = i
                                        model.previewRow = hit.row
                                    }
                            }
                        }
                    }
                    .onChange(of: model.selected) { sel in
                        guard sel < hits.count else { return }
                        proxy.scrollTo(hits[sel].id)
                    }
                }
                Divider()
                HStack {
                    Text("\(hits.count) of \(model.allRows.count) lessons match")
                        .font(DS.caption)
                        .foregroundColor(.secondary)
                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, DS.half)
            }
        }
    }

    private func hitRow(_ row: BrowseRow, isSelected: Bool) -> some View {
        HStack(spacing: 8) {
            Image(systemName: row.kind.symbol)
                .font(DS.label)
                .foregroundColor(row.kind.tint)
                .frame(width: 16)
            Text(row.title)
                .font(DS.body)
                .lineLimit(1)
            Spacer(minLength: 8)
            if row.clusterSize > 1 {
                Text("×\(row.clusterSize)")
                    .font(DS.caption.weight(.semibold).monospacedDigit())
                    .padding(.horizontal, DS.half).padding(.vertical, 1)
                    .background(row.kind.tint.opacity(0.18))
                    .clipShape(Capsule())
            }
            if let c = row.confidence {
                Text("\(Int(c * 100))%")
                    .font(DS.caption.monospacedDigit())
                    .foregroundColor(.secondary)
                    .frame(width: 34, alignment: .trailing)
            }
            Text(row.ageDays.map { $0 == 0 ? "today" : "\($0)d" } ?? "—")
                .font(DS.caption.monospacedDigit())
                .foregroundColor(.secondary)
                .frame(width: 40, alignment: .trailing)
        }
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(isSelected ? Color.accentColor.opacity(0.18) : Color.clear)
        .contentShape(Rectangle())
    }
}

// — Journal: cycle history with exact numbers + cross-nav into Browse —

struct JournalRowVM: Identifiable {
    let id: String
    let dateLabel: String
    let agoLabel: String
    let counts: String
    let tokens: Int
    let ageDays: Int
    /// Patterns whose first_seen falls in this cycle's window, as
    /// (label, Browse row id) chips — approximate but honest join.
    let chips: [(label: String, target: String)]
}

final class JournalModel: ObservableObject {
    @Published var rows: [JournalRowVM] = []
    @Published var heatEntries: [(date: Date, tokens: Int)] = []
    @Published var query = ""
    /// nil = all time; otherwise trailing N days.
    @Published var rangeDays: Int? = nil
    @Published var previewRow: BrowseRow? = nil
    var onJumpToBrowse: ((String) -> Void)?
    var lookupRow: ((String) -> BrowseRow?)?

    var filtered: [JournalRowVM] {
        var out = rows
        if let r = rangeDays { out = out.filter { $0.ageDays <= r } }
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        if !q.isEmpty {
            out = out.filter { row in
                row.dateLabel.lowercased().contains(q)
                    || row.counts.lowercased().contains(q)
                    || row.chips.contains { $0.label.lowercased().contains(q) }
            }
        }
        return out
    }
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
            ageDays: max(0, Int(Date().timeIntervalSince(ts) / 86400)),
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
        HStack(spacing: 0) {
            journalColumn
            if let preview = model.previewRow {
                Divider()
                RowPreviewPanel(
                    row: preview,
                    onOpenInBrowse: { id in
                        model.previewRow = nil
                        model.onJumpToBrowse?(id)
                    },
                    onPreview: { id in
                        if let r = model.lookupRow?(id) { model.previewRow = r }
                    },
                    onClose: { model.previewRow = nil })
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var journalColumn: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !model.heatEntries.isEmpty {
                HeatMapWrapper(entries: model.heatEntries)
                    .frame(height: 64)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
            }
            HStack(spacing: DS.unit) {
                TextField("Filter cycles…", text: $model.query)
                    .textFieldStyle(.roundedBorder)
                    .font(DS.body)
                    .frame(maxWidth: 260)
                rangeChip(nil, "All")
                rangeChip(7, "7d")
                rangeChip(30, "30d")
                Spacer()
                Text("\(model.filtered.count) of \(model.rows.count) cycles")
                    .font(DS.caption).foregroundColor(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, DS.unit)
            Divider()
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.filtered) { row in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 8) {
                                Text(row.dateLabel)
                                    .font(DS.body.weight(.semibold))
                                Text("·  \(row.agoLabel)")
                                    .font(DS.label)
                                    .foregroundColor(.secondary)
                                Spacer()
                                Text("\(row.tokens.formatted()) tokens")
                                    .font(DS.label.monospacedDigit())
                                    .foregroundColor(.secondary)
                            }
                            Text(row.counts)
                                .font(DS.label)
                                .foregroundColor(.secondary)
                            if !row.chips.isEmpty {
                                VStack(alignment: .leading, spacing: 2) {
                                    ForEach(Array(row.chips.enumerated()), id: \.offset) { _, chip in
                                        Button(action: {
                                            if let r = model.lookupRow?(chip.target) { model.previewRow = r }
                                        }) {
                                            HStack(spacing: 4) {
                                                Image(systemName: "arrow.right.circle")
                                                    .font(DS.micro)
                                                Text(chip.label + "…")
                                                    .font(DS.caption)
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
    }

    private func rangeChip(_ days: Int?, _ label: String) -> some View {
        let active = model.rangeDays == days
        return Button(action: { model.rangeDays = days }) {
            Text(label)
                .font(DS.label.weight(active ? .semibold : .regular))
                .padding(.horizontal, DS.unit).padding(.vertical, 4)
                .background(Color.secondary.opacity(active ? 0.28 : 0.10))
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
    }
}

// — Overview: felt-value first, then honest visualization —

struct OverviewPane: View {
    @ObservedObject var model: OverviewModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                feltValueCard
                statusRow
                if let d = model.data.digest { digestCard(d) }
                if !model.data.viz.topLessons.isEmpty { topLessons }
                if !model.data.viz.kindCounts.isEmpty { distribution }
                if !model.data.viz.timelines.isEmpty { timelines }
            }
            .padding(DS.padWide)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    /// The dream→behavior loop, first. Same content as the menu's top block.
    private var feltValueCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let pending = model.data.reviewPending {
                Button(action: { model.onOpenReview?() }) {
                    Label("Open weekly review (\(pending))", systemImage: "checklist")
                        .font(DS.body.weight(.semibold))
                }
                .buttonStyle(.borderedProminent)
                .tint(.orange)
                .onHover { inside in
                    if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                }
            }
            if let r = model.data.reflect {
                HStack(spacing: 6) {
                    Text("Mistakes:")
                        .font(DS.title)
                    Text("\(r.summary.landing) landing")
                        .font(DS.body)
                        .foregroundColor(.green)
                    Text("·").foregroundColor(.secondary)
                    Text("\(r.summary.worsening) worsening")
                        .font(DS.body)
                        .foregroundColor(r.summary.worsening > 0 ? .orange : .secondary)
                }
                if let worst = r.patterns.first(where: { $0.trend == "worsening" }) {
                    Text("↑ \(worst.slug)")
                        .font(DS.label)
                        .foregroundColor(.orange)
                }
            } else if model.data.reviewPending == nil {
                Text("No reflect data yet — run a few cycles.")
                    .font(DS.body).foregroundColor(.secondary)
            }
        }
        .padding(DS.pad)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.05))
        .cornerRadius(8)
    }

    /// The stat strip. Usage, last-active, and signals moved here from the
    /// dropdown's old ACTIVITY block in the Stage-4 menu diet.
    private var statusRow: some View {
        HStack(spacing: DS.unit) {
            if let s = model.data.state {
                statTile("CYCLES", "\(s.cycles)")
                statTile("TOKENS", fmtNum(s.tokens))
                if let last = s.lastDream { statTile("LAST DREAM", last) }
            }
            if let u = model.data.usage {
                statTile("USAGE", u.line, tint: u.warn ? .orange : nil)
            }
            if let la = model.data.lastActive { statTile("LAST ACTIVE", la) }
            if model.data.signals > 0 { statTile("SIGNALS", fmtNum(model.data.signals)) }
            Spacer()
        }
    }

    private func statTile(_ label: String, _ value: String, tint: Color? = nil) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(DS.micro)
                .foregroundColor(.secondary)
                .kerning(0.5)
            Text(value)
                .font(DS.title.monospacedDigit())
                .foregroundColor(tint ?? .primary)
        }
        .padding(.horizontal, DS.unit)
        .padding(.vertical, DS.half)
        .background(DS.surface)
        .cornerRadius(6)
    }

    /// "Recent Dreams Inference" — the 3-hourly prose synthesis that used to
    /// wall the dropdown; sentiment tints the text as the menu did.
    private func digestCard(_ d: (text: String, sentiment: String)) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            sectionHeader("RECENT DREAMS INFERENCE")
            Text(d.text)
                .font(DS.body)
                .foregroundColor(d.sentiment == "positive" ? .green
                               : d.sentiment == "negative" ? .orange : .primary)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            HStack(spacing: 10) {
                Text("updated every 3h")
                    .font(DS.caption)
                    .foregroundColor(.secondary)
                Button(action: { model.onRerunInference?() }) {
                    Label("Re-run", systemImage: "arrow.clockwise")
                        .font(DS.caption)
                }
                .buttonStyle(.borderless)
                .onHover { inside in
                    if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                }
            }
        }
        .padding(DS.pad)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(DS.surface)
        .cornerRadius(8)
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
                            .font(DS.label)
                            .lineLimit(1)
                            .frame(width: 340, alignment: .leading)
                        GeometryReader { geo in
                            RoundedRectangle(cornerRadius: 2)
                                .fill(lesson.kind.tint.opacity(0.65))
                                .frame(width: max(4, geo.size.width * CGFloat(lesson.size) / CGFloat(maxSize)))
                        }
                        .frame(height: 12)
                        Text("\(lesson.size)")
                            .font(DS.label.weight(.semibold).monospacedDigit())
                            .frame(width: 26, alignment: .trailing)
                        Image(systemName: "chevron.right")
                            .font(DS.micro)
                            .foregroundColor(.secondary.opacity(0.6))
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .onHover { inside in
                    if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                }
            }
        }
    }

    private var distribution: some View {
        VStack(alignment: .leading, spacing: 6) {
            sectionHeader("STORES")
            ForEach(Array(model.data.viz.kindCounts.enumerated()), id: \.offset) { _, kc in
                HStack(spacing: 8) {
                    Text(kc.0).font(DS.label).frame(width: 100, alignment: .leading)
                    Text("\(kc.1) items → \(kc.2) lessons")
                        .font(DS.label.monospacedDigit())
                        .foregroundColor(.secondary)
                }
            }
            let v = model.data.viz.valence
            let total = max(1, v.pos + v.neu + v.neg)
            HStack(spacing: 8) {
                Text("Valence").font(DS.label).frame(width: 100, alignment: .leading)
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
                    .font(DS.caption.monospacedDigit())
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
                        .font(DS.label)
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
                        .font(DS.caption.monospacedDigit())
                        .foregroundColor(.secondary)
                }
            }
        }
    }

    private func sectionHeader(_ s: String) -> some View {
        Text(s)
            .font(DS.micro)
            .foregroundColor(.secondary)
            .kerning(0.5)
    }
}
