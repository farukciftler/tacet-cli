//
//  Theme.swift
//  Tacet
//
//  Design language — spec §3. One surface; palette "Night ink": on light, navy
//  ink on paper; on dark, paper on a night ground. A single accent colour,
//  brass (`accent`) — for brand moments only, state is still told with words
//  and greys. Two typographic voices (SF Pro + New York serif), a 4/8/12/16/22
//  spacing scale.
//

import SwiftUI
import UIKit

// MARK: - Colour

/// Spec §3.1 colour tokens. Resolved dynamically per light/dark mode.
enum Palette {
    private static func dynamic(light: UInt, dark: UInt) -> Color {
        Color(uiColor: UIColor { trait in
            trait.userInterfaceStyle == .dark ? UIColor(hex: dark) : UIColor(hex: light)
        })
    }

    static let background = dynamic(light: 0xF7F3EA, dark: 0x0B1220) // paper / night
    static let ink        = dynamic(light: 0x131C2E, dark: 0xE9E4D8) // primary text, send fill — 15.4:1 / 14.8:1
    // Text tones were chosen against WCAG 2.1 AA (4.5:1 for small text); the
    // ratios are computed against the background token (light 0xF7F3EA, dark
    // 0x0B1220). Grey/muted are no longer neutral — they are cool tones from the
    // same navy family as the background: the palette should read as dilutions
    // of a single ink.
    static let grey    = dynamic(light: 0x4C5568, dark: 0x97A1B4) // secondary text, chip text — 6.8:1 / 7.2:1
    static let muted   = dynamic(light: 0x636D82, dark: 0x7D8698) // placeholder, labels — 4.7:1 / 5.1:1
    static let divider = dynamic(light: 0xE6E0D1, dark: 0x1E2A40) // hairline border, chip frame
    static let fill    = dynamic(light: 0xEDE7D9, dark: 0x17233A) // user bubble background
    /// Brass — the palette's only accent. Brand moments ONLY (mark, selection
    /// marker); NOT a state, warning or action colour. The light mode value was
    /// darkened: raw brass (0xC9A227) stayed at 2.2:1 on paper — now 5.1:1 / 7.7:1.
    static let accent  = dynamic(light: 0x7E6410, dark: 0xC9A227)
    static let error   = dynamic(light: 0xB4483C, dark: 0xD46A5E) // failed chip only — 4.8:1 / 5.4:1
}

extension UIColor {
    fileprivate convenience init(hex: UInt) {
        self.init(
            red:   CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue:  CGFloat(hex & 0xFF) / 255,
            alpha: 1
        )
    }
}

// MARK: - Typography

/// Spec §3.2 — two voices: the user in SF Pro, the Tacet reply in New York serif.
///
/// Every token is bound to a system text style; the point size in parentheses is
/// only that style's reference value at `.large`. Fixed point sizes
/// (`.system(size:)`) are not used — they do not scale under Dynamic Type and
/// stay half a screen tall at accessibility sizes.
enum Typography {
    /// User text — SF Pro (callout ≈ 16).
    static func user() -> Font { .system(.callout, design: .default) }
    /// Tacet reply — New York serif (body ≈ 17).
    static func tacet() -> Font { .system(.body, design: .serif) }
    /// Brand — New York medium (title3 ≈ 20).
    static func brand() -> Font { .system(.title3, design: .serif).weight(.medium) }
    /// Label — uppercase, tracking (caption2 ≈ 11). Tracking is applied on the view side.
    static func tag() -> Font { .system(.caption2, design: .default) }
    /// Chip / meta / date / subtitle (caption ≈ 12).
    static func chip() -> Font { .system(.caption, design: .default) }
    /// Small decorative icons such as the chevron at the end of a row (caption ≈ 12).
    static func iconSmall() -> Font { .system(.caption, design: .default) }
    /// Meaning-carrying icons at the head of a row (subheadline ≈ 15).
    static func icon() -> Font { .system(.subheadline, design: .default) }
    /// Primary action icons such as the top bar (body ≈ 17).
    static func iconLarge() -> Font { .system(.body, design: .default) }
}

// MARK: - Scale

/// Spec §3.3 — spacing, corners, lines.
///
/// The spacings are deliberately fixed: scaling all of them tears the layout
/// apart. Measures that sit next to text or act as touch targets are taken with
/// `@ScaledMetric` on the view side — the tokens here are used as their starting
/// values.
enum Spacing {
    static let s1: CGFloat = 4
    static let s2: CGFloat = 8
    static let s3: CGFloat = 12
    static let s4: CGFloat = 16
    static let s5: CGFloat = 22   // screen horizontal margin

    static let messageGap: CGFloat = 14    // vertical gap between messages
    static let chipReplyGap: CGFloat = 10  // chip ↔ its related reply

    static let hairline: CGFloat = 1
    static let inputCorner: CGFloat = 24
    static let chipCorner: CGFloat = 20

    /// Measures adjacent to text — used as the `@ScaledMetric` starting value in views.
    static let dot: CGFloat = 6          // list selection indicator diameter
    static let iconColumn: CGFloat = 18  // width of the leading icon column
    static let touchTarget: CGFloat = 44

    static let userBubbleWidth: CGFloat = 0.80 // fraction of screen width
    static let tacetReplyWidth: CGFloat = 0.88
}
