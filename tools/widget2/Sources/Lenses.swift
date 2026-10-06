import SwiftUI

// The three lenses. Each encodes one variable a table cannot show:
// the hypnogram encodes time (when cycles ran and how deep they went),
// the constellation encodes association (which patterns link, how strong),
// and the tide is texture under the mood word, whose sentence carries the fact.

// MARK: - Hypnogram

struct Hypnogram: View {
    var marks: [CycleMark]
    var now: Date
    var days: Int = 7
    var compact = false
    var selected: String?
    var range: ClosedRange<Date>?
    var onSelect: (CycleMark) -> Void = { _ in }
    var onRange: (ClosedRange<Date>?) -> Void = { _ in }
    var help: HoverHelp?

    @State private var hover: String?
    @State private var dragFrom: CGFloat?
    @State private var dragTo: CGFloat?

    private var start: Date { Calendar.current.startOfDay(for: now).addingTimeInterval(-Double(days - 1) * 86400) }
    private var end: Date { Calendar.current.startOfDay(for: now).addingTimeInterval(86400) }
    private var left: CGFloat { compact ? 4 : 64 }
    private let rows = ["awake", "ingest", "associate", "reader"]

    private func x(_ t: Date, _ w: CGFloat) -> CGFloat {
        left + CGFloat(t.timeIntervalSince(start) / end.timeIntervalSince(start)) * (w - left - 8)
    }
    private func date(_ px: CGFloat, _ w: CGFloat) -> Date {
        start.addingTimeInterval(Double((px - left) / (w - left - 8)) * end.timeIntervalSince(start))
    }
    private func y(_ depth: Double, _ h: CGFloat) -> CGFloat {
        let top: CGFloat = compact ? 5 : 14, bottom: CGFloat = compact ? 5 : 26
        return top + CGFloat(depth / 3) * (h - top - bottom)
    }
    private var visible: [CycleMark] { marks.filter { $0.at >= start && $0.at <= end } }

    private func nearest(_ p: CGPoint, _ w: CGFloat) -> CycleMark? {
        visible.min { abs(x($0.at, w) - p.x) < abs(x($1.at, w) - p.x) }.flatMap { abs(x($0.at, w) - p.x) < 10 ? $0 : nil }
    }

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width, h = geo.size.height
            Canvas { ctx, _ in
                let cal = Calendar.current
                // Day bands and labels.
                for d in 0..<days {
                    let ds = start.addingTimeInterval(Double(d) * 86400)
                    let x0 = x(ds, w), x1 = x(ds.addingTimeInterval(86400), w)
                    if let hv = hover, let m = visible.first(where: { $0.id == hv }), cal.isDate(m.at, inSameDayAs: ds) {
                        ctx.fill(Path(CGRect(x: x0, y: 0, width: x1 - x0, height: h)), with: .color(P.violet.opacity(0.06)))
                    }
                    ctx.stroke(Path { $0.move(to: CGPoint(x: x0, y: 4)); $0.addLine(to: CGPoint(x: x0, y: h - (compact ? 2 : 18))) },
                               with: .color(P.hair), lineWidth: 0.5)
                    if !compact {
                        let f = DateFormatter(); f.dateFormat = "EEE d"
                        let label = f.string(from: ds) + (cal.isDate(ds, inSameDayAs: now) ? " (today)" : "")
                        ctx.draw(Text(label).font(.system(size: 9, design: .monospaced)).foregroundStyle(P.fg3),
                                 at: CGPoint(x: x0 + 4, y: h - 9), anchor: .leading)
                    }
                }
                if !compact {
                    for (i, label) in rows.enumerated() {
                        ctx.stroke(Path { $0.move(to: CGPoint(x: left, y: y(Double(i), h))); $0.addLine(to: CGPoint(x: w - 8, y: y(Double(i), h))) },
                                   with: .color(P.hair), lineWidth: 0.5)
                        ctx.draw(Text(label).font(.system(size: 9, design: .monospaced)).foregroundStyle(P.fg3),
                                 at: CGPoint(x: 8, y: y(Double(i), h)), anchor: .leading)
                    }
                }
                // Selected range.
                if let r = range {
                    let a = x(r.lowerBound, w), b = x(r.upperBound, w)
                    ctx.fill(Path(CGRect(x: a, y: 0, width: b - a, height: h)), with: .color(P.blue.opacity(0.14)))
                }
                if let a = dragFrom, let b = dragTo {
                    ctx.fill(Path(CGRect(x: min(a, b), y: 0, width: abs(b - a), height: h)), with: .color(P.blue.opacity(0.14)))
                }
                // The trace: flat at awake, dipping to each cycle's depth.
                let segW: CGFloat = compact ? 3 : 7
                var path = Path()
                path.move(to: CGPoint(x: left, y: y(0, h)))
                for m in visible {
                    let mx = x(m.at, w)
                    path.addLine(to: CGPoint(x: mx, y: y(0, h)))
                    path.addLine(to: CGPoint(x: mx, y: y(m.depth, h)))
                    path.addLine(to: CGPoint(x: mx + segW, y: y(m.depth, h)))
                    path.addLine(to: CGPoint(x: mx + segW, y: y(0, h)))
                }
                path.addLine(to: CGPoint(x: x(now, w), y: y(0, h)))
                ctx.stroke(path, with: .color(P.violet), style: StrokeStyle(lineWidth: compact ? 1.1 : 1.6, lineJoin: .round))
                for m in visible {
                    let mx = x(m.at, w)
                    let lit = m.id == hover || m.id == selected
                    let c: Color = m.kind == "reader" ? P.teal : m.produced ? P.violet : P.fg3
                    let r = CGRect(x: mx - 1, y: y(m.depth, h) - (compact ? 1.5 : 3), width: segW + 2, height: compact ? 3 : 6)
                    ctx.fill(Path(roundedRect: r, cornerRadius: 2), with: .color(c.opacity(lit ? 1 : 0.75)))
                    if lit && !compact {
                        ctx.stroke(Path(roundedRect: r.insetBy(dx: -3, dy: -3), cornerRadius: 3), with: .color(P.blue), lineWidth: 1)
                    }
                }
                ctx.fill(Path(ellipseIn: CGRect(x: x(now, w) - 3, y: y(0, h) - 3, width: 6, height: 6)), with: .color(P.violet))
            }
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let p):
                    let m = nearest(p, w)
                    hover = m?.id
                    if let m { help?.text = hoverLine(m) }
                case .ended: hover = nil; help?.text = ""
                }
            }
            .gesture(compact ? nil : DragGesture(minimumDistance: 6)
                .onChanged { v in dragFrom = v.startLocation.x; dragTo = v.location.x }
                .onEnded { v in
                    let a = date(min(v.startLocation.x, v.location.x), w), b = date(max(v.startLocation.x, v.location.x), w)
                    dragFrom = nil; dragTo = nil
                    onRange(a...b)
                })
            .simultaneousGesture(SpatialTapGesture().onEnded { v in
                if let m = nearest(v.location, w) { onSelect(m) } else if compact, let m = visible.last { onSelect(m) }
            })
        }
    }

    private func hoverLine(_ m: CycleMark) -> String {
        let f = DateFormatter(); f.dateFormat = "EEE HH:mm"
        let what = m.kind == "reader" ? "reader run" : m.kind == "consolidation" ? "idle consolidation" : (m.produced ? "cycle, produced" : "cycle, idle")
        return "\(f.string(from: m.at)) · \(what) · \(m.lines.first ?? "")"
    }
}

// MARK: - Constellation

struct Constellation: View {
    var patterns: [PatternNode]
    var associations: [StatusDoc.Association]
    var focus: String?
    var big = false
    var dimmed: Set<String> = []
    /// The list row the keyboard is on; its star gets a ring so the map follows j/k.
    var cursor: String? = nil
    var onSelect: (PatternNode) -> Void = { _ in }
    var onClear: () -> Void = {}
    var help: HoverHelp?

    /// How close the pointer must be to a star's centre to hover or pick it.
    private var reach: CGFloat { big ? 22 : 14 }

    @State private var hover: String?
    @State private var pan: CGSize = .zero
    @State private var panStart: CGSize = .zero
    @State private var zoom: CGFloat = 1
    @State private var zoomStart: CGFloat = 1

    static let categories = ["approach", "user-preference", "tool-use", "domain", "architecture"]

    /// Stable position for a pattern: its category picks a region, its id
    /// picks a spot inside it, so the map does not reshuffle between reads.
    static func position(_ p: PatternNode, in size: CGSize) -> CGPoint {
        let cats = categories
        let i = cats.firstIndex(of: p.category) ?? cats.count
        let cols = CGFloat(cats.count + 1)
        let cx = size.width * (CGFloat(i) + 0.75) / cols
        let cy = size.height * (i % 2 == 0 ? 0.42 : 0.6)
        var hsh: UInt64 = 1469598103934665603
        for b in p.id.utf8 { hsh = (hsh ^ UInt64(b)) &* 1099511628211 }
        let a = Double(hsh % 6283) / 1000
        let rad = Double((hsh >> 16) % 1000) / 1000
        let rx = size.width / cols * 0.55, ry = size.height * 0.36
        return CGPoint(x: cx + CGFloat(cos(a) * sqrt(rad)) * rx, y: cy + CGFloat(sin(a) * sqrt(rad)) * ry)
    }

    private func place(_ p: PatternNode, _ size: CGSize) -> CGPoint {
        let b = Self.position(p, in: size)
        let c = CGPoint(x: size.width / 2, y: size.height / 2)
        return CGPoint(x: c.x + (b.x - c.x) * zoom + pan.width, y: c.y + (b.y - c.y) * zoom + pan.height)
    }

    var body: some View {
        GeometryReader { geo in
            let size = geo.size
            let byId = Dictionary(patterns.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            let lit: Set<String> = {
                guard let f = focus ?? hover ?? cursor, let n = byId[f] else { return [] }
                return Set(n.links + [f])
            }()
            Canvas { ctx, _ in
                if big {
                    for (i, c) in Self.categories.enumerated() {
                        let cols = CGFloat(Self.categories.count + 1)
                        let pt = CGPoint(x: size.width / 2 + (size.width * (CGFloat(i) + 0.75) / cols - size.width / 2) * zoom + pan.width,
                                         y: size.height / 2 + (size.height * (i % 2 == 0 ? 0.06 : 0.94) - size.height / 2) + pan.height)
                        ctx.draw(Text(c.uppercased()).font(.system(size: 9, weight: .semibold)).tracking(0.6).foregroundStyle(P.category(c)), at: pt)
                    }
                }
                for a in associations {
                    guard let pa = byId[a.a], let pb = byId[a.b] else { continue }
                    let on = lit.contains(a.a) && lit.contains(a.b)
                    var p = Path(); p.move(to: place(pa, size)); p.addLine(to: place(pb, size))
                    ctx.stroke(p, with: .color(on ? P.teal : P.fg3.opacity(lit.isEmpty ? 0.35 : 0.12)), lineWidth: on ? 1.4 : 0.6)
                }
                for p in patterns {
                    let pt = place(p, size)
                    let base: CGFloat = big ? 2 : 1.2
                    let r = base + CGFloat(p.strength) * (big ? 6 : 3)
                    let faded = (!lit.isEmpty && !lit.contains(p.id)) || dimmed.contains(p.id)
                    if p.trend == "worsening" || p.trend == "reinforced" {
                        let hr = r + (big ? 6 : 3)
                        ctx.fill(Path(ellipseIn: CGRect(x: pt.x - hr, y: pt.y - hr, width: hr * 2, height: hr * 2)),
                                 with: .color((p.trend == "worsening" ? P.red : P.green).opacity(faded ? 0.04 : 0.18)))
                    }
                    ctx.fill(Path(ellipseIn: CGRect(x: pt.x - r, y: pt.y - r, width: r * 2, height: r * 2)),
                             with: .color(P.category(p.category).opacity(faded ? 0.12 : 0.35 + p.strength * 0.65)))
                    if p.id == focus {
                        ctx.stroke(Path(ellipseIn: CGRect(x: pt.x - r - 3, y: pt.y - r - 3, width: r * 2 + 6, height: r * 2 + 6)), with: .color(P.blue), lineWidth: 1.2)
                    } else if p.id == cursor || p.id == hover {
                        ctx.stroke(Path(ellipseIn: CGRect(x: pt.x - r - 3, y: pt.y - r - 3, width: r * 2 + 6, height: r * 2 + 6)),
                                   with: .color(P.fg), style: StrokeStyle(lineWidth: 1, dash: [2, 2]))
                    }
                }
                // The hovered pattern's text, in a box beside its star.
                if big, let h = hover, let n = byId[h] {
                    let pt = place(n, size)
                    let label = ctx.resolve(Text(n.text).font(.system(size: 11)).foregroundStyle(P.fg))
                    let boxW: CGFloat = 260
                    let t = label.measure(in: CGSize(width: boxW - 16, height: .infinity))
                    let w = t.width + 16, hgt = t.height + 12
                    let x = pt.x + 14 + w > size.width ? pt.x - 14 - w : pt.x + 14
                    let y = min(max(pt.y - hgt / 2, 4), size.height - hgt - 4)
                    let box = CGRect(x: x, y: y, width: w, height: hgt)
                    ctx.fill(Path(roundedRect: box, cornerRadius: 6), with: .color(P.card2))
                    ctx.stroke(Path(roundedRect: box, cornerRadius: 6), with: .color(P.hair), lineWidth: 0.5)
                    ctx.draw(label, in: box.insetBy(dx: 8, dy: 6))
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if big {
                    HStack(spacing: 4) {
                        zoomButton("minus.magnifyingglass") { zoom = max(0.6, zoom / 1.3); zoomStart = zoom }
                        zoomButton("arrow.counterclockwise") { zoom = 1; zoomStart = 1; pan = .zero; panStart = .zero }
                        zoomButton("plus.magnifyingglass") { zoom = min(4, zoom * 1.3); zoomStart = zoom }
                    }
                    .padding(8)
                }
            }
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let pt):
                    let n = patterns.min { dist(place($0, size), pt) < dist(place($1, size), pt) }
                    if let n, dist(place(n, size), pt) < reach {
                        hover = n.id
                        // The pattern text shows in the box beside the star, so the status line stays one short line.
                        help?.text = big
                            ? "\(n.category) · strength \(String(format: "%.2f", n.strength)) · rank \(n.rank) of \(patterns.count) · \(n.trend) · click to open"
                            : "\(n.category) · \(n.trend)"
                    } else { hover = nil; help?.text = "" }
                case .ended: hover = nil; help?.text = ""
                }
            }
            .simultaneousGesture(SpatialTapGesture().onEnded { v in
                let n = patterns.min { dist(place($0, size), v.location) < dist(place($1, size), v.location) }
                if let n, dist(place(n, size), v.location) < reach { onSelect(n) } else { onClear() }
            })
            .gesture(big ? DragGesture(minimumDistance: 4)
                .onChanged { v in pan = CGSize(width: panStart.width + v.translation.width, height: panStart.height + v.translation.height) }
                .onEnded { _ in panStart = pan } : nil)
            .simultaneousGesture(big ? MagnifyGesture()
                .onChanged { v in zoom = max(0.6, min(4, zoomStart * v.magnification)) }
                .onEnded { _ in zoomStart = zoom } : nil)
        }
    }

    private func dist(_ a: CGPoint, _ b: CGPoint) -> CGFloat { hypot(a.x - b.x, a.y - b.y) }

    private func zoomButton(_ icon: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: icon).font(.system(size: 11)).foregroundStyle(P.fg2)
                .frame(width: 24, height: 22)
                .background(RoundedRectangle(cornerRadius: 5).fill(P.card2))
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(P.hair, lineWidth: 0.5))
        }
        .buttonStyle(.plain)
        .focusEffectDisabled()
    }
}

// MARK: - Tide

/// Texture under the dropdown's top strip. Its height follows the mood
/// level; the fact itself is the mood word's sentence.
struct Tide: View {
    var level: Int
    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width, h = geo.size.height
            Path { p in
                // Amplitudes scale with the band, so the swell reads at any height.
                let a = h * 0.4
                let base = h * (0.7 - 0.1 * CGFloat(min(level, 3)))
                p.move(to: CGPoint(x: 0, y: base))
                p.addCurve(to: CGPoint(x: w * 0.5, y: base - a * 0.25), control1: CGPoint(x: w * 0.15, y: base - a), control2: CGPoint(x: w * 0.3, y: base + a * 0.7))
                p.addCurve(to: CGPoint(x: w, y: base - a * 0.15), control1: CGPoint(x: w * 0.7, y: base - a * 1.1), control2: CGPoint(x: w * 0.85, y: base + a * 0.6))
                p.addLine(to: CGPoint(x: w, y: h)); p.addLine(to: CGPoint(x: 0, y: h)); p.closeSubpath()
            }
            .fill(LinearGradient(colors: [P.violet.opacity(0.14), P.blue.opacity(0.16), P.teal.opacity(0.12)], startPoint: .leading, endPoint: .trailing))
        }
        .allowsHitTesting(false)
    }
}
