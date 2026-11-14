//! Rendering. `draw` is a pure function of [`App`] plus the current frame area.

use lekima_domain::{PlaybackStatus, Track, format_duration_ms};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState},
};

use crate::app::{App, Focus, Mode};

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.clamp();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, app, header);
    draw_body(frame, app, body);
    draw_footer(frame, app, footer);

    if app.mode == Mode::Help {
        draw_help(frame, frame.area());
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .title(Line::from(vec![
            Span::styled(" lekima ", Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" {} ", app.source),
                Style::new().fg(Color::DarkGray),
            ),
        ]))
        .border_style(Style::new().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [info_area, gauge_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(inner);

    let (icon, title, subtitle) = match (&app.status.track, app.status.status) {
        (Some(track), status) => (
            status_icon(status),
            format!(" {} ", track.name),
            format!(
                "{} · {}",
                track.artist_names(),
                if track.album_name().is_empty() {
                    app.source.clone()
                } else {
                    track.album_name().to_owned()
                }
            ),
        ),
        (None, _) => (
            " ■ ",
            " nothing playing ".to_owned(),
            "press / to search, or :play <query>".to_owned(),
        ),
    };

    let info = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(icon, Style::new().fg(accent(app))),
            Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(subtitle, Style::new().fg(Color::Gray))),
    ]));
    frame.render_widget(info, info_area);

    let duration = app
        .status
        .track
        .as_ref()
        .map_or(1, |track| track.duration_ms)
        .max(1);
    let ratio = (app.status.position_ms as f64 / duration as f64).clamp(0.0, 1.0);
    let label = format!(
        "{} / {}   vol {:.0}%   repeat {}   queue {}",
        format_duration_ms(app.status.position_ms),
        format_duration_ms(duration),
        app.status.effective_volume() * 100.0,
        app.status.repeat.as_str(),
        app.status.queue_length,
    );
    let gauge = Gauge::default()
        .ratio(ratio)
        .label(Span::styled(label, Style::new().fg(Color::Gray)))
        .gauge_style(Style::new().fg(accent(app)))
        .use_unicode(true);
    frame.render_widget(gauge, gauge_area);
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let [sidebar, main] =
        Layout::horizontal([Constraint::Length(26), Constraint::Min(20)]).areas(area);

    draw_sidebar(frame, app, sidebar);
    match app.mode {
        Mode::Queue => draw_queue(frame, app, main),
        _ => draw_tracks(frame, app, main),
    }
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Sidebar && app.mode != Mode::Queue;
    let block = Block::bordered()
        .title(Line::from(" library "))
        .border_style(border_style(focused));
    let inner = block.inner(area);

    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .map(|playlist| {
            let count = if playlist.track_count > 0 {
                format!(" ({})", playlist.track_count)
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::raw(playlist.name.clone()),
                Span::styled(count, Style::new().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    if items.is_empty() {
        let hint = Paragraph::new(Text::from(vec![
            Line::from(Span::styled("No playlists", Style::new().fg(Color::Gray))),
            Line::from(Span::styled(
                "run `lekima login`",
                Style::new().fg(Color::DarkGray),
            )),
        ]));
        frame.render_widget(hint, inner);
        frame.render_widget(block, area);
        return;
    }

    let list = List::new(items)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");
    let mut state = ListState::default().with_selected(Some(app.sidebar_selection));
    frame.render_stateful_widget(list, inner, &mut state);
    frame.render_widget(block, area);
}

fn draw_tracks(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Main;
    let block = Block::bordered()
        .title(Line::from(format!(
            " {} · {} tracks ",
            app.source,
            app.tracks.len()
        )))
        .border_style(border_style(focused));
    let inner = block.inner(area);

    if app.tracks.is_empty() {
        let hint = Paragraph::new(Line::from(Span::styled(
            "press / to search, or Tab to the library",
            Style::new().fg(Color::Gray),
        )));
        frame.render_widget(hint, inner);
        frame.render_widget(block, area);
        return;
    }

    let playing_id = app.status.track.as_ref().map(|track| track.id);
    let rows: Vec<Row> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let marker = if Some(track.id) == playing_id {
                status_icon(app.status.status)
            } else {
                " "
            };
            Row::new(vec![
                format!("{marker} {}", index + 1),
                track.name.clone(),
                track.artist_names(),
                track.duration_label(),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(5),
        Constraint::Percentage(45),
        Constraint::Percentage(35),
        Constraint::Length(7),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec!["#", "TITLE", "ARTIST", "TIME"])
                .style(Style::new().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");

    let mut state = TableState::default().with_selected(if focused {
        Some(app.main_selection)
    } else {
        None
    });
    frame.render_stateful_widget(table, inner, &mut state);
    frame.render_widget(block, area);
}

fn draw_queue(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .title(Line::from(format!(" queue · {} ", app.queue.len())))
        .border_style(border_style(true));
    let inner = block.inner(area);

    if app.queue.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "queue is empty",
                Style::new().fg(Color::Gray),
            ))),
            inner,
        );
        frame.render_widget(block, area);
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
                Span::styled(format!("{marker} "), Style::new().fg(accent(app))),
                Span::raw(track_label(track)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");
    let mut state = ListState::default().with_selected(Some(app.queue_selection));
    frame.render_stateful_widget(list, inner, &mut state);
    frame.render_widget(block, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let (mode, hints) = match app.mode {
        Mode::Normal => (
            " NORMAL ",
            "j/k move · ⏎ play · space pause · / search · : command · Q queue · ? help",
        ),
        Mode::Search => (" / ", "type to search · ⏎ run · esc cancel"),
        Mode::Command => (" : ", "q · play <query> · vol <0-100> · queue · help"),
        Mode::Help => (" HELP ", "? or esc to close"),
        Mode::Queue => (" QUEUE ", "j/k move · ⏎ jump · esc close"),
    };

    let mut spans = vec![
        Span::styled(
            mode,
            Style::new()
                .fg(Color::Black)
                .bg(accent(app))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    if matches!(app.mode, Mode::Search | Mode::Command) {
        spans.push(Span::styled(
            format!("{}{}", app.input, "▌"),
            Style::new().add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(hints, Style::new().fg(Color::DarkGray)));
    }

    if let Some(message) = &app.message {
        spans.push(Span::raw("  ·  "));
        spans.push(Span::styled(
            message.text.clone(),
            if message.error {
                Style::new().fg(Color::Red)
            } else {
                Style::new().fg(Color::Green)
            },
        ));
    }
    if app.daemon_down {
        spans.push(Span::styled(
            "  ·  daemon offline (R to start)",
            Style::new().fg(Color::Yellow),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 68, 70);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(Line::from(" keys "))
        .border_style(Style::new().fg(Color::Gray));

    let lines = vec![
        Line::from(Span::styled(
            "MOVEMENT",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from("  j / k        down / up"),
        Line::from("  ctrl-d/u     half page"),
        Line::from("  g / G        top / bottom"),
        Line::from("  tab          switch sidebar / list"),
        Line::from(""),
        Line::from(Span::styled(
            "PLAYBACK",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from("  enter        play selection"),
        Line::from("  space        pause / resume"),
        Line::from("  n / p        next / previous"),
        Line::from("  h / l        seek -10s / +10s"),
        Line::from("  + / -        volume"),
        Line::from("  m            mute"),
        Line::from("  r            repeat off → all → one"),
        Line::from("  s            shuffle toggle"),
        Line::from("  a / A        enqueue / play next"),
        Line::from(""),
        Line::from(Span::styled(
            "APP",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from("  /            search"),
        Line::from("  :            command line"),
        Line::from("  Q            queue"),
        Line::from("  ?            this help"),
        Line::from("  q / ctrl-c   quit"),
    ];
    let paragraph = Paragraph::new(Text::from(lines)).block(block);
    frame.render_widget(paragraph, popup);
}

fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn track_label(track: &Track) -> String {
    format!("{} · {}", track.name, track.artist_names())
}

const fn status_icon(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "▶",
        PlaybackStatus::Paused => "⏸",
        PlaybackStatus::Stopped => "■",
    }
}

fn accent(app: &App) -> Color {
    if app.is_playing() {
        Color::Green
    } else {
        Color::Yellow
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Gray)
    } else {
        Style::new().fg(Color::DarkGray)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::draw;
    use crate::app::{App, Focus, Mode};

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn empty_state_renders_the_brand_and_a_hint() {
        let mut app = App::new();
        let screen = render(&mut app, 90, 24);
        assert!(screen.contains("lekima"));
        assert!(screen.contains("press / to search"));
    }

    #[test]
    fn playing_state_renders_the_track() {
        let mut app = App::new();
        app.status.status = lekima_domain::PlaybackStatus::Playing;
        app.status.track = Some(lekima_domain::Track {
            id: 1.into(),
            name: "Mota".to_owned(),
            artists: vec![lekima_domain::Artist {
                id: 2.into(),
                name: "Russian Circles".to_owned(),
            }],
            album: None,
            duration_ms: 300_000,
        });
        app.tracks = vec![app.status.track.clone().unwrap()];
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("Mota"));
        assert!(screen.contains("Russian Circles"));
    }

    #[test]
    fn help_and_queue_modes_do_not_panic_at_small_sizes() {
        let mut app = App::new();
        app.mode = Mode::Help;
        let _ = render(&mut app, 40, 12);
        app.mode = Mode::Queue;
        app.queue = vec![lekima_domain::Track {
            id: 1.into(),
            name: "queued".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 0,
        }];
        let screen = render(&mut app, 40, 12);
        assert!(screen.contains("queue"));
        app.mode = Mode::Search;
        app.input = "mota".to_owned();
        let screen = render(&mut app, 80, 24);
        assert!(screen.contains("mota"));
        app.focus = Focus::Sidebar;
        let _ = render(&mut app, 20, 8);
    }
}
