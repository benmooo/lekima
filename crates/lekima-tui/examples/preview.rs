//! Renders the TUI into an off-screen buffer and prints it.
//!
//! Run with `cargo run -p lekima-tui --example preview` to review the layout
//! without a terminal, a daemon or an account.

use lekima_domain::{Artist, PlaybackStatus, Playlist, RepeatMode, Track};
use lekima_tui::Theme;
use lekima_tui::app::{App, LyricsState};
use lekima_tui::ui::draw;
use ratatui::{Terminal, backend::TestBackend};

fn track(id: u64, name: &str, artist: &str, duration: u64) -> Track {
    Track {
        id: id.into(),
        name: name.to_owned(),
        artists: vec![Artist {
            id: 1.into(),
            name: artist.to_owned(),
        }],
        album: None,
        duration_ms: duration,
    }
}

fn main() {
    let mut app = App::new();
    app.status.status = PlaybackStatus::Playing;
    app.status.track = Some(track(1, "Cigarettes and lighters", "LoMo", 109_000));
    app.status.position_ms = 42_000;
    app.status.repeat = RepeatMode::All;
    app.status.queue_length = 25;
    app.status.queue_index = Some(0);
    app.frame = 7;
    app.notify("queued 3 tracks", false);
    app.status.liked = true;
    app.status.wave = vec![
        0.15, 0.35, 0.6, 0.8, 0.45, 0.7, 0.3, 0.55, 0.9, 0.5, 0.25, 0.65, 0.4, 0.75, 0.3, 0.55,
        0.85, 0.45, 0.65, 0.3,
    ];
    app.lyrics_state = LyricsState::Ready {
        track: 1.into(),
        lyrics: lekima_domain::parse_lrc(
            "[00:40.00]Is this the real life?\n[00:44.00]Is this just fantasy?\n[00:48.00]Caught in a landslide\n[00:52.00]No escape from reality",
        ),
    };
    app.lyrics_open = true;
    app.source = "search: mota".to_owned();
    app.tracks = vec![
        track(1, "Cigarettes and lighters", "LoMo", 109_000),
        track(2, "Mota", "Russian Circles", 392_000),
        track(3, "Hyperballad", "Björk", 321_000),
        track(4, "Everything In Its Right Place", "Radiohead", 251_000),
        track(5, "Teardrop", "Massive Attack", 330_000),
        track(6, "Windowlicker", "Aphex Twin", 366_000),
    ];
    app.queue = app.tracks.clone();
    app.playlists = vec![
        Playlist {
            id: 10.into(),
            name: "Late night".to_owned(),
            description: String::new(),
            track_count: 42,
            cover_url: None,
            owner: None,
        },
        Playlist {
            id: 11.into(),
            name: "Focus".to_owned(),
            description: String::new(),
            track_count: 18,
            cover_url: None,
            owner: None,
        },
        Playlist {
            id: 12.into(),
            name: "Daily".to_owned(),
            description: String::new(),
            track_count: 28,
            cover_url: None,
            owner: None,
        },
    ];

    let width = 126u16;
    let height = 22u16;
    let theme_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cyber".to_owned());
    let theme = Theme::resolve(&theme_name, "unicode", "compact");
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| draw(frame, &mut app, &theme))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    for y in 0..height {
        let line: String = (0..width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        println!("{}", line.trim_end());
    }
}
