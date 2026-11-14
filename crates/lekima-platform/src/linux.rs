//! MPRIS backend (`org.mpris.MediaPlayer2.lekima`).
//!
//! The D-Bus object is served by `mpris-server` on the daemon's Tokio runtime.
//! Transport methods push [`MediaCommand`]s into a channel; the daemon polls
//! them from the same loop that drives the TUI, so every surface shares one
//! command path.

use std::sync::{Arc, Mutex};

use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus as MprisPlaybackStatus, PlayerInterface,
    Property, RootInterface, Server, Time, TrackId, Volume,
    zbus::{self, fdo},
};
use tokio::sync::{mpsc, watch};

use crate::{MediaCommand, MediaControls, NowPlaying, Repeat};

const BUS_SUFFIX: &str = "lekima";

/// Starts the MPRIS service.
///
/// # Errors
///
/// Returns a D-Bus error when no session bus is available, so the caller can
/// fall back to the no-op backend.
pub async fn start() -> zbus::Result<Arc<dyn MediaControls>> {
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (watch_tx, watch_rx) = watch::channel(NowPlaying::default());
    let state = Arc::new(Mutex::new(NowPlaying::default()));

    let player = MprisPlayer {
        state: Arc::clone(&state),
        commands: commands_tx,
    };
    let server = Server::new(BUS_SUFFIX, player).await?;
    tokio::spawn(publish(server, watch_rx));

    Ok(Arc::new(MprisControls {
        state,
        commands: Mutex::new(commands_rx),
        watch: watch_tx,
    }))
}

struct MprisControls {
    state: Arc<Mutex<NowPlaying>>,
    commands: Mutex<mpsc::UnboundedReceiver<MediaCommand>>,
    watch: watch::Sender<NowPlaying>,
}

impl MediaControls for MprisControls {
    fn update(&self, state: &NowPlaying) {
        let changed = {
            let Ok(mut slot) = self.state.lock() else {
                return;
            };
            let changed = frame_differs(&slot, state);
            *slot = state.clone();
            changed
        };
        // Position changes are polled by clients; only emit when something
        // stored in a property actually changed.
        if changed {
            self.watch.send_replace(state.clone());
        }
    }

    fn poll(&self) -> Vec<MediaCommand> {
        let Ok(mut receiver) = self.commands.lock() else {
            return Vec::new();
        };
        let mut commands = Vec::new();
        while let Ok(command) = receiver.try_recv() {
            commands.push(command);
        }
        commands
    }

    fn backend(&self) -> &'static str {
        "mpris"
    }
}

/// Emits `PropertiesChanged` whenever a published value differs.
async fn publish(server: Server<MprisPlayer>, mut updates: watch::Receiver<NowPlaying>) {
    while updates.changed().await.is_ok() {
        let state = updates.borrow_and_update().clone();
        if server
            .properties_changed([
                Property::PlaybackStatus(status_of(&state)),
                Property::Metadata(metadata_of(&state)),
                Property::LoopStatus(loop_of(state.repeat)),
                Property::Shuffle(state.shuffle),
                Property::Volume(volume_of(&state)),
                Property::CanSeek(state.has_track),
            ])
            .await
            .is_err()
        {
            return;
        }
    }
}

struct MprisPlayer {
    state: Arc<Mutex<NowPlaying>>,
    commands: mpsc::UnboundedSender<MediaCommand>,
}

impl MprisPlayer {
    fn snapshot(&self) -> NowPlaying {
        self.state
            .lock()
            .map_or_else(|_| NowPlaying::default(), |state| state.clone())
    }

    fn send(&self, command: MediaCommand) {
        let _ = self.commands.send(command);
    }
}

impl RootInterface for MprisPlayer {
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("lekima".to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("lekima".to_owned())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl PlayerInterface for MprisPlayer {
    async fn next(&self) -> fdo::Result<()> {
        self.send(MediaCommand::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.send(MediaCommand::Previous);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.send(MediaCommand::Pause);
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        self.send(MediaCommand::Toggle);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.send(MediaCommand::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.send(MediaCommand::Play);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.send(MediaCommand::SeekBy(offset.as_millis()));
        Ok(())
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        let state = self.snapshot();
        if state.has_track && track_id == track_of(&state) {
            self.send(MediaCommand::Seek(
                u64::try_from(position.as_millis()).unwrap_or(0),
            ));
        }
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Ok(())
    }

    async fn playback_status(&self) -> fdo::Result<MprisPlaybackStatus> {
        Ok(status_of(&self.snapshot()))
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(loop_of(self.snapshot().repeat))
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> zbus::Result<()> {
        self.send(MediaCommand::SetRepeat(repeat_of(loop_status)));
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn set_rate(&self, rate: PlaybackRate) -> zbus::Result<()> {
        if (rate - 1.0).abs() < f64::EPSILON {
            Ok(())
        } else {
            Err(fdo::Error::NotSupported("playback rate is fixed".to_owned()).into())
        }
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.snapshot().shuffle)
    }

    async fn set_shuffle(&self, shuffle: bool) -> zbus::Result<()> {
        self.send(MediaCommand::SetShuffle(shuffle));
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(metadata_of(&self.snapshot()))
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(volume_of(&self.snapshot()))
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        self.send(MediaCommand::SetVolume(volume.clamp(0.0, 1.0)));
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_millis(
            i64::try_from(self.snapshot().position_ms).unwrap_or(i64::MAX),
        ))
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self.snapshot().has_next)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.snapshot().has_previous)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(self.snapshot().has_track)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.snapshot().has_track)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// True when anything except the live position differs.
fn frame_differs(previous: &NowPlaying, next: &NowPlaying) -> bool {
    previous.track_id != next.track_id
        || previous.title != next.title
        || previous.artist != next.artist
        || previous.album != next.album
        || previous.duration_ms != next.duration_ms
        || previous.playing != next.playing
        || previous.has_track != next.has_track
        || previous.has_next != next.has_next
        || previous.has_previous != next.has_previous
        || previous.volume != next.volume
        || previous.muted != next.muted
        || previous.shuffle != next.shuffle
        || previous.repeat != next.repeat
}

fn status_of(state: &NowPlaying) -> MprisPlaybackStatus {
    if state.playing {
        MprisPlaybackStatus::Playing
    } else if state.has_track {
        MprisPlaybackStatus::Paused
    } else {
        MprisPlaybackStatus::Stopped
    }
}

fn loop_of(repeat: Repeat) -> LoopStatus {
    match repeat {
        Repeat::Off => LoopStatus::None,
        Repeat::All => LoopStatus::Playlist,
        Repeat::One => LoopStatus::Track,
    }
}

const fn repeat_of(status: LoopStatus) -> Repeat {
    match status {
        LoopStatus::None => Repeat::Off,
        LoopStatus::Track => Repeat::One,
        LoopStatus::Playlist => Repeat::All,
    }
}

fn volume_of(state: &NowPlaying) -> Volume {
    if state.muted { 0.0 } else { state.volume }
}

fn track_of(state: &NowPlaying) -> TrackId {
    TrackId::try_from(format!("/dev/lekima/track/{}", state.track_id).as_str())
        .unwrap_or(TrackId::NO_TRACK)
}

fn metadata_of(state: &NowPlaying) -> Metadata {
    if !state.has_track {
        return Metadata::new();
    }
    let mut builder = Metadata::builder().title(state.title.clone());
    if !state.artist.is_empty() {
        builder = builder.artist([state.artist.clone()]);
    }
    if !state.album.is_empty() {
        builder = builder.album(state.album.clone());
    }
    if state.duration_ms > 0 {
        builder = builder.length(Time::from_millis(
            i64::try_from(state.duration_ms).unwrap_or(i64::MAX),
        ));
    }
    builder.trackid(track_of(state)).build()
}

#[cfg(test)]
mod tests {
    use mpris_server::{LoopStatus, PlaybackStatus};

    use super::{frame_differs, loop_of, metadata_of, repeat_of, status_of, track_of};
    use crate::{NowPlaying, Repeat};

    fn sample() -> NowPlaying {
        NowPlaying {
            track_id: 42,
            title: "Song".to_owned(),
            artist: "Artist".to_owned(),
            album: "Album".to_owned(),
            duration_ms: 210_000,
            position_ms: 5_000,
            playing: true,
            has_track: true,
            has_next: true,
            has_previous: false,
            volume: 0.7,
            muted: false,
            shuffle: false,
            repeat: Repeat::Off,
        }
    }

    #[test]
    fn playback_status_follows_the_snapshot() {
        let mut state = sample();
        assert_eq!(status_of(&state), PlaybackStatus::Playing);
        state.playing = false;
        assert_eq!(status_of(&state), PlaybackStatus::Paused);
        state.has_track = false;
        assert_eq!(status_of(&state), PlaybackStatus::Stopped);
    }

    #[test]
    fn repeat_maps_both_ways() {
        for repeat in [Repeat::Off, Repeat::All, Repeat::One] {
            assert_eq!(repeat_of(loop_of(repeat)), repeat);
        }
        assert_eq!(loop_of(Repeat::All), LoopStatus::Playlist);
    }

    #[test]
    fn metadata_carries_title_artist_album_and_length() {
        let metadata = metadata_of(&sample());
        assert_eq!(metadata.title(), Some("Song"));
        assert_eq!(
            metadata.artist().as_deref(),
            Some(&["Artist".to_owned()][..])
        );
        assert_eq!(metadata.album(), Some("Album"));
        assert_eq!(
            metadata.length().map(|time| time.as_millis()),
            Some(210_000)
        );
        assert_eq!(
            metadata.trackid().map(|id| id.to_string()),
            Some("/dev/lekima/track/42".to_owned())
        );
    }

    #[test]
    fn an_empty_player_reports_no_metadata() {
        let metadata = metadata_of(&NowPlaying::default());
        assert!(metadata.title().is_none());
    }

    #[test]
    fn track_ids_round_trip_through_object_paths() {
        assert_eq!(track_of(&sample()).to_string(), "/dev/lekima/track/42");
    }

    #[test]
    fn position_moves_do_not_trigger_property_signals() {
        let state = sample();
        let mut later = state.clone();
        later.position_ms += 900;
        assert!(!frame_differs(&state, &later));
        later.title = "Other".to_owned();
        assert!(frame_differs(&state, &later));
    }
}
