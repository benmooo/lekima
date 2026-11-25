//! Renders the TUI into an off-screen buffer and prints it.
//!
//! Run with `cargo run -p lekima-tui --example preview` to review the layout
//! without a terminal, a daemon or an account.

use lekima_domain::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, Comment, CommentReply, Comments,
    PlaybackStatus, Playlist, RepeatMode, SearchResults, SearchTotals, Track,
};
use lekima_ipc::Reply;
use lekima_tui::Theme;
use lekima_tui::app::{AlbumState, App, ArtistState, CommentsState, LyricsState, View};
use lekima_tui::ui::draw;
use ratatui::{Terminal, backend::TestBackend};

/// Wall clock in Unix milliseconds, so preview ages look current.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

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
    let mut filter = "all".to_owned();
    let mut color = "truecolor".to_owned();
    let mut motion = "auto".to_owned();
    let mut ansi = false;

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
            "--mode" | "--screen" => mode = arguments.next().unwrap_or(mode),
            "--color" => color = arguments.next().unwrap_or(color),
            "--motion" => motion = arguments.next().unwrap_or(motion),
            "--ansi" => ansi = true,
            "--filter" => filter = arguments.next().unwrap_or(filter),
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
                     [--screen|--mode normal|queue|help|settings|comments|history|search|results|artist|album] \
                     [--color truecolor|256|16|mono] [--motion auto|full|reduced] [--ansi] \
                     [--filter all|tracks|albums|artists|playlists] [--long-title] \
                     [--buffered] [--empty] [--no-lyrics] [--toast-error]"
                );
                return;
            }
            other => theme_name = other.to_owned(),
        }
    }

    {
        use lekima_tui::app::SearchFilter;

        app.search.filter = match filter.as_str() {
            "tracks" => SearchFilter::Tracks,
            "albums" => SearchFilter::Albums,
            "artists" => SearchFilter::Artists,
            "playlists" => SearchFilter::Playlists,
            _ => SearchFilter::All,
        };
        app.search.applied_filter = app.search.filter;
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

    if mode == "search" || mode == "results" {
        app.search.query = "mota".to_owned();
        let search_filter = match filter.as_str() {
            "tracks" => lekima_tui::app::SearchFilter::Tracks,
            "albums" => lekima_tui::app::SearchFilter::Albums,
            "artists" => lekima_tui::app::SearchFilter::Artists,
            "playlists" => lekima_tui::app::SearchFilter::Playlists,
            _ => lekima_tui::app::SearchFilter::All,
        };
        app.search.applied_filter = search_filter;
        app.search.loading = true;
        app.view = View::Search;
        app.apply(Ok(Reply::Search {
            query: "mota".to_owned(),
            kind: search_filter.kind(),
            results: SearchResults {
                tracks: app.tracks.clone(),
                albums: vec![
                    Album {
                        id: AlbumId(242_091_487),
                        name: "Mota".to_owned(),
                        cover_url: None,
                        artist: Some(Artist {
                            id: ArtistId(31_787_312),
                            name: "Aash".to_owned(),
                        }),
                        year: Some(2024),
                        track_count: Some(1),
                        description: String::new(),
                    },
                    Album {
                        id: AlbumId(3_061_999),
                        name: "Station".to_owned(),
                        cover_url: None,
                        artist: Some(Artist {
                            id: ArtistId(1),
                            name: "Russian Circles".to_owned(),
                        }),
                        year: Some(2008),
                        track_count: Some(8),
                        description: String::new(),
                    },
                ],
                artists: vec![Artist {
                    id: ArtistId(1),
                    name: "Russian Circles".to_owned(),
                }],
                playlists: vec![Playlist {
                    id: 3_778_678.into(),
                    name: "Post-rock essentials".to_owned(),
                    description: String::new(),
                    track_count: 42,
                    cover_url: None,
                    owner: None,
                }],
                totals: SearchTotals {
                    tracks: 6,
                    albums: 2,
                    artists: 1,
                    playlists: 1,
                },
                total: 10,
            },
        }));
    }
    if mode == "history" {
        let played =
            |id: u64, name: &str, artist: &str, age_ms: i64, played_ms: u64, done: bool| {
                lekima_domain::PlayRecord {
                    track: track(id, name, artist, 240_000),
                    started_at: now_ms() - age_ms,
                    played_ms,
                    completed: done,
                    kind: lekima_domain::QueueKind::Manual,
                }
            };
        app.plays = vec![
            played(2, "Teardrop", "Massive Attack", 4 * 60_000, 330_000, true),
            played(3, "Mota", "Russian Circles", 22 * 60_000, 392_000, true),
            played(4, "Late night", "LoMo", 3 * 3_600_000, 61_000, false),
            played(
                5,
                "Bohemian Rhapsody",
                "Queen",
                2 * 86_400_000,
                355_000,
                true,
            ),
        ];
        app.plays_selection = 1;
    }
    if mode == "artist" {
        app.view = View::Artist;
        app.artist = ArtistState {
            id: Some(ArtistId(6452)),
            detail: Some(ArtistDetail {
                id: ArtistId(6452),
                name: "周杰伦".to_owned(),
                avatar_url: None,
                cover_url: None,
                album_count: 49,
                track_count: 592,
                description: "华语流行乐男歌手、音乐人、演员、导演，毕业于淡江中学。".to_owned(),
                aliases: vec!["Jay Chou".to_owned(), "周董".to_owned()],
            }),
            albums: vec![
                Album {
                    id: AlbumId(1),
                    name: "太阳之子".to_owned(),
                    cover_url: None,
                    artist: None,
                    year: Some(2026),
                    track_count: Some(12),
                    description: String::new(),
                },
                Album {
                    id: AlbumId(2),
                    name: "最伟大的作品".to_owned(),
                    cover_url: None,
                    artist: None,
                    year: Some(2022),
                    track_count: Some(12),
                    description: String::new(),
                },
                Album {
                    id: AlbumId(3),
                    name: "Mojito".to_owned(),
                    cover_url: None,
                    artist: None,
                    year: Some(2021),
                    track_count: Some(1),
                    description: String::new(),
                },
            ],
            has_more: true,
            paging: false,
            loading: false,
            error: None,
        };
        app.source = "artist: 周杰伦".to_owned();
    }
    if mode == "album" {
        app.view = View::Album;
        let album = Album {
            id: AlbumId(242_091_487),
            name: "Mota".to_owned(),
            cover_url: None,
            artist: Some(Artist {
                id: ArtistId(31_787_312),
                name: "Aash".to_owned(),
            }),
            year: Some(2024),
            track_count: Some(6),
            description: String::new(),
        };
        app.album = AlbumState {
            id: Some(AlbumId(242_091_487)),
            detail: Some(AlbumDetail {
                album,
                tracks: vec![
                    track(101, "Mota", "Aash", 154_000),
                    track(102, "Dust", "Aash", 201_000),
                    track(103, "Signal", "Aash", 188_000),
                    track(104, "Drift", "Aash", 233_000),
                ],
            }),
            loading: false,
            error: None,
        };
        app.main_selection = 0;
        app.source = "album: Mota".to_owned();
    }

    {
        use lekima_tui::app::SearchFilter;

        app.search.filter = match filter.as_str() {
            "tracks" => SearchFilter::Tracks,
            "albums" => SearchFilter::Albums,
            "artists" => SearchFilter::Artists,
            "playlists" => SearchFilter::Playlists,
            _ => SearchFilter::All,
        };
        app.search.applied_filter = app.search.filter;
    }

    app.mode = match mode.as_str() {
        "queue" => lekima_tui::app::Mode::Queue,
        "help" => lekima_tui::app::Mode::Help,
        "comments" => lekima_tui::app::Mode::Comments,
        "history" => lekima_tui::app::Mode::History,
        "search" => {
            app.input = "mota".to_owned();
            app.cursor = app.input.len();
            app.search.query = "mota".to_owned();
            app.view = View::Search;
            lekima_tui::app::Mode::Search
        }
        "results" => lekima_tui::app::Mode::Normal,
        "settings" => {
            app.settings_selection = 3;
            lekima_tui::app::Mode::Settings
        }
        _ => lekima_tui::app::Mode::Normal,
    };
    let color_mode = Theme::color_mode(&color).unwrap_or_default();
    let theme = Theme::resolve(&theme_name, &glyphs, &density)
        .with_motion(&motion)
        .with_color_mode(color_mode);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| draw(frame, &mut app, &theme))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    if ansi {
        use ratatui::style::{Color, Modifier};

        fn sgr(color: Color, background: bool) -> Option<String> {
            let base = if background { 10 } else { 0 };
            Some(match color {
                Color::Reset => return None,
                Color::Black => format!("{}", 30 + base),
                Color::Red => format!("{}", 31 + base),
                Color::Green => format!("{}", 32 + base),
                Color::Yellow => format!("{}", 33 + base),
                Color::Blue => format!("{}", 34 + base),
                Color::Magenta => format!("{}", 35 + base),
                Color::Cyan => format!("{}", 36 + base),
                Color::Gray => format!("{}", 37 + base),
                Color::DarkGray => format!("{}", 90 + base),
                Color::LightRed => format!("{}", 91 + base),
                Color::LightGreen => format!("{}", 92 + base),
                Color::LightYellow => format!("{}", 93 + base),
                Color::LightBlue => format!("{}", 94 + base),
                Color::LightMagenta => format!("{}", 95 + base),
                Color::LightCyan => format!("{}", 96 + base),
                Color::White => format!("{}", 97 + base),
                Color::Indexed(index) => {
                    if background {
                        format!("48;5;{index}")
                    } else {
                        format!("38;5;{index}")
                    }
                }
                Color::Rgb(red, green, blue) => {
                    if background {
                        format!("48;2;{red};{green};{blue}")
                    } else {
                        format!("38;2;{red};{green};{blue}")
                    }
                }
            })
        }

        for y in 0..height {
            let mut line = String::new();
            for x in 0..width {
                let cell = &buffer[(x, y)];
                let mut codes = Vec::new();
                if let Some(code) = sgr(cell.fg, false) {
                    codes.push(code);
                }
                if let Some(code) = sgr(cell.bg, true) {
                    codes.push(code);
                }
                if cell.modifier.contains(Modifier::BOLD) {
                    codes.push("1".to_owned());
                }
                if !codes.is_empty() {
                    line.push_str(&format!("\u{1b}[{}m", codes.join(";")));
                }
                line.push_str(cell.symbol());
                line.push_str("\u{1b}[0m");
            }
            {
                use std::io::Write as _;
                let _ = writeln!(std::io::stdout().lock(), "{}", line.trim_end());
            }
        }
        return;
    }
    for y in 0..height {
        let line: String = (0..width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        {
            use std::io::Write as _;
            let _ = writeln!(std::io::stdout().lock(), "{}", line.trim_end());
        }
    }
}
