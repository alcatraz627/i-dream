import SwiftUI

/// The `i-dream index --json` document: every item i-dream knows about, in
/// one shape, for searching.
struct IndexDoc: Decodable {
    var generated: String
    var counts: [String: Int]
    var entries: [IndexEntry]
    /// Row numbers by id, built once, so following a link is a lookup.
    private(set) var byId: [String: [Int]] = [:]

    private enum CodingKeys: String, CodingKey { case generated, counts, entries }

    /// One row per item: a finding named in several weekly runs, or an
    /// evidence row read on several days, keeps its newest copy.
    init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        generated = try c.decode(String.self, forKey: .generated)
        counts = try c.decode([String: Int].self, forKey: .counts)
        let all = try c.decode([IndexEntry].self, forKey: .entries)
        var newest: [String: Int] = [:]
        for (i, e) in all.enumerated() where (newest[e.uid].map { (all[$0].at ?? "") < (e.at ?? "") } ?? true) { newest[e.uid] = i }
        entries = all.enumerated().filter { newest[$0.element.uid] == $0.offset }.map(\.element)
        for (i, e) in entries.enumerated() { byId[e.id, default: []].append(i) }
    }
}

struct IndexEntry: Decodable, Equatable {
    var kind: String
    var id: String
    var title: String
    var detail: String
    var at: String?
    var related: [String]?
    var fields: [String: String]?
    /// Unique across kinds: a slug and a hint can share an id.
    var uid: String { kind + ":" + id }
    /// Lowercased once at load, since every keystroke searches every row.
    private(set) var haystack = ""
    private(set) var titleLow = ""

    private enum CodingKeys: String, CodingKey { case kind, id, title, detail, at, related, fields }

    init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        id = try c.decode(String.self, forKey: .id)
        title = try c.decode(String.self, forKey: .title)
        detail = try c.decode(String.self, forKey: .detail)
        at = try c.decodeIfPresent(String.self, forKey: .at)
        related = try c.decodeIfPresent([String].self, forKey: .related)
        fields = try c.decodeIfPresent([String: String].self, forKey: .fields)
        titleLow = title.lowercased()
        haystack = (id + " " + title + " " + detail).lowercased()
    }
}

/// The ⌘K search: what is typed, which result is selected, and the item
/// opened in the detail card when it has no home pane yet.
struct GlobalSearch {
    var open = false
    var query = ""
    var selected = 0
    var detail: IndexEntry?
    /// Items opened through related ids, so Esc steps back through them.
    var trail: [IndexEntry] = []
}

enum SearchIndex {
    /// The order groups appear in, most-asked first.
    static let kinds = ["pattern", "finding", "slug", "proposal", "hint", "nudge", "evidence", "association",
                        "intention", "source", "job", "session"]
    static let perKind = 6

    /// Every word typed must appear somewhere in the item, case ignored.
    /// Title matches sort before matches found only in the detail.
    static func results(_ doc: IndexDoc?, _ query: String) -> [(kind: String, shown: [IndexEntry], total: Int)] {
        let words = query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        guard let doc, !words.isEmpty else { return [] }
        let hits = doc.entries.filter { e in words.allSatisfy { e.haystack.contains($0) } }
        let byKind = Dictionary(grouping: hits, by: \.kind)
        return kinds.compactMap { k in
            guard let all = byKind[k], !all.isEmpty else { return nil }
            let keyed = all.map { e in (e, words.allSatisfy { e.titleLow.contains($0) }) }
            let sorted = keyed.sorted { a, b in
                if a.1 != b.1 { return a.1 }
                return (a.0.at ?? "") > (b.0.at ?? "")
            }.map(\.0)
            return (k, Array(sorted.prefix(perKind)), all.count)
        }
    }

    static func flat(_ groups: [(kind: String, shown: [IndexEntry], total: Int)]) -> [IndexEntry] {
        groups.flatMap(\.shown)
    }
}

extension AppModel {
    /// Where an item lives in the dashboard, or nil when no pane can show it
    /// (a pattern past the drawn ones, an association, a job) and it opens in
    /// the search's detail card instead.
    func home(_ e: IndexEntry) -> Dive? {
        guard let r = record else { return nil }
        switch e.kind {
        case "pattern" where r.patterns.contains { $0.id == e.id }: return Dive(pane: .patterns, scope: .pattern(e.id))
        case "finding" where r.findings.contains { $0.cluster == e.id }: return Dive(pane: .reader, scope: .cluster(e.id))
        case "slug" where r.effects.contains { $0.slug == e.id }: return Dive(pane: .landing, scope: .slug(e.id))
        case "source" where r.sources.contains { $0.id == e.id }: return Dive(pane: .ledger, scope: .source(e.id))
        default: return ListSpec.pane(for: e.kind).map { Dive(pane: $0, scope: .item(e.id)) }
        }
    }

    func toggleSearch() {
        globalSearch.open.toggle()
        if globalSearch.open { globalSearch.selected = 0; globalSearch.detail = nil; globalSearch.trail = []; loadIndex() }
    }

    /// Return on a result: its home pane when it has one, the detail card otherwise.
    func openResult(_ e: IndexEntry) {
        if let d = home(e) {
            globalSearch.open = false
            open(d)
        } else {
            if let cur = globalSearch.detail { globalSearch.trail.append(cur) }
            globalSearch.detail = e
        }
    }

    /// Follow a related id to the item it names, in any kind.
    func follow(_ id: String) {
        guard let doc = index, let e = AppModel.resolve(id, in: doc) else {
            globalSearch.query = id; globalSearch.detail = nil; globalSearch.selected = 0
            return
        }
        openResult(e)
    }

    /// Keys while the search is open. Returns false for keys the text field owns.
    func searchKey(_ code: UInt16) -> Bool {
        let flat = SearchIndex.flat(SearchIndex.results(index, globalSearch.query))
        switch code {
        case 125: globalSearch.selected = clamp(globalSearch.selected + 1, flat.count)
        case 126: globalSearch.selected = clamp(globalSearch.selected - 1, flat.count)
        case 36: if globalSearch.detail == nil, let e = flat[safe: globalSearch.selected] { openResult(e) }
        case 53:
            if globalSearch.detail != nil { globalSearch.detail = globalSearch.trail.popLast() }
            else { globalSearch.open = false }
        default: return false
        }
        return true
    }
}

/// The search panel over the dashboard.
struct SearchPanel: View {
    @ObservedObject var model: AppModel
    @FocusState private var focused: Bool

    var body: some View {
        let groups = SearchIndex.results(model.index, model.globalSearch.query)
        let flat = SearchIndex.flat(groups)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(P.fg3)
                TextField("search everything: a pattern, slug, proposal, session, any word", text: query)
                    .textFieldStyle(.plain).font(F.body).focused($focused)
                Text(indexNote).font(F.mono).foregroundStyle(model.indexError != nil && model.index == nil ? P.amber : P.fg3).lineLimit(1).fixedSize()
            }
            .padding(12)
            .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
            if let d = model.globalSearch.detail {
                DetailCard(model: model, e: d)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(groups, id: \.kind) { g in
                            Text("\(g.kind)s · \(g.total)\(g.total > g.shown.count ? ", first \(g.shown.count) shown" : "")".uppercased())
                                .font(F.label).tracking(0.6).foregroundStyle(P.fg3).padding(.top, 8).padding(.horizontal, 12)
                            ForEach(g.shown, id: \.uid) { e in
                                resultRow(e, selected: flat.firstIndex(of: e) == model.globalSearch.selected)
                            }
                        }
                        if groups.isEmpty {
                            Text(emptyText).font(F.meta).foregroundStyle(P.fg3).lineLimit(4).padding(12)
                        }
                    }
                    .padding(.bottom, 8)
                }
                .frame(maxHeight: 460)
            }
        }
        .frame(width: 720)
        // Opaque: the pane underneath must not show through the results.
        .background(RoundedRectangle(cornerRadius: 10).fill(P.side))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(P.hair))
        .shadow(radius: 18)
        .onAppear { focused = true }
    }

    private var emptyText: String {
        if model.index == nil, let e = model.indexError { return "The index could not be read, so there is nothing to search: \(e.prefix(200))" }
        return model.globalSearch.query.isEmpty ? "Type any part of anything. ↑ ↓ move, return opens, esc closes." : "nothing matches every word"
    }

    /// Typing starts a fresh search: the selection goes to the top and an open card closes.
    private var query: Binding<String> {
        // The field writes its value back when it takes focus; only a real edit resets.
        Binding(get: { model.globalSearch.query }, set: { q in
            guard q != model.globalSearch.query else { return }
            model.globalSearch.query = q
            model.globalSearch.selected = 0
            model.globalSearch.detail = nil
            model.globalSearch.trail = []
        })
    }

    private var indexNote: String {
        guard let i = model.index else { return model.indexError == nil ? "reading the index…" : "index unavailable" }
        return "\(i.entries.count.formatted()) items"
    }

    private func resultRow(_ e: IndexEntry, selected: Bool) -> some View {
        Button { model.openResult(e) } label: {
            VStack(alignment: .leading, spacing: 1) {
                Text(e.title).font(F.meta).foregroundStyle(P.fg).lineLimit(1)
                Text(e.detail).font(F.meta).foregroundStyle(P.fg3).lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12).padding(.vertical, 4)
            .background(selected ? P.sel : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// One item read in full, with every id it points at as a link.
struct DetailCard: View {
    @ObservedObject var model: AppModel
    var e: IndexEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(e.kind.uppercased()).font(F.label).tracking(0.6).foregroundStyle(P.fg3)
            Text(e.title).font(F.body).foregroundStyle(P.fg).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            FieldLines(detail: e.detail, fields: e.fields ?? [:])
            HStack(spacing: 6) {
                IdChip(model: model, id: e.id)
                if let at = e.at.flatMap(parseDate) { Text("· \(ageText(Date().timeIntervalSince(at))) ago").font(F.meta).foregroundStyle(P.fg3) }
            }
            if let rel = e.related, !rel.isEmpty {
                SectionLabel(text: "points at")
                ForEach(rel.prefix(24), id: \.self) { id in
                    let target = model.index?.entries.first { $0.id == id }
                    Button { model.follow(id) } label: {
                        Text(target.map { "\($0.kind): \($0.title)" } ?? id).font(F.meta).foregroundStyle(P.teal).lineLimit(1)
                    }
                    .buttonStyle(.plain)
                }
                if rel.count > 24 { Text("and \(rel.count - 24) more").font(F.meta).foregroundStyle(P.fg3) }
            }
            OpenInClaudeRow(kind: e.kind, id: e.id, title: e.title)
            Text("esc goes back").font(F.meta).foregroundStyle(P.fg3)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
