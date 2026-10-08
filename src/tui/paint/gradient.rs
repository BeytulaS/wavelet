//! Color math for gradients and pills.
//!
//! Faithful port of noodle's `gradient.ts`, adapted to ratatui's `Color::Rgb`.
//! Everything here is pure math — no allocation in the hot path beyond the
//! caller's own span vector.

use ratatui::style::Color;

/// An RGB triple we can interpolate. ratatui's `Color::Rgb` is the render target,
/// but we lerp in this plain struct to keep the math obvious.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse a `#rrggbb` (or `rrggbb`) hex string. Falls back to black on garbage
    /// so a typo in a theme never panics the render loop.
    pub fn from_hex(hex: &str) -> Self {
        let h = hex.strip_prefix('#').unwrap_or(hex);
        let parse = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
        if h.len() >= 6 {
            Self::new(parse(0), parse(2), parse(4))
        } else {
            Self::new(0, 0, 0)
        }
    }

    /// Convert to a ratatui truecolor value.
    pub const fn to_color(self) -> Color {
        Color::Rgb(self.r, self.g, self.b)
    }
}

impl From<Rgb> for Color {
    fn from(c: Rgb) -> Self {
        c.to_color()
    }
}

/// Linear interpolation between two channel values.
#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let a = f32::from(a);
    let b = f32::from(b);
    (a + (b - a) * t).round().clamp(0.0, 255.0) as u8
}

/// Interpolate between two colors. `t` is clamped to `[0, 1]`.
pub fn lerp_color(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    Rgb::new(
        lerp_u8(a.r, b.r, t),
        lerp_u8(a.g, b.g, t),
        lerp_u8(a.b, b.b, t),
    )
}

/// Sample a multi-stop gradient at position `t` in `[0, 1]`.
///
/// Mirrors noodle's `interpolateGradient`: stops are spread evenly across the
/// range and we lerp within the active segment.
pub fn interpolate(stops: &[Rgb], t: f32) -> Rgb {
    match stops {
        [] => Rgb::new(0, 0, 0),
        [only] => *only,
        _ => {
            if t <= 0.0 {
                return stops[0];
            }
            if t >= 1.0 {
                return stops[stops.len() - 1];
            }
            let segment = t * (stops.len() - 1) as f32;
            let i = segment.floor() as usize;
            let local_t = segment - i as f32;
            lerp_color(stops[i], stops[i + 1], local_t)
        }
    }
}

/// Sample `n` evenly-spaced colors across a gradient. Handy for coloring `n`
/// visualizer bars or an `n`-cell progress bar in one shot.
pub fn sample(stops: &[Rgb], n: usize) -> Vec<Rgb> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![interpolate(stops, 0.0)];
    }
    (0..n)
        .map(|i| interpolate(stops, i as f32 / (n - 1) as f32))
        .collect()
}

