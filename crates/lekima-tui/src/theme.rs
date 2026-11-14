//! Visual language.
//!
//! The TUI deliberately avoids the boxed-widget look of the archived Go player.
//! It reads like a console: a single HUD line, a rule, a dense list, and a
//! command bar. Colour is an accent, not a decoration — dim text carries
//! structure, neon marks state.
//!
//! See `docs/sprint-2/tui-visual-design.md` for the rationale.

use ratatui::style::Color;

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
            selection_bg: Color::Rgb(0, 62, 52),
            selection_fg: Color::Rgb(210, 255, 240),
            panel: Color::Rgb(0, 122, 100),
            overlay: Color::Rgb(255, 46, 136),
            badge_bg: Color::Rgb(0, 255, 156),
            badge_fg: Color::Rgb(0, 0, 0),
            meter_full: Color::Rgb(0, 255, 156),
            meter_empty: Color::Rgb(42, 54, 62),
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
            selection_bg: Color::Rgb(66, 44, 0),
            selection_fg: Color::Rgb(255, 236, 190),
            panel: Color::Rgb(184, 128, 0),
            overlay: Color::Rgb(255, 90, 31),
            badge_bg: Color::Rgb(255, 176, 0),
            badge_fg: Color::Rgb(0, 0, 0),
            meter_full: Color::Rgb(255, 176, 0),
            meter_empty: Color::Rgb(52, 44, 32),
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
            selection_bg: Color::Reset,
            selection_fg: Color::White,
            panel: Color::Gray,
            overlay: Color::White,
            badge_bg: Color::White,
            badge_fg: Color::Black,
            meter_full: Color::White,
            meter_empty: Color::DarkGray,
        }
    }

    /// Resolves a configured theme name, honouring `NO_COLOR`.
    #[must_use]
    pub fn resolve(name: &str) -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::mono();
        }
        match name.trim().to_ascii_lowercase().as_str() {
            "amber" => Self::amber(),
            "mono" | "none" | "monochrome" => Self::mono(),
            _ => Self::cyber(),
        }
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
        assert_eq!(Theme::resolve("nope").accent, Theme::cyber().accent);
    }

    #[test]
    fn named_themes_resolve() {
        assert_eq!(Theme::resolve("amber").accent, Theme::amber().accent);
        assert!(Theme::resolve("mono").is_mono());
    }
}
