import Foundation

// The two JSON documents the widget reads, `i-dream status --json` and
// `i-dream reader --json`, mirrored field for field from
// docs/contracts/*.schema.json. The widget never opens a store file, so
// these types are the whole of what it can know. A field the schema marks
// required is non-optional here: when the CLI drops it, decoding fails and
// the surface says so, instead of drawing a blank as if it were zero.

struct StatusDoc: Decodable {
    struct Daemon: Decodable { var status: String; var pid: Int? }
    struct State: Decodable {
        var last_consolidation: String?
        var total_cycles: Int?
        var total_tokens_used: Int?
        var last_activity: String?
    }
    struct Lane: Decodable {
        var lane: String
        var status: String
        var reason: String
        var consumer: String
        var producer_age: String?
        var consumer_age: String?
        var consumer_state: String
        var cadence_hours: Double
        var producer_age_s: Double?
        var consumer_age_s: Double?
    }
    struct Lanes: Decodable { var green: Int; var yellow: Int; var red: Int; var lanes: [Lane] }
    struct Queue: Decodable { var depth: Int; var oldest: String? }
    struct Job: Decodable {
        var label: String
        var desc: String?
        var schedule: String
        var installed: Bool
        var loaded: Bool
        var last_exit: Int?
    }
    struct Log: Decodable { var file: String; var warn_count: Int; var error_count: Int; var last_error: String? }
    struct Build: Decodable {
        var version: String
        var daemon_stale: Bool?
        var binary_behind_source: Bool?
    }
    struct Awaiting: Decodable { var slug: String; var url: String }
    struct Reader: Decodable {
        var last_recon: String?
        var last_weekly: String?
        var clusters: Int
        var awaiting: [Awaiting]
        var latest_items: Int
        var latest_week: String?
        var landed: [String: Int]
        var weekly_held_since: String?
    }
    struct Domain: Decodable { var name: String; var events_in_window: Int; var newest: String? }
    struct Interventions: Decodable { var live: Int; var candidate: Int; var shadow: Int; var fired_7d: Int }
    struct Slug: Decodable {
        var slug: String
        var severity: String
        var total: Int
        var last7: Int
        var prior7: Int
        var delta7: Int
        var last: String?
    }
    struct Cycle: Decodable {
        var ts: String
        var cycle_id: String
        var sessions: Int
        var patterns: Int
        var associations: Int
        var insights: Int
        var tokens: Int
        var produced: Bool
    }
    struct Cycles: Decodable { var last_productive: String?; var recent: [Cycle]; var window_days: Int }
    struct Pattern: Decodable {
        var id: String
        var text: String
        var category: String
        var valence: String
        var strength: Double
        var confidence: Double
        var occurrences: Int
        var last_seen: String
        var last7: Int
        var prior7: Int
    }
    struct Association: Decodable { var id: String; var a: String; var b: String; var hypothesis: String; var confidence: Double }
    struct Patterns: Decodable {
        var total: Int
        var by_category: [String: Int]
        var top: [Pattern]
        var associations: [Association]
    }

    var daemon: Daemon
    var state: State?
    var state_error: String?
    var lanes: Lanes
    var queue: Queue
    var jobs: [Job]
    var log: Log?
    var build: Build?
    var reader: Reader
    var domains: [Domain]
    var interventions: Interventions
    var reflect: [Slug]
    var cycles: Cycles
    var patterns: Patterns
    var sources: [String: String]
}

struct ReaderDoc: Decodable {
    struct Stream: Decodable { var domain: String; var path: String?; var events_in_window: Int; var newest: String? }
    struct State: Decodable {
        var last_recon: String?
        var last_weekly: String?
        var weekly_pending_since: String?
        var pending_pages: [String]
        var streams: [Stream]
    }
    struct Cluster: Decodable {
        var id: String
        var kind: String
        var key: String
        var summary: String
        var evidence: [String]
        var domains: [String]
        var provenances: [String]
        var first_ts: String
        var last_ts: String
        var score: Double
    }
    struct Recon: Decodable {
        var at: String
        var since: String
        var streams: [Stream]
        var evidence_count: Int
        var clusters: [Cluster]
    }
    struct Proposal: Decodable { var target: String; var change: String }
    struct Named: Decodable {
        var cluster: String
        var kind: String
        var title: String
        var why: String
        var evidence_ids: [String]
        var proposal: Proposal?
    }
    struct Validated: Decodable { var items: [Named]; var process_note: String? }
    struct Landed: Decodable { var cluster: String; var title: String; var outcome: String; var detail: String?; var ts: String }
    struct Run: Decodable {
        var week: String
        var at: String?
        var forwarded: [Cluster]
        var named: Validated
        var items: [Named]
        var page_slug: String?
        var page_url: String?
        var tokens: Int?
        var held_by_gate: String?
        var landed: [Landed]
    }
    struct Evidence: Decodable {
        var id: String
        var domain: String
        var ts: String
        var provenance: String
        var slug: String?
        var session: String?
        var project: String?
        var text: String
    }

    var state: State
    var recon: Recon?
    var runs: [Run]
    var evidence: [Evidence]
}

/// Turn a contract timestamp into a date. The CLI writes RFC 3339 with
/// anywhere from zero to nine fractional digits and either `Z` or an offset,
/// which no single Foundation formatter accepts, so the fraction is split off
/// and added back by hand.
func parseDate(_ s: String?) -> Date? {
    guard let s, !s.isEmpty else { return nil }
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime]
    guard let dot = s.firstIndex(of: "."), s.distance(from: s.startIndex, to: dot) == 19 else {
        return f.date(from: s)
    }
    var end = s.index(after: dot)
    while end < s.endIndex, s[end].isNumber { end = s.index(after: end) }
    let frac = Double("0" + s[dot..<end]) ?? 0
    let base = String(s[..<dot]) + String(s[end...])
    return f.date(from: base).map { $0.addingTimeInterval(frac) }
}

/// Explain a decoding failure in the owner's words: which field of which
/// document went missing. This is the feed contract probe; it fires on the
/// first refresh after the CLI renames or drops a field the widget uses.
func describe(_ error: Error, doc: String) -> String {
    switch error {
    case DecodingError.keyNotFound(let key, let ctx):
        let path = (ctx.codingPath.map { $0.stringValue } + [key.stringValue]).joined(separator: ".")
        return "\(doc) no longer carries \(path)"
    case DecodingError.typeMismatch(_, let ctx), DecodingError.valueNotFound(_, let ctx):
        return "\(doc) field \(ctx.codingPath.map { $0.stringValue }.joined(separator: ".")) changed type"
    case DecodingError.dataCorrupted:
        return "\(doc) is not valid JSON"
    default:
        return "\(doc): \(error.localizedDescription)"
    }
}
