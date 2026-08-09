//! Theme registry, mirroring `interests.rs`. Each theme is a `[data-theme]`
//! variable block in `styles/themes.css`; this list is the single source of
//! truth for the switcher menu in the shell (id, menu label, tooltip blurb).
//! The first entry is the site's own finish, "mill and oxide" — it renders as
//! the ABSENCE of `data-theme` on `<html>`, though its CSS block still exists
//! so the switcher's swatch dots can wear it. Adding a theme means one entry
//! here plus one block in `themes.css`; the tests below hold the two in sync.

pub struct Theme {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu tooltip, in the footer's deadpan voice.
    pub blurb: &'static str,
    /// Especially whimsical: the menu row wears the big top 🎪, and only
    /// these themes carry a continuous tune (theme.js TUNES + the
    /// `.theme-music` reveal in themes.css — the test below keeps all
    /// three in agreement). Stings stay universal; the circus is opt-in.
    pub whimsical: bool,
}

/// Ordered as the menu renders: the house finish first, the sensible modes,
/// then descending order of sensibleness.
pub static THEMES: [Theme; 5] = [
    Theme {
        id: "oxide",
        label: "mill & oxide",
        blurb: "the house finish: steel paper, rust accents",
        whimsical: false,
    },
    Theme {
        id: "dark",
        label: "night shift",
        blurb: "dark mode; the mill after hours",
        whimsical: false,
    },
    Theme {
        id: "terminal",
        label: "terminal",
        blurb: "amber phosphor; the log at 60 Hz",
        whimsical: false,
    },
    Theme {
        id: "felix",
        label: "felix mode",
        blurb: "POV: you're about to throw a ball",
        whimsical: true,
    },
    Theme {
        id: "clown",
        label: "clown mode",
        blurb: "the contrast ratios remain, regrettably, compliant",
        whimsical: true,
    },
];

/// Inline in `<head>` before the stylesheet so the stored theme applies
/// before first paint (no flash of mill-and-oxide for a committed clown).
/// Storage distinguishes "never chose" (`null` — OS dark preference may pick
/// the dark theme) from an explicit choice of the default (respected as-is).
/// Kept dependency-free and em-dash-free; `emdash.rs` skips `<script>` but
/// only inside `<main>`, and this tag lives in `<head>` on trust.
pub const THEME_BOOT_JS: &str = "(function(){try{var t=localStorage.getItem('bens-theme');\
if(t&&t!=='oxide'&&t!=='dark'&&t!=='terminal'&&t!=='felix'&&t!=='clown'){localStorage.removeItem('bens-theme');t=null}\
if(!t&&matchMedia('(prefers-color-scheme: dark)').matches)t='dark';\
if(t&&t!=='oxide')document.documentElement.dataset.theme=t}catch(e){}})()";

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES_CSS: &str = include_str!("../../styles/themes.css");
    const THEME_JS: &str = include_str!("../../src/components/theme.js");

    /// Every `[data-theme="…"]` selector target in themes.css, ignoring
    /// comments (the file's prose mentions the selector syntax).
    fn css_theme_ids() -> Vec<String> {
        let mut css = String::new();
        let mut rest = THEMES_CSS;
        while let Some(start) = rest.find("/*") {
            css.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            rest = match after.find("*/") {
                Some(end) => &after[end + 2..],
                None => "",
            };
        }
        css.push_str(rest);

        let mut ids = Vec::new();
        let mut rest = css.as_str();
        while let Some(start) = rest.find("[data-theme=\"") {
            let after = &rest[start + "[data-theme=\"".len()..];
            let end = after.find('"').expect("unterminated data-theme selector");
            ids.push(after[..end].to_string());
            rest = &after[end..];
        }
        ids
    }

    #[test]
    fn every_registered_theme_has_a_css_block() {
        for theme in THEMES.iter() {
            // The bare token block specifically (`[data-theme="id"] {`) —
            // a surviving decoration rule must not satisfy this after the
            // palette block itself was deleted.
            let block = format!("[data-theme=\"{}\"] {{", theme.id);
            assert!(
                THEMES_CSS.contains(&block),
                "theme `{}` is registered but has no `{block}` token block in styles/themes.css",
                theme.id
            );
        }
    }

    #[test]
    fn every_css_block_is_a_registered_theme() {
        for css_id in css_theme_ids() {
            assert!(
                THEMES.iter().any(|t| t.id == css_id),
                "styles/themes.css styles [data-theme=\"{css_id}\"] but the registry doesn't list it"
            );
        }
    }

    #[test]
    fn ids_are_unique_kebab_and_the_default_leads() {
        // "oxide" is the default: the boot script and theme.js treat it as
        // "remove the attribute", so it must stay first and keep its id.
        assert_eq!(THEMES[0].id, "oxide", "the house finish renders first");
        for (i, theme) in THEMES.iter().enumerate() {
            assert!(
                theme
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "theme id `{}` isn't kebab-case",
                theme.id
            );
            assert!(!theme.label.is_empty() && !theme.blurb.is_empty());
            assert!(
                THEMES[..i].iter().all(|prior| prior.id != theme.id),
                "duplicate theme id `{}`",
                theme.id
            );
        }
    }

    /// The boot script, `theme.js`, and this registry agree on the contract:
    /// one storage key, the default id meaning "no attribute", and a `dark`
    /// registry entry for the boot script's OS-preference fallback.
    #[test]
    fn boot_script_and_theme_js_share_the_contract() {
        assert!(THEME_BOOT_JS.contains("'bens-theme'"));
        assert!(THEME_BOOT_JS.contains("'oxide'"));
        assert!(THEME_JS.contains("\"bens-theme\""));
        assert!(THEME_JS.contains("\"bens-theme-music\""));
        assert!(THEME_JS.contains("\"oxide\""));
        // The media the shell hands theme.js through the ♪ row's data
        // attributes (chrome.rs must keep rendering both). theme.js must
        // select by ATTRIBUTE, never by "[data-music-toggle]" — several
        // elements carry that toggle hook and the first one in DOM order
        // (the corner pill) has no media attributes; that exact bug once
        // silenced clown mode and benched the felix chaser.
        assert!(THEME_JS.contains("querySelector(\"[data-clown-tune]\")"));
        assert!(THEME_JS.contains("querySelector(\"[data-felix-chaser]\")"));
        // Pages with their own music (/podrick's anthem) mark it with
        // data-page-band and the theme tune yields; keep the contract.
        assert!(THEME_JS.contains("data-page-band"));
        // Every registered theme has a signature selection sting.
        for theme in THEMES.iter() {
            assert!(
                THEME_JS.contains(&format!("{}:", theme.id)),
                "theme `{}` has no sting entry in theme.js",
                theme.id
            );
        }
        // Only the especially whimsical themes carry a continuous tune, and
        // themes.css must reveal the music toggle for exactly those.
        for theme in THEMES.iter() {
            let reveal = format!("[data-theme=\"{}\"] .theme-music", theme.id);
            assert_eq!(
                THEMES_CSS.contains(&reveal),
                theme.whimsical,
                "`{}`: whimsical flag and the .theme-music reveal in themes.css disagree",
                theme.id
            );
        }
        if THEMES.len() > 1 {
            assert!(
                THEMES.iter().any(|t| t.id == "dark"),
                "the boot script auto-selects `dark` for OS dark mode; keep a theme with that id"
            );
        }
    }
}
