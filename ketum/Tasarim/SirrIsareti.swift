//
//  SirrIsareti.swift
//  ketum
//
//  Marka işareti — Arapça س (sin) harfinin tek çizgide sadeleştirilmiş hali.
//  Üç diş (dinleme), bir çanak (tutma); çizgi tek harekette akar ve geri dönmez
//  (giren söz çıkmaz). Daima tek renk: üstündeki metnin rengi. Nokta/renk/gölge yok.
//

import SwiftUI

/// İşaretin yol geometrisi (104×104 birim kutusuna göre, verilen rect'e ölçeklenir).
struct SirrIsareti: Shape {
    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 104
        let dx = rect.minX + (rect.width - 104 * s) / 2
        let dy = rect.minY + (rect.height - 104 * s) / 2
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: dx + x * s, y: dy + y * s) }
        var path = Path()
        path.move(to: p(86, 40))
        path.addCurve(to: p(76, 40), control1: p(83.5, 34.5), control2: p(78.5, 34.5))
        path.addCurve(to: p(66, 40), control1: p(73.5, 34.5), control2: p(68.5, 34.5))
        path.addCurve(to: p(56, 40), control1: p(63.5, 34.5), control2: p(58.5, 34.5))
        path.addCurve(to: p(41, 63), control1: p(54, 49.5), control2: p(50, 58.5))
        path.addCurve(to: p(21, 50.5), control1: p(29.5, 68.5), control2: p(18.5, 61.5))
        return path
    }
}

/// İşaretin çizili görünümü. Küçük boylarda daha kalın kontur (okunurluk).
struct SirrMark: View {
    var boyut: CGFloat = 20
    var renk: Color = Renk.murekkep

    var body: some View {
        // ≤48 pt'de "kalın kesim" hissi için oran biraz yükselir.
        let oran: CGFloat = boyut <= 48 ? 0.095 : 0.075
        SirrIsareti()
            .stroke(renk, style: StrokeStyle(lineWidth: max(1.4, boyut * oran),
                                             lineCap: .round, lineJoin: .round))
            .frame(width: boyut, height: boyut)
            .accessibilityHidden(true)
    }
}

#Preview {
    HStack(spacing: 20) {
        SirrMark(boyut: 64)
        SirrMark(boyut: 28)
        SirrMark(boyut: 20)
        HStack(spacing: 8) {
            SirrMark(boyut: 20)
            Text("sirr").font(.system(.title3, design: .serif).weight(.medium))
        }
    }
    .padding(40)
}
