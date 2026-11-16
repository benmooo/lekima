//! Renders the TUI into an off-screen buffer and prints it.
//!
//! Run with `cargo run -p lekima-tui --example preview` to review the layout
//! without a terminal, a daemon or an account.

use lekima_domain::{
    Artist, Comment, CommentReply, Comments, PlaybackStatus, Playlist, RepeatMode, Track,
};
use lekima_tui::Theme;
use lekima_tui::app::{App, CommentsState, LyricsState};
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

    let mut width = 126u16;
    let mut height = 22u16;
    let mut theme_name = "cyber".to_owned();
    let mut glyphs = "unicode".to_owned();
    let mut density = "compact".to_owned();
    let mut mode = "normal".to_owned();

    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--width" => {
                width = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(width);
            }
            "--height" => {
                height = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(height);
            }
            "--theme" => theme_name = arguments.next().unwrap_or(theme_name),
            "--glyphs" => glyphs = arguments.next().unwrap_or(glyphs),
            "--density" => density = arguments.next().unwrap_or(density),
            "--mode" => mode = arguments.next().unwrap_or(mode),
            "--long-title" => {
                if let Some(track) = app.status.track.as_mut() {
                    track.name =
                        "A very long track title that should never hide the clock".to_owned();
                }
            }
            "--buffered" => {
                app.status.buffered = vec![lekima_domain::BufferedRange {
                    start_ms: 0,
                    end_ms: 90_000,
                }];
            }
            "--empty" => {
                app.tracks.clear();
                app.source = "library".to_owned();
            }
            "--no-lyrics" => app.lyrics_open = false,
            "--toast-error" => {
                app.notify("provider rejected the session; run `lekima login`", true);
            }
            "--help" | "-h" => {
                println!(
                    "usage: preview [--width N] [--height N] [--theme NAME] \
                     [--glyphs unicode|ascii] [--density compact|comfortable] \
                     [--mode normal|queue|help|settings|comments] [--long-title] \
                     [--buffered] [--empty] [--no-lyrics] [--toast-error]"
                );
                return;
            }
            other => theme_name = other.to_owned(),
        }
    }

    if mode == "comments" {
        app.comments_state = CommentsState::Ready {
            track: 1.into(),
            comments: Comments {
                total: 20_535,
                has_more: true,
                hot: vec![Comment {
                    id: 1,
                    author: "NIGHTDRIVER".to_owned(),
                    avatar_url: None,
                    content: "The bassline at 01:12 is pure night-city fuel. Played this on repeat through Shibuya at 2am.".to_owned(),
                    timestamp_ms: 1_700_000_000_000,
                    liked_count: 44_054,
                    replies: vec![CommentReply {
                        author: "aka".to_owned(),
                        content: "the synth pad underneath is the real hook".to_owned(),
                    }],
                }],
                comments: vec![
                    Comment {
                        id: 2,
                        author: "程cheng艾影".to_owned(),
                        avatar_url: None,
                        content: "庐山牛逼，这是一条足够长的评论，用来确认它会按宽度换行而不是被截断。耳机里全是细节。".to_owned(),
                        timestamp_ms: 1_699_000_000_000,
                        liked_count: 1_238,
                        replies: Vec::new(),
                    },
                    Comment {
                        id: 3,
                        author: String::new(),
                        avatar_url: None,
                        content: String::new(),
                        timestamp_ms: 0,
                        liked_count: 0,
                        replies: Vec::new(),
                    },
                ],
            },
        };
    }

    app.mode = match mode.as_str() {
        "queue" => lekima_tui::app::Mode::Queue,
        "help" => lekima_tui::app::Mode::Help,
        "comments" => lekima_tui::app::Mode::Comments,
        "settings" => {
            app.settings_selection = 3;
            lekima_tui::app::Mode::Settings
        }
        _ => lekima_tui::app::Mode::Normal,
    };
    let theme = Theme::resolve(&theme_name, &glyphs, &density);
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
