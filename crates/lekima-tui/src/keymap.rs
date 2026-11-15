//! Single source of truth for the help overlay.
//!
//! The reducer still matches on keys directly (it needs the control flow), but
//! every documented binding lives here so the help screen cannot silently drift
//! from the footer hints. Sections are assigned to a help column so the overlay
//! fits an 80×24 terminal without scrolling.

/// One documented key binding.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub key: &'static str,
    pub description: &'static str,
}

/// A titled group of bindings.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub title: &'static str,
    /// 0 = left column, 1 = right column on wide terminals.
    pub column: usize,
    pub bindings: &'static [Binding],
}

pub const MOVE: &[Binding] = &[
    Binding {
        key: "j / k",
        description: "down / up",
    },
    Binding {
        key: "5j / 12G",
        description: "count prefix",
    },
    Binding {
        key: "ctrl-d/u",
        description: "half page",
    },
    Binding {
        key: "g / G",
        description: "top / bottom",
    },
    Binding {
        key: "tab / c-w 1/2",
        description: "library / list",
    },
    Binding {
        key: "ctrl-o/i",
        description: "history back/forward",
    },
];

pub const PLAY: &[Binding] = &[
    Binding {
        key: "enter",
        description: "play selection",
    },
    Binding {
        key: "space",
        description: "pause / resume",
    },
    Binding {
        key: "n / p",
        description: "next / previous",
    },
    Binding {
        key: "h / l",
        description: "seek -10s / +10s",
    },
    Binding {
        key: "+ / -",
        description: "volume",
    },
    Binding {
        key: "m",
        description: "mute",
    },
    Binding {
        key: "r",
        description: "off -> all -> one",
    },
    Binding {
        key: "s",
        description: "shuffle",
    },
    Binding {
        key: "a / A",
        description: "queue / play next",
    },
];

pub const QUEUE: &[Binding] = &[
    Binding {
        key: "j / k",
        description: "nav (transport keys)",
    },
    Binding {
        key: "x",
        description: "remove (confirm)",
    },
    Binding {
        key: "J / K",
        description: "move entry down / up",
    },
    Binding {
        key: "enter",
        description: "jump to entry",
    },
    Binding {
        key: "esc",
        description: "close the queue",
    },
];

pub const DISCOVER: &[Binding] = &[
    Binding {
        key: "d",
        description: "daily mix",
    },
    Binding {
        key: "F",
        description: "personal FM",
    },
    Binding {
        key: "t",
        description: "charts / translation",
    },
    Binding {
        key: "c / C",
        description: "cloud / comments",
    },
    Binding {
        key: "f",
        description: "like / unlike",
    },
    Binding {
        key: "L / [ ] / c-w 4",
        description: "lyrics / offset",
    },
];

pub const APP: &[Binding] = &[
    Binding {
        key: "/",
        description: "search",
    },
    Binding {
        key: ":",
        description: "command line",
    },
    Binding {
        key: "Q / c-w 3",
        description: "queue",
    },
    Binding {
        key: "S / R",
        description: "settings / daemon",
    },
    Binding {
        key: "?",
        description: "this help",
    },
    Binding {
        key: "q / ctrl-c",
        description: "quit",
    },
];

pub const SECTIONS: &[Section] = &[
    Section {
        title: "MOVE",
        column: 0,
        bindings: MOVE,
    },
    Section {
        title: "PLAY",
        column: 0,
        bindings: PLAY,
    },
    Section {
        title: "QUEUE",
        column: 1,
        bindings: QUEUE,
    },
    Section {
        title: "DISCOVER",
        column: 1,
        bindings: DISCOVER,
    },
    Section {
        title: "APP",
        column: 1,
        bindings: APP,
    },
];

/// Number of content lines for one column, including headers.
#[must_use]
pub fn column_lines(column: usize) -> usize {
    SECTIONS
        .iter()
        .filter(|section| section.column == column)
        .map(|section| 1 + section.bindings.len())
        .sum()
}

/// Total content lines when rendered as a single column.
#[must_use]
pub fn single_column_lines() -> usize {
    column_lines(0) + column_lines(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_columns_fit_a_standard_help_overlay() {
        // 80×24 leaves 22 content rows inside the double border.
        assert!(column_lines(0) <= 22, "left column: {}", column_lines(0));
        assert!(column_lines(1) <= 22, "right column: {}", column_lines(1));
    }

    #[test]
    fn every_binding_has_a_key_and_description() {
        for section in SECTIONS {
            assert!(!section.title.is_empty());
            for binding in section.bindings {
                assert!(!binding.key.is_empty(), "{}", section.title);
                assert!(!binding.description.is_empty(), "{}", section.title);
            }
        }
    }
}
