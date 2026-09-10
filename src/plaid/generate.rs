use std::collections::BTreeMap;

use rand::{
    Rng, RngExt, SeedableRng,
    distr::{Distribution, StandardUniform},
    rngs::ChaCha8Rng,
    seq::IndexedRandom,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Band, Pattern, Sett, Spec};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerateMode {
    All,
    Colors,
    Stripes,
}

// After release, bump when changing the RNG or recipes; the fixture guards replay.
// Saved documents contain the resulting cloth, independently of this version.
pub const GENERATOR_VERSION: u8 = 1;

const PALETTES: [[&str; 5]; 8] = [
    ["#192b24", "#071426", "#7e252d", "#e2c168", "#ece6d2"],
    ["#e9e4d8", "#b9c6cd", "#768f9e", "#d38a65", "#393943"],
    ["#f4d968", "#df542f", "#315275", "#faf1da", "#722f4c"],
    ["#f0ccd3", "#b2cba7", "#855079", "#f5edd9", "#41585f"],
    ["#1c294b", "#277d88", "#b84377", "#e8be49", "#c3dad8"],
    ["#e8e5de", "#434b51", "#b7b2a8", "#bf3f38", "#fffaf1"],
    ["#a44426", "#e5ab42", "#563441", "#315546", "#ecce9a"],
    ["#efe7bc", "#6f8c45", "#2b6667", "#d0a2b9", "#8c5147"],
];

/// Generate a complete plaid with `rng.random::<Spec>()`. The fields share a
/// recipe because stripe codes must refer to the palette generated with them.
impl Distribution<Spec> for StandardUniform {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Spec {
        let mut palette = ["A", "B", "C", "D", "E"]
            .into_iter()
            .map(|code| (code.into(), String::new()))
            .collect();
        recolor(&mut palette, rng);
        let codes: Vec<_> = palette.keys().cloned().collect();
        Spec {
            version: 1,
            palette,
            warp: stripes(rng, &codes),
            weft: None,
            repeat_px: *[64, 80, 96, 112, 144].choose(rng).unwrap(),
            rotation_deg: rng.random_range(0..=15),
        }
    }
}

pub fn generate(
    current: &Pattern,
    seed: &str,
    version: u8,
    mode: GenerateMode,
) -> Result<Pattern, String> {
    if version != GENERATOR_VERSION || seed.is_empty() || seed.len() > 128 {
        return Err(format!(
            "Use generator version {GENERATOR_VERSION} and a seed of 1–128 characters."
        ));
    }
    // Hash the human-readable seed once into ChaCha's fixed-size seed.
    let seed = Sha256::digest(format!("plaid-generator-{version}:{seed}"));
    let mut rng = ChaCha8Rng::from_seed(seed.into());
    let mut spec = current.spec().clone();
    match mode {
        GenerateMode::All => spec = rng.random(),
        GenerateMode::Colors => recolor(&mut spec.palette, &mut rng),
        GenerateMode::Stripes => {
            let codes: Vec<_> = spec.palette.keys().cloned().collect();
            spec.warp = stripes(&mut rng, &codes);
            spec.weft = spec.weft.map(|_| stripes(&mut rng, &codes));
        }
    }
    Pattern::new(spec)
}

fn recolor<R: Rng + ?Sized>(colors: &mut BTreeMap<String, String>, rng: &mut R) {
    let palette = PALETTES.choose(rng).unwrap();
    let offset = rng.random_range(0..palette.len() as u32) as usize;
    for (i, hex) in colors.values_mut().enumerate() {
        let rgb = super::Rgb::parse(palette[(i + offset) % palette.len()]).unwrap();
        // Small channel shifts keep a palette's family while making rerolls varied.
        let shift = rng.random_range(-12..=12i16);
        *hex = super::Rgb(
            (i16::from(rgb.0) + shift).clamp(0, 255) as u8,
            (i16::from(rgb.1) + shift).clamp(0, 255) as u8,
            (i16::from(rgb.2) + shift).clamp(0, 255) as u8,
        )
        .hex();
    }
}

fn stripes<R: Rng + ?Sized>(rng: &mut R, codes: &[String]) -> String {
    let offset = rng.random_range(0..codes.len() as u32) as usize;
    let mut band = |color: usize, sizes: &[u16]| Band {
        color: codes[(offset + color) % codes.len()].clone(),
        threads: *sizes.choose(rng).unwrap(),
    };
    let bands = vec![
        band(0, &[16, 24, 32, 48]),
        band(1, &[16, 24, 32, 40]),
        band(0, &[4, 8, 12]),
        band(2, &[2, 4, 6]),
        band(3, &[1, 2, 3]),
        band(2, &[2, 4, 6]),
        band(4, &[12, 20, 32]),
    ];
    Sett {
        mirrored: rng.random_ratio(3, 4),
        bands,
    }
    .notation()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeds_repeat_and_partial_rerolls_preserve_other_inputs() {
        let current = Pattern::new(Spec {
            weft: Some("...K16 Y8...".into()),
            ..Spec::default()
        })
        .unwrap();
        let a = generate(&current, "shirt", GENERATOR_VERSION, GenerateMode::All).unwrap();
        assert_eq!(
            a.text(),
            generate(&current, "shirt", GENERATOR_VERSION, GenerateMode::All)
                .unwrap()
                .text()
        );
        assert_ne!(
            a.text(),
            generate(&current, "shirt-2", GENERATOR_VERSION, GenerateMode::All)
                .unwrap()
                .text()
        );
        let colors = generate(&current, "shirt", GENERATOR_VERSION, GenerateMode::Colors).unwrap();
        assert_eq!(current.spec().warp, colors.spec().warp);
        assert_eq!(current.spec().weft, colors.spec().weft);
        assert_eq!(current.spec().repeat_px, colors.spec().repeat_px);
        assert_eq!(current.spec().rotation_deg, colors.spec().rotation_deg);
        let stripes =
            generate(&current, "shirt", GENERATOR_VERSION, GenerateMode::Stripes).unwrap();
        assert_eq!(current.spec().palette, stripes.spec().palette);
        assert_eq!(current.spec().repeat_px, stripes.spec().repeat_px);
        assert_eq!(current.spec().rotation_deg, stripes.spec().rotation_deg);
        assert!(stripes.spec().weft.is_some());
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        for i in 0..100 {
            Pattern::new(rng.random::<Spec>()).unwrap();
            generate(
                &current,
                &i.to_string(),
                GENERATOR_VERSION,
                GenerateMode::All,
            )
            .unwrap();
        }
    }

    #[test]
    fn generator_v1_replay_fixture() {
        let fingerprints = [
            GenerateMode::All,
            GenerateMode::Colors,
            GenerateMode::Stripes,
        ]
        .map(|mode| {
            generate(&Pattern::default(), "shirt", 1, mode)
                .unwrap()
                .fingerprint()
        });
        assert_eq!(
            fingerprints,
            [
                "0d6815d6d1af57920234f1593d07ca21c81abc19f2ab996701d0edc01d6eea0a",
                "bc605c1a5fc912a4bf07f3bc3d74a227ed8c9dfcbc290830eb033a826f221c3b",
                "c0a3fbbc894ae468ae108146f7e7648817116081cf26f361608cb34816db2549",
            ]
        );
    }

    #[test]
    fn rejects_unsupported_generators_and_invalid_seeds() {
        let current = Pattern::default();
        for version in [0, GENERATOR_VERSION + 1] {
            assert!(generate(&current, "shirt", version, GenerateMode::All).is_err());
        }
        for seed in [String::new(), "x".repeat(129)] {
            assert!(generate(&current, &seed, GENERATOR_VERSION, GenerateMode::All).is_err());
        }
    }
}
