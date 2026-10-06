import SwiftUI

// The lenses. Nights encodes time (did it run each day, did it find
// anything); the tide is texture under the mood word.

// MARK: - Nights

/// The week as seven day columns: did i-dream run each day, and did each run
/// find anything. One block per run, filled when it found something, hollow
/// when it found nothing, teal for a reader run. Clicking a day narrows the
/// Flow cards to that day; clicking it again clears.
struct Nights: View {
    var marks: [CycleMark]
    var now: Date
    var selected: String?
    var range: ClosedRange<Date>?
    var onSelect: (CycleMark) -> Void = { _ in }
    var onRange: (ClosedRange<Date>?) -> Void = { _ in }

    private var days: [Date] {
        let today = Calendar.current.startOfDay(for: now)
        return (0..<7).reversed().map { today.addingTimeInterval(-Double($0) * 86400) }
    }

    var body: some View {
        let f = DateFormatter()
        f.dateFormat = "EEE"
        return HStack(alignment: .bottom, spacing: 10) {
            ForEach(days, id: \.self) { day in
                let span = day...day.addingTimeInterval(86399)
                let runs = marks.filter { span.contains($0.at) && $0.kind != "consolidation" }.sorted { $0.at < $1.at }
                let picked = range == span
                VStack(spacing: 6) {
                    Text(runs.isEmpty ? "–" : "\(runs.filter(\.produced).count)/\(runs.count)")
                        .font(F.mono).foregroundStyle(runs.isEmpty ? P.fg3 : P.fg2)
                    VStack(spacing: 3) {
                        Spacer(minLength: 0)
                        ForEach(runs.reversed()) { m in block(m) }
                    }
                    .frame(height: 48)
                    Text(Calendar.current.isDateInToday(day) ? "Today" : f.string(from: day))
                        .font(F.meta.weight(picked ? .semibold : .regular)).foregroundStyle(picked ? P.fg : P.fg2)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 7).fill(picked ? P.hair : .clear))
                .contentShape(Rectangle())
                .onTapGesture { onRange(picked ? nil : span) }
            }
        }
        .padding(.horizontal, 10)
    }

    private func block(_ m: CycleMark) -> some View {
        let color = m.kind == "reader" ? P.teal : P.violet
        return RoundedRectangle(cornerRadius: 3)
            .fill(m.produced ? color : .clear)
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(m.produced ? color : P.fg3, lineWidth: 1))
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(P.fg, lineWidth: selected == m.id ? 1.5 : 0).padding(-2))
            .frame(width: 28, height: 9)
            .onTapGesture { onSelect(m) }
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
                let base = h * (0.82 - 0.1 * CGFloat(min(level, 3)))
                p.move(to: CGPoint(x: 0, y: base))
                p.addCurve(to: CGPoint(x: w * 0.5, y: base - 4), control1: CGPoint(x: w * 0.15, y: base - 14), control2: CGPoint(x: w * 0.3, y: base + 10))
                p.addCurve(to: CGPoint(x: w, y: base - 2), control1: CGPoint(x: w * 0.7, y: base - 16), control2: CGPoint(x: w * 0.85, y: base + 8))
                p.addLine(to: CGPoint(x: w, y: h)); p.addLine(to: CGPoint(x: 0, y: h)); p.closeSubpath()
            }
            .fill(LinearGradient(colors: [P.violet.opacity(0.14), P.blue.opacity(0.16), P.teal.opacity(0.12)], startPoint: .leading, endPoint: .trailing))
        }
        .allowsHitTesting(false)
    }
}
