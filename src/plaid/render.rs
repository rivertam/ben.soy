use std::fmt::Write;

use super::{Pattern, Sett};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(hex: &str) -> Result<Self, String> {
        if hex.len() != 7
            || !hex.starts_with('#')
            || !hex[1..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Colors must use six-digit hex notation, such as #192b24.".into());
        }
        Ok(Self(
            u8::from_str_radix(&hex[1..3], 16).unwrap(),
            u8::from_str_radix(&hex[3..5], 16).unwrap(),
            u8::from_str_radix(&hex[5..7], 16).unwrap(),
        ))
    }
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
    pub fn rgba(self) -> (u8, u8, u8, u8) {
        (self.0, self.1, self.2, 255)
    }
    pub fn mix(self, other: Self, amount: f64) -> Self {
        let channel =
            |a: u8, b: u8| (f64::from(a) * (1.0 - amount) + f64::from(b) * amount).round() as u8;
        Self(
            channel(self.0, other.0),
            channel(self.1, other.1),
            channel(self.2, other.2),
        )
    }
    fn luminance(self) -> f64 {
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.0) + 0.7152 * linear(self.1) + 0.0722 * linear(self.2)
    }
    pub fn contrast(self, other: Self) -> f64 {
        let a = self.luminance();
        let b = other.luminance();
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    fn alpha(self, opacity: f64) -> String {
        format!("rgb({} {} {} / {opacity:.3})", self.0, self.1, self.2)
    }
}

/// Both CSS tokens and bitmap/SVG foregrounds consume this derived finish.
#[derive(Clone, Debug)]
pub struct Finish {
    pub light: bool,
    pub page: Rgb,
    pub card: Rgb,
    pub ink: Rgb,
    pub ink2: Rgb,
    pub muted: Rgb,
    pub hairline: Rgb,
    pub accent: Rgb,
    pub hot: Rgb,
    pub patina: Rgb,
    pub steel: Rgb,
    pub brass: Rgb,
    pub wash_opacity: f64,
}

impl Finish {
    fn candidate(pattern: &Pattern, light: bool) -> Self {
        let page = if light {
            Rgb(250, 249, 244)
        } else {
            Rgb(17, 22, 27)
        };
        let ink = if light {
            Rgb(20, 25, 32)
        } else {
            Rgb(249, 246, 235)
        };
        let ink2 = ink.mix(page, 0.07);
        let muted = ink.mix(page, 0.17);
        let chroma = |rgb: &&Rgb| rgb.0.max(rgb.1).max(rgb.2) - rgb.0.min(rgb.1).min(rgb.2);
        let color = *pattern.colors.values().max_by_key(chroma).unwrap();
        let accessible = |color: Rgb| {
            (0..=100)
                .map(|i| color.mix(ink, f64::from(i) / 100.0))
                .find(|c| c.contrast(page) >= 7.0)
                .unwrap_or(ink)
        };
        let mut finish = Self {
            light,
            page,
            card: page,
            ink,
            ink2,
            muted,
            hairline: ink.mix(page, 0.55),
            accent: accessible(color),
            hot: accessible(color.mix(ink, 0.3)),
            patina: accessible(Rgb(64, 144, 96)),
            steel: accessible(Rgb(70, 123, 164)),
            brass: accessible(Rgb(178, 125, 39)),
            wash_opacity: 0.0,
        };
        // Test every crossing and both texture extrema. Average luminance
        // cannot protect text over a cloth containing black AND white bands.
        let crossings = pattern.crossings();
        finish.wash_opacity = (0..=100)
            .map(|i| f64::from(i) / 100.0)
            .find(|opacity| {
                crossings.iter().all(|color| {
                    finish
                        .text_colors()
                        .iter()
                        .all(|ink| ink.contrast(color.mix(page, *opacity)) >= 4.6)
                })
            })
            .unwrap_or(1.0);
        finish
    }

    fn text_colors(&self) -> [Rgb; 8] {
        [
            self.ink,
            self.ink2,
            self.muted,
            self.accent,
            self.hot,
            self.patina,
            self.steel,
            self.brass,
        ]
    }

    pub fn tokens(&self) -> String {
        let mut css = format!(
            "color-scheme:{};",
            if self.light { "light" } else { "dark" }
        );
        for (name, rgb) in [
            ("page", self.page),
            ("card", self.card),
            ("ink", self.ink),
            ("ink2", self.ink2),
            ("muted", self.muted),
            ("hairline", self.hairline),
            ("oxide", self.accent),
            ("oxide-hot", self.hot),
            ("patina", self.patina),
            ("steel", self.steel),
            ("brass", self.brass),
        ] {
            write!(css, "--color-{name}:{};", rgb.hex()).unwrap();
        }
        css
    }
}

impl Pattern {
    fn crossings(&self) -> Vec<Rgb> {
        let mut colors = Vec::new();
        for warp in &self.warp.bands {
            for weft in &self.weft.bands {
                let cross = self.colors[&warp.color].mix(self.colors[&weft.color], 0.5);
                colors.push(cross.mix(Rgb(255, 255, 255), 0.045));
                colors.push(cross.mix(Rgb(0, 0, 0), 0.055));
            }
        }
        colors
    }

    pub fn finish(&self) -> Finish {
        let dark = Finish::candidate(self, false);
        let light = Finish::candidate(self, true);
        if light.wash_opacity < dark.wash_opacity {
            light
        } else {
            dark
        }
    }

    fn gradient(&self, sett: &Sett, angle: i16, opacity: f64, repeat: f64) -> String {
        let unit = repeat / f64::from(self.warp.total());
        let mut position = 0.0;
        let stops: Vec<_> = sett
            .expanded()
            .iter()
            .map(|band| {
                let start = position;
                position += f64::from(band.threads) * unit;
                format!(
                    "{} {start:.4}px {position:.4}px",
                    self.colors[&band.color].alpha(opacity)
                )
            })
            .collect();
        format!("repeating-linear-gradient({angle}deg,{})", stops.join(","))
    }

    pub fn background(&self, backed: bool, repeat: f64) -> String {
        let angle = self.spec.rotation_deg;
        let mut layers = Vec::new();
        if backed {
            let finish = self.finish();
            let wash = finish.page.alpha(finish.wash_opacity);
            layers.push(format!("linear-gradient({wash},{wash})"));
        }
        // Twill repeats independently of the sett: odd thread totals cannot
        // introduce a texture seam at the end of a stripe repeat.
        layers.push(format!("repeating-linear-gradient({}deg,rgb(255 255 255 / .045) 0 .7071px,transparent .7071px 1.4142px,rgb(0 0 0 / .055) 1.4142px 2.1213px,transparent 2.1213px 2.8284px)", 135 + angle));
        layers.push(self.gradient(&self.weft, 180 + angle, 0.5, repeat));
        layers.push(self.gradient(&self.warp, 90 + angle, 1.0, repeat));
        layers.join(",")
    }

    pub fn stylesheet(&self) -> String {
        let tokens = self.finish().tokens();
        let background = self.background(true, f64::from(self.spec.repeat_px));
        let swatch = self.background(false, 18.0);
        format!(
            "[data-theme=\"plaid\"]{{{tokens}}}\n\
             :root[data-theme=\"plaid\"] body{{background-color:var(--color-page);background-image:{background};color:var(--color-ink);text-shadow:none}}\n\
             [data-theme=\"plaid\"].theme-dot,:root[data-theme=\"plaid\"] .theme-dd>summary>.theme-dot{{background-image:{swatch}}}\n\
             [data-theme=\"plaid\"] ::selection{{color:var(--color-page);background:var(--color-oxide-hot)}}"
        )
    }

    /// Draft styles affect only the preview, never the editor's own theme.
    pub fn preview_stylesheet(&self) -> String {
        format!(
            "[data-plaid-preview]{{{}background-image:{};color:var(--color-ink);text-shadow:none}}[data-plaid-cloth]{{background-image:{}}}",
            self.finish().tokens(),
            self.background(true, f64::from(self.spec.repeat_px)),
            self.background(false, f64::from(self.spec.repeat_px))
        )
    }

    /// `prefix` is a caller-owned static identifier, never a document field.
    pub fn svg_defs(&self, prefix: &'static str) -> String {
        let mut svg = String::new();
        let rotation = self.spec.rotation_deg;
        for (axis, sett) in [("warp", &self.warp), ("weft", &self.weft)] {
            let total = f64::from(sett.total());
            let length = total * f64::from(self.spec.repeat_px) / f64::from(self.warp.total());
            let (x, y) = if axis == "warp" {
                (length, 0.0)
            } else {
                (0.0, length)
            };
            write!(svg, r#"<linearGradient id="{prefix}-{axis}" x1="0" y1="0" x2="{x:.4}" y2="{y:.4}" gradientUnits="userSpaceOnUse" spreadMethod="repeat" gradientTransform="rotate({rotation})">"#).unwrap();
            let mut position = 0.0;
            for band in sett.expanded() {
                let start = position / total;
                position += f64::from(band.threads);
                let end = position / total;
                let color = self.colors[&band.color].hex();
                write!(svg, r#"<stop offset="{start:.8}" stop-color="{color}"/><stop offset="{end:.8}" stop-color="{color}"/>"#).unwrap();
            }
            svg.push_str("</linearGradient>");
        }
        // Match the CSS diagonal: x+y repeats every four pixels, regardless
        // of the sett dimensions. Vector gradients also avoid expensive
        // bicubic sampling of tiny pattern bitmaps in the social renderer.
        write!(svg, r##"<linearGradient id="{prefix}-twill" x1="0" y1="0" x2="2" y2="2" gradientUnits="userSpaceOnUse" spreadMethod="repeat" gradientTransform="rotate({rotation})">
<stop offset="0" stop-color="#fff" stop-opacity=".045"/><stop offset=".25" stop-color="#fff" stop-opacity=".045"/>
<stop offset=".25" stop-opacity="0"/><stop offset=".5" stop-opacity="0"/>
<stop offset=".5" stop-color="#000" stop-opacity=".055"/><stop offset=".75" stop-color="#000" stop-opacity=".055"/>
<stop offset=".75" stop-opacity="0"/><stop offset="1" stop-opacity="0"/>
</linearGradient>"##).unwrap();
        svg
    }

    pub fn svg_background(&self, prefix: &'static str, bounds: [u32; 4], backed: bool) -> String {
        let [x, y, width, height] = bounds;
        let mut svg = String::new();
        for (layer, opacity) in [("warp", 1.0), ("weft", 0.5), ("twill", 1.0)] {
            write!(svg, r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="url(#{prefix}-{layer})" opacity="{opacity}"/>"#).unwrap();
        }
        if backed {
            let finish = self.finish();
            write!(svg, r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="{}" opacity="{}"/>"#, finish.page.hex(), finish.wash_opacity).unwrap();
        }
        svg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plaid::{GENERATOR_VERSION, GenerateMode, generate};

    #[test]
    fn every_text_role_contrasts_with_crossings_and_surfaces() {
        let initial = Pattern::default();
        let mut patterns = vec![initial.clone()];
        let mut spec = initial.spec().clone();
        spec.palette = [
            ("K".into(), "#000000".into()),
            ("W".into(), "#ffffff".into()),
        ]
        .into();
        spec.warp = "K/25 W/31".into();
        patterns.push(Pattern::new(spec).unwrap());
        for i in 0..24 {
            patterns.push(
                generate(
                    &initial,
                    &i.to_string(),
                    GENERATOR_VERSION,
                    GenerateMode::All,
                )
                .unwrap(),
            );
        }
        for pattern in patterns {
            let finish = pattern.finish();
            for background in pattern
                .crossings()
                .into_iter()
                .map(|c| c.mix(finish.page, finish.wash_opacity))
                .chain([finish.card])
            {
                for ink in finish.text_colors() {
                    assert!(ink.contrast(background) >= 4.5);
                }
            }
        }
    }

    #[test]
    fn light_and_dark_cloth_choose_appropriate_finishes() {
        let mut spec = Pattern::default().spec().clone();
        for hex in spec.palette.values_mut() {
            *hex = "#eeeeee".into();
        }
        assert!(Pattern::new(spec.clone()).unwrap().finish().light);
        for hex in spec.palette.values_mut() {
            *hex = "#111111".into();
        }
        assert!(!Pattern::new(spec).unwrap().finish().light);
    }

    #[test]
    fn svg_renders_odd_and_independent_repeats_without_unbounded_tiles() {
        let mut spec = Pattern::default().spec().clone();
        spec.warp = "K/25 Y/2".into();
        spec.weft = Some("...R3 B7...".into());
        let pattern = Pattern::new(spec).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><defs>{}</defs>{}</svg>"#,
            pattern.svg_defs("test"),
            pattern.svg_background("test", [0, 0, 128, 128], true)
        );
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(128, 128).unwrap();
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        assert!(pixmap.pixels().iter().all(|p| p.alpha() == 255));
        assert!(pattern.stylesheet().len() < 15_000);
    }
}
