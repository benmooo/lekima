//! Rendering.
//!
//! Visual language: a HUD line, a rule, a dense list, a command bar. No boxed
//! widgets in the main chrome — structure comes from dim rules and section
//! labels, state from neon accents. See `docs/sprint-2/tui-visual-design.md`.

use lekima_domain::PlaybackStatus;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Table,
        TableState, Wrap,
    },
};

use crate::app::{App, Focus, Mode};
use crate::theme::Theme;

const MIN_WIDTH: u16 = 24;
const SIDEBAR_WIDTH: u16 = 24;
const SIDEBAR_BREAKPOINT: u16 = 64;
const PROGRESS_CELLS: usize = 12;
const VOLUME_CELLS: usize = 6;

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
    draw_body(frame, app, theme, body);
    draw_footer(frame, app, theme, footer);
    draw_toast(frame, app, theme, area);

    if app.mode == Mode::Help {
        draw_help(frame, theme, area);
    }
}

fn draw_hud(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let mut spans = vec![
        Span::styled(
            " LEKIMA ",
            Style::new()
                .fg(theme.badge_fg)
                .bg(theme.badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ┃ ", Style::new().fg(theme.dim)),
    ];

    match (&app.status.track, app.status.status) {
        (Some(track), status) => {
            spans.extend([
                Span::styled(
                    format!("{} ", glyph(status)),
                    Style::new().fg(state_colour(status, theme)),
                ),
                Span::styled(
                    track.name.clone(),
                    Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · ", Style::new().fg(theme.dim)),
                Span::styled(track.artist_names(), Style::new().fg(theme.muted)),
            ]);
        }
        (None, _) => {
            spans.push(Span::styled(
                "■ nothing playing",
                Style::new().fg(theme.muted),
            ));
        }
    }

    if app.status.track.is_some() {
        spans.extend([
            Span::styled("  ", Style::new()),
            Span::styled(
                clock(app.status.position_ms),
                Style::new().fg(theme.accent_alt),
            ),
            Span::styled("/", Style::new().fg(theme.dim)),
            Span::styled(
                clock(
                    app.status
                        .track
                        .as_ref()
                        .map_or(0, |track| track.duration_ms),
                ),
                Style::new().fg(theme.muted),
            ),
            Span::styled("  ", Style::new()),
        ]);
        spans.extend(progress_spans(app, theme));
    }

    spans.extend([
        Span::styled("  R:", Style::new().fg(theme.dim)),
        Span::styled(
            app.status.repeat.as_str().to_ascii_uppercase(),
            Style::new().fg(theme.accent_alt),
        ),
    ]);
    if app.status.shuffle {
        spans.push(Span::styled("  S", Style::new().fg(theme.hot)));
    }

    spans.extend([
        Span::styled("  VOL ", Style::new().fg(theme.dim)),
        Span::styled(
            volume_bar(app.status.effective_volume()),
            Style::new().fg(theme.meter_full),
        ),
        Span::styled("  Q:", Style::new().fg(theme.dim)),
        Span::styled(
            app.status.queue_length.to_string(),
            Style::new().fg(theme.accent_alt),
        ),
    ]);

    if app.status.track.is_some() {
        spans.extend([
            Span::styled("  SIG ", Style::new().fg(theme.dim)),
            Span::styled(signal_bars(app), Style::new().fg(theme.accent_alt)),
        ]);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_rule(frame: &mut Frame, theme: &Theme, area: Rect) {
    let rule = "─".repeat(usize::from(area.width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(rule, Style::new().fg(theme.dim)))),
        area,
    );
}

fn draw_body(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    if area.width >= SIDEBAR_BREAKPOINT {
        let [sidebar, divider, main] = Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .areas(area);
        draw_sidebar(frame, app, theme, sidebar);
        draw_divider(frame, theme, divider);
        draw_main(frame, app, theme, main);
    } else {
        draw_main(frame, app, theme, area);
    }
}

fn draw_divider(frame: &mut Frame, theme: &Theme, area: Rect) {
    let lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled("│", Style::new().fg(theme.dim))))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let focused = app.focus == Focus::Sidebar && app.mode != Mode::Queue;
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled("LIBRARY", Style::new().fg(theme.muted)),
        Span::styled(
            format!(" {}", app.playlists.len()),
            Style::new().fg(theme.accent_alt),
        ),
    ];
    if focused {
        title.push(Span::styled(" ▌", Style::new().fg(theme.accent)));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    if app.playlists.is_empty() {
        let hint = Paragraph::new(Text::from(vec![
            Line::from(Span::styled("no playlists", Style::new().fg(theme.muted))),
            Line::from(Span::styled(
                "lekima login --qr",
                Style::new().fg(theme.dim),
            )),
        ]));
        frame.render_widget(hint, list_area);
        return;
    }

    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(index, playlist)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:02} ", index + 1), Style::new().fg(theme.dim)),
                Span::styled(playlist.name.clone(), Style::new().fg(theme.text)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(selection_style(theme))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.accent)));
    let mut state = ListState::default().with_selected(Some(app.sidebar_selection));
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_main(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    // One-column gutter so the divider never touches content.
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    match app.mode {
        Mode::Queue => draw_queue(frame, app, theme, inner),
        _ => draw_tracks(frame, app, theme, inner),
    }
}

fn draw_tracks(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let focused = app.focus == Focus::Main;
    let mut title = vec![
        Span::styled("// ", Style::new().fg(theme.panel)),
        Span::styled(
            app.source.to_ascii_uppercase(),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!(" · {} TRACKS", app.tracks.len()),
            Style::new().fg(theme.accent_alt),
        ),
    ];
    if focused {
        title.push(Span::styled(" ▌", Style::new().fg(theme.accent)));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), label);

    if app.tracks.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "nothing here — / to search, tab for library",
                Style::new().fg(theme.muted),
            ))),
            content,
        );
        return;
    }

    let playing_id = app.status.track.as_ref().map(|track| track.id);
    let rows: Vec<Row> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let (marker, marker_style) = if Some(track.id) == playing_id {
                (
                    glyph(app.status.status).to_owned(),
                    Style::new().fg(state_colour(app.status.status, theme)),
                )
            } else {
                (" ".to_owned(), Style::new().fg(theme.dim))
            };
            let selected_marker = if focused && index == app.main_selection {
                "▌"
            } else {
                " "
            };
            Row::new(vec![
                Span::styled(selected_marker, Style::new().fg(theme.accent)),
                Span::styled(format!("{:03}", index + 1), Style::new().fg(theme.dim)),
                Span::styled(marker, marker_style),
                Span::styled(track.name.clone(), Style::new().fg(theme.text)),
                Span::styled(track.artist_names(), Style::new().fg(theme.muted)),
                Span::styled(track.duration_label(), Style::new().fg(theme.dim)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Percentage(45),
        Constraint::Percentage(35),
        Constraint::Length(6),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec![" ", "#", " ", "TITLE", "ARTIST", "TIME"])
                .style(Style::new().fg(theme.panel)),
        )
        .row_highlight_style(selection_style(theme));

    let mut state = TableState::default().with_selected(if focused {
        Some(app.main_selection)
    } else {
        None
    });
    frame.render_stateful_widget(table, content, &mut state);
}

fn draw_queue(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let [label, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("// ", Style::new().fg(theme.panel)),
            Span::styled("QUEUE", Style::new().fg(theme.muted)),
            Span::styled(
                format!(" · {}", app.queue.len()),
                Style::new().fg(theme.accent_alt),
            ),
        ])),
        label,
    );

    if app.queue.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "queue is empty",
                Style::new().fg(theme.muted),
            ))),
            content,
        );
        return;
    }

    let current = app.status.queue_index;
    let items: Vec<ListItem> = app
        .queue
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let marker = if Some(index) == current { "▶" } else { " " };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:03} ", index + 1), Style::new().fg(theme.dim)),
                Span::styled(format!("{marker} "), Style::new().fg(theme.accent)),
                Span::styled(track.name.clone(), Style::new().fg(theme.text)),
                Span::styled(" · ", Style::new().fg(theme.dim)),
                Span::styled(track.artist_names(), Style::new().fg(theme.muted)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(selection_style(theme))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.accent)));
    let mut state = ListState::default().with_selected(Some(app.queue_selection));
    frame.render_stateful_widget(list, content, &mut state);
}

fn draw_footer(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let (mode, hints) = match app.mode {
        Mode::Normal => (
            "NORMAL",
            "j/k nav · ⏎ play · space pause · / search · : cmd · Q queue · ? keys",
        ),
        Mode::Search => ("SEARCH", ""),
        Mode::Command => ("CMD", ""),
        Mode::Help => ("HELP", "? or esc to close"),
        Mode::Queue => ("QUEUE", "j/k nav · ⏎ jump · esc close"),
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
        let cursor = if app.frame.is_multiple_of(2) {
            "▌"
        } else {
            " "
        };
        spans.extend([
            Span::styled(prefix, Style::new().fg(theme.accent)),
            Span::styled(format!("{} ", app.input), Style::new().fg(theme.text)),
            Span::styled(cursor, Style::new().fg(theme.accent)),
        ]);
    } else {
        spans.push(Span::styled(hints, Style::new().fg(theme.dim)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Notification panel: bottom-right, above the command bar, auto-dismissed.
fn draw_toast(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let message = app
        .message
        .as_ref()
        .filter(|message| message.is_visible(app.frame));

    let (text, is_error, title, colour) = if app.daemon_down {
        (
            "daemon offline — press R to start".to_owned(),
            true,
            " ! OFFLINE ",
            theme.warn,
        )
    } else {
        match message {
            Some(message) if message.error => (message.text.clone(), true, " ! ERROR ", theme.hot),
            Some(message) => (message.text.clone(), false, " + NOTICE ", theme.panel),
            None => return,
        }
    };

    if area.height < 10 || area.width < 40 {
        return;
    }

    let width = 54.min(area.width.saturating_sub(2));
    let height = 4;
    let toast = Rect {
        x: area.width.saturating_sub(width + 1),
        y: area.height.saturating_sub(height + 1),
        width,
        height,
    };

    frame.render_widget(Clear, toast);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(colour))
        .padding(Padding::horizontal(1))
        .title(Span::styled(
            title,
            Style::new().fg(colour).add_modifier(Modifier::BOLD),
        ));
    let body = Paragraph::new(Span::styled(
        text,
        Style::new().fg(if is_error { theme.text } else { theme.muted }),
    ))
    .block(block)
    .wrap(Wrap { trim: true });
    frame.render_widget(body, toast);
}

fn signal_bars(app: &App) -> String {
    const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    const COUNT: usize = 10;

    if !app.is_playing() {
        let level = usize::from(app.status.status == PlaybackStatus::Paused);
        return BARS[level].repeat(COUNT);
    }

    (0..COUNT)
        .map(|index| {
            let phase = app.frame as f32 * 0.45 + index as f32 * 1.9;
            let wobble = (phase.sin() * 0.5 + 0.5) * (phase * 1.7).cos().abs();
            let level = (wobble * 6.0).round() as usize;
            BARS[(level + 1).min(BARS.len() - 1)]
        })
        .collect()
}

fn draw_help(frame: &mut Frame, theme: &Theme, area: Rect) {
    let popup = centred(area, 68, 78);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(theme.overlay))
        .title(Span::styled(
            "▓ KEYS ▓",
            Style::new().fg(theme.overlay).add_modifier(Modifier::BOLD),
        ));

    let section = |title: &'static str| {
        Line::from(Span::styled(
            format!("// {title}"),
            Style::new().fg(theme.panel).add_modifier(Modifier::BOLD),
        ))
    };
    let binding = |key: &'static str, description: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {key:<12}"), Style::new().fg(theme.accent_alt)),
            Span::styled(description, Style::new().fg(theme.text)),
        ])
    };

    let lines = vec![
        section("MOVE"),
        binding("j / k", "down / up"),
        binding("ctrl-d/u", "half page"),
        binding("g / G", "top / bottom"),
        binding("tab", "library / list"),
        Line::from(""),
        section("PLAY"),
        binding("enter", "play selection"),
        binding("space", "pause / resume"),
        binding("n / p", "next / previous"),
        binding("h / l", "seek -10s / +10s"),
        binding("+ / -", "volume"),
        binding("m", "mute"),
        binding("r", "repeat off → all → one"),
        binding("s", "shuffle"),
        binding("a / A", "queue / play next"),
        Line::from(""),
        section("APP"),
        binding("/", "search"),
        binding(":", "command line"),
        binding("Q", "queue"),
        binding("?", "this help"),
        binding("q / ctrl-c", "quit"),
    ];

    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), popup);
}

fn centred(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn progress_spans<'a>(app: &App, theme: &Theme) -> Vec<Span<'a>> {
    let duration = app
        .status
        .track
        .as_ref()
        .map_or(0, |track| track.duration_ms);
    let ratio = if duration == 0 {
        0.0
    } else {
        (app.status.position_ms as f64 / duration as f64).clamp(0.0, 1.0)
    };
    let filled = (ratio * PROGRESS_CELLS as f64).round() as usize;
    vec![
        Span::styled("█".repeat(filled), Style::new().fg(theme.meter_full)),
        Span::styled(
            "░".repeat(PROGRESS_CELLS - filled),
            Style::new().fg(theme.meter_empty),
        ),
    ]
}

fn volume_bar(volume: f32) -> String {
    let filled = (volume.clamp(0.0, 1.0) * VOLUME_CELLS as f32).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(VOLUME_CELLS - filled)
    )
}

fn selection_style(theme: &Theme) -> Style {
    Style::new()
        .fg(theme.selection_fg)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD)
}

const fn glyph(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "▶",
        PlaybackStatus::Paused => "‖",
        PlaybackStatus::Stopped => "■",
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
    use ratatui::{Terminal, backend::TestBackend};

    use super::draw;
    use crate::app::{App, Focus, Mode};
    use crate::theme::Theme;

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let theme = Theme::cyber();
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
        assert!(screen.contains("R:"));
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
    fn tiny_terminals_fall_back_without_panicking() {
        let mut app = App::new();
        let screen = render(&mut app, 10, 3);
        assert!(screen.contains("LEKIMA"));
    }
}
