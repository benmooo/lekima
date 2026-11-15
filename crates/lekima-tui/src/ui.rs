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
        Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Table,
        TableState, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    App, CommentsState, Focus, LyricsState, Mode, SETTINGS_ROWS, SIDEBAR_STATICS, View,
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
            let spinner = theme.glyphs.spinner[(app.frame % 4) as usize];
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
        groups.push(spectrum_group(app, theme));
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
    vec![Span::styled(
        format!("[{}]", theme.glyphs.shuffle),
        Style::new().fg(if app.status.shuffle {
            theme.accent_alt
        } else {
            theme.dim
        }),
    )]
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
    groups.iter().map(|group| spans_width(group) + 2).sum()
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
    let mut items: Vec<ListItem> = Vec::with_capacity(app.sidebar_len() + 2);

    for (index, name) in SIDEBAR_STATICS.iter().enumerate() {
        let selected = index == app.sidebar_selection;
        let marker = if selected { theme.glyphs.active } else { " " };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                marker,
                Style::new().fg(if focused { theme.accent } else { theme.muted }),
            ),
            Span::styled("[", Style::new().fg(theme.dim)),
            Span::styled(keys[index], Style::new().fg(theme.accent_alt)),
            Span::styled("] ", Style::new().fg(theme.dim)),
            Span::styled((*name).to_owned(), Style::new().fg(theme.text)),
        ])));
    }

    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("PLAYLISTS", Style::new().fg(theme.muted)),
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
                "  no playlists - press / to search",
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
        (_, View::Toplists) => draw_toplists(frame, app, theme, inner),
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
    let source = clean(&app.source).to_ascii_uppercase();
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
        title.push(Span::styled(position, Style::new().fg(theme.dim)));
    }
    if focused {
        title.push(Span::styled(
            format!(" {}", theme.glyphs.active),
            Style::new().fg(theme.accent),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    if app.tracks.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "nothing here - press / to search or Tab for the library",
                Style::new().fg(theme.muted),
            ))),
            content,
        );
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
        header.push("TIME");
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
            widths.push(Constraint::Length(6));
        }
        widths
    }
}

fn track_columns(width: u16) -> TrackColumns {
    let width = usize::from(width);
    let time = width >= 40;
    let fixed = 8 + usize::from(time) * 6;
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
                theme.glyphs.spinner[(app.frame % 4) as usize].to_owned()
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
    let secondary = if past { theme.dim } else { theme.muted };
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
            track.duration_label(),
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
            Paragraph::new(Line::from(Span::styled(
                "no charts - press t to load toplists",
                Style::new().fg(theme.muted),
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
            "// LYRICS",
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

fn pad_right(value: &str, width: usize) -> String {
    let mut padded = value.to_owned();
    let missing = width.saturating_sub(display_width(value));
    padded.push_str(&" ".repeat(missing));
    padded
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
            Span::styled("QUEUE", Style::new().fg(theme.muted)),
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
            Paragraph::new(Line::from(Span::styled(
                "nothing queued - press / to search or Enter to play",
                Style::new().fg(theme.muted),
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
        header.push("TIME");
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

fn effective_duration(state: &PlaybackState, track: &Track) -> u64 {
    match state.stream_duration_ms {
        Some(stream) if stream > 0 && stream.saturating_add(2_000) < track.duration_ms => stream,
        _ => track.duration_ms,
    }
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
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let track_name = app
        .status
        .track
        .as_ref()
        .map_or_else(String::new, |track| sanitize_line(&track.name));
    let total = app
        .comments_state
        .ready()
        .map_or(0, |comments| comments.hot.len() + comments.comments.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("// ", Style::new().fg(theme.panel)),
            Span::styled("COMMENTS", Style::new().fg(theme.muted)),
            Span::styled(
                format!(" {} {total}", theme.glyphs.separator),
                Style::new().fg(theme.accent_alt),
            ),
            Span::styled(
                format!(" {} {track_name}", theme.glyphs.separator),
                Style::new().fg(theme.dim),
            ),
            Span::styled(
                format!(" {}", theme.glyphs.active),
                Style::new().fg(theme.accent),
            ),
        ])),
        label,
    );

    let width = usize::from(content.width).saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();
    match &app.comments_state {
        CommentsState::Idle | CommentsState::Loading { .. } => {
            let dots = match (app.frame / 5) % 4 {
                0 => "",
                1 => ".",
                2 => "..",
                _ => "...",
            };
            lines.push(Line::from(Span::styled(
                format!("loading comments{dots}"),
                Style::new().fg(theme.muted),
            )));
        }
        CommentsState::Empty { .. } => {
            lines.push(Line::from(Span::styled(
                "no comments for this track",
                Style::new().fg(theme.muted),
            )));
        }
        CommentsState::Error { message, .. } => {
            lines.push(Line::from(Span::styled(
                format!("! {}", sanitize_line(message)),
                Style::new().fg(theme.hot),
            )));
        }
        CommentsState::Ready { comments, .. } => {
            append_comment_section(&mut lines, "HOT", &comments.hot, width, theme);
            append_comment_section(&mut lines, "ALL", &comments.comments, width, theme);
            if comments.has_more {
                lines.push(Line::from(Span::styled(
                    "more comments available",
                    Style::new().fg(theme.dim),
                )));
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
    lines.push(Line::from(Span::styled(
        format!("// {title} {} {}", theme.glyphs.separator, comments.len()),
        Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
    )));
    for comment in comments {
        let author = if comment.author.is_empty() {
            "anonymous"
        } else {
            comment.author.as_str()
        };
        let content = truncate_to_width(
            &format!(
                "{:>5} {} {}: {}",
                comment.liked_count,
                theme.glyphs.liked,
                sanitize_line(author),
                sanitize_line(&comment.content)
            ),
            width,
            theme.glyphs.ellipsis,
        );
        lines.push(Line::from(Span::styled(
            content,
            Style::new().fg(theme.text),
        )));
        for reply in &comment.replies {
            let reply = truncate_to_width(
                &format!(
                    "       {} {}: {}",
                    theme.glyphs.enter,
                    sanitize_line(&reply.author),
                    sanitize_line(&reply.content)
                ),
                width,
                theme.glyphs.ellipsis,
            );
            lines.push(Line::from(Span::styled(
                reply,
                Style::new().fg(theme.muted),
            )));
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
        Mode::Comments => (
            "COMMENTS",
            "j/k scroll · ctrl-d/u page · r refresh · C/esc close".to_owned(),
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

    if matches!(app.mode, Mode::Search | Mode::Command) {
        let prefix = if app.mode == Mode::Search { ">" } else { ":" };
        spans.push(Span::styled(prefix, Style::new().fg(theme.accent)));
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
    let enter = theme.glyphs.enter;
    let sep = theme.glyphs.separator;
    if width >= 108 {
        format!(
            "j/k {sep} {enter} play {sep} space {sep} Q queue {sep} S theme {sep} / search {sep} : cmd {sep} ctrl-o/i hist {sep} ? keys"
        )
    } else if width >= 80 {
        format!("j/k {sep} {enter} play {sep} space {sep} / search {sep} Q queue {sep} ? keys")
    } else if width >= 52 {
        format!("j/k {sep} {enter} {sep} / {sep} Q {sep} ?")
    } else if app.pending_count.is_some() {
        "digits, then j/k or G".to_owned()
    } else {
        format!("j/k {sep} / {sep} ?")
    }
}

fn queue_hints(theme: &Theme, width: u16) -> String {
    let enter = theme.glyphs.enter;
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
    let blink = (app.frame / CURSOR_BLINK_FRAMES).is_multiple_of(2);
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

/// Notification chip: bottom-right, above the command bar, auto-dismissed.
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
            "daemon offline - press R to start".to_owned(),
            true,
            "! OFFLINE",
            theme.warn,
        )
    } else {
        match message {
            Some(message) if message.error => (message.text.clone(), true, "! ERROR", theme.hot),
            Some(message) => (message.text.clone(), false, "+ NOTICE", theme.panel),
            None => return,
        }
    };

    if area.height < 6 || area.width < 30 {
        return;
    }

    let width = 56.min(area.width.saturating_sub(2));
    let height = if is_error { 2 } else { 1 };
    let mut x = area.width.saturating_sub(width + 1);
    let mut y = area.height.saturating_sub(height + 2).max(1);
    match lyrics {
        Some(LyricsPlacement::Side(rect)) => {
            x = x.min(rect.x.saturating_sub(width + 1));
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

    let fg = theme.badge_fg;
    let bg = colour;
    let padding = 2;
    let available = usize::from(width).saturating_sub(padding);
    let cleaned = clean(&text);
    let text = if is_error {
        cleaned.into_owned()
    } else {
        truncate_to_width(&cleaned, available, theme.glyphs.ellipsis)
    };
    let line = Line::from(vec![
        Span::styled(
            format!("{title} "),
            Style::new().fg(fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(text, Style::new().fg(fg)),
    ]);

    frame.render_widget(Clear, toast);
    let body = Paragraph::new(line)
        .wrap(Wrap { trim: true })
        .block(
            Block::new()
                .padding(Padding::horizontal(1))
                .style(Style::new().bg(bg)),
        )
        .style(Style::new().fg(fg).bg(bg));
    frame.render_widget(body, toast);
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
        .border_type(overlay_border(theme))
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
    lines.push(Line::from(Span::styled(
        format!(
            "h/l change {} j/k select {} esc close {} saved on exit",
            theme.glyphs.separator, theme.glyphs.separator, theme.glyphs.separator
        ),
        Style::new().fg(theme.muted),
    )));
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
            .border_type(overlay_border(theme))
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
        .border_type(overlay_border(theme))
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

fn overlay_border(theme: &Theme) -> BorderType {
    if theme.glyphs.ascii {
        BorderType::Plain
    } else {
        BorderType::Double
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
            (theme.glyphs.progress_full, theme.panel)
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

    use super::{display_width, draw, truncate_to_width};
    use crate::app::{App, Focus, Mode};
    use crate::theme::Theme;

    fn render(app: &mut App, width: u16, height: u16) -> String {
        render_with(app, width, height, Theme::cyber())
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

    #[test]
    fn empty_state_renders_the_brand_and_a_hint() {
        let mut app = App::new();
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("LEKIMA"));
        assert!(screen.contains("nothing here"));
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
        assert!(screen.contains("VOL"));
        assert!(screen.contains("OFF"));
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
        assert!(screen.contains("NOTICE"));
        assert!(screen.contains("queued 3 tracks"));

        app.frame = 200;
        let screen = render(&mut app, 90, 24);
        assert!(!screen.contains("queued 3 tracks"));
    }

    #[test]
    fn errors_use_the_error_toast() {
        let mut app = App::new();
        app.notify("provider rejected the request", true);
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("ERROR"));
        assert!(screen.contains("provider rejected"));
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
        assert!(screen.contains("[d]"));
        assert!(screen.contains("PLAYLISTS"));
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
            Theme::resolve("cyber", "ascii", "comfortable"),
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
    fn hud_keeps_the_clock_and_volume_when_the_title_is_long() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(track(
            "A very long track title that should never hide the clock or volume meter",
        ));
        app.status.position_ms = 42_000;
        let screen = render(&mut app, 80, 24);
        assert!(screen.contains("00:42/05:00"));
        assert!(screen.contains("VOL"));
        let screen = render(&mut app, 60, 20);
        assert!(screen.contains("00:42/05:00"));
        assert!(screen.contains("VOL"));
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
        assert!(screen.contains("press / to search"));
        app.mode = Mode::Queue;
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("nothing queued"));
        assert!(screen.contains("Enter to play"));
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
        assert!(shaded.contains("━━━━━"), "{shaded}");
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
        assert!(screen.contains("hot take"), "{screen}");
        assert!(screen.contains("regular"), "{screen}");
        assert!(screen.contains("more comments available"), "{screen}");
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
}
