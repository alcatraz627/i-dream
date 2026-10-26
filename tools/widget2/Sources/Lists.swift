import SwiftUI

// The Lists group: one home for every kind of item the panes only mention,
// each a searchable list whose rows open to show everything the item points at.

/// What each list shows: the index kinds it holds, its columns, and the
/// column its filter chips are made from.
struct ListSpec {
    var kinds: [String]
    var columns: [String]
    var chipColumn: String?
    var sub: String

    static func of(_ p: Pane) -> ListSpec? {
        switch p {
        case .proposals: ListSpec(kinds: ["proposal"], columns: ["status", "category", "finding"], chipColumn: "status",
                                  sub: "Every proposal in the backlog. Finding names the weekly finding you filed it from; the slug's movement after it shows on that finding's effect in Landing.")
        case .hints: ListSpec(kinds: ["hint", "nudge"], columns: ["state", "sessions", "cited", "promoted by"], chipColumn: "state",
                              sub: "Every hint and nudge: its state, how many sessions were shown it, whether Claude cited it, and who made it live. Cited reads the firing scan; an item no scan has judged says not checked yet. Open a row to promote or demote it.")
        case .intentions: ListSpec(kinds: ["intention"], columns: ["fires", "cited", "keywords"], chipColumn: nil,
                                   sub: "Reminders a future session gets when its first prompt matches the keywords.")
        case .insights: ListSpec(kinds: ["insight"], columns: ["confidence", "cited", "intention"], chipColumn: nil,
                                 sub: "Associations the wake phase promoted to insights, with the intention each one produced, read from each intention's source. None made means no intention came from it.")
        case .sessions: ListSpec(kinds: ["session"], columns: ["shown", "cited"], chipColumn: nil,
                                 sub: "Sessions i-dream showed something to, and sessions evidence rows name: which items each was shown, and how many Claude cited once a scan judged it. Open one for its transcript path. Corrections and votes per session are not shown here yet.")
        case .evidence: ListSpec(kinds: ["evidence"], columns: ["domain", "by"], chipColumn: "domain",
                                 sub: "Every evidence row any kept daily recon read, newest copy of each, plus atone events older than those files. Open a row for the file it was read from.")
        case .gates: ListSpec(kinds: ["gate"], columns: ["week"], chipColumn: nil,
                              sub: "Hooks your answers on a weekly page switched on, beside the hints and nudges that only advise.")
        default: nil
        }
    }

    /// The list that is the home of an index kind, if any.
    static func pane(for kind: String) -> Pane? {
        Pane.lists.first { of($0)?.kinds.contains(kind) == true }
    }
}

/// One list's search, chip, selection and open row.
struct ListState: Equatable {
    var search = ""
    var chip: String?
    var selected = 0
    var expanded: String?
}

extension Pane {
    static let lists: [Pane] = [.proposals, .hints, .intentions, .insights, .sessions, .evidence, .gates]
}

extension AppModel {
    /// The rows a list shows: its kinds, filtered by search and chip, newest first.
    func listRows(_ p: Pane) -> [IndexEntry] {
        guard let spec = ListSpec.of(p), let doc = index else { return [] }
        let st = lists[p] ?? ListState()
        let words = st.search.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        return doc.entries
            .filter { spec.kinds.contains($0.kind) }
            .filter { e in st.chip.map { c in spec.chipColumn.map { e.fields?[$0] ?? "?" } == c } ?? true }
            .filter { e in let h = e.haystack; return words.allSatisfy { h.contains($0) } }
            .sorted { ($0.at ?? "") > ($1.at ?? "") }
    }

    func list(_ p: Pane) -> Binding<ListState> {
        Binding(get: { self.lists[p] ?? ListState() }, set: { self.lists[p] = $0 })
    }

    /// Open whatever an id names: its home pane, its list row, or the search's
    /// detail card for a kind with no home yet. An id the index does not know
    /// becomes a search for it.
    @discardableResult
    func openId(_ raw: String) -> Bool {
        guard let doc = index, let e = Self.resolve(raw, in: doc) else {
            globalSearch = GlobalSearch(open: true, query: raw)
            loadIndex()
            return false
        }
        if let d = home(e) {
            globalSearch.open = false
            open(d)
        } else {
            globalSearch = GlobalSearch(open: true, query: "", detail: e)
        }
        return true
    }

    /// The entry an id names. Ids are unique within a kind, not across kinds,
    /// so the kinds with a home win over those without.
    static func resolve(_ id: String, in doc: IndexDoc) -> IndexEntry? {
        let hits = doc.entries.filter { $0.id == id }
        let order = ["finding", "pattern", "slug", "source", "proposal", "hint", "nudge", "insight", "intention", "gate",
                     "session", "evidence", "association", "run", "cycle", "job"]
        return hits.min { (order.firstIndex(of: $0.kind) ?? 99) < (order.firstIndex(of: $1.kind) ?? 99) }
    }

    /// Flip a hint or nudge, then re-read the index so the row shows its new state.
    func flipIntervention(_ id: String, promote: Bool) {
        let args = ["promotions", promote ? "--promote" : "--demote", id]
        let run = runCLI
        DispatchQueue.global(qos: .userInitiated).async {
            let (ok, out) = run(args)
            DispatchQueue.main.async {
                self.flipNote = (id, ok ? out.trimmingCharacters(in: .whitespacesAndNewlines) : "could not flip: \(out)", !ok || out.contains("✗"))
                self.loadIndex(force: true)
            }
        }
    }

    /// Back: the place before the last jump, ⌘[.
    func goBack() {
        // An open search or card is the most recent place: step out of it first.
        if globalSearch.open {
            if globalSearch.detail != nil, let prev = globalSearch.trail.popLast() { globalSearch.detail = prev }
            // A card opened from a chip has no search behind it: close the panel.
            else if globalSearch.detail != nil, !globalSearch.query.isEmpty { globalSearch.detail = nil }
            else { globalSearch = GlobalSearch() }
            return
        }
        guard let prev = history.popLast() else { return }
        restoring = true
        pane = prev.pane
        if let st = prev.list { lists[prev.pane] = st }
        restoring = false
    }

    /// The rows a list draws: the opened row first when the cap would hide it,
    /// then the first rows up to the cap.
    func visibleRows(_ p: Pane) -> [IndexEntry] {
        let rows = listRows(p)
        let head = Array(rows.prefix(ListPane.cap))
        guard let id = lists[p]?.expanded, !head.contains(where: { $0.id == id }),
              let opened = rows.first(where: { $0.id == id }) else { return head }
        return [opened] + head
    }
}

/// An id that is a link: click opens its home, the button copies it.
struct IdChip: View {
    @ObservedObject var model: AppModel
    var id: String
    var label: String?

    var body: some View {
        HStack(spacing: 3) {
            Button { model.openId(id) } label: {
                Text(label ?? id).font(.system(size: 9.5 * F.k, design: .monospaced)).foregroundStyle(P.blue).lineLimit(1)
            }
            .buttonStyle(.plain)
            .help("open \(id)")
            Button {
                NSPasteboard.general.clearContents(); NSPasteboard.general.setString(id, forType: .string)
            } label: { Image(systemName: "doc.on.doc").font(.system(size: 8)).foregroundStyle(P.fg3) }
            .buttonStyle(.plain)
            .help("copy \(id)")
        }
        .padding(.horizontal, 5).padding(.vertical, 2)
        .background(RoundedRectangle(cornerRadius: 4).fill(P.card2))
    }
}

/// A list home: header, search, filter chips, and rows that open.
struct ListPane: View {
    @ObservedObject var model: AppModel
    var pane: Pane
    var r: Record
    static let cap = 200

    var body: some View {
        let spec = ListSpec.of(pane)!
        let rows = model.listRows(pane)
        let st = model.list(pane)
        VStack(alignment: .leading, spacing: 10) {
            PaneHeader(pane: pane, sub: spec.sub)
            if model.index == nil {
                Text(model.indexError ?? "reading the index…").font(F.meta).foregroundStyle(P.fg3)
            }
            SearchField(text: st.search, prompt: "search \(pane.title.lowercased())", token: model.searchFocusToken)
            if let col = spec.chipColumn { chips(col, spec) }
            VStack(alignment: .leading, spacing: 0) {
                header(spec)
                ForEach(Array(model.visibleRows(pane).enumerated()), id: \.element.uid) { _, e in
                    row(e, spec, current: e.id == rows[safe: st.wrappedValue.selected]?.id)
                }
                if rows.count > Self.cap {
                    Text("\(rows.count - Self.cap) more; narrow the search to see them").font(F.meta).foregroundStyle(P.fg3).padding(8)
                }
                if rows.isEmpty, model.index != nil {
                    Text(st.wrappedValue.search.isEmpty && st.wrappedValue.chip == nil ? "none yet" : "nothing matches").font(F.meta).foregroundStyle(P.fg3).padding(8)
                }
            }
            .card()
        }
        .onAppear { model.loadIndex() }
    }

    private func chips(_ col: String, _ spec: ListSpec) -> some View {
        let all = (model.index?.entries ?? []).filter { spec.kinds.contains($0.kind) }
        let counts = Dictionary(grouping: all, by: { $0.fields?[col] ?? "?" }).mapValues(\.count)
        return WrapLayout(spacing: 6) {
            ForEach(counts.sorted { $0.value > $1.value }.prefix(10), id: \.key) { v, n in
                Chip(text: v, count: n, on: model.lists[pane]?.chip == v) {
                    let cur = model.lists[pane]?.chip
                    model.lists[pane, default: ListState()].chip = cur == v ? nil : v
                    model.lists[pane, default: ListState()].selected = 0
                }
            }
        }
    }

    private func header(_ spec: ListSpec) -> some View {
        HStack(spacing: 8) {
            Text("ITEM").frame(maxWidth: .infinity, alignment: .leading)
            ForEach(spec.columns, id: \.self) { c in Text(c.uppercased()).frame(width: width(c), alignment: .leading) }
            Text("WHEN").frame(width: 60, alignment: .trailing)
        }
        .font(F.label).tracking(0.6).foregroundStyle(P.fg3)
        .padding(.horizontal, 8).padding(.vertical, 5)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    private func width(_ c: String) -> CGFloat {
        switch c {
        case "cited": 190
        case "keywords": 180
        case "promoted by", "intention", "confidence": 110
        case "category", "domain", "status", "state": 90
        default: 70
        }
    }

    private func row(_ e: IndexEntry, _ spec: ListSpec, current: Bool) -> some View {
        let open = model.lists[pane]?.expanded == e.id
        return VStack(alignment: .leading, spacing: 6) {
            Button {
                let rows = model.listRows(pane)
                model.lists[pane, default: ListState()].selected = rows.firstIndex(of: e) ?? 0
                model.lists[pane, default: ListState()].expanded = open ? nil : e.id
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(e.title.isEmpty ? e.id : e.title).font(F.meta).foregroundStyle(P.fg).lineLimit(open ? nil : 1)
                        Text(e.id).font(F.mono).foregroundStyle(P.fg3).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    ForEach(spec.columns, id: \.self) { c in
                        Text(e.fields?[c] ?? "").font(F.meta).foregroundStyle(P.fg2).lineLimit(1).frame(width: width(c), alignment: .leading)
                    }
                    AgeText(age: e.at.flatMap(parseDate).map { r.now.timeIntervalSince($0) }).frame(width: 60, alignment: .trailing)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open { detail(e) }
        }
        .padding(.horizontal, 8).padding(.vertical, 6)
        .background(current ? P.sel : Color.clear)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    private func detail(_ e: IndexEntry) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(e.detail).font(F.meta).foregroundStyle(P.fg2).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            if let src = e.fields?["source"], !src.isEmpty {
                HStack(spacing: 6) { Text("read from \(src)").font(F.mono).foregroundStyle(P.fg3).textSelection(.enabled); CopyButton(value: src) }
            }
            if let t = e.fields?["transcript"] {
                HStack(spacing: 6) { Text(t).font(F.mono).foregroundStyle(P.fg3).textSelection(.enabled); CopyButton(value: t) }
            }
            HStack(spacing: 6) { Text("id").font(F.label).foregroundStyle(P.fg3); IdChip(model: model, id: e.id) }
            if e.kind == "hint" || e.kind == "nudge" { flipControls(e) }
            if let rel = e.related, !rel.isEmpty {
                SectionLabel(text: "points at")
                WrapLayout(spacing: 4) {
                    ForEach(rel.prefix(40), id: \.self) { id in
                        IdChip(model: model, id: id, label: model.index.flatMap { AppModel.resolve(id, in: $0) }.map { "\($0.kind): \($0.title.prefix(48))" })
                    }
                }
                if rel.count > 40 { Text("and \(rel.count - 40) more").font(F.meta).foregroundStyle(P.fg3) }
            }
            let back = backlinks(e)
            if !back.isEmpty {
                SectionLabel(text: "pointed at by")
                WrapLayout(spacing: 4) {
                    ForEach(back.prefix(40), id: \.uid) { b in IdChip(model: model, id: b.id, label: "\(b.kind): \(b.title.prefix(48))") }
                }
            }
        }
        .padding(.leading, 4)
    }

    /// Promote to live or demote to shadow, through `i-dream promotions`.
    private func flipControls(_ e: IndexEntry) -> some View {
        let state = e.fields?["state"] ?? ""
        return HStack(spacing: 10) {
            if state != "live" {
                Button { model.flipIntervention(e.id, promote: true) } label: { Label("Promote to live", systemImage: "arrow.up.circle").font(F.meta) }
                    .buttonStyle(.plain).foregroundStyle(P.blue)
            }
            if state != "shadow" {
                Button { model.flipIntervention(e.id, promote: false) } label: { Label("Demote to shadow", systemImage: "arrow.down.circle").font(F.meta) }
                    .buttonStyle(.plain).foregroundStyle(P.blue)
            }
            if let n = model.flipNote, n.id == e.id {
                Text(n.text).font(F.meta).foregroundStyle(n.failed ? P.red : P.fg2)
            }
        }
    }

    /// Items whose related ids name this one: the way back along a link.
    private func backlinks(_ e: IndexEntry) -> [IndexEntry] {
        (model.index?.entries ?? []).filter { $0.related?.contains(e.id) == true && $0.uid != e.uid }
    }
}
