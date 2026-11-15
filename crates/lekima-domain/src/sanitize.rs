//! Terminal text hygiene.
//!
//! Provider metadata is untrusted: track titles, playlist names and lyrics can
//! contain control characters, escape sequences or bidi overrides. Frontends
//! must never write those bytes to a terminal. This module is the shared helper
//! required by `AGENTS.md`; adapters sanitize on the way in and frontends may
//! sanitize again before display.
//!
//! The function is deliberately lossy: control characters become spaces (then
//! trimmed), escape sequences disappear, and bidi/zero-width controls are
//! dropped. Normal text, including CJK, combining marks and emoji, is preserved.

/// Sanitizes a single line for terminal display.
#[must_use]
pub fn sanitize_line(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut characters = input.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '\u{1b}' => skip_escape(&mut characters),
            '\u{2028}' | '\u{2029}' => output.push(' '),
            character if character.is_control() => output.push(' '),
            '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{061c}' => {}
            '\u{200b}'..='\u{200d}' | '\u{feff}' | '\u{2060}' => {}
            character => output.push(character),
        }
    }

    output.trim().to_owned()
}

/// Sanitizes every line while preserving the line structure.
#[must_use]
pub fn sanitize_multiline(input: &str) -> String {
    input
        .lines()
        .map(sanitize_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn skip_escape(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match characters.next() {
        // CSI: parameters/intermediates until a final byte.
        Some('[') => {
            for character in characters.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&character) {
                    break;
                }
            }
        }
        // OSC, DCS, SOS, PM and APC: skip until BEL or ST, whichever comes first.
        Some(']' | 'P' | 'X' | '^' | '_') => {
            while let Some(character) = characters.next() {
                if character == '\u{7}' {
                    break;
                }
                if character == '\u{1b}' && characters.peek() == Some(&'\\') {
                    characters.next();
                    break;
                }
            }
        }
        // Two-byte escapes and a trailing lone ESC.
        Some(_) | None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_normal_text_alone() {
        assert_eq!(
            sanitize_line("  Mota · Russian Circles  "),
            "Mota · Russian Circles"
        );
        assert_eq!(sanitize_line("夜に駆ける"), "夜に駆ける");
        assert_eq!(sanitize_line("café"), "café");
    }

    #[test]
    fn strips_csi_and_osc_sequences() {
        assert_eq!(sanitize_line("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(sanitize_line("\u{1b}]0;owned\u{7}hello"), "hello");
        assert_eq!(sanitize_line("\u{1b}]8;;https://x\u{1b}\\link"), "link");
    }

    #[test]
    fn replaces_controls_and_removes_bidi_and_zero_width() {
        assert_eq!(sanitize_line("a\nb\tc"), "a b c");
        assert_eq!(sanitize_line("ab\u{202e}cd\u{2066}ef"), "abcdef");
        assert_eq!(sanitize_line("a\u{200b}b\u{feff}c"), "abc");
    }

    #[test]
    fn multiline_keeps_line_structure() {
        assert_eq!(
            sanitize_multiline("[00:01]\u{1b}[31mred\n[00:02]blue"),
            "[00:01]red\n[00:02]blue"
        );
    }
}
