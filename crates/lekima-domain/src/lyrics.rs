//! LRC parsing and timed lyrics.
//!
//! Pure value-in/value-out so the daemon can return parsed lines over IPC and
//! the TUI never touches provider text. Handles metadata tags, multiple
//! timestamps per line, and untimed lyric files.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    pub at_ms: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// Original LRC, for `lekima lyrics --raw` and debugging.
    pub raw: String,
}

impl Lyrics {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Printable text without timestamps, with translations when present.
    #[must_use]
    pub fn plain_translated(&self) -> String {
        let mut output = String::new();
        for line in &self.lines {
            output.push_str(&line.text);
            if let Some(translation) = &line.translation {
                output.push('\n');
                output.push_str(translation);
            }
            output.push('\n');
        }
        output.trim_end().to_owned()
    }

    /// Printable text without timestamps.
    #[must_use]
    pub fn plain(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Index of the line active at `position_ms`, if any.
    #[must_use]
    pub fn active_index(&self, position_ms: u64) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        match self.lines.partition_point(|line| line.at_ms <= position_ms) {
            0 => None,
            index => Some(index - 1),
        }
    }
}

/// Parses an LRC document into sorted, timed lines.
///
/// Rules:
/// - `[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]` and `[hh:mm:ss.xx]` stamps.
/// - Multiple stamps on one line duplicate the text.
/// - `[ar:]`, `[ti:]`, `[by:]`, `[al:]` and similar metadata are dropped.
/// - `[offset:+ms]` shifts every timestamp.
/// - A file with no timestamps becomes a single block of lines at 0 ms, so the
///   pane still has something to show.
#[must_use]
pub fn parse_lrc(raw: &str) -> Lyrics {
    parse_with_translation(raw, None)
}

/// Parses original and translation documents, merging by exact timestamp.
#[must_use]
pub fn parse_lrc_with_translation(original: &str, translation: Option<&str>) -> Lyrics {
    parse_with_translation(original, translation)
}

fn parse_with_translation(raw: &str, translation: Option<&str>) -> Lyrics {
    let mut offset: i64 = 0;
    let mut entries: Vec<(i64, String, bool)> = Vec::new();

    for line in raw.lines() {
        let mut rest = line.trim();
        let mut times: Vec<i64> = Vec::new();
        let mut metadata = false;

        while let Some(open) = rest.strip_prefix('[') {
            let Some(close) = open.find(']') else {
                break;
            };
            let tag = &open[..close];
            rest = open[close + 1..].trim_start();
            if let Some(stamp) = parse_time_tag(tag) {
                times.push(stamp);
            } else if let Some(value) = tag.strip_prefix("offset:") {
                offset = value.trim().parse::<i64>().unwrap_or(0);
            } else if tag.contains(':') {
                metadata = true;
            }
        }

        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        let text = super::sanitize::sanitize_line(text);
        if text.is_empty() {
            continue;
        }
        if times.is_empty() {
            if !metadata {
                entries.push((0, text, true));
            }
            continue;
        }
        for time in times {
            entries.push((time, text.clone(), false));
        }
    }

    let timed = entries.iter().any(|(_, _, untimed)| !*untimed);
    let mut lines: Vec<LyricLine> = entries
        .into_iter()
        .map(|(at, text, _)| LyricLine {
            at_ms: if timed {
                u64::try_from(at.saturating_add(offset).max(0)).unwrap_or(0)
            } else {
                0
            },
            text,
            translation: None,
        })
        .collect();

    lines.sort_by_key(|line| line.at_ms);

    if let Some(translation) = translation {
        let translated = parse_with_translation(translation, None);
        let lookup = |at: u64| {
            translated
                .lines
                .iter()
                .find(|line| line.at_ms == at)
                .map(|line| line.text.clone())
        };
        for line in &mut lines {
            line.translation = lookup(line.at_ms);
        }
    }
    Lyrics {
        lines,
        raw: super::sanitize::sanitize_multiline(raw),
    }
}

/// Parses one timestamp tag into milliseconds.
fn parse_time_tag(tag: &str) -> Option<i64> {
    let parts: Vec<&str> = tag.split(':').collect();
    match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = minutes.trim().parse::<i64>().ok()?;
            let seconds = seconds.trim().parse::<f64>().ok()?;
            Some(minutes * 60_000 + (seconds * 1000.0).round() as i64)
        }
        [hours, minutes, seconds] => {
            let hours = hours.trim().parse::<i64>().ok()?;
            let minutes = minutes.trim().parse::<i64>().ok()?;
            let seconds = seconds.trim().parse::<f64>().ok()?;
            Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as i64)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Lyrics, parse_lrc};

    #[test]
    fn parses_timed_lines_and_strips_metadata() {
        let raw = "[ar:Queen]\n[00:01.50]first\n[00:10.00]second\n";
        let lyrics = parse_lrc(raw);
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].at_ms, 1_500);
        assert_eq!(lyrics.lines[0].text, "first");
        assert_eq!(lyrics.lines[1].at_ms, 10_000);
        assert_eq!(lyrics.plain(), "first\nsecond");
    }

    #[test]
    fn duplicates_text_for_multiple_timestamps() {
        let lyrics = parse_lrc("[00:01.00][00:05.00]again");
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].at_ms, 1_000);
        assert_eq!(lyrics.lines[1].at_ms, 5_000);
    }

    #[test]
    fn applies_offset_and_sorts() {
        let lyrics = parse_lrc("[offset:500]\n[00:02.00]later\n[00:01.00]earlier");
        assert_eq!(lyrics.lines[0].text, "earlier");
        assert_eq!(lyrics.lines[0].at_ms, 1_500);
        assert_eq!(lyrics.lines[1].at_ms, 2_500);
    }

    #[test]
    fn untimed_files_still_render() {
        let lyrics = parse_lrc("first line\n\nsecond line");
        assert_eq!(lyrics.plain(), "first line\nsecond line");
        assert!(lyrics.lines.iter().all(|line| line.at_ms == 0));
    }

    #[test]
    fn merges_translations_by_exact_timestamp() {
        let lyrics = super::parse_lrc_with_translation(
            "[00:01.00]hello\n[00:02.00]world",
            Some("[00:01.00]你好\n[00:03.00]ignored"),
        );
        assert_eq!(lyrics.lines[0].translation.as_deref(), Some("你好"));
        assert_eq!(lyrics.lines[1].translation, None);
        assert_eq!(lyrics.plain_translated(), "hello\n你好\nworld");
    }

    #[test]
    fn active_line_uses_the_last_stamp_at_or_before_position() {
        let lyrics = parse_lrc("[00:01.00]a\n[00:05.00]b\n[00:09.00]c");
        assert_eq!(lyrics.active_index(0), None);
        assert_eq!(lyrics.active_index(4_999), Some(0));
        assert_eq!(lyrics.active_index(5_000), Some(1));
        assert_eq!(lyrics.active_index(60_000), Some(2));
        assert_eq!(Lyrics::default().active_index(1), None);
    }
}
