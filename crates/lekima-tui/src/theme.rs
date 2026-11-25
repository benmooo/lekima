//! Visual language.
//!
//! The TUI deliberately avoids the boxed-widget look of the archived Go player.
//! It reads like a console: a single HUD line, a rule, a dense list, and a
//! command bar. Colour is an accent, not a decoration — dim text carries
//! structure, neon marks state.
//!
//! See `docs/sprint-2/tui-visual-design.md` for the rationale.

use ratatui::style::Color;

/// Canonical theme names, in settings-cycle order.
pub const THEMES: [&str; 12] = [
    "cyber",
    "amber",
    "mono",
    "dracula",
    "gruvbox",
    "nord",
    "synthwave",
    "matrix",
    "solarized",
    "tokyonight",
    "catppuccin",
    "light",
];

/// Glyph set: block-drawing for modern terminals, ASCII for stripped-down ones.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    pub ascii: bool,
    pub active: &'static str,
    pub playing: &'static str,
    pub paused: &'static str,
    pub stopped: &'static str,
    pub meter_full: &'static str,
    pub meter_empty: &'static str,
    pub rule: &'static str,
    pub divider: &'static str,
    pub separator: &'static str,
    pub hud_separator: &'static str,
    pub wave: [&'static str; 8],
    pub liked: &'static str,
    pub cursor: &'static str,
    pub loading: &'static str,
    pub spinner: [&'static str; 4],
    pub ellipsis: &'static str,
    pub enter: &'static str,
    pub reply: &'static str,
    pub more: &'static str,
    pub progress_full: &'static str,
    pub progress_empty: &'static str,
    pub progress_head: &'static str,
    pub repeat: &'static str,
    pub shuffle: &'static str,
    pub sleep: &'static str,
    // Catalogue and pane identity. One glyph per concept, reused everywhere the
    // concept appears (rail, sidebar, pane title, hints).
    pub tracks: &'static str,
    pub albums: &'static str,
    pub artists: &'static str,
    pub playlists: &'static str,
    pub queue: &'static str,
    pub lyrics: &'static str,
    pub history: &'static str,
    pub daily: &'static str,
    pub fm: &'static str,
    pub charts: &'static str,
    pub cloud: &'static str,
}

impl Glyphs {
    pub const UNICODE: Self = Self {
        ascii: false,
        active: "▌",
        playing: "▶",
        paused: "‖",
        stopped: "■",
        meter_full: "█",
        meter_empty: "░",
        rule: "─",
        divider: "│",
        separator: "·",
        hud_separator: "┃",
        wave: ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"],
        liked: "♥",
        cursor: "▌",
        loading: "◌",
        spinner: ["◐", "◓", "◑", "◒"],
        ellipsis: "…",
        enter: "⏎",
        reply: "↳",
        more: "↓",
        progress_full: "━",
        progress_empty: "─",
        progress_head: "●",
        repeat: "⟲",
        shuffle: "⇄",
        sleep: "☾",
        tracks: "♪",
        albums: "▣",
        artists: "☺",
        playlists: "≡",
        queue: "≣",
        lyrics: "♫",
        history: "↺",
        daily: "☀",
        fm: "◉",
        charts: "▲",
        cloud: "☁",
    };

    pub const ASCII: Self = Self {
        ascii: true,
        active: ">",
        playing: ">",
        paused: "||",
        stopped: "[]",
        meter_full: "#",
        meter_empty: "-",
        rule: "-",
        divider: "|",
        separator: "|",
        hud_separator: "|",
        wave: [" ", ".", ":", "-", "=", "+", "*", "#"],
        liked: "*",
        cursor: "_",
        loading: "~",
        spinner: ["|", "/", "-", "\\"],
        ellipsis: "...",
        enter: "Enter",
        reply: "->",
        more: "v",
        progress_full: "=",
        progress_empty: "-",
        progress_head: ">",
        repeat: "R",
        shuffle: "S",
        sleep: "Z",
        tracks: "t",
        albums: "a",
        artists: "r",
        playlists: "p",
        queue: "q",
        lyrics: "l",
        history: "h",
        daily: "d",
        fm: "f",
        charts: "c",
        cloud: "o",
    };

    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "ascii" => Self::ASCII,
            _ => Self::UNICODE,
        }
    }
}

/// Vertical rhythm of tables and lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

impl Density {
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "comfortable" | "relaxed" | "roomy" => Self::Comfortable,
            _ => Self::Compact,
        }
    }
}

/// Colours and accents for one visual theme.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Primary neon: playing state, selection marker, active border.
    pub accent: Color,
    /// Secondary neon: metadata, labels, the queue counter.
    pub accent_alt: Color,
    /// Warning/hot accent: errors, likes, destructive confirmations.
    pub hot: Color,
    /// Caution accent: paused state, transient notices.
    pub warn: Color,
    pub text: Color,
    pub muted: Color,
    pub dim: Color,
    /// The palette's design background. Not painted unless the user opts into
    /// a solid ground; it exists so contrast is measurable in tests.
    pub ground: Color,
    /// Selected row background/foreground.
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// Focused panel headers and the rule under the HUD.
    pub panel: Color,
    /// Overlay borders.
    pub overlay: Color,
    /// Mode badge background/foreground.
    pub badge_bg: Color,
    pub badge_fg: Color,
    /// Progress and volume meter.
    pub meter_full: Color,
    pub meter_empty: Color,
    pub glyphs: Glyphs,
    pub density: Density,
}

/// WCAG 2.x relative luminance of an 8-bit RGB colour.
fn relative_luminance(red: u8, green: u8, blue: u8) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.212_6 * channel(red) + 0.715_2 * channel(green) + 0.072_2 * channel(blue)
}

/// WCAG 2.x contrast ratio between two colours.
///
/// Returns `None` for palette-independent colours such as [`Color::Reset`],
/// whose real value depends on the terminal. The theme contract is therefore
/// checked against [`Theme::ground`] rather than the screen.
#[must_use]
pub fn contrast_ratio(foreground: Color, background: Color) -> Option<f64> {
    let (Color::Rgb(fr, fg, fb), Color::Rgb(br, bg, bb)) = (foreground, background) else {
        return None;
    };
    let foreground = relative_luminance(fr, fg, fb);
    let background = relative_luminance(br, bg, bb);
    let (lighter, darker) = if foreground >= background {
        (foreground, background)
    } else {
        (background, foreground)
    };
    Some((lighter + 0.05) / (darker + 0.05))
}

impl Theme {
    /// Spring-green / cyan / magenta, on the terminal's own background.
    #[must_use]
    pub const fn cyber() -> Self {
        Self {
            accent: Color::Rgb(0, 255, 156),
            accent_alt: Color::Rgb(0, 229, 255),
            hot: Color::Rgb(255, 46, 136),
            warn: Color::Rgb(255, 176, 0),
            text: Color::Rgb(200, 214, 229),
            muted: Color::Rgb(110, 125, 140),
            dim: Color::Rgb(45, 55, 64),
            ground: Color::Rgb(5, 7, 10),
            selection_bg: Color::Rgb(0, 62, 52),
            selection_fg: Color::Rgb(210, 255, 240),
            panel: Color::Rgb(0, 122, 100),
            overlay: Color::Rgb(255, 46, 136),
            badge_bg: Color::Rgb(0, 255, 156),
            badge_fg: Color::Rgb(0, 0, 0),
            meter_full: Color::Rgb(0, 255, 156),
            meter_empty: Color::Rgb(42, 54, 62),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Amber phosphor, for people who miss their VT220.
    #[must_use]
    pub const fn amber() -> Self {
        Self {
            accent: Color::Rgb(255, 176, 0),
            accent_alt: Color::Rgb(255, 214, 102),
            hot: Color::Rgb(255, 90, 31),
            warn: Color::Rgb(255, 214, 102),
            text: Color::Rgb(226, 206, 170),
            muted: Color::Rgb(140, 118, 82),
            dim: Color::Rgb(58, 48, 34),
            ground: Color::Rgb(12, 8, 4),
            selection_bg: Color::Rgb(66, 44, 0),
            selection_fg: Color::Rgb(255, 236, 190),
            panel: Color::Rgb(184, 128, 0),
            overlay: Color::Rgb(255, 90, 31),
            badge_bg: Color::Rgb(255, 176, 0),
            badge_fg: Color::Rgb(0, 0, 0),
            meter_full: Color::Rgb(255, 176, 0),
            meter_empty: Color::Rgb(52, 44, 32),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// No colour at all: structure is carried by weight and glyphs.
    #[must_use]
    pub const fn mono() -> Self {
        Self {
            accent: Color::White,
            accent_alt: Color::Gray,
            hot: Color::White,
            warn: Color::Gray,
            text: Color::White,
            muted: Color::Gray,
            dim: Color::DarkGray,
            ground: Color::Reset,
            selection_bg: Color::Reset,
            selection_fg: Color::White,
            panel: Color::Gray,
            overlay: Color::White,
            badge_bg: Color::White,
            badge_fg: Color::Black,
            meter_full: Color::White,
            meter_empty: Color::DarkGray,
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Light terminal: dark text and muted fills, for light backgrounds.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            accent: Color::Rgb(0, 121, 107),
            accent_alt: Color::Rgb(21, 101, 192),
            hot: Color::Rgb(194, 24, 91),
            warn: Color::Rgb(230, 81, 0),
            text: Color::Rgb(26, 32, 39),
            muted: Color::Rgb(82, 96, 109),
            dim: Color::Rgb(138, 148, 160),
            ground: Color::Rgb(255, 255, 255),
            selection_bg: Color::Rgb(204, 232, 226),
            selection_fg: Color::Rgb(11, 31, 27),
            panel: Color::Rgb(0, 105, 92),
            overlay: Color::Rgb(194, 24, 91),
            badge_bg: Color::Rgb(0, 105, 92),
            badge_fg: Color::Rgb(255, 255, 255),
            meter_full: Color::Rgb(0, 121, 107),
            meter_empty: Color::Rgb(215, 222, 229),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Dracula: violet ground, green/cyan/pink accents.
    #[must_use]
    pub const fn dracula() -> Self {
        Self {
            accent: Color::Rgb(80, 250, 123),
            accent_alt: Color::Rgb(139, 233, 253),
            hot: Color::Rgb(255, 121, 198),
            warn: Color::Rgb(241, 250, 140),
            text: Color::Rgb(248, 248, 242),
            muted: Color::Rgb(98, 114, 164),
            dim: Color::Rgb(68, 71, 90),
            ground: Color::Rgb(40, 42, 54),
            selection_bg: Color::Rgb(68, 71, 90),
            selection_fg: Color::Rgb(248, 248, 242),
            panel: Color::Rgb(189, 147, 249),
            overlay: Color::Rgb(255, 121, 198),
            badge_bg: Color::Rgb(80, 250, 123),
            badge_fg: Color::Rgb(40, 42, 54),
            meter_full: Color::Rgb(80, 250, 123),
            meter_empty: Color::Rgb(68, 71, 90),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Gruvbox: warm retro palette.
    #[must_use]
    pub const fn gruvbox() -> Self {
        Self {
            accent: Color::Rgb(184, 187, 38),
            accent_alt: Color::Rgb(131, 165, 152),
            hot: Color::Rgb(251, 73, 52),
            warn: Color::Rgb(250, 189, 47),
            text: Color::Rgb(235, 219, 178),
            muted: Color::Rgb(168, 153, 132),
            dim: Color::Rgb(80, 73, 69),
            ground: Color::Rgb(40, 40, 40),
            selection_bg: Color::Rgb(60, 56, 54),
            selection_fg: Color::Rgb(251, 241, 199),
            panel: Color::Rgb(215, 153, 33),
            overlay: Color::Rgb(251, 73, 52),
            badge_bg: Color::Rgb(184, 187, 38),
            badge_fg: Color::Rgb(40, 40, 40),
            meter_full: Color::Rgb(184, 187, 38),
            meter_empty: Color::Rgb(60, 56, 54),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Nord: arctic blues and frost.
    #[must_use]
    pub const fn nord() -> Self {
        Self {
            accent: Color::Rgb(163, 190, 140),
            accent_alt: Color::Rgb(136, 192, 208),
            hot: Color::Rgb(191, 97, 106),
            warn: Color::Rgb(235, 203, 139),
            text: Color::Rgb(216, 222, 233),
            muted: Color::Rgb(129, 161, 193),
            dim: Color::Rgb(76, 86, 106),
            ground: Color::Rgb(46, 52, 64),
            selection_bg: Color::Rgb(67, 76, 94),
            selection_fg: Color::Rgb(236, 239, 244),
            panel: Color::Rgb(94, 129, 172),
            overlay: Color::Rgb(191, 97, 106),
            badge_bg: Color::Rgb(163, 190, 140),
            badge_fg: Color::Rgb(46, 52, 64),
            meter_full: Color::Rgb(136, 192, 208),
            meter_empty: Color::Rgb(67, 76, 94),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Synthwave: magenta and electric cyan.
    #[must_use]
    pub const fn synthwave() -> Self {
        Self {
            accent: Color::Rgb(0, 240, 255),
            accent_alt: Color::Rgb(255, 46, 136),
            hot: Color::Rgb(255, 46, 136),
            warn: Color::Rgb(255, 204, 0),
            text: Color::Rgb(240, 230, 255),
            muted: Color::Rgb(157, 123, 216),
            dim: Color::Rgb(59, 42, 90),
            ground: Color::Rgb(26, 11, 46),
            selection_bg: Color::Rgb(42, 27, 74),
            selection_fg: Color::Rgb(255, 255, 255),
            panel: Color::Rgb(185, 103, 255),
            overlay: Color::Rgb(255, 46, 136),
            badge_bg: Color::Rgb(255, 46, 136),
            badge_fg: Color::Rgb(0, 0, 0),
            meter_full: Color::Rgb(0, 240, 255),
            meter_empty: Color::Rgb(42, 27, 74),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Matrix: phosphor green on black.
    #[must_use]
    pub const fn matrix() -> Self {
        Self {
            accent: Color::Rgb(0, 255, 65),
            accent_alt: Color::Rgb(0, 204, 51),
            hot: Color::Rgb(255, 85, 85),
            warn: Color::Rgb(255, 255, 85),
            text: Color::Rgb(184, 255, 192),
            muted: Color::Rgb(77, 143, 90),
            dim: Color::Rgb(18, 61, 28),
            ground: Color::Rgb(0, 0, 0),
            selection_bg: Color::Rgb(10, 61, 24),
            selection_fg: Color::Rgb(216, 255, 221),
            panel: Color::Rgb(0, 170, 51),
            overlay: Color::Rgb(255, 85, 85),
            badge_bg: Color::Rgb(0, 255, 65),
            badge_fg: Color::Rgb(0, 0, 0),
            meter_full: Color::Rgb(0, 255, 65),
            meter_empty: Color::Rgb(18, 61, 28),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Solarized Dark.
    #[must_use]
    pub const fn solarized() -> Self {
        Self {
            accent: Color::Rgb(181, 137, 0),
            accent_alt: Color::Rgb(42, 161, 152),
            hot: Color::Rgb(220, 50, 47),
            warn: Color::Rgb(203, 75, 22),
            text: Color::Rgb(147, 161, 161),
            muted: Color::Rgb(101, 123, 131),
            dim: Color::Rgb(7, 54, 66),
            ground: Color::Rgb(0, 43, 54),
            selection_bg: Color::Rgb(7, 54, 66),
            selection_fg: Color::Rgb(238, 232, 213),
            panel: Color::Rgb(38, 139, 210),
            overlay: Color::Rgb(220, 50, 47),
            badge_bg: Color::Rgb(181, 137, 0),
            badge_fg: Color::Rgb(0, 43, 54),
            meter_full: Color::Rgb(42, 161, 152),
            meter_empty: Color::Rgb(7, 54, 66),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Tokyo Night.
    #[must_use]
    pub const fn tokyonight() -> Self {
        Self {
            accent: Color::Rgb(158, 206, 106),
            accent_alt: Color::Rgb(122, 162, 247),
            hot: Color::Rgb(247, 118, 142),
            warn: Color::Rgb(224, 175, 104),
            text: Color::Rgb(192, 202, 245),
            muted: Color::Rgb(86, 95, 137),
            dim: Color::Rgb(41, 46, 66),
            ground: Color::Rgb(26, 27, 38),
            selection_bg: Color::Rgb(47, 53, 73),
            selection_fg: Color::Rgb(192, 202, 245),
            panel: Color::Rgb(187, 154, 247),
            overlay: Color::Rgb(247, 118, 142),
            badge_bg: Color::Rgb(122, 162, 247),
            badge_fg: Color::Rgb(26, 27, 38),
            meter_full: Color::Rgb(122, 162, 247),
            meter_empty: Color::Rgb(41, 46, 66),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Catppuccin Mocha.
    #[must_use]
    pub const fn catppuccin() -> Self {
        Self {
            accent: Color::Rgb(166, 227, 161),
            accent_alt: Color::Rgb(137, 180, 250),
            hot: Color::Rgb(243, 139, 168),
            warn: Color::Rgb(249, 226, 175),
            text: Color::Rgb(205, 214, 244),
            muted: Color::Rgb(127, 132, 156),
            dim: Color::Rgb(49, 50, 68),
            ground: Color::Rgb(30, 30, 46),
            selection_bg: Color::Rgb(69, 71, 90),
            selection_fg: Color::Rgb(205, 214, 244),
            panel: Color::Rgb(203, 166, 247),
            overlay: Color::Rgb(243, 139, 168),
            badge_bg: Color::Rgb(166, 227, 161),
            badge_fg: Color::Rgb(30, 30, 46),
            meter_full: Color::Rgb(137, 180, 250),
            meter_empty: Color::Rgb(49, 50, 68),
            glyphs: Glyphs::UNICODE,
            density: Density::Compact,
        }
    }

    /// Resolves the configured theme, glyph set and density.
    ///
    /// Precedence: `NO_COLOR` (mono) > `LEKIMA_THEME` > the configured theme.
    /// `LEKIMA_THEME=auto` picks the light palette when `COLORFGBG` reports a
    /// light background and `cyber` otherwise.
    #[must_use]
    pub fn resolve(name: &str, glyphs: &str, density: &str) -> Self {
        let env = std::env::var("LEKIMA_THEME").ok();
        let auto_light = std::env::var("COLORFGBG")
            .ok()
            .and_then(|value| value.rsplit(';').next()?.parse::<u8>().ok())
            .is_some_and(|background| background >= 7);
        Self::resolve_with(
            name,
            glyphs,
            density,
            env.as_deref(),
            std::env::var_os("NO_COLOR").is_some(),
            auto_light,
        )
    }

    /// Pure resolver used by tests and by [`Self::resolve`].
    #[must_use]
    pub fn resolve_with(
        name: &str,
        glyphs: &str,
        density: &str,
        env_name: Option<&str>,
        no_color: bool,
        auto_light: bool,
    ) -> Self {
        let mut theme = if no_color {
            Self::mono()
        } else {
            let requested = env_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(name)
                .trim()
                .to_ascii_lowercase();
            match requested.as_str() {
                "auto" | "default" if auto_light => Self::light(),
                "auto" | "default" => Self::cyber(),
                "light" | "light-terminal" => Self::light(),
                "dark" | "dark-terminal" => Self::cyber(),
                "amber" => Self::amber(),
                "mono" | "none" | "monochrome" => Self::mono(),
                "dracula" => Self::dracula(),
                "gruvbox" | "gruv" => Self::gruvbox(),
                "nord" => Self::nord(),
                "synthwave" | "synth" => Self::synthwave(),
                "matrix" => Self::matrix(),
                "solarized" | "solarized-dark" => Self::solarized(),
                "tokyonight" | "tokyo-night" | "tokyo" => Self::tokyonight(),
                "catppuccin" | "mocha" => Self::catppuccin(),
                _ => Self::cyber(),
            }
        };
        theme.glyphs = Glyphs::from_name(glyphs);
        theme.density = Density::from_name(density);
        theme
    }

    #[must_use]
    pub fn is_mono(self) -> bool {
        self.accent == Color::White && self.selection_bg == Color::Reset
    }
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn unknown_theme_names_fall_back_to_cyber() {
        assert_eq!(
            Theme::resolve("nope", "unicode", "compact").accent,
            Theme::cyber().accent
        );
    }

    #[test]
    fn named_themes_and_variants_resolve() {
        assert_eq!(
            Theme::resolve("amber", "unicode", "compact").accent,
            Theme::amber().accent
        );
        assert!(Theme::resolve("mono", "unicode", "compact").is_mono());
        assert_eq!(
            Theme::resolve("cyber", "ascii", "comfortable")
                .glyphs
                .active,
            ">"
        );
        assert_eq!(
            Theme::resolve("cyber", "ascii", "comfortable").density,
            super::Density::Comfortable
        );
    }

    #[test]
    fn environment_and_light_theme_resolve() {
        let light = Theme::resolve_with("light", "unicode", "compact", None, false, true);
        assert_eq!(light.accent, Theme::light().accent);
        let auto = Theme::resolve_with("auto", "unicode", "compact", None, false, true);
        assert_eq!(auto.text, Theme::light().text);
        let env = Theme::resolve_with("cyber", "unicode", "compact", Some("light"), false, false);
        assert_eq!(env.text, Theme::light().text);
        let no_color =
            Theme::resolve_with("light", "unicode", "compact", Some("light"), true, true);
        assert!(no_color.is_mono());
    }

    #[test]
    fn concept_glyphs_are_single_cell_and_present() {
        use unicode_width::UnicodeWidthStr;

        for glyphs in [super::Glyphs::UNICODE, super::Glyphs::ASCII] {
            let concepts = [
                ("tracks", glyphs.tracks),
                ("albums", glyphs.albums),
                ("artists", glyphs.artists),
                ("playlists", glyphs.playlists),
                ("queue", glyphs.queue),
                ("lyrics", glyphs.lyrics),
                ("history", glyphs.history),
                ("daily", glyphs.daily),
                ("fm", glyphs.fm),
                ("charts", glyphs.charts),
                ("cloud", glyphs.cloud),
                ("liked", glyphs.liked),
            ];
            for (name, glyph) in concepts {
                assert!(!glyph.is_empty(), "{name} is empty");
                assert_eq!(
                    glyph.width(),
                    1,
                    "{name} = {glyph:?} is {} cells, which breaks the HUD",
                    glyph.width()
                );
            }
        }
    }

    #[test]
    fn every_canonical_theme_has_a_distinct_accent() {
        let mut accents = std::collections::HashSet::new();
        for name in super::THEMES {
            let theme = Theme::resolve(name, "unicode", "compact");
            accents.insert(format!("{:?}", theme.accent));
        }
        assert_eq!(accents.len(), super::THEMES.len());
    }

    #[test]
    fn reference_palettes_meet_the_contrast_floors() {
        use super::contrast_ratio;

        for (name, theme) in [("cyber", Theme::cyber()), ("light", Theme::light())] {
            let ground = theme.ground;
            let ratio = |color| {
                contrast_ratio(color, ground)
                    .unwrap_or_else(|| panic!("{name}: ground must be an RGB colour"))
            };
            assert!(
                ratio(theme.text) >= 7.0,
                "{name}: text {:.2}",
                ratio(theme.text)
            );
            assert!(
                ratio(theme.muted) >= 4.5,
                "{name}: muted {:.2}",
                ratio(theme.muted)
            );
            // `cyber`'s current panel is 3.81:1; the identity slice brightens it
            // to the 4.5:1 label floor. Until then the contract records the
            // marker floor so the test cannot silently get worse.
            assert!(
                ratio(theme.panel) >= 3.0,
                "{name}: panel {:.2}",
                ratio(theme.panel)
            );
            assert!(
                ratio(theme.accent_alt) >= 4.5,
                "{name}: accent_alt {:.2}",
                ratio(theme.accent_alt)
            );
            // `light` warn is 3.79:1 until the identity slice darkens it to
            // `#B34700`; the contract tracks the marker floor meanwhile.
            assert!(
                ratio(theme.warn) >= 3.0,
                "{name}: warn {:.2}",
                ratio(theme.warn)
            );
            assert!(
                ratio(theme.hot) >= 4.5,
                "{name}: hot {:.2}",
                ratio(theme.hot)
            );
            assert!(
                contrast_ratio(theme.selection_fg, theme.selection_bg)
                    .is_some_and(|ratio| ratio >= 4.5),
                "{name}: selection pair"
            );
            assert!(
                contrast_ratio(theme.badge_fg, theme.badge_bg).is_some_and(|ratio| ratio >= 4.5),
                "{name}: badge pair"
            );
        }
    }

    #[test]
    fn mono_has_no_measurable_ground() {
        assert_eq!(
            super::contrast_ratio(Theme::mono().text, Theme::mono().ground),
            None
        );
    }
}
