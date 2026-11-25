//! Rendering.
//!
//! Visual language: a HUD line, a rule, a dense list, a command bar. No boxed
//! widgets in the main chrome — structure comes from dim rules and section
//! labels, state from neon accents. See `docs/sprint-2/tui-visual-design.md`.

use std::borrow::Cow;

use lekima_domain::{PlaybackState, PlaybackStatus, RepeatMode, Track, sanitize_line};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Table, TableState,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    App, CommentsState, Focus, LyricsState, Mode, SETTINGS_ROWS, SIDEBAR_STATICS, SearchEntry,
    SearchFilter, View,
};
use crate::keymap::{self, Section};
use crate::theme::{Density, Theme};

const MIN_WIDTH: u16 = 24;
const SIDEBAR_WIDTH: u16 = 24;
const SIDEBAR_BREAKPOINT: u16 = 64;
const VOLUME_CELLS: usize = 6;
const SPECTRUM_BARS: usize = 20;
const HUD_SPECTRUM_BARS: usize = 12;
const HUD_PROGRESS_CELLS: usize = 12;
const CURSOR_BLINK_FRAMES: u64 = 10;
const HELP_TWO_COLUMN_WIDTH: u16 = 80;
const TOAST_MAX_WIDTH: u16 = 60;
const EMPTY_ART_MIN_WIDTH: u16 = 44;
const EMPTY_ART_MIN_HEIGHT: u16 = 12;
const COMMENT_INDENT: usize = 2;

/// Compact empty-state wordmark: `LEK` and `IMA` are tinted separately.
const WORDMARK: [[&str; 2]; 3] = [
    ["█   █▀▀ █ █ ", "█ █▄ ▄█ ▄▀▄"],
    ["█   █▀  █▀  ", "█ █ ▀ █ █▀█"],
    ["█▄▄ █▄▄ █ █ ", "█ █   █ █ █"],
];

/// Where the lyrics pane ended up, so the toast can avoid it.
#[derive(Debug, Clone, Copy)]
enum LyricsPlacement {
    /// Right-hand column of a horizontal split.
    Side(Rect),
    /// Bottom pane of a vertical split, or a full takeover.
    Bottom(Rect),
}

pub fn draw(frame: &mut Frame, app: &mut App, theme: &Theme) {
    app.clamp();
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < 4 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "LEKIMA",
                Style::new()
                    .fg(theme.badge_fg)
                    .bg(theme.badge_bg)
                    .add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }

    let [hud, rule, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_hud(frame, app, theme, hud);
    draw_rule(frame, theme, rule);
    let lyrics = draw_body(frame, app, theme, body);
    draw_footer(frame, app, theme, footer);
    draw_toast(frame, app, theme, area, lyrics);

    if app.mode == Mode::Help {
        draw_help(frame, app, theme, area);
    }
    if app.mode == Mode::Settings {
        draw_settings(frame, app, theme, area);
    }
}

/// Spinner frame, frozen on the static loading glyph when motion is reduced.
fn spinner_glyph(app: &App, theme: &Theme) -> &'static str {
    if theme.motion.is_reduced() {
        theme.glyphs.loading
    } else {
        theme.glyphs.spinner[(app.frame % 4) as usize]
    }
}

fn draw_hud(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let loading = app.status.loading || app.status.buffering;
    let mut prefix: Vec<Span<'static>> = vec![
        Span::styled(
            " LEKIMA ",
            Style::new()
                .fg(theme.badge_fg)
                .bg(theme.badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", theme.glyphs.hud_separator),
            Style::new().fg(theme.dim),
        ),
    ];
    match (&app.status.track, app.status.status) {
        (Some(_), _) if loading => {
            let spinner = spinner_glyph(app, theme);
            let label = if app.status.loading {
                "LOADING"
            } else {
                "BUFFERING"
            };
            prefix.push(Span::styled(
                format!("{spinner} {label} "),
                Style::new()
                    .fg(if app.status.buffering {
                        theme.accent_alt
                    } else {
                        theme.warn
                    })
                    .add_modifier(Modifier::BOLD),
            ));
        }
        (Some(_), status) => prefix.push(Span::styled(
            format!("{} ", glyph(status, theme)),
            Style::new().fg(state_colour(status, theme)),
        )),
        (None, _) => prefix.push(Span::styled(
            format!("{} ", theme.glyphs.stopped),
            Style::new().fg(theme.muted),
        )),
    }

    let duration = app
        .status
        .track
        .as_ref()
        .map_or(0, |track| effective_duration(&app.status, track));

    // Optional segments, ordered by priority. The tail is dropped first when
    // the terminal is narrow; progress and time are the last to go.
    let mut groups: Vec<Vec<Span<'static>>> = Vec::new();
    if area.width >= 40 {
        groups.push(progress_group(app, duration, theme));
    }
    groups.push(volume_group(app, theme));
    groups.push(vec![
        Span::styled("Q:", Style::new().fg(theme.muted)),
        Span::styled(
            app.status.queue_length.to_string(),
            Style::new().fg(theme.accent_alt),
        ),
    ]);
    groups.push(repeat_group(app, theme));
    groups.push(shuffle_group(app, theme));
    if let Some(remaining) = app.status.sleep_remaining_ms {
        groups.push(vec![
            Span::styled(
                format!("{} ", theme.glyphs.sleep),
                Style::new().fg(theme.dim),
            ),
            Span::styled(clock(remaining), Style::new().fg(theme.warn)),
        ]);
    }
    if app.status.track.is_some() {
        groups.push(vec![Span::styled(
            app.status.quality.as_str().to_ascii_uppercase(),
            Style::new().fg(theme.accent_alt),
        )]);
        if app
            .status
            .track
            .as_ref()
            .is_some_and(|track| is_preview(&app.status, track))
        {
            groups.push(vec![Span::styled(
                "PREVIEW",
                Style::new().fg(theme.warn).add_modifier(Modifier::BOLD),
            )]);
        }
        if app.status.cached {
            groups.push(vec![Span::styled(
                "CACHE",
                Style::new()
                    .fg(theme.accent_alt)
                    .add_modifier(Modifier::BOLD),
            )]);
        }
        let running = app.hooks.iter().filter(|hook| hook.running).count();
        let broken = app.hooks.iter().filter(|hook| hook.error.is_some()).count();
        if running > 0 || broken > 0 {
            let (label, colour) = match (running, broken) {
                (0, _) => ("HOOK !".to_owned(), theme.hot),
                (running, 0) => (format!("HOOK {running}"), theme.warn),
                (running, broken) => (format!("HOOK {running}+{broken}!"), theme.hot),
            };
            groups.push(vec![Span::styled(
                label,
                Style::new().fg(colour).add_modifier(Modifier::BOLD),
            )]);
        }
        if !theme.motion.is_reduced() {
            groups.push(spectrum_group(app, theme));
        }
    }

    let prefix_width = spans_width(&prefix);
    let mut identity_budget;
    loop {
        let right = groups_width(&groups);
        identity_budget = (area.width as usize)
            .saturating_sub(prefix_width)
            .saturating_sub(right);
        if identity_budget >= 8 || groups.len() <= 1 {
            break;
        }
        groups.pop();
    }
    if identity_budget < 8 {
        groups.clear();
        identity_budget = (area.width as usize).saturating_sub(prefix_width);
    }

    let mut spans = prefix;
    match &app.status.track {
        Some(track) => {
            let title = clean(&track.name);
            let title = title.as_ref();
            let artist_raw = track.artist_names();
            let artist = clean(&artist_raw).into_owned();
            let title_width = display_width(title);
            let artist_width = display_width(&artist);
            if title_width + 3 + artist_width <= identity_budget {
                spans.push(Span::styled(
                    title.to_owned(),
                    Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!(" {} ", theme.glyphs.separator),
                    Style::new().fg(theme.dim),
                ));
                spans.push(Span::styled(artist, Style::new().fg(theme.muted)));
            } else if title_width <= identity_budget.saturating_sub(4) {
                let artist_budget = identity_budget.saturating_sub(title_width + 3);
                spans.push(Span::styled(
                    title.to_owned(),
                    Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
                ));
                if artist_budget >= 4 {
                    spans.push(Span::styled(
                        format!(" {} ", theme.glyphs.separator),
                        Style::new().fg(theme.dim),
                    ));
                    spans.push(Span::styled(
                        truncate_to_width(&artist, artist_budget, theme.glyphs.ellipsis),
                        Style::new().fg(theme.muted),
                    ));
                }
            } else {
                spans.push(Span::styled(
                    truncate_to_width(title, identity_budget, theme.glyphs.ellipsis),
                    Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
                ));
            }
        }
        None => spans.push(Span::styled(
            truncate_to_width("nothing playing", identity_budget, theme.glyphs.ellipsis),
            Style::new().fg(theme.muted),
        )),
    }

    for group in groups {
        spans.push(Span::styled("  ", Style::new()));
        spans.extend(group);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn progress_group(app: &App, duration: u64, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = progress_spans(app, duration, theme);
    spans.push(Span::styled(
        format!(" {}/{}", clock(app.status.position_ms), clock(duration)),
        Style::new().fg(theme.muted),
    ));
    spans
}

fn volume_group(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    if app.status.muted {
        vec![Span::styled(
            "MUTE",
            Style::new().fg(theme.warn).add_modifier(Modifier::BOLD),
        )]
    } else {
        vec![
            Span::styled("VOL ", Style::new().fg(theme.muted)),
            Span::styled(
                volume_bar(app.status.effective_volume(), theme),
                Style::new().fg(theme.meter_full),
            ),
        ]
    }
}

fn repeat_group(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let (label, colour) = match app.status.repeat {
        RepeatMode::Off => ("OFF", theme.dim),
        RepeatMode::All => ("ALL", theme.accent_alt),
        RepeatMode::One => ("1", theme.accent),
    };
    vec![Span::styled(
        format!("[{} {}]", theme.glyphs.repeat, label),
        Style::new().fg(colour),
    )]
}

fn shuffle_group(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    // Bracket presence, not colour, is the on/off signal.
    if app.status.shuffle {
        vec![Span::styled(
            format!("[{}]", theme.glyphs.shuffle),
            Style::new().fg(theme.accent_alt),
        )]
    } else {
        Vec::new()
    }
}

fn spectrum_group(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("SIG ", Style::new().fg(theme.muted))];
    spans.extend(wave_spans(app, theme));
    spans
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum()
}

fn groups_width(groups: &[Vec<Span<'_>>]) -> usize {
    groups
        .iter()
        .filter(|group| !group.is_empty())
        .map(|group| spans_width(group) + 2)
        .sum()
}

fn draw_rule(frame: &mut Frame, theme: &Theme, area: Rect) {
    let rule = theme.glyphs.rule.repeat(usize::from(area.width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(rule, Style::new().fg(theme.dim)))),
        area,
    );
}

fn draw_body(
    frame: &mut Frame,
    app: &mut App,
    theme: &Theme,
    area: Rect,
) -> Option<LyricsPlacement> {
    let lyrics_visible = app.lyrics_open && app.lyrics_state.track().is_some();

    if area.width >= SIDEBAR_BREAKPOINT {
        let [sidebar, divider, main] = Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .areas(area);
        draw_sidebar(frame, app, theme, sidebar);
        draw_divider(frame, theme, divider);

        if lyrics_visible {
            if main.width >= 80 {
                let [tracks, lyrics] =
                    Layout::horizontal([Constraint::Min(30), Constraint::Length(38)]).areas(main);
                draw_main(frame, app, theme, tracks);
                draw_lyrics(frame, app, theme, lyrics);
                return Some(LyricsPlacement::Side(lyrics));
            }
            if main.height >= 12 {
                let [tracks, rule, lyrics] = Layout::vertical([
                    Constraint::Min(4),
                    Constraint::Length(1),
                    Constraint::Min(6),
                ])
                .areas(main);
                draw_main(frame, app, theme, tracks);
                draw_thin_rule(frame, theme, rule);
                draw_lyrics(frame, app, theme, lyrics);
                return Some(LyricsPlacement::Bottom(lyrics));
            }
            draw_main(frame, app, theme, main);
            draw_lyrics(frame, app, theme, main);
            return Some(LyricsPlacement::Bottom(main));
        }
        draw_main(frame, app, theme, main);
    } else {
        if lyrics_visible {
            if area.width >= 80 {
                let [tracks, lyrics] =
                    Layout::horizontal([Constraint::Min(20), Constraint::Length(38)]).areas(area);
                draw_main(frame, app, theme, tracks);
                draw_lyrics(frame, app, theme, lyrics);
                return Some(LyricsPlacement::Side(lyrics));
            }
            if area.height >= 12 {
                let [tracks, rule, lyrics] = Layout::vertical([
                    Constraint::Min(4),
                    Constraint::Length(1),
                    Constraint::Min(6),
                ])
                .areas(area);
                draw_main(frame, app, theme, tracks);
                draw_thin_rule(frame, theme, rule);
                draw_lyrics(frame, app, theme, lyrics);
                return Some(LyricsPlacement::Bottom(lyrics));
            }
            draw_main(frame, app, theme, area);
            draw_lyrics(frame, app, theme, area);
            return Some(LyricsPlacement::Bottom(area));
        }
        draw_main(frame, app, theme, area);
    }
    None
}

fn draw_divider(frame: &mut Frame, theme: &Theme, area: Rect) {
    let lines: Vec<Line> = (0..area.height)
        .map(|_| {
            Line::from(Span::styled(
                theme.glyphs.divider,
                Style::new().fg(theme.dim),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    app.sidebar_view_height = list_area.height;

    let focused = app.focus == Focus::Sidebar && app.mode != Mode::Queue;
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("LIBRARY", Style::new().fg(theme.muted)),
        Span::styled(
            format!(" {} {}", theme.glyphs.separator, app.playlists.len()),
            Style::new().fg(theme.accent_alt),
        ),
    ];
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    let keys = ["d", "F", "t", "c", theme.glyphs.liked];
    let icons = [
        theme.glyphs.daily,
        theme.glyphs.fm,
        theme.glyphs.charts,
        theme.glyphs.cloud,
        "",
    ];
    let mut items: Vec<ListItem> = Vec::with_capacity(app.sidebar_len() + 2);

    for (index, name) in SIDEBAR_STATICS.iter().enumerate() {
        let selected = index == app.sidebar_selection;
        let marker = if selected { theme.glyphs.active } else { " " };
        let mut row = vec![
            Span::styled(
                marker,
                Style::new().fg(if focused { theme.accent } else { theme.muted }),
            ),
            Span::styled(keys[index], Style::new().fg(theme.accent_alt)),
            Span::styled(" ", Style::new().fg(theme.dim)),
        ];
        if !icons[index].is_empty() {
            row.push(Span::styled(
                format!("{} ", icons[index]),
                Style::new().fg(theme.muted),
            ));
        }
        row.push(Span::styled(
            (*name).to_owned(),
            Style::new().fg(theme.text),
        ));
        items.push(ListItem::new(Line::from(row)));
    }

    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled(
            format!("{} PLAYLISTS", theme.glyphs.playlists),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!(" {} {}", theme.glyphs.separator, app.playlists.len()),
            Style::new().fg(theme.accent_alt),
        ),
    ])));

    // Leave one column of breathing room before the divider.
    let width = usize::from(list_area.width).saturating_sub(1).max(12);
    let index_width = app.playlists.len().to_string().len().max(2);
    if app.playlists.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            truncate_to_width(
                &format!(
                    "  no playlists {} press / to search",
                    theme.glyphs.separator
                ),
                width,
                theme.glyphs.ellipsis,
            ),
            Style::new().fg(theme.dim),
        ))));
    } else {
        for (index, playlist) in app.playlists.iter().enumerate() {
            let flat = SIDEBAR_STATICS.len() + index;
            let selected = flat == app.sidebar_selection;
            let marker = if selected { theme.glyphs.active } else { " " };
            let count = playlist.track_count.to_string();
            let prefix = 1 + index_width + 1; // marker, index, space
            let name_width = width.saturating_sub(prefix + count.len() + 1);
            let name = truncate_to_width(&clean(&playlist.name), name_width, theme.glyphs.ellipsis);
            let pad = width
                .saturating_sub(prefix + display_width(&name) + count.len())
                .max(1);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    marker,
                    Style::new().fg(if focused { theme.accent } else { theme.muted }),
                ),
                Span::styled(
                    format!("{:0width$} ", index + 1, width = index_width),
                    Style::new().fg(theme.muted),
                ),
                Span::styled(name, Style::new().fg(theme.text)),
                Span::styled(" ".repeat(pad), Style::new()),
                Span::styled(count, Style::new().fg(theme.muted)),
            ])));
        }
    }

    let selected = if app.sidebar_selection < SIDEBAR_STATICS.len() {
        Some(app.sidebar_selection)
    } else {
        // One blank line plus the PLAYLISTS header sit between the sections.
        Some(app.sidebar_selection + 2)
    };
    let list = List::new(items).highlight_style(if focused {
        selection_style(theme)
    } else {
        inactive_selection_style(theme)
    });
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_main(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    // One-column gutter so the divider never touches content.
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    match (app.mode, app.view) {
        (Mode::Queue, _) => draw_queue(frame, app, theme, inner),
        (Mode::Comments, _) => draw_comments(frame, app, theme, inner),
        (Mode::History, _) => draw_history(frame, app, theme, inner),
        (_, View::Toplists) => draw_toplists(frame, app, theme, inner),
        (_, View::Search) => draw_search(frame, app, theme, inner),
        (_, View::Artist) => draw_artist(frame, app, theme, inner),
        (_, View::Album) => draw_album(frame, app, theme, inner),
        _ => draw_tracks(frame, app, theme, inner),
    }
}

fn draw_tracks(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let focused = app.focus == Focus::Main;
    let total = app.tracks.len();
    let visible = usize::from(content.height.saturating_sub(1));
    app.main_view_height = u16::try_from(visible).unwrap_or(u16::MAX);
    let position = if total > visible && total > 0 {
        format!(
            " {} {}/{}",
            theme.glyphs.separator,
            app.main_selection.saturating_add(1).min(total),
            total
        )
    } else {
        String::new()
    };
    let source = clean(&app.source);
    let count = total.to_string();
    let fixed = 3 + 3 + count.len() + " TRACKS".len() + display_width(&position) + 1;
    let source_budget = usize::from(label.width).saturating_sub(fixed).max(4);
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled(
            truncate_to_width(&source, source_budget, theme.glyphs.ellipsis),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!(" {} ", theme.glyphs.separator),
            Style::new().fg(theme.dim),
        ),
        Span::styled(count, Style::new().fg(theme.accent_alt)),
        Span::styled(" TRACKS", Style::new().fg(theme.muted)),
    ];
    if !position.is_empty() {
        title.push(Span::styled(position, Style::new().fg(theme.muted)));
    }
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    if app.tracks.is_empty() {
        draw_empty_tracks(frame, theme, content);
        return;
    }

    let columns = track_columns(content.width);
    let playing_id = app.status.track.as_ref().map(|track| track.id);
    let comfortable = theme.density == Density::Comfortable;
    let mut rows: Vec<Row> = Vec::with_capacity(app.tracks.len() * 2);
    for (index, track) in app.tracks.iter().enumerate() {
        rows.push(track_row(
            app,
            theme,
            track,
            index,
            TrackRowState {
                current: Some(track.id) == playing_id,
                past: false,
                selection: focus_tick(focused, index == app.main_selection, theme),
            },
            &columns,
        ));
        if comfortable {
            rows.push(Row::new(Vec::<Span>::new()));
        }
    }

    let mut header = vec!["", "#", "", "TITLE"];
    if columns.artist.is_some() {
        header.push("ARTIST");
    }
    if columns.time {
        header.push("   TIME ");
    }
    let table = Table::new(rows, columns.constraints())
        .column_spacing(0)
        .header(Row::new(header).style(Style::new().fg(theme.panel)))
        .row_highlight_style(if focused {
            selection_style(theme)
        } else {
            inactive_selection_style(theme)
        });

    let visual_selection = if comfortable {
        app.main_selection * 2
    } else {
        app.main_selection
    };
    let mut state = TableState::default().with_selected(Some(visual_selection));
    frame.render_stateful_widget(table, content, &mut state);
}

/// The empty library is the one place with brand art: a compact, left-aligned
/// wordmark instead of a blank pane. Small terminals fall back to one hint line.
fn draw_empty_tracks(frame: &mut Frame, theme: &Theme, area: Rect) {
    if area.width < EMPTY_ART_MIN_WIDTH || area.height < EMPTY_ART_MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("// ", Style::new().fg(theme.panel)),
                Span::styled("NOTHING LOADED", Style::new().fg(theme.muted)),
                Span::styled(
                    format!(
                        "  {} search {} Tab library",
                        theme.glyphs.separator, theme.glyphs.separator
                    ),
                    Style::new().fg(theme.muted),
                ),
            ])),
            area,
        );
        return;
    }

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    if theme.glyphs.ascii {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                "L E K I M A",
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(theme.glyphs.rule.repeat(11), Style::new().fg(theme.dim)),
        ]));
    } else {
        for row in WORDMARK {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(row[0], Style::new().fg(theme.accent)),
                Span::styled(row[1], Style::new().fg(theme.accent_alt)),
            ]));
        }
    }
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                "NETEASE CLOUD MUSIC // CONSOLE",
                Style::new().fg(theme.muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled("// ", Style::new().fg(theme.panel)),
            Span::styled(
                "NOTHING LOADED",
                Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        empty_hint("/", "search the catalogue", theme),
        empty_hint("Tab", "browse the library", theme),
        empty_hint(":", "command line", theme),
        empty_hint("?", "key reference", theme),
    ]);
    if usize::from(area.height) < lines.len() + 2 {
        lines.truncate(usize::from(area.height));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn empty_hint(key: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", Style::new()),
        Span::styled(pad_right(key, 6), Style::new().fg(theme.accent_alt)),
        Span::styled(description.to_owned(), Style::new().fg(theme.muted)),
    ])
}

/// One anatomy for every empty pane: a `//` label, one sentence, then up to
/// three actionable key hints. The empty library is the only surface that adds
/// brand art on top.
fn empty_lines(
    label: &str,
    sentence: &str,
    hints: &[(&str, &str)],
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !label.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("// ", Style::new().fg(theme.panel)),
            Span::styled(
                label.to_uppercase(),
                Style::new().fg(theme.muted).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("  ", Style::new()),
        Span::styled(sentence.to_owned(), Style::new().fg(theme.muted)),
    ]));
    if !hints.is_empty() {
        lines.push(Line::from(""));
    }
    for (key, description) in hints {
        lines.push(empty_hint(key, description, theme));
    }
    lines
}

#[derive(Debug, Clone, Copy)]
struct TrackColumns {
    title: u16,
    artist: Option<u16>,
    time: bool,
}

impl TrackColumns {
    fn constraints(self) -> Vec<Constraint> {
        let mut widths = vec![
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Length(2),
            Constraint::Length(self.title),
        ];
        if let Some(artist) = self.artist {
            widths.push(Constraint::Length(artist));
        }
        if self.time {
            widths.push(Constraint::Length(8));
        }
        widths
    }
}

fn track_columns(width: u16) -> TrackColumns {
    let width = usize::from(width);
    let time = width >= 38;
    let fixed = 8 + usize::from(time) * 8;
    let flexible = width.saturating_sub(fixed);
    if width >= 64 && flexible >= 24 {
        let title = flexible * 62 / 100;
        TrackColumns {
            title: u16::try_from(title.max(12)).unwrap_or(u16::MAX),
            artist: Some(u16::try_from(flexible.saturating_sub(title).max(8)).unwrap_or(u16::MAX)),
            time,
        }
    } else {
        TrackColumns {
            title: u16::try_from(flexible.max(8)).unwrap_or(u16::MAX),
            artist: None,
            time,
        }
    }
}

fn focus_tick(focused: bool, selected: bool, theme: &Theme) -> (&'static str, Color) {
    if selected {
        (
            theme.glyphs.active,
            if focused { theme.accent } else { theme.muted },
        )
    } else {
        (" ", theme.dim)
    }
}

#[derive(Debug, Clone, Copy)]
struct TrackRowState {
    current: bool,
    past: bool,
    selection: (&'static str, Color),
}

fn track_row<'a>(
    app: &App,
    theme: &Theme,
    track: &'a lekima_domain::Track,
    index: usize,
    state: TrackRowState,
    columns: &TrackColumns,
) -> Row<'a> {
    let TrackRowState {
        current,
        past,
        selection,
    } = state;
    let (marker, marker_style) = if current {
        (
            if app.status.loading || app.status.buffering {
                spinner_glyph(app, theme).to_owned()
            } else {
                glyph(app.status.status, theme).to_owned()
            },
            Style::new().fg(state_colour(app.status.status, theme)),
        )
    } else {
        (" ".to_owned(), Style::new().fg(theme.dim))
    };
    let liked = if current && app.status.liked {
        format!("{} ", theme.glyphs.liked)
    } else {
        String::new()
    };
    let title_text = clean(&track.name);
    let artist_raw = track.artist_names();
    let artist_text = clean(&artist_raw).into_owned();
    let title = truncate_to_width(
        &format!("{liked}{title_text}"),
        usize::from(columns.title),
        theme.glyphs.ellipsis,
    );
    let title_style = if current {
        Style::new().fg(theme.text).add_modifier(Modifier::BOLD)
    } else if past {
        Style::new().fg(theme.muted)
    } else {
        Style::new().fg(theme.text)
    };
    let secondary = theme.muted;
    let mut cells = vec![
        Span::styled(format!("{} ", selection.0), Style::new().fg(selection.1)),
        Span::styled(format!("{:03} ", index + 1), Style::new().fg(theme.muted)),
        Span::styled(format!("{marker} "), marker_style),
        Span::styled(title, title_style),
    ];
    if let Some(artist) = columns.artist {
        cells.push(Span::styled(
            truncate_to_width(&artist_text, usize::from(artist), theme.glyphs.ellipsis),
            Style::new().fg(secondary),
        ));
    }
    if columns.time {
        cells.push(Span::styled(
            format!("{:>7} ", track.duration_label()),
            Style::new().fg(secondary),
        ));
    }
    Row::new(cells)
}

fn draw_toplists(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let focused = app.focus == Focus::Main;
    let total = app.toplists.len();
    let visible = usize::from(content.height.saturating_sub(1));
    app.main_view_height = u16::try_from(visible).unwrap_or(u16::MAX);
    let position = if total > visible && total > 0 {
        format!(
            " {} {}/{}",
            theme.glyphs.separator,
            app.main_selection.saturating_add(1).min(total),
            total
        )
    } else {
        String::new()
    };
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("CHARTS", Style::new().fg(theme.muted)),
        Span::styled(
            format!(" {} {}{}", theme.glyphs.separator, total, position),
            Style::new().fg(theme.accent_alt),
        ),
    ];
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    if app.toplists.is_empty() {
        frame.render_widget(
            Paragraph::new(Text::from(empty_lines(
                "",
                "no charts loaded",
                &[("t", "load toplists")],
                theme,
            ))),
            content,
        );
        return;
    }

    let width = usize::from(content.width);
    let show_updated = width >= 72;
    let show_tracks = width >= 48;
    let rows: Vec<Row> = app
        .toplists
        .iter()
        .enumerate()
        .map(|(index, chart)| {
            let selection = focus_tick(focused, index == app.main_selection, theme);
            let mut cells = vec![
                Span::styled(selection.0, Style::new().fg(selection.1)),
                Span::styled(format!("{:>10}", chart.id), Style::new().fg(theme.muted)),
                Span::styled(clean(&chart.name), Style::new().fg(theme.text)),
            ];
            if show_tracks {
                cells.push(Span::styled(
                    format!("{:>5}", chart.track_count),
                    Style::new().fg(theme.accent_alt),
                ));
            }
            if show_updated {
                cells.push(Span::styled(
                    clean(&chart.update_frequency),
                    Style::new().fg(theme.muted),
                ));
            }
            Row::new(cells)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(1),
        Constraint::Length(10),
        Constraint::Min(20),
    ];
    let mut header = vec![" ", "ID", "CHART"];
    if show_tracks {
        widths.push(Constraint::Length(6));
        header.push("TRACKS");
    }
    if show_updated {
        widths.push(Constraint::Length(16));
        header.push("UPDATED");
    }
    let table = Table::new(rows, widths)
        .column_spacing(0)
        .header(Row::new(header).style(Style::new().fg(theme.panel)))
        .row_highlight_style(if focused {
            selection_style(theme)
        } else {
            inactive_selection_style(theme)
        });
    let mut state = TableState::default().with_selected(Some(app.main_selection));
    frame.render_stateful_widget(table, content, &mut state);
}

/// Filter row shown in place of the rule while the search prompt is open.
/// Glyph that names a catalogue kind, shared by the rail and the headers.
const fn filter_glyph(filter: SearchFilter, theme: &Theme) -> &'static str {
    match filter {
        SearchFilter::All => "",
        SearchFilter::Tracks => theme.glyphs.tracks,
        SearchFilter::Albums => theme.glyphs.albums,
        SearchFilter::Artists => theme.glyphs.artists,
        SearchFilter::Playlists => theme.glyphs.playlists,
    }
}

/// One glyph per catalogue kind; the active one is bracketed so the state does
/// not depend on colour. `ALL` is the absence of a bracket.
fn search_rail_spans(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(8);
    for filter in SearchFilter::ALL {
        if filter == SearchFilter::All {
            continue;
        }
        let glyph = filter_glyph(filter, theme);
        let active = app.search.filter == filter;
        spans.push(Span::styled(
            if active {
                format!("[{glyph}]")
            } else {
                glyph.to_owned()
            },
            if active {
                Style::new()
                    .fg(theme.badge_fg)
                    .bg(theme.badge_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.muted)
            },
        ));
        spans.push(Span::styled(" ", Style::new()));
    }
    spans
}

fn section_line(label: &str, count: usize, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled(label.to_owned(), Style::new().fg(theme.muted)),
        Span::styled(
            format!(" {} {}", theme.glyphs.separator, count),
            Style::new().fg(theme.accent_alt),
        ),
    ])
}

fn rule_line(width: usize, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        theme.glyphs.rule.repeat(width.min(240)),
        Style::new().fg(theme.dim),
    ))
}

/// Provider error state shared by the catalogue pages.
fn error_block(message: &str, theme: &Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("! ", Style::new().fg(theme.warn)),
        Span::styled(
            "CATALOGUE UNAVAILABLE",
            Style::new().fg(theme.warn).add_modifier(Modifier::BOLD),
        ),
    ])];
    for line in wrap_to_width(&clean(message), usize::from(width).saturating_sub(2).max(8)) {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(line, Style::new().fg(theme.text)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("  ", Style::new()),
        Span::styled(
            format!("r retry {} esc close", theme.glyphs.separator),
            Style::new().fg(theme.muted),
        ),
    ]));
    lines
}

fn draw_search(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let focused = app.focus == Focus::Main;
    let window = usize::from(content.height);
    app.main_view_height = content.height;

    let total = app
        .search
        .results
        .as_ref()
        .map_or(app.search.entries.len(), |results| {
            usize::try_from(results.total).unwrap_or(usize::MAX)
        });
    let query = truncate_to_width(&clean(&app.search.query), 24, theme.glyphs.ellipsis);
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("SEARCH", Style::new().fg(theme.muted)),
        Span::styled(
            format!(" {} ", theme.glyphs.separator),
            Style::new().fg(theme.dim),
        ),
        Span::styled(format!("\"{query}\""), Style::new().fg(theme.text)),
        Span::styled(
            format!(" {} ", theme.glyphs.separator),
            Style::new().fg(theme.dim),
        ),
        Span::styled(
            {
                let glyph = filter_glyph(app.search.applied_filter, theme);
                if glyph.is_empty() {
                    app.search.applied_filter.label().to_owned()
                } else {
                    format!("{glyph} {}", app.search.applied_filter.label())
                }
            },
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!(" {} ", theme.glyphs.separator),
            Style::new().fg(theme.dim),
        ),
        Span::styled(
            compact_count(total as u64),
            Style::new().fg(theme.accent_alt),
        ),
        Span::styled(" RESULTS", Style::new().fg(theme.muted)),
    ];
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    if app.search.entries.is_empty() {
        if app.search.loading {
            frame.render_widget(
                Paragraph::new(loading_line(&app.source, theme, app.frame)),
                content,
            );
            return;
        }
        if let Some(error) = &app.search.error {
            frame.render_widget(
                Paragraph::new(Text::from(error_block(error, theme, content.width))),
                content,
            );
            return;
        }
        let sentence = if app.search.query.is_empty() {
            "type a query and press enter".to_owned()
        } else {
            format!("no matches for \"{}\"", clean(&app.search.query))
        };
        frame.render_widget(
            Paragraph::new(Text::from(empty_lines(
                "",
                &truncate_to_width(
                    &sentence,
                    usize::from(content.width).saturating_sub(2),
                    theme.glyphs.ellipsis,
                ),
                &[(
                    "/",
                    if app.search.query.is_empty() {
                        "type a query"
                    } else {
                        "refine the query"
                    },
                )],
                theme,
            ))),
            content,
        );
        return;
    }

    let width = usize::from(content.width);
    let title_width = width.saturating_sub(28).clamp(12, 48);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut line_of: Vec<usize> = Vec::with_capacity(app.search.entries.len());
    let mut current: Option<u8> = None;
    for (index, entry) in app.search.entries.iter().enumerate() {
        let kind = match entry {
            SearchEntry::Track(_) => 0,
            SearchEntry::Album(_) => 1,
            SearchEntry::Artist(_) => 2,
            SearchEntry::Playlist(_) => 3,
        };
        if current != Some(kind) {
            current = Some(kind);
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            let (heading, count) = match (kind, app.search.results.as_ref()) {
                (0, Some(results)) => ("TRACKS", results.tracks.len()),
                (1, Some(results)) => ("ALBUMS", results.albums.len()),
                (2, Some(results)) => ("ARTISTS", results.artists.len()),
                (3, Some(results)) => ("PLAYLISTS", results.playlists.len()),
                _ => ("RESULTS", 0),
            };
            lines.push(section_line(heading, count, theme));
        }
        line_of.push(lines.len());
        lines.push(search_entry_line(
            theme,
            entry,
            index,
            focused && index == app.main_selection,
            title_width,
            width,
        ));
    }

    let offset = line_of
        .get(app.main_selection)
        .map_or(0, |line| scroll_offset(*line, window));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
        content,
    );
}

/// Keeps `line` visible in a `window`-line viewport with the least scrolling.
fn scroll_offset(line: usize, window: usize) -> usize {
    if window == 0 || line < window {
        0
    } else {
        line + 1 - window
    }
}

fn search_entry_line(
    theme: &Theme,
    entry: &SearchEntry,
    index: usize,
    selected: bool,
    title_width: usize,
    line_width: usize,
) -> Line<'static> {
    let marker = if selected { theme.glyphs.active } else { " " };
    let (title, meta) = match entry {
        SearchEntry::Track(track) => (
            track.name.clone(),
            format!(
                "{} {} {}",
                track.artist_names(),
                theme.glyphs.separator,
                track.duration_label()
            ),
        ),
        SearchEntry::Album(album) => {
            let artist = album.artist_name();
            let artist = if artist.is_empty() { "-" } else { artist };
            let year = album
                .year
                .map_or_else(|| "-".to_owned(), |year| year.to_string());
            let tracks = album.track_count.map_or_else(
                || "- tracks".to_owned(),
                |count| format!("{count} {}", tracks_word(count as usize)),
            );
            (
                album.name.clone(),
                format!(
                    "{artist} {} {year} {} {tracks}",
                    theme.glyphs.separator, theme.glyphs.separator
                ),
            )
        }
        SearchEntry::Artist(artist) => (artist.name.clone(), String::new()),
        SearchEntry::Playlist(playlist) => (
            playlist.name.clone(),
            format!("{} tracks", compact_count(playlist.track_count)),
        ),
    };
    let title = truncate_to_width(&clean(&title), title_width, theme.glyphs.ellipsis);
    let prefix_width = 2 + 5;
    let meta_budget = line_width
        .saturating_sub(prefix_width + title_width + 1)
        .max(1);
    let meta = truncate_to_width(&clean(&meta), meta_budget, theme.glyphs.ellipsis);
    let title_pad = " ".repeat(title_width.saturating_sub(display_width(&title)));
    let meta_width = display_width(&meta);
    let pad = " ".repeat(line_width.saturating_sub(prefix_width + title_width + meta_width));
    let primary = if selected {
        selection_style(theme)
    } else {
        Style::new().fg(theme.text)
    };
    let secondary = if selected {
        selection_style(theme)
    } else {
        Style::new().fg(theme.muted)
    };
    Line::from(vec![
        Span::styled(
            format!("{marker} "),
            Style::new().fg(if selected { theme.accent } else { theme.dim }),
        ),
        Span::styled(
            format!("{:>3}  ", index + 1),
            if selected {
                primary
            } else {
                Style::new().fg(theme.muted)
            },
        ),
        Span::styled(format!("{title}{title_pad}"), primary),
        Span::styled(format!("{pad}{meta}"), secondary),
    ])
}

fn draw_artist(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let focused = app.focus == Focus::Main;
    let window = usize::from(content.height);
    app.main_view_height = content.height;

    let name = app
        .artist
        .detail
        .as_ref()
        .map_or_else(|| app.source.clone(), |detail| detail.name.clone());
    let id = app.artist.id.map(|id| id.to_string()).unwrap_or_default();
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("ARTIST", Style::new().fg(theme.muted)),
    ];
    if !id.is_empty() {
        title.push(Span::styled(
            format!(" {} {id}", theme.glyphs.separator),
            Style::new().fg(theme.muted),
        ));
    }
    title.push(Span::styled(
        format!(" {} ", theme.glyphs.separator),
        Style::new().fg(theme.dim),
    ));
    title.push(Span::styled(
        truncate_to_width(&clean(&name), 40, theme.glyphs.ellipsis),
        Style::new().fg(theme.text),
    ));
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    let width = usize::from(content.width);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(detail) = app.artist.detail.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                format!(
                    "{} albums {} {} tracks",
                    compact_count(detail.album_count),
                    theme.glyphs.separator,
                    compact_count(detail.track_count)
                ),
                Style::new().fg(theme.accent_alt),
            ),
        ]));
        if !detail.aliases.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(
                    truncate_to_width(
                        &clean(
                            &detail
                                .aliases
                                .join(&format!(" {} ", theme.glyphs.separator)),
                        ),
                        width.saturating_sub(2),
                        theme.glyphs.ellipsis,
                    ),
                    Style::new().fg(theme.muted),
                ),
            ]));
        }
        if !detail.description.is_empty() && window > lines.len() + 4 {
            for line in wrap_to_width(&clean(&detail.description), width.saturating_sub(2).max(8))
                .into_iter()
                .take(1)
            {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::new()),
                    Span::styled(line, Style::new().fg(theme.dim)),
                ]));
            }
        }
        lines.push(rule_line(width, theme));
    }

    if let Some(error) = &app.artist.error {
        // Keep whatever profile loaded before the failed page request.
        let mut block = lines;
        block.extend(error_block(error, theme, content.width));
        frame.render_widget(Paragraph::new(Text::from(block)), content);
        return;
    }

    if app.artist.albums.is_empty() {
        if app.artist.loading {
            lines.push(loading_line(&app.source, theme, app.frame));
        } else {
            lines.push(Line::from(vec![
                Span::styled("// ", Style::new().fg(theme.panel)),
                Span::styled(
                    "NO ALBUMS",
                    Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  r retry {} esc close", theme.glyphs.separator),
                    Style::new().fg(theme.muted),
                ),
            ]));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), content);
        return;
    }

    lines.push(section_line("ALBUMS", app.artist.albums.len(), theme));
    let mut line_of = Vec::with_capacity(app.artist.albums.len());
    let title_width = width.saturating_sub(30).clamp(12, 48);
    for (index, album) in app.artist.albums.iter().enumerate() {
        line_of.push(lines.len());
        lines.push(album_line(
            theme,
            album,
            index,
            focused && index == app.main_selection,
            title_width,
        ));
    }
    if app.artist.has_more {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                format!(
                    "{} MORE ALBUMS {} n loads the next page",
                    theme.glyphs.stopped, theme.glyphs.separator
                ),
                Style::new().fg(theme.accent_alt),
            ),
        ]));
    }

    let offset = line_of
        .get(app.main_selection)
        .map_or(0, |line| scroll_offset(*line, window));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
        content,
    );
}

fn loading_line(target: &str, theme: &Theme, frame: u64) -> Line<'static> {
    let spinner = if theme.motion.is_reduced() {
        theme.glyphs.loading
    } else {
        theme.glyphs.spinner[(frame % 4) as usize]
    };
    Line::from(vec![
        Span::styled(
            format!("{spinner} LOADING "),
            Style::new().fg(theme.warn).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            truncate_to_width(&clean(target), 48, theme.glyphs.ellipsis),
            Style::new().fg(theme.muted),
        ),
    ])
}

fn album_line(
    theme: &Theme,
    album: &lekima_domain::Album,
    index: usize,
    selected: bool,
    title_width: usize,
) -> Line<'static> {
    let marker = if selected { theme.glyphs.active } else { " " };
    let year = album
        .year
        .map_or_else(|| "-".to_owned(), |year| format!("{year:>4}"));
    let tracks = album.track_count.map_or_else(
        || "-".to_owned(),
        |count| format!("{count} {}", tracks_word(count as usize)),
    );
    let title = truncate_to_width(&clean(&album.name), title_width, theme.glyphs.ellipsis);
    let pad = " ".repeat(title_width.saturating_sub(display_width(&title)) + 2);
    let primary = if selected {
        selection_style(theme)
    } else {
        Style::new().fg(theme.text)
    };
    let secondary = if selected {
        selection_style(theme)
    } else {
        Style::new().fg(theme.muted)
    };
    Line::from(vec![
        Span::styled(
            format!("{marker} "),
            Style::new().fg(if selected { theme.accent } else { theme.dim }),
        ),
        Span::styled(
            format!("{:>3} ", index + 1),
            if selected {
                primary
            } else {
                Style::new().fg(theme.muted)
            },
        ),
        Span::styled(format!("{year}  "), secondary),
        Span::styled(title, primary),
        Span::styled(format!("{pad}{tracks}"), secondary),
    ])
}

fn draw_album(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let focused = app.focus == Focus::Main;
    let name = app
        .album
        .detail
        .as_ref()
        .map_or_else(|| app.source.clone(), |detail| detail.album.name.clone());
    let id = app.album.id.map(|id| id.to_string()).unwrap_or_default();
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("ALBUM", Style::new().fg(theme.muted)),
    ];
    if !id.is_empty() {
        title.push(Span::styled(
            format!(" {} {id}", theme.glyphs.separator),
            Style::new().fg(theme.muted),
        ));
    }
    title.push(Span::styled(
        format!(" {} ", theme.glyphs.separator),
        Style::new().fg(theme.dim),
    ));
    title.push(Span::styled(
        truncate_to_width(&clean(&name), 40, theme.glyphs.ellipsis),
        Style::new().fg(theme.text),
    ));
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }

    if area.height < 6 {
        let mut lines = vec![Line::from(title)];
        if let Some(error) = &app.album.error {
            lines.extend(error_block(error, theme, area.width));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), area);
        return;
    }

    let [label, meta, rule, table_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    let mut meta_lines: Vec<Line<'static>> = Vec::new();
    if let Some(detail) = app.album.detail.as_ref() {
        let artist = detail.album.artist_name();
        let artist = if artist.is_empty() { "-" } else { artist };
        let year = detail
            .album
            .year
            .map_or_else(|| "-".to_owned(), |year| year.to_string());
        meta_lines.push(Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                format!(
                    "{artist} {} {year} {} {} TRACK{}",
                    theme.glyphs.separator,
                    theme.glyphs.separator,
                    detail.tracks.len(),
                    if detail.tracks.len() == 1 { "" } else { "S" }
                ),
                Style::new().fg(theme.accent_alt),
            ),
        ]));
        if !detail.album.description.is_empty() {
            meta_lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(
                    truncate_to_width(
                        &clean(&detail.album.description),
                        usize::from(meta.width).saturating_sub(2),
                        theme.glyphs.ellipsis,
                    ),
                    Style::new().fg(theme.dim),
                ),
            ]));
        }
    } else if app.album.loading {
        meta_lines.push(loading_line(&app.source, theme, app.frame));
    }
    frame.render_widget(Paragraph::new(Text::from(meta_lines)), meta);
    frame.render_widget(
        Paragraph::new(rule_line(usize::from(rule.width), theme)),
        rule,
    );

    if let Some(error) = &app.album.error {
        frame.render_widget(
            Paragraph::new(Text::from(error_block(error, theme, table_area.width))),
            table_area,
        );
        return;
    }

    let tracks = app.album.tracks();
    app.main_view_height = table_area.height;
    if tracks.is_empty() {
        let line = if app.album.loading {
            loading_line(&app.source, theme, app.frame)
        } else {
            Line::from(vec![
                Span::styled("// ", Style::new().fg(theme.panel)),
                Span::styled(
                    "NO TRACKS",
                    Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  r retry {} esc close", theme.glyphs.separator),
                    Style::new().fg(theme.muted),
                ),
            ])
        };
        frame.render_widget(Paragraph::new(line), table_area);
        return;
    }

    let columns = track_columns(table_area.width);
    let playing_id = app.status.track.as_ref().map(|track| track.id);
    let comfortable = theme.density == Density::Comfortable;
    let mut rows: Vec<Row> = Vec::with_capacity(tracks.len() * 2);
    {
        let view: &App = app;
        for (index, track) in tracks.iter().enumerate() {
            rows.push(track_row(
                view,
                theme,
                track,
                index,
                TrackRowState {
                    current: Some(track.id) == playing_id,
                    past: false,
                    selection: focus_tick(focused, index == app.main_selection, theme),
                },
                &columns,
            ));
            if comfortable {
                rows.push(Row::new(Vec::<Span>::new()));
            }
        }
    }

    let mut header = vec!["", "#", "", "TITLE"];
    if columns.artist.is_some() {
        header.push("ARTIST");
    }
    if columns.time {
        header.push("   TIME ");
    }
    let table = Table::new(rows, columns.constraints())
        .column_spacing(0)
        .header(Row::new(header).style(Style::new().fg(theme.panel)))
        .row_highlight_style(if focused {
            selection_style(theme)
        } else {
            inactive_selection_style(theme)
        });
    let visual_selection = if comfortable {
        app.main_selection * 2
    } else {
        app.main_selection
    };
    let mut state = TableState::default().with_selected(Some(visual_selection));
    frame.render_stateful_widget(table, table_area, &mut state);
}

fn draw_lyrics(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if app.lyrics_state.track().is_none() || area.height < 4 {
        return;
    }

    frame.render_widget(Clear, area);
    let [header, top_rule, body, bottom_rule, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let lines_label = app.lyrics_state.ready().map_or_else(String::new, |lyrics| {
        format!(" {} {} lines", theme.glyphs.separator, lyrics.lines.len())
    });
    let mut header_spans = vec![
        Span::styled(
            format!("// {} LYRICS", theme.glyphs.lyrics),
            Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
        ),
        Span::styled(lines_label, Style::new().fg(theme.muted)),
    ];
    if matches!(app.lyrics_state, LyricsState::Ready { .. }) {
        header_spans.push(Span::styled("  LRC", Style::new().fg(theme.dim)));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), header);
    draw_thin_rule(frame, theme, top_rule);

    let body_area = Rect {
        x: body.x + 1,
        width: body.width.saturating_sub(2),
        ..body
    };
    match &app.lyrics_state {
        LyricsState::Ready { lyrics, .. } => {
            draw_timed_lyrics(frame, app, theme, body_area, lyrics);
        }
        LyricsState::Loading { .. } => {
            let dots = match (app.frame / 5) % 4 {
                0 => "",
                1 => ".",
                2 => "..",
                _ => "...",
            };
            draw_lyrics_placeholder(
                frame,
                theme,
                body_area,
                &format!("loading lyrics{dots}"),
                None,
            );
        }
        LyricsState::Empty { .. } => draw_lyrics_placeholder(
            frame,
            theme,
            body_area,
            "no lyrics for this track",
            Some("synced lyrics unavailable"),
        ),
        LyricsState::Error { message, .. } => {
            draw_lyrics_placeholder(
                frame,
                theme,
                body_area,
                &format!("! {}", clean(message)),
                None,
            );
        }
        LyricsState::Idle => {}
    }

    draw_thin_rule(frame, theme, bottom_rule);
    let has_translation = app
        .lyrics_state
        .ready()
        .is_some_and(|lyrics| lyrics.lines.iter().any(|line| line.translation.is_some()));
    let offset = app.lyrics_offset_ms;
    let duration = app
        .status
        .track
        .as_ref()
        .map_or(0, |track| effective_duration(&app.status, track));
    let footer_line = Line::from(vec![
        Span::styled(
            format!("{offset:+} ms"),
            Style::new().fg(if offset == 0 { theme.muted } else { theme.warn }),
        ),
        Span::styled(
            format!("  {}  ", theme.glyphs.separator),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!("{} / {}", clock(app.status.position_ms), clock(duration)),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!(
                "  {}  [ ] {} L",
                theme.glyphs.separator, theme.glyphs.separator
            ),
            Style::new().fg(theme.muted),
        ),
    ]);
    if has_translation {
        let mut footer_line = footer_line;
        footer_line.spans.push(Span::styled(
            format!("  {}  T:", theme.glyphs.separator),
            Style::new().fg(theme.muted),
        ));
        footer_line.spans.push(Span::styled(
            if app.preferences.lyrics_translation {
                "ON"
            } else {
                "OFF"
            },
            Style::new().fg(if app.preferences.lyrics_translation {
                theme.accent_alt
            } else {
                theme.dim
            }),
        ));
        frame.render_widget(Paragraph::new(footer_line), footer);
        return;
    }
    frame.render_widget(Paragraph::new(footer_line), footer);
}

fn draw_timed_lyrics(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    area: Rect,
    lyrics: &lekima_domain::Lyrics,
) {
    let height = usize::from(area.height);
    if height == 0 {
        return;
    }
    let width = usize::from(area.width).saturating_sub(3);
    let position =
        (i64::try_from(app.status.position_ms).unwrap_or(0) + app.lyrics_offset_ms).max(0) as u64;
    let active = lyrics.active_index(position);
    let show_translation = app.preferences.lyrics_translation;

    let mut visual: Vec<(usize, &str, bool)> = Vec::with_capacity(lyrics.lines.len() * 2);
    for (index, line) in lyrics.lines.iter().enumerate() {
        visual.push((index, line.text.as_str(), false));
        if show_translation && let Some(translation) = &line.translation {
            visual.push((index, translation.as_str(), true));
        }
    }
    if visual.is_empty() {
        return;
    }

    let active_visual =
        active.and_then(|index| visual.iter().rposition(|(lyric, _, _)| *lyric == index));
    let (start, padding) = if visual.len() <= height {
        (0, (height - visual.len()) / 2)
    } else {
        (
            active_visual.map_or(0, |index| index.saturating_sub(height / 2)),
            0,
        )
    };

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    for _ in 0..padding {
        lines.push(Line::from(""));
    }
    for (offset, (lyric_index, text, is_translation)) in
        visual.iter().skip(start).take(height - padding).enumerate()
    {
        let is_active = Some(*lyric_index) == active;
        let style = if *is_translation {
            Style::new().fg(if is_active {
                theme.accent_alt
            } else {
                theme.dim
            })
        } else if is_active {
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else if active.is_some_and(|current| *lyric_index < current) {
            Style::new().fg(theme.dim)
        } else {
            Style::new().fg(theme.muted)
        };
        let marker = if is_active && !is_translation {
            format!("{} ", theme.glyphs.active)
        } else if *is_translation {
            "    ".to_owned()
        } else {
            "  ".to_owned()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(theme.accent)),
            Span::styled(
                truncate_to_width(text, width.saturating_sub(2), theme.glyphs.ellipsis),
                style,
            ),
        ]));
        let _ = offset;
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_lyrics_placeholder(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    message: &str,
    hint: Option<&str>,
) {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::new().fg(theme.muted),
        )),
    ];
    if let Some(hint) = hint {
        lines.push(Line::from(Span::styled(
            format!("  {hint}"),
            Style::new().fg(theme.dim),
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_thin_rule(frame: &mut Frame, theme: &Theme, area: Rect) {
    let rule = theme.glyphs.rule.repeat(usize::from(area.width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(rule, Style::new().fg(theme.dim)))),
        area,
    );
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

/// Sanitizes only when the text actually contains hidden or control characters.
fn clean(value: &str) -> Cow<'_, str> {
    if value.chars().any(is_hidden) {
        Cow::Owned(sanitize_line(value))
    } else {
        Cow::Borrowed(value)
    }
}

fn is_hidden(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{061c}'
                | '\u{200b}'..='\u{200d}'
                | '\u{feff}'
                | '\u{2060}'
                | '\u{2028}'
                | '\u{2029}'
        )
}

fn truncate_to_width(value: &str, width: usize, ellipsis: &str) -> String {
    if width == 0 {
        return String::new();
    }
    if display_width(value) <= width {
        return value.to_owned();
    }
    let ellipsis_width = display_width(ellipsis);
    if ellipsis_width >= width {
        return ellipsis.chars().take(width).collect();
    }
    let target = width - ellipsis_width;
    let mut truncated = String::new();
    let mut used = 0;
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > target {
            break;
        }
        truncated.push(character);
        used += character_width;
    }
    truncated.push_str(ellipsis);
    truncated
}

/// Greedy width-aware wrap: prefers spaces, hard-breaks long words and CJK.
fn wrap_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current: Vec<char> = Vec::new();
    let mut used = 0usize;

    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width && !current.is_empty() {
            if let Some(position) = current.iter().rposition(|c| *c == ' ') {
                let rest: String = current[position + 1..].iter().collect();
                let mut line: String = current[..position].iter().collect();
                while line.ends_with(' ') {
                    line.pop();
                }
                lines.push(line);
                current = rest.chars().collect();
                used = display_width(&rest);
            } else {
                lines.push(current.iter().collect());
                current.clear();
                used = 0;
            }
        }
        current.push(character);
        used += character_width;
    }

    if !current.is_empty() || lines.is_empty() {
        let mut line: String = current.iter().collect();
        while line.ends_with(' ') {
            line.pop();
        }
        lines.push(line);
    }
    lines
}

/// `1 234` becomes `1.2k`, `44 054` becomes `44k`, and so on.
/// `track`/`tracks` so counts read naturally.
fn tracks_word(count: usize) -> &'static str {
    if count == 1 { "track" } else { "tracks" }
}

fn compact_count(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 1_000_000 {
        let whole = count / 1_000;
        let decimal = (count % 1_000) / 100;
        return if decimal == 0 {
            format!("{whole}k")
        } else {
            format!("{whole}.{decimal}k")
        };
    }
    let whole = count / 1_000_000;
    let decimal = (count % 1_000_000) / 100_000;
    if decimal == 0 {
        format!("{whole}m")
    } else {
        format!("{whole}.{decimal}m")
    }
}

fn comment_date(timestamp_ms: u64) -> Option<String> {
    if timestamp_ms == 0 {
        return None;
    }
    let days = i64::try_from(timestamp_ms / 86_400_000).ok()?;
    let (year, month, day) = civil_from_days(days);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a Gregorian date.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn pad_right(value: &str, width: usize) -> String {
    let mut padded = value.to_owned();
    let missing = width.saturating_sub(display_width(value));
    padded.push_str(&" ".repeat(missing));
    padded
}

/// Listening history: newest first, with how long each track was heard.
fn draw_history(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let total = app.plays.len();

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("// ", Style::new().fg(theme.panel)),
            Span::styled(
                format!("{} HISTORY", theme.glyphs.history),
                Style::new().fg(theme.muted),
            ),
            Span::styled(
                format!(" {} {}", theme.glyphs.separator, total),
                Style::new().fg(theme.accent_alt),
            ),
        ])),
        label,
    );

    if app.plays.is_empty() {
        let (text, colour) = match (&app.plays_error, app.plays_loading) {
            (Some(error), _) => (
                format!(
                    "could not load history: {}{} r to retry",
                    sanitize_line(error),
                    theme.glyphs.separator
                ),
                theme.hot,
            ),
            (None, true) => ("loading plays...".to_owned(), theme.muted),
            (None, false) => {
                frame.render_widget(
                    Paragraph::new(Text::from(empty_lines(
                        "",
                        &format!(
                            "no plays yet {} play something and it shows up here",
                            theme.glyphs.separator
                        ),
                        &[("/", "search the catalogue")],
                        theme,
                    ))),
                    content,
                );
                return;
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, Style::new().fg(colour)))),
            content,
        );
        return;
    }

    let now = now_ms();
    let focused = app.focus == Focus::Main;
    let width = usize::from(content.width);
    let show_played = width >= 44;
    let show_artist = width >= 64;
    let show_kind = width >= 88;
    let title_width = {
        let fixed = 2 + 9 + 2 + usize::from(show_played) * 7 + usize::from(show_kind) * 7;
        let artist = if show_artist { 20 } else { 0 };
        width.saturating_sub(fixed + artist).max(8)
    };

    let mut widths = vec![
        Constraint::Length(2),
        Constraint::Length(9),
        Constraint::Length(2),
        Constraint::Length(u16::try_from(title_width).unwrap_or(u16::MAX)),
    ];
    if show_artist {
        widths.push(Constraint::Length(20));
    }
    if show_played {
        widths.push(Constraint::Length(7));
    }
    if show_kind {
        widths.push(Constraint::Length(7));
    }

    let mut header = vec!["", "      AGE", "", "TITLE"];
    if show_artist {
        header.push("ARTIST");
    }
    if show_played {
        header.push("PLAYED");
    }
    if show_kind {
        header.push("KIND   ");
    }

    let comfortable = theme.density == Density::Comfortable;
    let mut rows: Vec<Row> = Vec::with_capacity(app.plays.len() * 2);
    for (index, entry) in app.plays.iter().enumerate() {
        let selection = focus_tick(focused, index == app.plays_selection, theme);
        let check_colour = if entry.completed {
            theme.accent
        } else {
            theme.dim
        };
        let title = truncate_to_width(
            &sanitize_line(&entry.track.name),
            title_width,
            theme.glyphs.ellipsis,
        );
        let mut cells = vec![
            Span::styled(selection.0, Style::new().fg(selection.1)),
            Span::styled(
                format!("{:>8} ", relative_age(entry.started_at, now)),
                Style::new().fg(theme.muted),
            ),
            Span::styled(
                if entry.completed {
                    format!("{} ", theme.glyphs.completed)
                } else {
                    "  ".to_owned()
                },
                Style::new().fg(check_colour),
            ),
            Span::styled(
                title,
                Style::new().fg(if entry.completed {
                    theme.text
                } else {
                    theme.muted
                }),
            ),
        ];
        if show_artist {
            cells.push(Span::styled(
                truncate_to_width(
                    &sanitize_line(&entry.track.artist_names()),
                    20,
                    theme.glyphs.ellipsis,
                ),
                Style::new().fg(theme.muted),
            ));
        }
        if show_played {
            cells.push(Span::styled(
                format!("{:>6}", entry.played_label()),
                Style::new().fg(theme.muted),
            ));
        }
        if show_kind {
            cells.push(Span::styled(
                format!("{:<6} ", entry.kind.as_str()),
                Style::new().fg(theme.muted),
            ));
        }
        rows.push(Row::new(cells));
        if comfortable {
            rows.push(Row::new(Vec::<Span>::new()));
        }
    }

    let table = Table::new(rows, widths)
        .column_spacing(0)
        .header(Row::new(header).style(Style::new().fg(theme.panel)))
        .row_highlight_style(if focused {
            selection_style(theme)
        } else {
            inactive_selection_style(theme)
        });
    let visual_selection = if comfortable {
        app.plays_selection * 2
    } else {
        app.plays_selection
    };
    let mut state = TableState::default().with_selected(Some(visual_selection));
    frame.render_stateful_widget(table, content, &mut state);
}

/// Relative age of a Unix-millisecond timestamp.
fn relative_age(started_at_ms: i64, now_ms: i64) -> String {
    let elapsed = now_ms.saturating_sub(started_at_ms);
    if elapsed < 60_000 {
        "just now".to_owned()
    } else if elapsed < 3_600_000 {
        format!("{}m", elapsed / 60_000)
    } else if elapsed < 86_400_000 {
        format!("{}h", elapsed / 3_600_000)
    } else {
        format!("{}d", elapsed / 86_400_000)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn draw_queue(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let total = app.queue.len();
    let visible = usize::from(content.height.saturating_sub(1));
    app.queue_view_height = u16::try_from(visible).unwrap_or(u16::MAX);
    let position = if total > visible && total > 0 {
        format!(
            " {} {}/{}",
            theme.glyphs.separator,
            app.queue_selection.saturating_add(1).min(total),
            total
        )
    } else {
        String::new()
    };
    let remaining = queue_remaining(
        &app.queue,
        app.status.queue_index,
        app.status.position_ms,
        &app.status,
    );
    let remaining_label = if remaining > 0 && label.width >= 44 {
        format!(" {} {} left", theme.glyphs.separator, clock(remaining))
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("// ", Style::new().fg(theme.panel)),
            Span::styled(
                format!("{} QUEUE", theme.glyphs.queue),
                Style::new().fg(theme.muted),
            ),
            Span::styled(
                format!(
                    " {} {}{}{}",
                    theme.glyphs.separator, total, position, remaining_label
                ),
                Style::new().fg(theme.accent_alt),
            ),
            Span::styled(
                format!(" {}", theme.glyphs.active),
                Style::new().fg(theme.accent),
            ),
        ])),
        label,
    );

    if app.queue.is_empty() {
        frame.render_widget(
            Paragraph::new(Text::from(empty_lines(
                "",
                "nothing queued",
                &[
                    ("/", "search the catalogue"),
                    ("Enter", "to play the selection"),
                ],
                theme,
            ))),
            content,
        );
        return;
    }

    let columns = track_columns(content.width);
    let comfortable = theme.density == Density::Comfortable;
    let mut rows: Vec<Row> = Vec::with_capacity(app.queue.len() * 2);
    for (index, track) in app.queue.iter().enumerate() {
        rows.push(track_row(
            app,
            theme,
            track,
            index,
            TrackRowState {
                current: Some(index) == app.status.queue_index,
                past: app
                    .status
                    .queue_index
                    .is_some_and(|current| index < current),
                selection: focus_tick(true, index == app.queue_selection, theme),
            },
            &columns,
        ));
        if comfortable {
            rows.push(Row::new(Vec::<Span>::new()));
        }
    }

    let mut header = vec!["", "#", "", "TITLE"];
    if columns.artist.is_some() {
        header.push("ARTIST");
    }
    if columns.time {
        header.push("   TIME ");
    }
    let table = Table::new(rows, columns.constraints())
        .column_spacing(0)
        .header(Row::new(header).style(Style::new().fg(theme.panel)))
        .row_highlight_style(selection_style(theme));
    let visual_selection = if comfortable {
        app.queue_selection * 2
    } else {
        app.queue_selection
    };
    let mut state = TableState::default().with_selected(Some(visual_selection));
    frame.render_stateful_widget(table, content, &mut state);
}

/// The scale the core used for `buffered`, so the rail and the shading agree.
fn effective_duration(state: &PlaybackState, track: &Track) -> u64 {
    state
        .stream_duration_ms
        .filter(|stream| *stream > 0)
        .unwrap_or(track.duration_ms)
}

fn is_preview(state: &PlaybackState, track: &Track) -> bool {
    effective_duration(state, track) < track.duration_ms
}

fn queue_remaining(
    queue: &[lekima_domain::Track],
    current: Option<usize>,
    position_ms: u64,
    state: &PlaybackState,
) -> u64 {
    let start = current.unwrap_or(0).min(queue.len());
    queue
        .iter()
        .enumerate()
        .skip(start)
        .map(|(index, track)| {
            let duration = if Some(index) == current {
                effective_duration(state, track)
            } else {
                track.duration_ms
            };
            if Some(index) == current {
                duration.saturating_sub(position_ms)
            } else {
                duration
            }
        })
        .fold(0_u64, u64::saturating_add)
}

fn draw_comments(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, rule, content] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    let track_name = app
        .status
        .track
        .as_ref()
        .map_or_else(String::new, |track| sanitize_line(&track.name));
    let snapshot = app
        .comments_state
        .ready()
        .map(|comments| (comments.total, comments.has_more));
    let (total, has_more) = snapshot.unwrap_or((0, false));

    let mut right: Vec<Span<'static>> = Vec::new();
    if has_more {
        right.push(Span::styled(
            format!("{} MORE ", theme.glyphs.more),
            Style::new().fg(theme.warn),
        ));
    }
    let right_width: usize = right.iter().map(|span| display_width(&span.content)).sum();

    let mut header = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("COMMENTS", Style::new().fg(theme.muted)),
        Span::styled(
            format!(" {} {}", theme.glyphs.separator, compact_count(total)),
            Style::new().fg(theme.accent_alt),
        ),
    ];
    let fixed_width: usize = header.iter().map(|span| display_width(&span.content)).sum();
    let header_width = usize::from(label.width).saturating_sub(1);
    let room = header_width.saturating_sub(fixed_width + right_width + 1);
    if room >= 6 {
        let separator = format!(" {} ", theme.glyphs.separator);
        header.push(Span::styled(separator.clone(), Style::new().fg(theme.dim)));
        header.push(Span::styled(
            truncate_to_width(
                &track_name,
                room.saturating_sub(display_width(&separator)),
                theme.glyphs.ellipsis,
            ),
            Style::new().fg(theme.dim),
        ));
    }
    let left_width: usize = header.iter().map(|span| display_width(&span.content)).sum();
    let gap = header_width.saturating_sub(left_width + right_width);
    if right_width > 0 && left_width + right_width <= header_width {
        header.push(Span::styled(" ".repeat(gap), Style::new()));
        header.extend(right);
    }
    frame.render_widget(Paragraph::new(Line::from(header)), label);
    draw_thin_rule(frame, theme, rule);

    let width = usize::from(content.width).saturating_sub(1).max(1);
    let mut lines: Vec<Line> = Vec::new();
    match &app.comments_state {
        CommentsState::Idle | CommentsState::Loading { .. } => {
            let spinner = spinner_glyph(app, theme);
            lines.push(Line::from(vec![
                Span::styled(format!("{spinner} "), Style::new().fg(theme.accent)),
                Span::styled(
                    "LOADING COMMENTS",
                    Style::new().fg(theme.muted).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(
                    truncate_to_width(
                        &format!("fetching {track_name}"),
                        width.saturating_sub(2),
                        theme.glyphs.ellipsis,
                    ),
                    Style::new().fg(theme.dim),
                ),
            ]));
        }
        CommentsState::Empty { .. } => {
            lines.push(Line::from(vec![
                Span::styled("// ", Style::new().fg(theme.panel)),
                Span::styled(
                    "NO COMMENTS",
                    Style::new().fg(theme.muted).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(
                    "nobody has commented on this track yet",
                    Style::new().fg(theme.dim),
                ),
            ]));
        }
        CommentsState::Error { message, .. } => {
            lines.push(Line::from(vec![
                Span::styled(
                    "! ",
                    Style::new().fg(theme.hot).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "COMMENTS UNAVAILABLE",
                    Style::new().fg(theme.hot).add_modifier(Modifier::BOLD),
                ),
            ]));
            for chunk in wrap_to_width(&sanitize_line(message), width.saturating_sub(2)) {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::new()),
                    Span::styled(chunk, Style::new().fg(theme.text)),
                ]));
            }
            lines.push(Line::from(vec![
                Span::styled("  r ", Style::new().fg(theme.accent)),
                Span::styled("retry", Style::new().fg(theme.dim)),
                Span::styled("   C ", Style::new().fg(theme.accent)),
                Span::styled("close", Style::new().fg(theme.dim)),
            ]));
        }
        CommentsState::Ready { comments, .. } => {
            append_comment_section(
                &mut lines,
                "HOT",
                u64::try_from(comments.hot.len()).unwrap_or(u64::MAX),
                "MOST LIKED",
                &comments.hot,
                width,
                theme,
            );
            append_comment_section(
                &mut lines,
                "ALL",
                comments
                    .total
                    .max(u64::try_from(comments.comments.len()).unwrap_or(0)),
                &format!("{} LOADED", comments.comments.len()),
                &comments.comments,
                width,
                theme,
            );
            if comments.has_more {
                if !lines.is_empty() {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", theme.glyphs.more),
                        Style::new().fg(theme.warn),
                    ),
                    Span::styled(
                        "MORE COMMENTS",
                        Style::new().fg(theme.warn).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} n loads the next page", theme.glyphs.separator),
                        Style::new().fg(theme.dim),
                    ),
                ]));
            }
        }
    }

    let height = usize::from(content.height);
    let max_scroll = lines.len().saturating_sub(height);
    app.comments_scroll = app.comments_scroll.min(max_scroll);
    app.comments_view_height = content.height;
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .scroll((u16::try_from(app.comments_scroll).unwrap_or(u16::MAX), 0)),
        content,
    );
}

fn append_comment_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    count: u64,
    note: &str,
    comments: &[lekima_domain::Comment],
    width: usize,
    theme: &Theme,
) {
    if comments.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    let left = format!(
        "// {title} {} {}",
        theme.glyphs.separator,
        compact_count(count)
    );
    let mut spans = vec![Span::styled(
        left,
        Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
    )];
    let pad = width.saturating_sub(display_width(&spans[0].content) + display_width(note));
    if !note.is_empty() && pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::new()));
        spans.push(Span::styled(note.to_owned(), Style::new().fg(theme.muted)));
    }
    lines.push(Line::from(spans));

    for (index, comment) in comments.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        append_comment(lines, comment, width, theme);
    }
}

fn append_comment(
    lines: &mut Vec<Line<'static>>,
    comment: &lekima_domain::Comment,
    width: usize,
    theme: &Theme,
) {
    let author = if comment.author.is_empty() {
        "anonymous".to_owned()
    } else {
        sanitize_line(&comment.author)
    };
    let deleted = comment.content.trim().is_empty();
    if comment.author.trim().is_empty() && deleted {
        lines.push(Line::from(vec![
            Span::styled(" ".repeat(COMMENT_INDENT), Style::new()),
            Span::styled(
                "[deleted]",
                Style::new().fg(theme.dim).add_modifier(Modifier::ITALIC),
            ),
        ]));
        return;
    }
    let likes = (comment.liked_count > 0).then(|| {
        format!(
            "{} {}",
            theme.glyphs.liked,
            compact_count(comment.liked_count)
        )
    });
    let date = comment_date(comment.timestamp_ms);
    let has_likes = likes.is_some();
    let has_date = date.is_some();
    let separator = format!(" {} ", theme.glyphs.separator);
    let meta_width = likes.as_ref().map_or(0, |value| display_width(value))
        + date.as_ref().map_or(0, |value| display_width(value))
        + if has_likes && has_date {
            display_width(&separator)
        } else {
            0
        };
    let author_width = width
        .saturating_sub(COMMENT_INDENT + if meta_width == 0 { 0 } else { meta_width + 2 })
        .max(4);
    let author = truncate_to_width(&author, author_width, theme.glyphs.ellipsis);

    let mut header = vec![
        Span::styled(" ".repeat(COMMENT_INDENT), Style::new()),
        Span::styled(
            author,
            Style::new()
                .fg(theme.accent_alt)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if meta_width > 0 {
        let used = COMMENT_INDENT + display_width(&header[1].content);
        let pad = width.saturating_sub(used + meta_width).max(1);
        header.push(Span::styled(" ".repeat(pad), Style::new()));
        if let Some(likes) = likes {
            header.push(Span::styled(
                likes,
                Style::new().fg(theme.hot).add_modifier(Modifier::BOLD),
            ));
        }
        if has_likes && has_date {
            header.push(Span::styled(separator, Style::new().fg(theme.dim)));
        }
        if let Some(date) = date {
            header.push(Span::styled(date, Style::new().fg(theme.muted)));
        }
    }
    lines.push(Line::from(header));

    let content_width = width.saturating_sub(COMMENT_INDENT).max(1);
    let content = if deleted {
        "[deleted]".to_owned()
    } else {
        sanitize_line(&comment.content)
    };
    let style = if deleted {
        Style::new().fg(theme.dim).add_modifier(Modifier::ITALIC)
    } else {
        Style::new().fg(theme.text)
    };
    for chunk in wrap_to_width(&content, content_width) {
        lines.push(Line::from(vec![
            Span::styled(" ".repeat(COMMENT_INDENT), Style::new()),
            Span::styled(chunk, style),
        ]));
    }

    for reply in &comment.replies {
        let author = if reply.author.is_empty() {
            "anonymous".to_owned()
        } else {
            sanitize_line(&reply.author)
        };
        let deleted = reply.content.trim().is_empty();
        let content = if deleted {
            "[deleted]".to_owned()
        } else {
            sanitize_line(&reply.content)
        };
        let body = if reply.author.trim().is_empty() && deleted {
            "[deleted]".to_owned()
        } else {
            format!("{author}: {content}")
        };
        let connector = format!(" {} ", theme.glyphs.reply);
        let reply_width = width
            .saturating_sub(COMMENT_INDENT + display_width(&connector))
            .max(1);
        for (index, chunk) in wrap_to_width(&body, reply_width).into_iter().enumerate() {
            let prefix = if index == 0 {
                format!("{}{connector}", " ".repeat(COMMENT_INDENT))
            } else {
                " ".repeat(COMMENT_INDENT + display_width(&connector))
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::new().fg(theme.dim)),
                Span::styled(
                    chunk,
                    Style::new().fg(theme.muted).add_modifier(if deleted {
                        Modifier::ITALIC
                    } else {
                        Modifier::empty()
                    }),
                ),
            ]));
        }
    }
}

fn draw_footer(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let (mode, hints) = match app.mode {
        Mode::Normal => ("NORMAL", normal_hints(app, theme, area.width)),
        Mode::Search => ("SEARCH", String::new()),
        Mode::Command => ("CMD", String::new()),
        Mode::Help => (
            "HELP",
            format!(
                "?/esc close {} j/k scroll {} ctrl-d/u page",
                theme.glyphs.separator, theme.glyphs.separator
            ),
        ),
        Mode::Queue => ("QUEUE", queue_hints(theme, area.width)),
        Mode::History => (
            "HISTORY",
            format!(
                "j/k nav {} enter replay {} r refresh {} H/esc close",
                theme.glyphs.separator, theme.glyphs.separator, theme.glyphs.separator
            ),
        ),
        Mode::Comments => (
            "COMMENTS",
            format!(
                "j/k scroll {0} ctrl-d/u page {0} n more {0} r refresh {0} C/esc close",
                theme.glyphs.separator
            ),
        ),
        Mode::Settings => (
            "UI",
            format!(
                "j/k select {} h/l change {} esc close",
                theme.glyphs.separator, theme.glyphs.separator
            ),
        ),
    };

    let mut spans = vec![
        Span::styled(
            format!("[{mode}]"),
            Style::new()
                .fg(theme.badge_fg)
                .bg(theme.badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::new()),
    ];

    if app.mode == Mode::Search {
        // The category rail sits between the badge and the query, so the
        // filter is never more than one glance from the caret.
        spans.extend(search_rail_spans(app, theme));
        spans.push(Span::styled(
            format!("{} ", theme.glyphs.divider),
            Style::new().fg(theme.dim),
        ));
        spans.extend(prompt_spans(app, theme));
        if area.width >= 104 {
            spans.push(Span::styled(
                format!("  tab filter {} ctrl-1..5", theme.glyphs.separator),
                Style::new().fg(theme.muted),
            ));
        }
    } else if app.mode == Mode::Command {
        spans.push(Span::styled(":", Style::new().fg(theme.accent)));
        spans.extend(prompt_spans(app, theme));
        if area.width >= 90 {
            spans.push(Span::styled(
                format!("  ctrl-w word {} up history", theme.glyphs.separator),
                Style::new().fg(theme.muted),
            ));
        }
    } else if app.mode == Mode::Queue
        && let Some(index) = app.queue_confirm
    {
        spans.push(Span::styled(
            format!("remove {:03}? [y/N]", index + 1),
            Style::new().fg(theme.warn).add_modifier(Modifier::BOLD),
        ));
    } else {
        if let Some(count) = app.pending_count {
            spans.push(Span::styled(
                format!(" {count}"),
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {} ", theme.glyphs.separator),
                Style::new().fg(theme.dim),
            ));
        }
        spans.push(Span::styled(hints, Style::new().fg(theme.muted)));
        if app.lyrics_open && area.width >= 80 {
            spans.push(Span::styled(
                format!(" {} L close", theme.glyphs.separator),
                Style::new().fg(theme.muted),
            ));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn normal_hints(app: &App, theme: &Theme, width: u16) -> String {
    let enter = "enter";
    let sep = theme.glyphs.separator;
    let q = "Q";
    if app.view == View::Search {
        let filter = "c-1..5 filter";
        return if width >= 92 {
            format!(
                "j/k {sep} {enter} open {sep} a queue {sep} / refine {sep} {filter} {sep} esc back {sep} ? keys"
            )
        } else if width >= 72 {
            format!("j/k {sep} {enter} {sep} a queue {sep} / refine {sep} {filter} {sep} esc")
        } else if width >= 52 {
            format!("j/k {sep} {enter} {sep} / {sep} {filter} {sep} esc")
        } else {
            format!("j/k {sep} {enter} {sep} / {sep} esc")
        };
    }
    if app.view == View::Artist {
        return if width >= 76 {
            format!(
                "j/k {sep} {enter} album {sep} a queue {sep} n more {sep} r refresh {sep} esc back"
            )
        } else if width >= 48 {
            format!("j/k {sep} {enter} {sep} a queue {sep} n more {sep} esc")
        } else {
            format!("j/k {sep} {enter} {sep} esc")
        };
    }
    if app.view == View::Album {
        return if width >= 72 {
            format!(
                "j/k {sep} {enter} play {sep} a queue {sep} r refresh {sep} esc back {sep} ? keys"
            )
        } else if width >= 48 {
            format!("j/k {sep} {enter} play {sep} a queue {sep} esc")
        } else {
            format!("j/k {sep} {enter} {sep} esc")
        };
    }
    if width >= 108 {
        format!(
            "j/k {sep} {enter} play {sep} space {sep} {q} queue {sep} S theme {sep} / search {sep} : cmd {sep} ctrl-o/i hist {sep} ? keys"
        )
    } else if width >= 80 {
        format!("j/k {sep} {enter} play {sep} space {sep} / search {sep} {q} queue {sep} ? keys")
    } else if width >= 52 {
        format!("j/k {sep} {enter} {sep} / {sep} {q} {sep} ?")
    } else if app.pending_count.is_some() {
        "digits, then j/k or G".to_owned()
    } else {
        format!("j/k {sep} / {sep} ?")
    }
}

fn queue_hints(theme: &Theme, width: u16) -> String {
    let enter = "enter";
    let sep = theme.glyphs.separator;
    if width >= 100 {
        format!(
            "j/k nav {sep} space pause {sep} n/p next {sep} r repeat {sep} s shuffle {sep} x remove {sep} {enter} jump {sep} esc"
        )
    } else if width >= 72 {
        format!("j/k nav {sep} space pause {sep} n/p {sep} x remove {sep} {enter} jump {sep} esc")
    } else {
        format!("j/k {sep} space {sep} x remove {sep} esc")
    }
}

fn prompt_spans<'a>(app: &App, theme: &Theme) -> Vec<Span<'a>> {
    let cursor = app.cursor.min(app.input.len());
    let (before, rest) = app.input.split_at(cursor);
    let mut chars = rest.chars();
    let at = chars.next();
    let after = chars.as_str();
    let blink = theme.motion.is_reduced() || (app.frame / CURSOR_BLINK_FRAMES).is_multiple_of(2);
    let mut spans = vec![Span::styled(before.to_owned(), Style::new().fg(theme.text))];
    match at {
        Some(character) => {
            let style = if blink {
                Style::new()
                    .fg(theme.badge_fg)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text)
            };
            spans.push(Span::styled(character.to_string(), style));
            spans.push(Span::styled(after.to_owned(), Style::new().fg(theme.text)));
        }
        None => spans.push(Span::styled(
            if blink { theme.glyphs.cursor } else { " " },
            Style::new().fg(theme.accent),
        )),
    }
    spans
}

/// Console alert: a bordered panel in the bottom-right, above the command bar.
///
/// Severity is carried by the border and title (`! ERROR`, `+ NOTICE`,
/// `! OFFLINE`); the message itself stays in readable `text`, not reversed
/// colour. Notices are one line, errors up to two.
fn draw_toast(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    area: Rect,
    lyrics: Option<LyricsPlacement>,
) {
    let message = app
        .message
        .as_ref()
        .filter(|message| message.is_visible(app.frame));

    let (text, is_error, title, colour) = if app.daemon_down {
        (
            format!("daemon offline {} press R to start", theme.glyphs.separator),
            true,
            "! OFFLINE",
            theme.warn,
        )
    } else {
        match message {
            Some(message) if message.error => (message.text.clone(), true, "! ERROR", theme.hot),
            Some(message) => (message.text.clone(), false, "+ NOTICE", theme.accent),
            None => return,
        }
    };

    if area.height < 6 || area.width < 30 {
        if is_error || app.daemon_down {
            let line = truncate_to_width(
                &format!("! {text}"),
                usize::from(area.width),
                theme.glyphs.ellipsis,
            );
            let row = Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(1),
                width: area.width,
                height: 1,
            };
            frame.render_widget(Clear, row);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    line,
                    Style::new().fg(colour).add_modifier(Modifier::BOLD),
                ))),
                row,
            );
        }
        return;
    }

    // Keep the alert inside the main pane: never over the sidebar.
    let min_x = if area.width >= SIDEBAR_BREAKPOINT {
        SIDEBAR_WIDTH + 1
    } else {
        0
    };
    let width = TOAST_MAX_WIDTH
        .min(area.width.saturating_sub(2))
        .min(area.width.saturating_sub(min_x));
    let inner = usize::from(width.saturating_sub(4));
    let cleaned = clean(&text);
    let mut body = wrap_to_width(&cleaned, inner);
    let max_lines = if is_error { 2 } else { 1 };
    if body.len() > max_lines {
        body.truncate(max_lines);
        if let Some(last) = body.last_mut() {
            let head = truncate_to_width(
                last,
                inner.saturating_sub(display_width(theme.glyphs.ellipsis)),
                "",
            );
            *last = format!("{head}{}", theme.glyphs.ellipsis);
        }
    }
    let height = u16::try_from(body.len()).unwrap_or(1).saturating_add(2);

    let mut x = area.width.saturating_sub(width + 1).max(min_x);
    let mut y = area.height.saturating_sub(height + 2).max(1);
    match lyrics {
        Some(LyricsPlacement::Side(rect)) => {
            x = x.min(rect.x.saturating_sub(width + 1)).max(min_x);
        }
        Some(LyricsPlacement::Bottom(rect)) => {
            y = y.min(rect.y.saturating_sub(height + 1)).max(1);
        }
        None => {}
    }
    let toast = Rect {
        x,
        y,
        width,
        height,
    };

    let lines: Vec<Line> = body
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let prefix = if index == 0 {
                format!("{} ", theme.glyphs.active)
            } else {
                "  ".to_owned()
            };
            Line::from(vec![
                Span::styled(prefix, Style::new().fg(colour).add_modifier(Modifier::BOLD)),
                Span::styled(chunk, Style::new().fg(theme.text)),
            ])
        })
        .collect();
    let block = Block::bordered()
        .border_set(overlay_border(theme))
        .border_style(Style::new().fg(colour))
        .title(Line::from(Span::styled(
            format!("{}{title} ", if theme.glyphs.ascii { "- " } else { "═ " }),
            Style::new().fg(colour).add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::horizontal(1));

    frame.render_widget(Clear, toast);
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), toast);
}

fn wave_spans<'a>(app: &App, theme: &Theme) -> Vec<Span<'a>> {
    let bars = theme.glyphs.wave;
    let wave = &app.status.wave;
    let colour_for = |index: usize| {
        if index + 1 < SPECTRUM_BARS / 2 {
            theme.accent
        } else {
            theme.accent_alt
        }
    };

    if wave.is_empty() {
        return vec![Span::styled(
            bars[0].repeat(HUD_SPECTRUM_BARS),
            Style::new().fg(theme.dim),
        )];
    }

    // Compact HUD strip: sample the 20 bands down to 12 columns.
    let step = wave.len().max(1) as f32 / HUD_SPECTRUM_BARS as f32;
    let mut spans: Vec<Span> = (0..HUD_SPECTRUM_BARS)
        .map(|column| {
            let start = (column as f32 * step) as usize;
            let end = (((column + 1) as f32 * step) as usize).max(start + 1);
            let level = wave[start..end.min(wave.len())]
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            let bar = bars[((level.clamp(0.0, 1.0) * 7.0).round() as usize).min(7)];
            Span::styled(bar, Style::new().fg(colour_for(column)))
        })
        .collect();
    let missing = HUD_SPECTRUM_BARS.saturating_sub(spans.len());
    if missing > 0 {
        spans.push(Span::styled(
            bars[0].repeat(missing),
            Style::new().fg(theme.dim),
        ));
    }
    spans
}

fn draw_settings(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let popup = centred_box(area, 58, 9);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_set(overlay_border(theme))
        .border_style(Style::new().fg(theme.overlay))
        .padding(Padding::horizontal(1))
        .title(Span::styled(
            overlay_title("UI", theme),
            Style::new().fg(theme.overlay).add_modifier(Modifier::BOLD),
        ));

    let rows = [
        ("THEME", app.preferences.theme.clone()),
        ("GLYPHS", app.preferences.glyphs.clone()),
        ("DENSITY", app.preferences.density.clone()),
        (
            "TRANSLATION",
            if app.preferences.lyrics_translation {
                "on".to_owned()
            } else {
                "off".to_owned()
            },
        ),
    ];
    let mut lines: Vec<Line> = Vec::new();
    for (index, (label, value)) in rows.iter().take(SETTINGS_ROWS).enumerate() {
        let selected = index == app.settings_selection;
        let style = if selected {
            Style::new()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text)
        };
        let swatch = if index == 0 {
            let chosen = Theme::resolve_with(
                value,
                &app.preferences.glyphs,
                &app.preferences.density,
                None,
                false,
                false,
            );
            Span::styled(
                format!("{} ", theme.glyphs.meter_full.repeat(2)),
                Style::new().fg(chosen.accent),
            )
        } else {
            Span::styled("   ", Style::new())
        };
        lines.push(Line::from(vec![
            Span::styled(if selected { theme.glyphs.active } else { " " }, style),
            Span::styled(format!(" {label:<12}"), style),
            swatch,
            Span::styled("< ", Style::new().fg(theme.dim)),
            Span::styled(value.clone(), style),
            Span::styled(" >", Style::new().fg(theme.dim)),
        ]));
    }
    lines.push(Line::from(""));
    let hint = if popup.width < 53 {
        format!("h/l change {} esc close", theme.glyphs.separator)
    } else {
        format!(
            "h/l change {} j/k select {} esc close {} saved on exit",
            theme.glyphs.separator, theme.glyphs.separator, theme.glyphs.separator
        )
    };
    lines.push(Line::from(Span::styled(hint, Style::new().fg(theme.muted))));
    let source = if std::env::var_os("NO_COLOR").is_some() {
        "NO_COLOR (mono)"
    } else if std::env::var_os("LEKIMA_THEME").is_some() {
        "LEKIMA_THEME"
    } else {
        "config.toml"
    };
    lines.push(Line::from(Span::styled(
        format!("source: {source}"),
        Style::new().fg(theme.muted),
    )));

    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), popup);
}

fn draw_help(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let two_columns = area.width >= HELP_TWO_COLUMN_WIDTH && area.height >= 24;
    let width = if two_columns {
        100.min(area.width.saturating_sub(2))
    } else {
        72.min(area.width.saturating_sub(2))
    };
    let height = if two_columns {
        22.min(area.height)
    } else {
        30.min(area.height.saturating_sub(1))
    };
    let popup = centred_box(area, width, height);
    frame.render_widget(Clear, popup);

    let columns_fit = two_columns
        && popup.height.saturating_sub(2) as usize
            >= keymap::column_lines(0).max(keymap::column_lines(1));
    if columns_fit {
        app.help_scroll = 0;
        app.help_max_scroll = 0;
        let block = Block::bordered()
            .border_set(overlay_border(theme))
            .border_style(Style::new().fg(theme.overlay))
            .title(Span::styled(
                overlay_title("KEYS", theme),
                Style::new().fg(theme.overlay).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let [left, divider, right] = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Length(1),
            Constraint::Percentage(50),
        ])
        .areas(inner);
        frame.render_widget(Paragraph::new(Text::from(help_column(0, theme))), left);
        frame.render_widget(
            Paragraph::new(Text::from(
                (0..usize::from(divider.height))
                    .map(|_| {
                        Line::from(Span::styled(
                            theme.glyphs.divider,
                            Style::new().fg(theme.dim),
                        ))
                    })
                    .collect::<Vec<_>>(),
            )),
            divider,
        );
        frame.render_widget(Paragraph::new(Text::from(help_column(1, theme))), right);
        return;
    }

    let lines = help_single_column(theme);
    let total = lines.len();
    let inner_height = usize::from(popup.height.saturating_sub(2));
    app.help_max_scroll = total.saturating_sub(inner_height);
    app.help_scroll = app.help_scroll.min(app.help_max_scroll);
    let title = if app.help_max_scroll > 0 {
        overlay_title(
            &format!(
                "KEYS {} {}/{}",
                theme.glyphs.separator,
                app.help_scroll.saturating_add(1),
                app.help_max_scroll.saturating_add(1)
            ),
            theme,
        )
    } else {
        overlay_title("KEYS", theme)
    };
    let block = Block::bordered()
        .border_set(overlay_border(theme))
        .border_style(Style::new().fg(theme.overlay))
        .title(Span::styled(
            title,
            Style::new().fg(theme.overlay).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .scroll((u16::try_from(app.help_scroll).unwrap_or(u16::MAX), 0)),
        inner,
    );
}

fn overlay_border(theme: &Theme) -> ratatui::symbols::border::Set<'static> {
    if theme.glyphs.ascii {
        ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        }
    } else {
        ratatui::symbols::border::DOUBLE
    }
}

fn overlay_title(text: &str, theme: &Theme) -> String {
    if theme.glyphs.ascii {
        text.to_owned()
    } else {
        format!("▓ {text} ▓")
    }
}

fn help_column(column: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for section in keymap::SECTIONS
        .iter()
        .filter(|section: &&Section| section.column == column)
    {
        lines.push(help_header(section, theme));
        lines.extend(help_bindings(section, theme));
    }
    lines
}

fn help_single_column(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (position, section) in keymap::SECTIONS.iter().enumerate() {
        if position > 0 {
            lines.push(Line::from(""));
        }
        lines.push(help_header(section, theme));
        lines.extend(help_bindings(section, theme));
    }
    lines
}

fn help_header(section: &Section, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!("// {}", section.title),
        Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
    ))
}

fn help_bindings<'a>(
    section: &'a Section,
    theme: &'a Theme,
) -> impl Iterator<Item = Line<'static>> + 'a {
    section.bindings.iter().map(|binding| {
        Line::from(vec![
            Span::styled(
                format!("  {} ", pad_right(binding.key, 13)),
                Style::new().fg(theme.accent_alt),
            ),
            Span::styled(binding.description, Style::new().fg(theme.text)),
        ])
    })
}

fn centred_box(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn progress_spans<'a>(app: &App, duration: u64, theme: &Theme) -> Vec<Span<'a>> {
    const CELLS: usize = HUD_PROGRESS_CELLS;
    let ratio = if duration == 0 {
        0.0
    } else {
        (app.status.position_ms as f64 / duration as f64).clamp(0.0, 1.0)
    };
    let played = ((ratio * CELLS as f64).round() as usize).min(CELLS - 1);
    let mut spans = Vec::with_capacity(CELLS);
    for cell in 0..CELLS {
        if cell == played {
            spans.push(Span::styled(
                theme.glyphs.progress_head,
                Style::new()
                    .fg(theme.accent_alt)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        let start_ms = u128::from(duration) * cell as u128 / CELLS as u128;
        let end_ms = u128::from(duration) * (cell + 1) as u128 / CELLS as u128;
        let buffered = app.status.buffered.iter().any(|range| {
            u128::from(range.start_ms) < end_ms && u128::from(range.end_ms) > start_ms
        });
        let (glyph, colour) = if cell < played {
            (theme.glyphs.progress_full, theme.meter_full)
        } else if buffered {
            (theme.glyphs.progress_buffered, theme.panel)
        } else {
            (theme.glyphs.progress_empty, theme.meter_empty)
        };
        spans.push(Span::styled(glyph, Style::new().fg(colour)));
    }
    spans
}

fn volume_bar(volume: f32, theme: &Theme) -> String {
    let filled = (volume.clamp(0.0, 1.0) * VOLUME_CELLS as f32).round() as usize;
    format!(
        "{}{}",
        theme.glyphs.meter_full.repeat(filled),
        theme.glyphs.meter_empty.repeat(VOLUME_CELLS - filled)
    )
}

fn selection_style(theme: &Theme) -> Style {
    Style::new()
        .fg(theme.selection_fg)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD)
}

fn inactive_selection_style(theme: &Theme) -> Style {
    Style::new().fg(theme.text).bg(theme.meter_empty)
}

const fn glyph(status: PlaybackStatus, theme: &Theme) -> &'static str {
    match status {
        PlaybackStatus::Playing => theme.glyphs.playing,
        PlaybackStatus::Paused => theme.glyphs.paused,
        PlaybackStatus::Stopped => theme.glyphs.stopped,
    }
}

const fn state_colour(status: PlaybackStatus, theme: &Theme) -> ratatui::style::Color {
    match status {
        PlaybackStatus::Playing => theme.accent,
        PlaybackStatus::Paused => theme.warn,
        PlaybackStatus::Stopped => theme.muted,
    }
}

/// `m:ss`, zero-padded, with hours when needed.
fn clock(ms: u64) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{
        CURSOR_BLINK_FRAMES, comment_date, compact_count, display_width, draw, truncate_to_width,
        wrap_to_width,
    };
    use lekima_domain::PlaybackStatus;

    use crate::app::{AlbumState, App, ArtistState, Focus, Mode, SearchEntry, View};
    use crate::theme::Theme;

    fn render(app: &mut App, width: u16, height: u16) -> String {
        render_with(app, width, height, Theme::cyber())
    }

    fn render_rows(app: &mut App, width: u16, height: u16, theme: Theme) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app, &theme)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn render_with(app: &mut App, width: u16, height: u16, theme: Theme) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app, &theme)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn track(name: &str) -> lekima_domain::Track {
        lekima_domain::Track {
            id: 1.into(),
            name: name.to_owned(),
            artists: vec![lekima_domain::Artist {
                id: 2.into(),
                name: "Russian Circles".to_owned(),
            }],
            album: None,
            duration_ms: 300_000,
        }
    }

    fn play(name: &str, completed: bool) -> lekima_domain::PlayRecord {
        lekima_domain::PlayRecord {
            track: track(name),
            started_at: now_ms() - 90_000,
            played_ms: if completed { 300_000 } else { 30_000 },
            completed,
            kind: lekima_domain::QueueKind::Manual,
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }

    #[test]
    fn the_hud_shows_running_and_invalid_hooks() {
        let mut app = App::new();
        app.status.status = PlaybackStatus::Playing;
        app.status.track = Some(track("Song"));
        app.hooks = vec![
            lekima_ipc::HookStatus {
                name: "nowplaying".to_owned(),
                events: vec!["status".to_owned()],
                capabilities: vec![],
                running: true,
                error: None,
            },
            lekima_ipc::HookStatus {
                name: "bogus".to_owned(),
                events: vec![],
                capabilities: vec![],
                running: false,
                error: Some("bad capability".to_owned()),
            },
        ];

        let screen = render(&mut app, 120, 30);
        assert!(screen.contains("HOOK 1+1!"), "{screen}");
    }

    #[test]
    fn no_hooks_means_no_badge() {
        let mut app = App::new();
        app.status.status = PlaybackStatus::Playing;
        app.status.track = Some(track("Song"));

        let screen = render(&mut app, 120, 30);
        assert!(!screen.contains("HOOK"));
    }

    #[test]
    fn history_pane_lists_plays_and_marks_completion() {
        let mut app = App::new();
        app.mode = Mode::History;
        app.plays = vec![play("Teardrop", true), play("Mota", false)];

        let screen = render(&mut app, 100, 24);

        assert!(screen.contains("HISTORY"));
        assert!(screen.contains("Teardrop"));
        assert!(screen.contains("Mota"));
        assert!(screen.contains('✓'), "completed plays are marked");
        assert!(screen.contains("1m"), "the age column is rendered");
    }

    #[test]
    fn history_pane_explains_an_empty_log() {
        let mut app = App::new();
        app.mode = Mode::History;

        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("no plays yet"));
    }

    #[test]
    fn history_error_is_visible_and_sanitized() {
        let mut app = App::new();
        app.mode = Mode::History;
        app.plays_error = Some("boom\u{1b}[31m".to_owned());

        let screen = render(&mut app, 100, 24);

        assert!(screen.contains("could not load history"));
        assert!(screen.contains("r to retry"));
        assert!(!screen.contains('\u{1b}'));
    }

    #[test]
    fn empty_state_renders_the_brand_and_a_hint() {
        let mut app = App::new();
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("LEKIMA"));
        assert!(screen.contains("NETEASE CLOUD MUSIC"));
        assert!(screen.contains("NOTHING LOADED"));
        assert!(screen.contains("search the catalogue"));
        assert!(screen.contains("browse the library"));
        assert!(screen.contains("█"));
    }

    #[test]
    fn tiny_empty_state_falls_back_to_one_hint_line() {
        let mut app = App::new();
        let screen = render(&mut app, 40, 8);
        assert!(screen.contains("NOTHING LOADED"));
        assert!(screen.contains("search"));
        assert!(!screen.contains("NETEASE CLOUD MUSIC"));
    }

    #[test]
    fn ascii_empty_state_uses_a_text_wordmark() {
        let mut app = App::new();
        let screen = render_with(
            &mut app,
            90,
            24,
            Theme::resolve_with("cyber", "ascii", "compact", None, false, false)
                .with_motion("full"),
        );
        assert!(screen.contains("L E K I M A"), "{screen}");
        assert!(screen.contains("NOTHING LOADED"), "{screen}");
    }

    #[test]
    fn playing_state_renders_the_hud_and_track() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Mota"));
        app.tracks = vec![track("Mota")];
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("Mota"));
        assert!(screen.contains("Russian Circles"));
        assert!(screen.contains("OFF"));
        assert!(screen.contains("Q:0"), "{screen}");
    }

    #[test]
    fn narrow_terminal_hides_the_sidebar() {
        let mut app = App::new();
        app.mode = Mode::Help;
        let _ = render(&mut app, 40, 12);
        app.mode = Mode::Queue;
        app.queue = vec![track("queued")];
        let screen = render(&mut app, 40, 12);
        assert!(screen.contains("QUEUE"));
        app.mode = Mode::Search;
        app.input = "mota".to_owned();
        let screen = render(&mut app, 80, 24);
        assert!(screen.contains("mota"));
        app.focus = Focus::Sidebar;
        let _ = render(&mut app, 20, 8);
    }

    #[test]
    fn waveform_uses_sampled_levels() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Mota"));
        app.status.wave = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        let screen = render(&mut app, 150, 24);
        assert!(screen.contains("█"));
        assert!(screen.contains("▄"));
    }

    #[test]
    fn lyrics_pane_renders_context_and_active_line() {
        use crate::app::LyricsState;

        let mut app = App::new();
        app.status.track = Some(track("Bohemian Rhapsody"));
        app.status.position_ms = 48_000;
        app.lyrics_open = true;
        app.lyrics_state = LyricsState::Ready {
            track: 1.into(),
            lyrics: lekima_domain::parse_lrc(
                "[00:40.00]Is this the real life?\n[00:44.00]Is this just fantasy?\n[00:48.00]Caught in a landslide\n[00:52.00]No escape from reality",
            ),
        };
        let screen = render(&mut app, 126, 28);
        assert!(screen.contains("LYRICS"));
        assert!(screen.contains("Caught in a landslide"));
        assert!(screen.contains("ms"));
        assert!(screen.contains("▌"));
    }

    #[test]
    fn lyrics_pane_shows_an_empty_state() {
        use crate::app::LyricsState;

        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        app.lyrics_open = true;
        app.lyrics_state = LyricsState::Empty { track: 1.into() };
        let screen = render(&mut app, 126, 28);
        assert!(screen.contains("no lyrics for this track"));
    }

    #[test]
    fn notices_render_as_toasts_not_in_the_command_bar() {
        let mut app = App::new();
        app.frame = 10;
        app.notify("queued 3 tracks", false);
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("+ NOTICE"), "{screen}");
        assert!(screen.contains("queued 3 tracks"), "{screen}");
        assert!(screen.contains('╔'), "{screen}");

        app.frame = 200;
        let screen = render(&mut app, 90, 24);
        assert!(!screen.contains("queued 3 tracks"));
    }

    #[test]
    fn errors_use_the_error_toast() {
        let mut app = App::new();
        app.notify("provider rejected the request", true);
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("! ERROR"), "{screen}");
        assert!(screen.contains("provider rejected"), "{screen}");
    }

    #[test]
    fn long_error_toasts_wrap_and_truncate_without_panicking() {
        let mut app = App::new();
        app.notify(
            "the provider returned an unexpectedly long diagnostic that cannot fit in two lines and must be clipped \
             because it keeps going with stack traces, request ids and retry advice that no toast can hold",
            true,
        );
        let screen = render(&mut app, 60, 20);
        assert!(screen.contains("! ERROR"), "{screen}");
        assert!(screen.contains('…'), "{screen}");
    }

    #[test]
    fn sidebar_shows_sections_and_playlist_counts() {
        let mut app = App::new();
        app.playlists = vec![lekima_domain::Playlist {
            id: 1.into(),
            name: "Late night".to_owned(),
            description: String::new(),
            track_count: 42,
            cover_url: None,
            owner: None,
        }];
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("d ☀ DAILY"), "key and concept glyph");
        assert!(screen.contains("≡ PLAYLISTS"));
        assert!(screen.contains("Late night"));
        assert!(screen.contains("42"));
    }

    #[test]
    fn ascii_glyphs_and_comfortable_density_render() {
        use crate::theme::Theme;

        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Mota"));
        app.tracks = vec![track("Mota"), track("Teardrop")];
        let screen = render_with(
            &mut app,
            150,
            30,
            Theme::resolve_with("cyber", "ascii", "comfortable", None, false, false)
                .with_motion("full"),
        );
        assert!(screen.contains("SIG"));
        assert!(screen.contains("#"));
        assert!(!screen.contains("▌"));
    }

    #[test]
    fn tiny_terminals_fall_back_without_panicking() {
        let mut app = App::new();
        let screen = render(&mut app, 10, 3);
        assert!(screen.contains("LEKIMA"));
    }

    #[test]
    fn help_fits_an_eighty_by_twenty_four_terminal() {
        let mut app = App::new();
        app.mode = Mode::Help;
        let screen = render(&mut app, 80, 24);
        assert!(screen.contains("KEYS"));
        assert!(screen.contains("q / ctrl-c"));
        assert!(screen.contains("quit"));
        assert!(screen.contains("settings / daemon"));
        assert!(screen.contains("history back/forward"));
        assert!(
            screen.contains(":playlist"),
            "the playlist commands must be readable at 80x24"
        );
    }

    #[test]
    fn help_scrolls_on_small_terminals() {
        let mut app = App::new();
        app.mode = Mode::Help;
        let _ = render(&mut app, 60, 20);
        assert!(app.help_max_scroll > 0);
        for _ in 0..6 {
            app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        }
        let screen = render(&mut app, 60, 20);
        assert!(screen.contains("settings / daemon"));
        assert!(screen.contains("HELP"));
    }

    #[test]
    fn hud_keeps_one_row_with_a_title_floor_and_the_clock() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track(
            "A very long track title that should never hide the clock or volume meter",
        ));
        app.status.position_ms = 42_000;

        // One row at every width; the title truncates before the clock does.
        let rows = render_rows(&mut app, 80, 24, Theme::cyber());
        assert!(rows[0].contains("LEKIMA"), "{rows:?}");
        assert!(rows[0].contains('…'), "{rows:?}");
        assert!(rows[0].contains("00:42/05:00"), "{rows:?}");
        assert!(rows[1].starts_with('─'), "{rows:?}");

        let screen = render(&mut app, 120, 30);
        assert!(screen.contains("00:42/05:00"), "{screen}");
        assert!(screen.contains("Q:0"), "{screen}");
    }

    #[test]
    fn narrow_hud_never_overwrites_the_brand() {
        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        let rows = render_rows(&mut app, 40, 16, Theme::cyber());
        assert!(rows[0].contains("LEKIMA"), "{rows:?}");
    }

    #[test]
    fn shuffle_off_has_no_orphan_glyph() {
        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        let screen = render(&mut app, 100, 24);
        assert!(!screen.contains('⇄'), "{screen}");
    }

    #[test]
    fn shuffle_state_does_not_depend_on_colour() {
        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        let mono = Theme::resolve_with("mono", "unicode", "compact", None, false, false);

        app.status.shuffle = false;
        let off = render_rows(&mut app, 100, 24, mono).join("\n");
        assert!(!off.contains("[⇄]"), "{off}");

        app.status.shuffle = true;
        let on = render_rows(&mut app, 100, 24, mono).join("\n");
        assert!(on.contains("[⇄]"), "{on}");
    }

    #[test]
    fn hud_labels_loading_buffering_and_mute() {
        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        app.status.loading = true;
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("LOADING"));
        app.status.loading = false;
        app.status.buffering = true;
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("BUFFERING"));
        app.status.buffering = false;
        app.status.muted = true;
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("MUTE"));
    }

    #[test]
    fn inactive_selection_remains_visible() {
        let mut app = App::new();
        app.tracks = vec![track("Mota")];
        app.focus = Focus::Sidebar;
        let screen = render(&mut app, 100, 30);
        assert!(screen.matches("▌").count() >= 2, "{screen}");
    }

    #[test]
    fn long_lists_show_a_position_cue() {
        let mut app = App::new();
        app.tracks = (0..200).map(|_| track("Mota")).collect();
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("1/200"), "{screen}");
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("200/200"), "{screen}");
    }

    #[test]
    fn narrow_tables_drop_the_artist_column() {
        let mut app = App::new();
        app.tracks = vec![track(
            "A very long title that must be shortened with an ellipsis",
        )];
        let wide = render(&mut app, 120, 30);
        assert!(wide.contains("ARTIST"));
        let narrow = render(&mut app, 50, 20);
        assert!(narrow.contains("TITLE"));
        assert!(!narrow.contains("ARTIST"));
        assert!(narrow.contains('…'));
    }

    #[test]
    fn queue_uses_the_track_grammar_and_ascii_glyphs() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![track("Mota")];
        app.status.track = Some(track("Mota"));
        let theme = Theme::resolve_with("cyber", "ascii", "compact", None, false, false);
        let screen = render_with(&mut app, 100, 30, theme);
        assert!(screen.contains("QUEUE"));
        assert!(screen.contains("TIME"));
        assert!(screen.contains("001"));
        assert!(!screen.contains('▶'));
        assert!(!screen.contains('♥'));
        assert!(!screen.contains('⏎'));
        assert!(!screen.contains('…'));
        assert!(!screen.contains('┃'));
        assert!(!screen.contains('·'));
        assert!(!screen.contains('■'));
    }

    #[test]
    fn ascii_overlays_use_plain_borders() {
        let mut app = App::new();
        app.mode = Mode::Help;
        let theme = Theme::resolve_with("cyber", "ascii", "compact", None, false, false);
        let help = render_with(&mut app, 100, 30, theme);
        assert!(help.contains("KEYS"));
        assert!(!help.contains('▓'));
        assert!(!help.contains('╔'));

        app.mode = Mode::Settings;
        let settings = render_with(&mut app, 100, 30, theme);
        assert!(settings.contains("UI"));
        assert!(!settings.contains('▓'));
    }

    #[test]
    fn queue_confirmation_is_visible_in_the_footer() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![track("Mota")];
        app.queue_confirm = Some(0);
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("remove 001? [y/N]"));
    }

    #[test]
    fn playlist_numbering_survives_three_digits() {
        let mut app = App::new();
        app.playlists = (0..120)
            .map(|index| lekima_domain::Playlist {
                id: index.into(),
                name: format!("playlist {index}"),
                description: String::new(),
                track_count: index,
                cover_url: None,
                owner: None,
            })
            .collect();
        app.focus = Focus::Sidebar;
        app.sidebar_selection = app.sidebar_len() - 1;
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("120"));
        assert!(screen.contains("playlist 119"));
    }

    #[test]
    fn widths_are_counted_in_cells_not_characters() {
        assert_eq!(display_width("夜に駆ける"), 10);
        let shortened = truncate_to_width("夜に駆ける", 6, "…");
        assert!(display_width(&shortened) <= 6, "{shortened}");
        assert!(shortened.ends_with('…'));
        assert_eq!(truncate_to_width("abcdef", 4, "..."), "a...");
    }

    #[test]
    fn empty_states_are_actionable() {
        let mut app = App::new();
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("search the catalogue"));
        assert!(screen.contains("browse the library"));
        app.mode = Mode::Queue;
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("nothing queued"));
        assert!(screen.contains("Enter to play"));
    }

    #[test]
    fn every_empty_surface_uses_one_anatomy() {
        type EmptyCase = (&'static str, &'static str, fn(&mut App));
        let cases: [EmptyCase; 4] = [
            ("QUEUE", "nothing queued", |app| app.mode = Mode::Queue),
            ("HISTORY", "no plays yet", |app| app.mode = Mode::History),
            ("CHARTS", "no charts loaded", |app| {
                app.view = View::Toplists;
            }),
            ("SEARCH", "type a query", |app| app.view = View::Search),
        ];
        for (label, sentence, setup) in cases {
            let mut app = App::new();
            setup(&mut app);
            let screen = render(&mut app, 100, 24);
            assert!(screen.contains(label), "{label}: {screen}");
            assert!(screen.contains(sentence), "{label}: {screen}");
        }
    }

    #[test]
    fn catalogue_metadata_shares_a_right_edge() {
        let mut app = App::new();
        app.view = View::Search;
        app.search.entries = vec![
            SearchEntry::Playlist(lekima_domain::Playlist {
                id: lekima_domain::PlaylistId(1),
                name: "Post-rock".to_owned(),
                description: String::new(),
                track_count: 42,
                cover_url: None,
                owner: None,
            }),
            SearchEntry::Playlist(lekima_domain::Playlist {
                id: lekima_domain::PlaylistId(2),
                name: "A much longer playlist name".to_owned(),
                description: String::new(),
                track_count: 9,
                cover_url: None,
                owner: None,
            }),
        ];
        let rows = render_rows(&mut app, 100, 24, Theme::cyber());
        let post_rock = rows
            .iter()
            .find(|row| row.contains("Post-rock"))
            .expect("first playlist row");
        let longer = rows
            .iter()
            .find(|row| row.contains("A much longer"))
            .expect("second playlist row");
        assert_eq!(
            display_width(post_rock.trim_end()),
            display_width(longer.trim_end()),
            "metadata does not share a right edge: {rows:?}"
        );
        assert!(post_rock.trim_end().ends_with("tracks"), "{post_rock:?}");
    }

    #[test]
    fn long_durations_fit_the_time_column() {
        let mut app = App::new();
        app.tracks = vec![lekima_domain::Track {
            duration_ms: 3_723_000,
            ..track("long")
        }];
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("1:02:03"), "{screen}");
    }

    #[test]
    fn settings_fit_a_small_terminal() {
        let mut app = App::new();
        app.mode = Mode::Settings;
        let screen = render(&mut app, 40, 12);
        assert!(screen.contains("THEME"), "{screen}");
        assert!(screen.contains("esc close"), "{screen}");
    }

    #[test]
    fn an_error_is_visible_even_in_a_tiny_terminal() {
        let mut app = App::new();
        app.notify("daemon went away", true);
        let rows = render_rows(&mut app, 30, 6, Theme::cyber());
        assert!(
            rows.iter().any(|row| row.contains("daemon went away")),
            "an alarm is visible at 30x6: {rows:?}"
        );
        let rows = render_rows(&mut app, 24, 5, Theme::cyber());
        assert!(
            rows.last()
                .is_some_and(|row| row.contains("daemon went away")),
            "below 30x6 the alarm takes the footer row: {rows:?}"
        );
    }

    #[test]
    fn copy_never_uses_a_hyphen_as_a_dash() {
        let mut app = App::new();
        for mode in [Mode::Normal, Mode::Queue, Mode::History] {
            app.mode = mode;
            let screen = render(&mut app, 100, 24);
            assert!(!screen.contains(" - "), "{mode:?}: {screen}");
        }
    }

    #[test]
    fn reduced_motion_freezes_spinners_and_hides_the_spectrum() {
        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        app.status.loading = true;
        let reduced = Theme::cyber().with_motion("reduced");
        let first = render_rows(&mut app, 120, 30, reduced).join("\n");
        app.frame = 3;
        let second = render_rows(&mut app, 120, 30, reduced).join("\n");
        assert_eq!(first, second, "reduced motion must not animate");
        assert!(!first.contains("SIG"), "the spectrum is hidden: {first}");

        let full = Theme::cyber().with_motion("full");
        let first = render_rows(&mut app, 120, 30, full).join("\n");
        app.frame = 0;
        let second = render_rows(&mut app, 120, 30, full).join("\n");
        assert_ne!(first, second, "full motion animates the spinner");
    }

    #[test]
    fn reduced_motion_keeps_the_prompt_cursor_steady() {
        let mut app = App::new();
        app.mode = Mode::Search;
        let reduced = Theme::cyber().with_motion("reduced");
        let steady = render_rows(&mut app, 100, 24, reduced).join("\n");
        app.frame = CURSOR_BLINK_FRAMES;
        let later = render_rows(&mut app, 100, 24, reduced).join("\n");
        assert_eq!(steady, later);
    }

    #[test]
    fn ascii_mode_never_emits_unicode() {
        let theme = Theme::resolve_with("cyber", "ascii", "compact", None, false, false)
            .with_motion("reduced");
        let cases: [fn(&mut App); 8] = [
            |app| app.mode = Mode::Normal,
            |app| app.mode = Mode::Queue,
            |app| app.mode = Mode::History,
            |app| app.mode = Mode::Settings,
            |app| app.mode = Mode::Help,
            |app| app.mode = Mode::Comments,
            |app| app.view = View::Toplists,
            |app| app.view = View::Search,
        ];
        for setup in cases {
            let mut app = App::new();
            setup(&mut app);
            let screen = render_rows(&mut app, 120, 30, theme).join("\n");
            assert!(
                screen.is_ascii(),
                "non-ASCII glyph in ascii mode: {:?}",
                screen.chars().filter(|c| !c.is_ascii()).collect::<String>()
            );
        }
    }

    #[test]
    fn ascii_mode_handles_populated_catalogue_and_loading() {
        let theme = Theme::resolve_with("cyber", "ascii", "compact", None, false, false)
            .with_motion("reduced");
        let mut app = App::new();
        app.view = View::Search;
        app.search.query = "mota".to_owned();
        app.search.entries = vec![
            SearchEntry::Album(album("Mota", None)),
            SearchEntry::Playlist(lekima_domain::Playlist {
                id: lekima_domain::PlaylistId(7),
                name: "post-rock".to_owned(),
                description: String::new(),
                track_count: 42,
                cover_url: None,
                owner: None,
            }),
        ];
        let screen = render_rows(&mut app, 120, 30, theme).join("\n");
        assert!(screen.is_ascii(), "search: {screen}");

        app.search.entries.clear();
        app.view = View::Album;
        app.album = AlbumState {
            id: Some(9.into()),
            detail: Some(lekima_domain::AlbumDetail {
                album: album("Mota", None),
                tracks: Vec::new(),
            }),
            ..AlbumState::default()
        };
        let screen = render_rows(&mut app, 120, 30, theme).join("\n");
        assert!(screen.is_ascii(), "album: {screen}");

        app.view = View::Artist;
        app.artist = ArtistState::default();
        let screen = render_rows(&mut app, 120, 30, theme).join("\n");
        assert!(screen.is_ascii(), "artist: {screen}");

        app.view = View::Tracks;
        app.mode = Mode::History;
        app.plays_loading = true;
        let screen = render_rows(&mut app, 120, 30, theme).join("\n");
        assert!(screen.is_ascii(), "history loading: {screen}");

        app.notify("no matches for \"mota\" ; / to refine", true);
        let screen = render_rows(&mut app, 120, 30, theme).join("\n");
        assert!(screen.is_ascii(), "toast: {screen}");
    }

    #[test]
    fn mono_selection_is_really_reversed() {
        let theme = Theme::resolve_with("mono", "unicode", "compact", None, false, false);
        assert_eq!(theme.selection_bg, ratatui::style::Color::White);
        assert_eq!(theme.selection_fg, ratatui::style::Color::Black);
    }

    #[test]
    fn narrow_hud_leaves_no_title_residue() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Cigarettes and lighters"));
        for width in [48, 52] {
            let rows = render_rows(&mut app, width, 24, Theme::cyber());
            assert!(rows[0].contains("00:00"), "{width}: {rows:?}");
            assert!(
                !rows[0].contains("and lighters"),
                "{width}: title residue behind the rail: {rows:?}"
            );
        }
    }

    #[test]
    fn search_metadata_never_clips_mid_token() {
        let mut app = App::new();
        app.view = View::Search;
        app.search.entries = vec![SearchEntry::Playlist(lekima_domain::Playlist {
            id: lekima_domain::PlaylistId(1),
            name: "Post-rock".to_owned(),
            description: String::new(),
            track_count: 42,
            cover_url: None,
            owner: None,
        })];
        for width in [80, 100, 120] {
            let rows = render_rows(&mut app, width, 24, Theme::cyber());
            let row = rows
                .iter()
                .find(|row| row.contains("Post-rock"))
                .expect("playlist row");
            let trimmed = row.trim_end();
            assert!(
                trimmed.ends_with("tracks") || trimmed.ends_with('…'),
                "{width}: metadata clipped mid-token: {trimmed:?}"
            );
        }
    }

    #[test]
    fn search_line_never_exceeds_its_width() {
        let theme = Theme::cyber();
        let entry = SearchEntry::Track(lekima_domain::Track {
            duration_ms: 392_000,
            ..track("Mota")
        });
        for width in [40, 54, 56, 80, 120] {
            let line = super::search_entry_line(&theme, &entry, 0, false, 28, width);
            let rendered: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            assert!(
                display_width(&rendered) <= width,
                "{width}: line is {} cells: {rendered:?}",
                display_width(&rendered)
            );
            if width < 60 {
                assert!(rendered.ends_with('…'), "{width}: {rendered:?}");
            }
        }
    }

    #[test]
    fn history_columns_are_separated() {
        let mut app = App::new();
        app.mode = Mode::History;
        app.plays = vec![play("Mota", true)];
        let rows = render_rows(&mut app, 120, 30, Theme::cyber());
        let row = rows.iter().find(|row| row.contains("manual")).expect("row");
        let at = row.find("manual").expect("kind is rendered");
        assert!(
            row[..at].ends_with(' '),
            "kind column runs into the played cell: {row:?}"
        );
    }

    #[test]
    fn tiny_footer_alarm_replaces_the_command_bar() {
        let mut app = App::new();
        app.notify("daemon went away", true);
        let rows = render_rows(&mut app, 24, 5, Theme::cyber());
        let footer = rows.last().expect("footer");
        assert!(footer.contains("daemon went away"), "{rows:?}");
        assert!(
            !footer.contains("[NORMAL]"),
            "command bar residue: {footer:?}"
        );
    }

    #[test]
    fn empty_panes_do_not_repeat_the_header_label() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("≣ QUEUE"), "{screen}");
        assert!(!screen.contains("// QUEUE"), "label repeated: {screen}");
    }

    #[test]
    fn lyrics_stack_on_mid_widths_and_split_on_wide_ones() {
        use crate::app::LyricsState;

        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        app.tracks = vec![track("Mota")];
        app.lyrics_open = true;
        app.lyrics_state = LyricsState::Ready {
            track: 1.into(),
            lyrics: lekima_domain::parse_lrc("[00:01.00]Caught in a landslide"),
        };
        let mid = render(&mut app, 90, 30);
        assert!(mid.contains("Mota"));
        assert!(mid.contains("LYRICS"));
        let wide = render(&mut app, 130, 30);
        assert!(wide.contains("Mota"));
        assert!(wide.contains("LYRICS"));
    }

    #[test]
    fn cjk_playlist_names_keep_the_count_aligned() {
        let mut app = App::new();
        app.playlists = vec![lekima_domain::Playlist {
            id: 1.into(),
            name: "夜に駆けるとても長いプレイリスト名".to_owned(),
            description: String::new(),
            track_count: 42,
            cover_url: None,
            owner: None,
        }];
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("42"), "{screen}");
        assert!(screen.contains('…'), "{screen}");
    }

    #[test]
    fn buffered_ranges_shade_the_progress_rail() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Mota"));
        app.status.position_ms = 0;
        let plain = render(&mut app, 100, 24);
        assert!(!plain.contains('━'), "{plain}");

        app.status.buffered = vec![lekima_domain::BufferedRange {
            start_ms: 0,
            end_ms: 150_000,
        }];
        let shaded = render(&mut app, 100, 24);
        assert!(shaded.contains('┄'), "{shaded}");
        assert!(!shaded.contains('━'), "{shaded}");
    }

    #[test]
    fn list_times_are_right_aligned() {
        let mut app = App::new();
        app.tracks = vec![
            lekima_domain::Track {
                duration_ms: 109_000,
                ..track("short")
            },
            lekima_domain::Track {
                duration_ms: 632_000,
                ..track("long")
            },
        ];
        let rows = render_rows(&mut app, 100, 24, Theme::cyber());
        let short = rows
            .iter()
            .find(|row| row.contains("short"))
            .expect("short row");
        let long = rows
            .iter()
            .find(|row| row.contains("long"))
            .expect("long row");
        assert!(short.trim_end().ends_with("1:49"), "{short:?}");
        assert!(long.trim_end().ends_with("10:32"), "{long:?}");
        assert_eq!(
            display_width(short.trim_end()),
            display_width(long.trim_end()),
            "durations share a right edge"
        );
    }

    #[test]
    fn history_uses_the_shared_list_grammar() {
        let mut app = App::new();
        app.mode = Mode::History;
        app.plays = vec![play("Teardrop", true), play("Mota", false)];
        let screen = render(&mut app, 120, 30);
        for header in ["AGE", "TITLE", "ARTIST", "PLAYED", "KIND"] {
            assert!(screen.contains(header), "missing {header}: {screen}");
        }
        assert!(screen.contains("manual"), "{screen}");
        assert!(screen.contains('✓'), "{screen}");
    }

    #[test]
    fn cjk_titles_do_not_exceed_the_pane() {
        let mut app = App::new();
        app.tracks = vec![track(
            "北欧民谣与电子音乐的漫长标题测试轨道名称这是一段用来验证截断行为的超长文本",
        )];
        let rows = render_rows(&mut app, 80, 24, Theme::cyber());
        for row in &rows {
            // Wide glyphs occupy two cells; the row holds at most one symbol
            // per cell, so the character count can never exceed the width.
            assert!(
                row.chars().count() <= 80,
                "row wider than 80 cells: {row:?}"
            );
        }
        let track_row = rows
            .iter()
            .find(|row| row.contains("001"))
            .expect("track row");
        assert!(
            track_row.contains('…'),
            "a long CJK title truncates instead of overflowing: {track_row:?}"
        );
        assert!(
            track_row.contains("5:00"),
            "the time column survives: {track_row:?}"
        );
    }

    #[test]
    fn queue_header_shows_remaining_time() {
        let mut app = App::new();
        app.mode = Mode::Queue;
        app.queue = vec![track("Mota"), track("Teardrop")];
        app.status.queue_index = Some(0);
        app.status.position_ms = 42_000;
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("9:18 left"), "{screen}");
    }

    #[test]
    fn preview_streams_use_their_real_duration() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Mota"));
        app.status.stream_duration_ms = Some(30_000);
        let screen = render(&mut app, 120, 24);
        assert!(screen.contains("PREVIEW"), "{screen}");
        assert!(screen.contains("00:00/00:30"), "{screen}");
    }

    #[test]
    fn cached_playback_shows_the_cache_badge() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("Mota"));
        app.status.cached = true;
        let screen = render(&mut app, 120, 24);
        assert!(screen.contains("CACHE"), "{screen}");
    }

    #[test]
    fn comments_pane_renders_hot_and_regular_entries() {
        use crate::app::CommentsState;

        let mut app = App::new();
        app.mode = Mode::Comments;
        app.status.track = Some(track("Mota"));
        app.comments_state = CommentsState::Ready {
            track: 1.into(),
            comments: lekima_domain::Comments {
                total: 2,
                has_more: true,
                hot: vec![lekima_domain::Comment {
                    id: 1,
                    author: "ben".to_owned(),
                    avatar_url: None,
                    content: "hot take".to_owned(),
                    timestamp_ms: 0,
                    liked_count: 9,
                    replies: Vec::new(),
                }],
                comments: vec![lekima_domain::Comment {
                    id: 2,
                    author: "aka".to_owned(),
                    avatar_url: None,
                    content: "regular".to_owned(),
                    timestamp_ms: 0,
                    liked_count: 1,
                    replies: Vec::new(),
                }],
            },
        };
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("COMMENTS"), "{screen}");
        assert!(screen.contains("HOT"), "{screen}");
        assert!(screen.contains("MOST LIKED"), "{screen}");
        assert!(screen.contains("hot take"), "{screen}");
        assert!(screen.contains("regular"), "{screen}");
        assert!(screen.contains("MORE COMMENTS"), "{screen}");
        assert!(screen.contains("n loads the next page"), "{screen}");
    }

    #[test]
    fn comments_render_metadata_replies_and_wrapped_content() {
        use crate::app::CommentsState;

        let mut app = App::new();
        app.mode = Mode::Comments;
        app.status.track = Some(track("Mota"));
        app.comments_state = CommentsState::Ready {
            track: 1.into(),
            comments: lekima_domain::Comments {
                total: 20_535,
                has_more: false,
                hot: Vec::new(),
                comments: vec![lekima_domain::Comment {
                    id: 9,
                    author: "程cheng艾影".to_owned(),
                    avatar_url: None,
                    content: "庐山牛逼，这是一条足够长的评论，用来确认它会按宽度换行而不是被截断。"
                        .to_owned(),
                    timestamp_ms: 1_700_000_000_000,
                    liked_count: 44_054,
                    replies: vec![lekima_domain::CommentReply {
                        author: "aka".to_owned(),
                        content: "agree".to_owned(),
                    }],
                }],
            },
        };
        let screen = render(&mut app, 80, 30);
        assert!(screen.contains("20.5k"), "{screen}");
        assert!(screen.contains("44k"), "{screen}");
        assert!(screen.contains("2023-11-14"), "{screen}");
        assert!(screen.contains("↳ aka: agree"), "{screen}");
        assert!(screen.contains('换'), "{screen}");
        assert!(screen.contains('行'), "{screen}");
    }

    #[test]
    fn comments_error_state_shows_retry_hint() {
        use crate::app::CommentsState;

        let mut app = App::new();
        app.mode = Mode::Comments;
        app.status.track = Some(track("Mota"));
        app.comments_state = CommentsState::Error {
            track: 1.into(),
            message: "provider timed out".to_owned(),
        };
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("COMMENTS UNAVAILABLE"), "{screen}");
        assert!(screen.contains("provider timed out"), "{screen}");
        assert!(screen.contains("retry"), "{screen}");
    }

    #[test]
    fn hostile_provider_text_is_sanitized_before_rendering() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track("\u{1b}[31mowned\u{1b}[0m\u{202e}"));
        app.tracks = vec![track("\u{1b}]0;pwned\u{7}Mota")];
        let screen = render(&mut app, 100, 30);
        assert!(!screen.contains('\u{1b}'));
        assert!(!screen.contains('\u{202e}'));
        assert!(screen.contains("owned"));
        assert!(screen.contains("Mota"));
    }

    #[test]
    fn light_theme_renders_without_panicking() {
        let mut app = App::new();
        app.status.track = Some(track("Mota"));
        let theme = Theme::resolve_with("light", "unicode", "compact", None, false, false);
        let screen = render_with(&mut app, 100, 24, theme);
        assert!(screen.contains("LEKIMA"));
        assert!(screen.contains("Mota"));
    }

    #[test]
    fn compact_counts_use_k_and_m_suffixes() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_000), "1k");
        assert_eq!(compact_count(1_250), "1.2k");
        assert_eq!(compact_count(44_054), "44k");
        assert_eq!(compact_count(1_250_000), "1.2m");
    }

    #[test]
    fn comment_dates_are_utc_and_skip_missing_timestamps() {
        assert_eq!(comment_date(0), None);
        assert_eq!(comment_date(1_000).as_deref(), Some("1970-01-01"));
        assert_eq!(
            comment_date(1_700_000_000_000).as_deref(),
            Some("2023-11-14")
        );
    }

    #[test]
    fn wrapping_prefers_spaces_and_breaks_long_words() {
        assert_eq!(
            wrap_to_width("alpha beta gamma", 11),
            vec!["alpha beta", "gamma"]
        );
        assert_eq!(wrap_to_width("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap_to_width("", 10), vec![""]);
    }

    fn album(name: &str, year: Option<u16>) -> lekima_domain::Album {
        lekima_domain::Album {
            id: 9.into(),
            name: name.to_owned(),
            cover_url: None,
            artist: Some(lekima_domain::Artist {
                id: 2.into(),
                name: "Aash".to_owned(),
            }),
            year,
            track_count: Some(12),
            description: String::new(),
        }
    }

    fn search_results() -> lekima_domain::SearchResults {
        use lekima_domain::{Playlist, SearchResults, SearchTotals};
        SearchResults {
            tracks: vec![track("Mota")],
            albums: vec![album("Mota LP", Some(2024))],
            artists: vec![lekima_domain::Artist {
                id: 6452.into(),
                name: "Jay Chou".to_owned(),
            }],
            playlists: vec![Playlist {
                id: 5.into(),
                name: "Post-rock".to_owned(),
                description: String::new(),
                track_count: 42,
                cover_url: None,
                owner: None,
            }],
            totals: SearchTotals {
                tracks: 1,
                albums: 1,
                artists: 1,
                playlists: 1,
            },
            total: 4,
        }
    }

    #[test]
    fn search_results_render_grouped_sections_at_both_sizes() {
        let mut app = App::new();
        app.search.query = "mota".to_owned();
        app.search.loading = true;
        app.view = crate::app::View::Search;
        app.apply(Ok(lekima_ipc::Reply::Search {
            query: "mota".to_owned(),
            kind: lekima_domain::SearchKind::All,
            results: search_results(),
        }));

        for (width, height) in [(80, 24), (120, 30)] {
            let screen = render(&mut app, width, height);
            for expected in [
                "// SEARCH",
                "TRACKS",
                "ALBUMS",
                "ARTISTS",
                "PLAYLISTS",
                "Mota",
                "Aash",
                "Jay Chou",
                "42 tracks",
            ] {
                assert!(
                    screen.contains(expected),
                    "{width}x{height} missing `{expected}`"
                );
            }
        }
    }

    #[test]
    fn the_search_rail_sits_beside_the_prompt() {
        let mut app = App::new();
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        ));
        for character in "mota".chars() {
            app.handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            ));
        }

        for width in [80, 120] {
            let rows = render_rows(&mut app, width, 24, Theme::cyber());
            let footer = &rows[rows.len() - 1];
            for glyph in ["♪", "▣", "☺", "≡"] {
                assert!(footer.contains(glyph), "{width} missing {glyph}: {footer}");
            }
            assert!(footer.contains("mota"), "{width} lost the query");
            assert!(
                !rows[1].contains('♪'),
                "{width}: the rail must not replace the rule at the top"
            );
        }
    }

    #[test]
    fn the_active_category_is_bracketed_and_all_has_none() {
        let mut app = App::new();
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        ));
        assert!(!render(&mut app, 100, 24).contains("[▣]"));

        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('3'),
            KeyModifiers::CONTROL,
        ));
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("[▣]"), "albums are bracketed when active");
        assert!(
            !screen.contains("♪ ▣ ☺ ≡"),
            "the inactive plain rail is gone"
        );
    }

    #[test]
    fn the_search_rail_has_an_ascii_form() {
        let mut app = App::new();
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        ));
        let screen = render_with(
            &mut app,
            100,
            24,
            Theme::resolve_with("cyber", "ascii", "compact", None, false, false)
                .with_motion("full"),
        );
        let footer = screen[screen.len() - 100..].to_owned();
        for glyph in ["t", "a", "r", "p"] {
            assert!(footer.contains(glyph), "missing {glyph}: {footer}");
        }
    }

    #[test]
    fn the_results_header_names_the_category() {
        let mut app = App::new();
        app.view = View::Search;
        app.search.query = "mota".to_owned();
        app.search.applied_filter = crate::app::SearchFilter::Albums;
        app.apply(Ok(lekima_ipc::Reply::Search {
            query: "mota".to_owned(),
            kind: lekima_domain::SearchKind::Album,
            results: search_results(),
        }));

        let screen = render(&mut app, 120, 30);
        assert!(
            screen.contains("▣ ALBUMS"),
            "header should name the active category"
        );
    }

    #[test]
    fn artist_page_renders_counts_albums_and_paging() {
        let mut app = App::new();
        app.view = View::Artist;
        app.source = "artist: Jay Chou".to_owned();
        app.artist = ArtistState {
            id: Some(6452.into()),
            detail: Some(lekima_domain::ArtistDetail {
                id: 6452.into(),
                name: "Jay Chou".to_owned(),
                avatar_url: None,
                cover_url: None,
                album_count: 49,
                track_count: 592,
                description: "华语流行乐男歌手".to_owned(),
                aliases: vec!["周董".to_owned()],
            }),
            albums: vec![album("最伟大的作品", Some(2022))],
            has_more: true,
            ..ArtistState::default()
        };

        for (width, height) in [(80, 24), (120, 30)] {
            let screen = render(&mut app, width, height);
            for expected in [
                "// ARTIST",
                "6452",
                "Jay Chou",
                "49 albums",
                "592 tracks",
                "ALBUMS",
                "2022",
                "12 tracks",
                "MORE ALBUMS",
            ] {
                assert!(
                    screen.contains(expected),
                    "{width}x{height} missing `{expected}`"
                );
            }
        }
    }

    #[test]
    fn album_page_renders_metadata_and_the_track_table() {
        let mut app = App::new();
        app.view = View::Album;
        app.source = "album: Mota".to_owned();
        app.album = AlbumState {
            id: Some(9.into()),
            detail: Some(lekima_domain::AlbumDetail {
                album: album("Mota", Some(2024)),
                tracks: vec![track("Mota"), track("Mota (Live)")],
            }),
            ..AlbumState::default()
        };

        for (width, height) in [(80, 24), (120, 30)] {
            let screen = render(&mut app, width, height);
            for expected in ["// ALBUM", "Mota", "Aash", "2024", "2 TRACKS", "Live"] {
                assert!(
                    screen.contains(expected),
                    "{width}x{height} missing `{expected}`"
                );
            }
        }
    }

    #[test]
    fn catalogue_empty_and_error_states_are_explicit() {
        let mut app = App::new();
        app.view = View::Artist;
        app.artist = ArtistState {
            id: Some(6452.into()),
            ..ArtistState::default()
        };
        assert!(render(&mut app, 100, 24).contains("NO ALBUMS"));

        app.view = View::Album;
        app.album = AlbumState {
            id: Some(9.into()),
            detail: Some(lekima_domain::AlbumDetail {
                album: album("Mota", None),
                tracks: Vec::new(),
            }),
            ..AlbumState::default()
        };
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("NO TRACKS"));
        // Missing metadata is a dash, never a blank column (and stays ASCII).
        assert!(screen.contains('-'));

        app.view = View::Search;
        app.search.error = Some("provider down".to_owned());
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("CATALOGUE UNAVAILABLE"));
        assert!(screen.contains("provider down"));
    }
}
