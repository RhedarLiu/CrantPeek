//! Bundled fonts and the per-language family map.
//!
//! Every font ships with the app so macOS and Windows render identically, and
//! so the two platforms need no OS font assumptions.
//!
//! gpui's `Font` carries `family` / `features` / `fallbacks` / `weight` /
//! `style` and **no language or script field**, so a single Pan-CJK face cannot
//! be told to draw Japanese glyph forms through the OpenType `locl` feature.
//! Because Simplified Chinese and Japanese share codepoints with different
//! glyph shapes (Han unification — see `out/font-a-shadcn.png` in the spike),
//! the regional forms are selected by naming the family for each text run. The
//! caller already knows the language, so this stays deterministic.

use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::theme::Theme;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
const NOTO_SANS_SC: &[u8] = include_bytes!("../assets/fonts/NotoSansSC.ttf");
const NOTO_SANS_JP: &[u8] = include_bytes!("../assets/fonts/NotoSansJP.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono.ttf");
const NOTO_SERIF: &[u8] = include_bytes!("../assets/fonts/NotoSerif.ttf");

/// One complete, coherent font bundle selectable as a theme.
pub struct FontSet {
    /// Shown in settings.
    pub name: &'static str,
    /// Latin and UI chrome.
    pub latin: &'static str,
    /// Simplified Chinese.
    pub sc: &'static str,
    /// Japanese.
    pub jp: &'static str,
    /// Code, identifiers and error output.
    pub mono: &'static str,
    /// Whether the CJK faces are bundled rather than taken from the OS.
    pub cjk_bundled: bool,
    bytes: &'static [&'static [u8]],
}

impl FontSet {
    pub fn bytes(&self) -> Vec<Cow<'static, [u8]>> {
        self.bytes
            .iter()
            .map(|bytes| Cow::Borrowed(*bytes))
            .collect()
    }
}

/// The serif CJK faces are not bundled yet (Noto Serif SC could not be
/// downloaded intact), so that theme falls back to the OS. Kept behind `cfg`
/// so the names are at least correct per platform.
#[cfg(target_os = "macos")]
const SERIF_SC: &str = "Songti SC";
#[cfg(target_os = "macos")]
const SERIF_JP: &str = "Hiragino Mincho ProN";
#[cfg(windows)]
const SERIF_SC: &str = "SimSun";
#[cfg(windows)]
const SERIF_JP: &str = "Yu Mincho";

/// Default bundle: Inter for Latin, Noto Sans for CJK, JetBrains Mono for code.
pub const DEFAULT: FontSet = FontSet {
    name: "Inter + Noto Sans",
    latin: "Inter",
    sc: "Noto Sans SC",
    jp: "Noto Sans JP",
    mono: "JetBrains Mono",
    cjk_bundled: true,
    bytes: &[INTER, NOTO_SANS_SC, NOTO_SANS_JP, JETBRAINS_MONO],
};

/// Optional serif theme, for long-form reading. Latin and mono are bundled;
/// the CJK faces come from the OS for now.
pub const SERIF: FontSet = FontSet {
    name: "Noto Serif（衬线）",
    latin: "Noto Serif",
    sc: SERIF_SC,
    jp: SERIF_JP,
    mono: "JetBrains Mono",
    cjk_bundled: false,
    bytes: &[NOTO_SERIF, JETBRAINS_MONO],
};

pub const ALL: &[&FontSet] = &[&DEFAULT, &SERIF];

/// Registers the bundled faces. Must run before the first window is opened,
/// because GPUI measures text when a view is first laid out.
pub fn register(cx: &mut App) -> anyhow::Result<()> {
    // Union of every set, so switching themes at runtime needs no re-register.
    let mut fonts: Vec<Cow<'static, [u8]>> = Vec::new();
    for set in ALL {
        fonts.extend(set.bytes());
    }
    cx.text_system().add_fonts(fonts)?;
    Ok(())
}

/// Applies a set to the theme so UI text and code pick up the right families.
pub fn apply(cx: &mut App, set: &FontSet) {
    Theme::update(cx, |theme| {
        theme.font_family = set.latin.into();
        theme.mono_font_family = set.mono.into();
    });
}

/// Reports bundled families that failed to register, which is how a truncated
/// font file shows up (GPUI silently drops faces it cannot parse).
pub fn missing(cx: &App) -> Vec<&'static str> {
    let available: Vec<String> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    let mut missing = Vec::new();
    for set in ALL {
        if !set.cjk_bundled {
            continue;
        }
        for family in [set.latin, set.sc, set.jp, set.mono] {
            if !available.iter().any(|name| name == family) {
                missing.push(family);
            }
        }
    }
    missing.sort_unstable();
    missing.dedup();
    missing
}
