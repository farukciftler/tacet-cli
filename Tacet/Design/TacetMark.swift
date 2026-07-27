//
//  TacetMark.swift
//  Tacet
//
//  The brand mark — "Ensō with a dot": a zen brush circle left open on
//  purpose, and a brass dot sitting in the opening. The circle is the silence,
//  the dot is the full stop that ends the sentence — the gap is never closed,
//  the dot completes it instead. Two colours by design: the ring takes the
//  text colour above it, the dot is always `Palette.accent` (brass) unless the
//  caller says otherwise.
//

import SwiftUI

/// The ring's path geometry (in a 104×104 unit box, scaled to the given rect).
///
/// Layout in the unit box:
///  · circle — centre (52, 52), radius 29, stroked (round caps), NOT closed:
///    the arc runs clockwise from −24° all the way to 287°, leaving the
///    opening at the upper right (287°…336°).
///  · the brass dot lives at −48.5° on the same radius — inside the opening —
///    and is drawn by `TacetMark`, not here, so it can carry its own colour.
struct TacetMarkShape: Shape {
    /// Brush width in the unit box. Grows slightly at small sizes.
    var lineWidth: CGFloat = 7.5

    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 104
        let dx = rect.minX + (rect.width - 104 * s) / 2
        let dy = rect.minY + (rect.height - 104 * s) / 2
        let centre = CGPoint(x: dx + 52 * s, y: dy + 52 * s)

        var path = Path()
        path.addArc(center: centre, radius: 29 * s,
                    startAngle: .degrees(-24), endAngle: .degrees(287),
                    clockwise: false)
        return path.strokedPath(StrokeStyle(lineWidth: max(1, lineWidth * s),
                                            lineCap: .round))
    }
}

/// The drawn appearance of the mark: ring in `color`, dot in `dotColor`.
/// At small sizes both the brush and the dot thicken proportionally so the
/// mark does not dissolve into a thin thread.
struct TacetMark: View {
    var size: CGFloat = 20
    var color: Color = Palette.ink
    var dotColor: Color = Palette.accent

    var body: some View {
        let small = size <= 48
        // Dot centre at −48.5° on radius 29 of the 104-box:
        // (52 + 29·cos, 52 + 29·sin) / 104 ≈ (0.6865, 0.2913).
        let dot = (small ? 0.135 : 0.106) * size
        ZStack(alignment: .topLeading) {
            TacetMarkShape(lineWidth: small ? 9.5 : 7.5)
                .fill(color)
            Circle()
                .fill(dotColor)
                .frame(width: dot, height: dot)
                .position(x: 0.6865 * size, y: 0.2913 * size)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// The waiting state — the ensō spins, the dot orbits with the opening.
/// Every "Tacet is working" moment across the app uses this one view so the
/// wait looks the same everywhere. With reduced motion it stands still.
struct SpinningTacetMark: View {
    var size: CGFloat = 16
    var reduceMotion: Bool = false
    @State private var spinning = false

    var body: some View {
        TacetMark(size: size)
            .rotationEffect(.degrees(spinning ? 360 : 0))
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.linear(duration: 1.4).repeatForever(autoreverses: false)) {
                    spinning = true
                }
            }
    }
}

#Preview {
    HStack(spacing: 20) {
        TacetMark(size: 64)
        TacetMark(size: 28)
        TacetMark(size: 20)
        SpinningTacetMark(size: 20)
        HStack(spacing: 8) {
            TacetMark(size: 20)
            Text("Tacet").font(.system(.title3, design: .serif).weight(.medium))
        }
    }
    .padding(40)
}
