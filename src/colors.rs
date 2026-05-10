//! Static curated palette for the color tab. Names are searchable,
//! hex strings are what land in the clipboard on click. Roughly
//! Tailwind 500/700 hues plus neutrals — covers ~90% of what people
//! reach for in casual UI work.

pub const PALETTE: &[(&str, &str)] = &[
    ("red", "#ef4444"),
    ("red dark", "#b91c1c"),
    ("orange", "#f97316"),
    ("orange dark", "#c2410c"),
    ("amber", "#f59e0b"),
    ("yellow", "#eab308"),
    ("lime", "#84cc16"),
    ("green", "#22c55e"),
    ("green dark", "#15803d"),
    ("emerald", "#10b981"),
    ("teal", "#14b8a6"),
    ("cyan", "#06b6d4"),
    ("sky", "#0ea5e9"),
    ("blue", "#3b82f6"),
    ("blue dark", "#1d4ed8"),
    ("indigo", "#6366f1"),
    ("violet", "#8b5cf6"),
    ("purple", "#a855f7"),
    ("fuchsia", "#d946ef"),
    ("pink", "#ec4899"),
    ("rose", "#f43f5e"),
    ("white", "#ffffff"),
    ("gray light", "#e5e7eb"),
    ("gray", "#6b7280"),
    ("gray dark", "#374151"),
    ("black", "#000000"),
];

/// Convert `#rrggbb` to an `(r, g, b)` triple of u8s, falling back to
/// neutral gray on parse failure.
pub fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let s = hex.trim_start_matches('#');
    if s.len() != 6 {
        return (128, 128, 128);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(128);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(128);
    (r, g, b)
}
