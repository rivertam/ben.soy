//! Choices in the shell's appearance menu. The tmux session is the site
//! itself: it is first, is server-rendered, and is represented by the absence
//! of `data-theme` on `<html>`. Every other entry is an explicit finish laid
//! over that default shell.
//!
//! The tmux palette has a `[data-theme="tmux"]` preview block only so its menu
//! swatch can resolve the right tokens. Its layout and keyboard behavior live
//! in `styles/session.css` and `components/browser/session*.js`; those
//! session behaviors remain core even though the palette uses the same
//! package interface as every alternate appearance.

use topcoat::asset::{Asset, asset};

pub const DEFAULT_THEME_ID: &str = "tmux";
pub const THEME_STORAGE_KEY: &str = "bens-theme";

pub struct Theme {
    pub id: &'static str,
    pub label: &'static str,
    /// Fingerprinted entry point implementing the browser package interface.
    pub module: Asset,
    /// Optional named assets supplied to that entry point. A synthesized tune
    /// has no `music_asset`; the package's `music` export owns tune presence.
    pub music_asset: Option<Asset>,
    pub image_asset: Option<Asset>,
    /// Especially whimsical appearances get the menu marker and carry music.
    pub whimsical: bool,
}

/// The default session leads, followed by increasingly optional finishes.
pub static THEMES: [Theme; 5] = [
    Theme {
        id: DEFAULT_THEME_ID,
        label: "tmux",
        module: asset!("../components/browser/themes/tmux/index.js"),
        music_asset: None,
        image_asset: None,
        whimsical: false,
    },
    Theme {
        id: "oxide",
        label: "mill & oxide",
        module: asset!("../components/browser/themes/oxide/index.js"),
        music_asset: None,
        image_asset: None,
        whimsical: false,
    },
    Theme {
        id: "dark",
        label: "night shift",
        module: asset!("../components/browser/themes/dark/index.js"),
        music_asset: None,
        image_asset: None,
        whimsical: false,
    },
    Theme {
        id: "felix",
        label: "felix mode",
        module: asset!("../components/browser/themes/felix/index.js"),
        music_asset: None,
        image_asset: Some(asset!("../components/felix-chaser.webp")),
        whimsical: true,
    },
    Theme {
        id: "clown",
        label: "clown mode",
        module: asset!("../components/browser/themes/clown/index.js"),
        music_asset: Some(asset!("../components/circus.mp3")),
        image_asset: None,
        whimsical: true,
    },
];

/// Runs before CSS so a remembered alternate finish never flashes the default
/// session. Rust derives the allowlist from `THEMES`; the browser no longer
/// carries a second hand-maintained registry. A stored `tmux` value from older
/// builds is accepted and canonicalized to the attribute-less default.
///
/// Kept dependency-free and em-dash-free; `emdash.rs` skips `<script>` only
/// inside `<main>`, while this trusted tag lives in `<head>`.
pub fn boot_script() -> String {
    let allowed = THEMES
        .iter()
        .map(|theme| format!("'{}'", theme.id))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "(function(){{try{{var r=document.documentElement,t=localStorage.getItem('{THEME_STORAGE_KEY}'),a=[{allowed}];\
if(t&&!a.includes(t)){{localStorage.removeItem('{THEME_STORAGE_KEY}');t=null}}\
if(!t||t==='{DEFAULT_THEME_ID}')delete r.dataset.theme;else r.dataset.theme=t}}catch(e){{}}}})()"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES_CSS: &str = include_str!("../../styles/themes.css");
    const SESSION_CSS: &str = include_str!("../../styles/session.css");
    const ALL_CSS: &str = concat!(
        include_str!("../components/browser/themes/tmux/theme.css"),
        include_str!("../components/browser/themes/oxide/theme.css"),
        include_str!("../components/browser/themes/dark/theme.css"),
        include_str!("../components/browser/themes/felix/theme.css"),
        include_str!("../components/browser/themes/clown/theme.css"),
    );
    const PACKAGES: [(&str, &str, &str); 5] = [
        (
            "tmux",
            include_str!("../components/browser/themes/tmux/theme.css"),
            include_str!("../components/browser/themes/tmux/index.js"),
        ),
        (
            "oxide",
            include_str!("../components/browser/themes/oxide/theme.css"),
            include_str!("../components/browser/themes/oxide/index.js"),
        ),
        (
            "dark",
            include_str!("../components/browser/themes/dark/theme.css"),
            include_str!("../components/browser/themes/dark/index.js"),
        ),
        (
            "felix",
            include_str!("../components/browser/themes/felix/theme.css"),
            include_str!("../components/browser/themes/felix/index.js"),
        ),
        (
            "clown",
            include_str!("../components/browser/themes/clown/theme.css"),
            include_str!("../components/browser/themes/clown/index.js"),
        ),
    ];
    const APPEARANCE_JS: &str = include_str!("../components/browser/appearance.js");
    const NAVIGATION_JS: &str = include_str!("../components/browser/navigation.js");
    const SESSION_JS: &str = include_str!("../components/browser/session.js");
    const HINTS_JS: &str = include_str!("../components/browser/navigation/hints.js");
    const VIMIUM_JS: &str = include_str!("../components/browser/session/vimium.js");
    const CHROME_RS: &str = include_str!("../components/chrome.rs");
    const RAIL_RS: &str = include_str!("../components/rail.rs");
    const HOME_RS: &str = include_str!("../app/home.rs");
    const LOG_RS: &str = include_str!("../app/log.rs");
    const SITE_CSS: &str = include_str!("../../styles/site.css");

    /// Every `[data-theme="…"]` selector target in package styles, ignoring
    /// comments (their prose mentions selector syntax too).
    fn css_theme_ids() -> Vec<String> {
        let mut css = String::new();
        let mut rest = ALL_CSS;
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
    fn every_registered_choice_has_one_homogeneous_package() {
        assert_eq!(THEMES.len(), PACKAGES.len());
        for (theme, (package_id, css, js)) in THEMES.iter().zip(PACKAGES) {
            assert_eq!(theme.id, package_id, "registry and package order diverged");
            let block = format!("[data-theme=\"{}\"] {{", theme.id);
            assert!(
                css.contains(&block),
                "theme `{}` is registered but its package has no `{block}` token block",
                theme.id
            );
            assert!(THEMES_CSS.contains(&format!("themes/{}/theme.css", theme.id)));
            assert!(js.contains(&format!("export const id = \"{}\";", theme.id)));
            for export in ["tone", "activate", "deactivate", "selected"] {
                assert!(
                    js.contains(&format!("export function {export}")),
                    "theme `{}` does not export {export}()",
                    theme.id
                );
            }
            assert!(js.contains("export const colorScheme"));
            assert!(js.contains("export const music"));
        }
    }

    #[test]
    fn every_css_choice_is_registered() {
        for css_id in css_theme_ids() {
            assert!(
                THEMES.iter().any(|theme| theme.id == css_id),
                "package CSS styles [data-theme=\"{css_id}\"] but the registry doesn't list it"
            );
        }
    }

    #[test]
    fn package_metadata_matches_css_and_rust_assets() {
        for (theme, (_, css, js)) in THEMES.iter().zip(PACKAGES) {
            let color_scheme = if js.contains("colorScheme = \"dark\"") {
                "dark"
            } else if js.contains("colorScheme = \"light\"") {
                "light"
            } else {
                panic!("theme `{}` has no valid color scheme", theme.id);
            };
            assert!(
                css.contains(&format!("color-scheme: {color_scheme};")),
                "theme `{}` exports a color scheme that disagrees with CSS",
                theme.id
            );

            let carries_music = !js.contains("export const music = null;");
            assert_eq!(
                carries_music, theme.whimsical,
                "theme `{}`: music export and whimsical marker disagree",
                theme.id
            );
            assert_eq!(
                js.contains("kind: \"audio\""),
                theme.music_asset.is_some(),
                "theme `{}`: audio descriptor and Rust music asset disagree",
                theme.id
            );
            assert_eq!(
                js.contains("assets.image"),
                theme.image_asset.is_some(),
                "theme `{}`: image use and Rust image asset disagree",
                theme.id
            );
            let reveal = format!("[data-theme=\"{}\"] .theme-music", theme.id);
            assert_eq!(
                css.contains(&reveal),
                carries_music,
                "theme `{}`: music export and toggle CSS disagree",
                theme.id
            );
        }
    }

    #[test]
    fn ids_and_modules_are_unique_and_the_default_session_leads() {
        assert_eq!(THEMES[0].id, DEFAULT_THEME_ID);
        for (i, theme) in THEMES.iter().enumerate() {
            assert!(
                theme
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "theme id `{}` isn't kebab-case",
                theme.id
            );
            assert!(!theme.label.is_empty());
            assert!(
                THEMES[..i].iter().all(|prior| prior.id != theme.id),
                "duplicate theme id `{}`",
                theme.id
            );
            assert!(
                THEMES[..i]
                    .iter()
                    .all(|prior| prior.module.id() != theme.module.id()),
                "duplicate theme module for `{}`",
                theme.id
            );
        }
    }

    #[test]
    fn rust_owns_registration_and_the_runtime_is_theme_agnostic() {
        let boot = boot_script();
        assert!(boot.contains(THEME_STORAGE_KEY));
        for theme in THEMES.iter() {
            assert!(boot.contains(&format!("'{}'", theme.id)));
        }
        assert!(boot.contains("delete r.dataset.theme"));
        assert!(CHROME_RS.contains("<html lang=\"en\">"));
        assert!(!CHROME_RS.contains("<html lang=\"en\" data-theme"));
        for hook in [
            "data-default-theme=(themes::DEFAULT_THEME_ID)",
            "data-theme-key=(themes::THEME_STORAGE_KEY)",
            "data-theme-module=(theme.module)",
            "data-theme-music=(theme.music_asset)",
            "data-theme-image=(theme.image_asset)",
        ] {
            assert!(CHROME_RS.contains(hook), "chrome lost {hook}");
        }
        assert!(APPEARANCE_JS.contains("config.dataset.defaultTheme"));
        assert!(APPEARANCE_JS.contains("config.dataset.themeKey"));
        assert!(APPEARANCE_JS.contains("import(registration.module)"));
        assert!(!APPEARANCE_JS.contains("clownModule"));
        assert!(!APPEARANCE_JS.contains("felixModule"));
        assert!(SESSION_CSS.contains(":root:not([data-theme]) .tmux-bar"));
    }

    #[test]
    fn site_navigation_and_session_keep_distinct_interfaces() {
        for hook in [
            "data-navigation-runtime",
            "data-hints-module",
            "data-session-runtime",
            "data-vimium-module",
        ] {
            assert!(CHROME_RS.contains(hook), "chrome lost {hook}");
        }
        assert!(NAVIGATION_JS.contains("import(config.dataset.hintsModule)"));
        assert!(SESSION_JS.contains("import(config.dataset.vimiumModule)"));
        assert!(HINTS_JS.contains("export function createHints"));
        assert!(VIMIUM_JS.contains("export function createVimiumNotice"));
        assert!(NAVIGATION_JS.contains(".rail-row, [data-rail-item]"));
        assert!(NAVIGATION_JS.contains("currentRail.dataset.railHref"));
        assert!(NAVIGATION_JS.contains("[data-rail-enter]"));
        for key in ["j", "k", "f", "Enter"] {
            assert!(
                NAVIGATION_JS.contains(&format!("case \"{key}\"")),
                "site navigation lost {key}"
            );
        }
        assert!(!NAVIGATION_JS.contains("dataset.theme"));
        assert!(SESSION_JS.contains("site:navigationkey"));

        // Rail rows opt in intrinsically; their mount points only provide an
        // Enter action when one exists.
        assert!(RAIL_RS.contains("enter_href"));
        assert!(RAIL_RS.contains("data-rail-href"));
        assert!(HOME_RS.contains("data-rail-item"));
        assert!(HOME_RS.contains("data-rail-href"));
        assert!(LOG_RS.contains("data-rail-item"));
        assert!(LOG_RS.contains("data-rail-enter"));
        for selector in ["[data-rail-current] {", ".key-hints {", ".key-hint {"] {
            assert!(SITE_CSS.contains(selector), "site CSS lost {selector}");
            assert!(
                !SESSION_CSS.contains(selector),
                "site-wide selector {selector} leaked back into session CSS"
            );
        }

        // A page band still takes precedence over package-specific music.
        assert!(APPEARANCE_JS.contains("data-page-band"));

        for hook in [
            "data-session-bar",
            "data-session-window",
            "data-session-clock",
            "data-session-title",
            "data-session-message",
        ] {
            assert!(CHROME_RS.contains(hook), "chrome lost {hook}");
        }
        assert!(SESSION_JS.contains("querySelector(\"[data-session-bar]\")"));
        assert!(SESSION_JS.contains("querySelectorAll(\"[data-session-window]\")"));
        assert!(SESSION_JS.contains("querySelector(\"[data-session-clock]\")"));
        assert!(CHROME_RS.contains("tmux-note"));
        assert!(SESSION_CSS.contains(".tmux-note"));
        assert!(CHROME_RS.contains("tmux-login"));
        assert!(HOME_RS.contains("netrw-login"));
        assert!(HOME_RS.contains("netrw-logout"));
        assert!(SESSION_CSS.contains(".tmux-login"));
        assert!(SITE_CSS.contains(".netrw-login"));
    }
}
