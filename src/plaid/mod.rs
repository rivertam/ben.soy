//! A small, versioned tartan document. Threadcounts describe cloth; display
//! size and rotation do not change its bands. See docs/plaid.md.

mod generate;
mod render;
pub mod store;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use generate::{GENERATOR_VERSION, GenerateMode, generate};
pub use render::{Finish, Rgb};

pub const MAX_COLORS: usize = 16;
pub const MAX_BANDS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u8,
    pub palette: BTreeMap<String, String>,
    pub warp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weft: Option<String>,
    pub repeat_px: u16,
    pub rotation_deg: i16,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            version: 1,
            palette: [
                ("K", "#192b24"),
                ("B", "#071426"),
                ("R", "#7e252d"),
                ("Y", "#e2c168"),
            ]
            .into_iter()
            .map(|(code, hex)| (code.into(), hex.into()))
            .collect(),
            warp: "...K24 B24 K12 R6 Y2 R6 K28...".into(),
            weft: None,
            repeat_px: 96,
            rotation_deg: 10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Band {
    pub color: String,
    pub threads: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Sett {
    pub mirrored: bool,
    pub bands: Vec<Band>,
}

impl Sett {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 2048 {
            return Err("A threadcount must be at most 2,048 characters.".into());
        }
        let text = text.trim();
        let repeating = text.starts_with("...") && text.ends_with("...") && text.len() >= 6;
        let inner = if repeating {
            &text[3..text.len() - 3]
        } else {
            text
        };
        let tokens: Vec<_> = inner.split_whitespace().collect();
        if !(2..=MAX_BANDS).contains(&tokens.len()) {
            return Err(format!("Use between 2 and {MAX_BANDS} bands per axis."));
        }
        let mut bands = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let boundary = token.bytes().take_while(u8::is_ascii_alphabetic).count();
            let code = token[..boundary].to_ascii_uppercase();
            if !valid_code(&code) {
                return Err("Color codes must contain 1–3 letters.".into());
            }
            let number = &token[boundary..];
            let pivot = number.starts_with('/');
            if pivot != (!repeating && (index == 0 || index + 1 == tokens.len())) {
                return Err(
                    "Use / on both end bands for a mirrored sett, or wrap a repeating sett in ..."
                        .into(),
                );
            }
            let number = number.strip_prefix('/').unwrap_or(number);
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("{token} needs a whole-number thread count."));
            }
            let threads = number
                .parse::<u16>()
                .ok()
                .filter(|n| (1..=512).contains(n))
                .ok_or_else(|| "Each stripe needs 1–512 threads.".to_string())?;
            bands.push(Band {
                color: code,
                threads,
            });
        }
        let sett = Self {
            mirrored: !repeating,
            bands,
        };
        if sett.total() > 4096 {
            return Err("An expanded sett may contain at most 4,096 threads.".into());
        }
        Ok(sett)
    }

    pub fn notation(&self) -> String {
        let bands = self
            .bands
            .iter()
            .enumerate()
            .map(|(i, band)| {
                let pivot = if self.mirrored && (i == 0 || i + 1 == self.bands.len()) {
                    "/"
                } else {
                    ""
                };
                format!("{}{pivot}{}", band.color, band.threads)
            })
            .collect::<Vec<_>>()
            .join(" ");
        if self.mirrored {
            bands
        } else {
            format!("...{bands}...")
        }
    }

    /// Pivots are full widths. Reflect only the interior, never double a pivot.
    pub fn expanded(&self) -> Vec<&Band> {
        let mut bands: Vec<_> = self.bands.iter().collect();
        if self.mirrored {
            bands.extend(self.bands[1..self.bands.len() - 1].iter().rev());
        }
        bands
    }

    pub fn total(&self) -> u32 {
        self.expanded()
            .iter()
            .map(|band| u32::from(band.threads))
            .sum()
    }
}

fn valid_code(code: &str) -> bool {
    (1..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_uppercase())
}

/// Only this constructor admits a definition to renderers or persistence.
#[derive(Clone, Debug)]
pub struct Pattern {
    spec: Spec,
    pub(crate) colors: BTreeMap<String, Rgb>,
    warp: Sett,
    weft: Sett,
}

impl Default for Pattern {
    fn default() -> Self {
        Self::new(Spec::default()).expect("the built-in plaid is valid")
    }
}

impl Pattern {
    pub fn new(mut spec: Spec) -> Result<Self, String> {
        if spec.version != 1 {
            return Err("This editor supports plaid version 1.".into());
        }
        if !(2..=MAX_COLORS).contains(&spec.palette.len()) {
            return Err(format!("Use between 2 and {MAX_COLORS} palette colors."));
        }
        if !(24..=320).contains(&spec.repeat_px) || !(-45..=45).contains(&spec.rotation_deg) {
            return Err(
                "Repeat size must be 24–320 px and rotation must be −45–45 degrees.".into(),
            );
        }
        let mut colors = BTreeMap::new();
        for (code, hex) in &spec.palette {
            let code = code.to_ascii_uppercase();
            if !valid_code(&code) || colors.contains_key(&code) {
                return Err("Use unique palette codes containing 1–3 letters.".into());
            }
            colors.insert(code, Rgb::parse(hex)?);
        }
        let warp = Sett::parse(&spec.warp).map_err(|e| format!("Vertical bands: {e}"))?;
        let weft = match &spec.weft {
            Some(text) => Sett::parse(text).map_err(|e| format!("Horizontal bands: {e}"))?,
            None => warp.clone(),
        };
        for band in warp.bands.iter().chain(&weft.bands) {
            if !colors.contains_key(&band.color) {
                return Err(format!(
                    "Add {} to the palette before using it in a stripe.",
                    band.color
                ));
            }
        }
        spec.palette = colors
            .iter()
            .map(|(code, rgb)| (code.clone(), rgb.hex()))
            .collect();
        spec.warp = warp.notation();
        spec.weft = (weft != warp).then(|| weft.notation());
        Ok(Self {
            spec,
            colors,
            warp,
            weft,
        })
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }
    pub fn warp(&self) -> &Sett {
        &self.warp
    }
    pub fn weft(&self) -> &Sett {
        &self.weft
    }
    pub fn text(&self) -> String {
        serde_json::to_string_pretty(&self.spec).expect("a plaid spec serializes")
    }
    pub fn fingerprint(&self) -> String {
        digest(
            serde_json::to_vec(&self.spec)
                .expect("a plaid spec serializes")
                .as_slice(),
        )
    }
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_pivots_have_full_width_and_repeat_round_trips() {
        let sett = Sett::parse(" b/24 W4 r/2 ").unwrap();
        assert_eq!(sett.notation(), "B/24 W4 R/2");
        assert_eq!(
            sett.expanded()
                .iter()
                .map(|b| (b.color.as_str(), b.threads))
                .collect::<Vec<_>>(),
            [("B", 24), ("W", 4), ("R", 2), ("W", 4)]
        );
        assert_eq!(sett.total(), 34);
        for notation in ["K/2 W/3", "...K3 W9 R1..."] {
            let parsed = Sett::parse(notation).unwrap();
            assert_eq!(Sett::parse(&parsed.notation()).unwrap(), parsed);
        }
    }

    #[test]
    fn malformed_or_unbounded_threadcounts_never_reach_rendering() {
        for text in [
            "",
            "K24 W4",
            "...K/2 W4...",
            "K/2 W/4 R/2",
            "K/0 W/2",
            "K/-1 W/2",
            "K/513 W/2",
            "K/2 W/2...",
            "<script>2 W/2",
            "é/2 W/2",
        ] {
            assert!(Sett::parse(text).is_err(), "{text}");
        }
        let too_many = format!("...{}...", vec!["K512"; 33].join(" "));
        assert!(Sett::parse(&too_many).is_err());
    }

    #[test]
    fn canonical_documents_have_stable_fingerprints_and_independent_axes() {
        let a = Pattern::default();
        let mut spec = a.spec().clone();
        spec.warp = spec.warp.to_lowercase();
        spec.weft = Some(a.spec().warp.clone());
        let b = Pattern::new(spec.clone()).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(
            Pattern::new(serde_json::from_str(&a.text()).unwrap())
                .unwrap()
                .text(),
            a.text()
        );
        spec.weft = Some("Y/2 B/32".into());
        assert_ne!(
            Pattern::new(spec.clone()).unwrap().fingerprint(),
            a.fingerprint()
        );
        spec.palette.remove("Y");
        assert!(Pattern::new(spec).is_err());
        let mut spec = Spec::default();
        spec.palette.insert("K".into(), "red;display:none".into());
        assert!(Pattern::new(spec).is_err());
    }
}
