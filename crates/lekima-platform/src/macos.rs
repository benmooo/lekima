//! macOS Now Playing host (`lekima media-host`).
//!
//! MediaPlayer only exposes a process to the system when an `NSApplication`
//! with a run loop owns it on the main thread. The daemon's main thread belongs
//! to the async runtime, so this half runs as a small helper process:
//!
//! - stdin: one JSON [`NowPlaying`] snapshot per line, replaced continuously;
//! - stdout: one JSON [`MediaCommand`] per line for media keys and Control
//!   Center;
//! - the process exits when the daemon closes stdin.
//!
//! The app runs as an accessory (no Dock icon, no window) with a Cocoa run
//! loop, which is what makes it eligible to become the system Now Playing app.

use std::{
    io::{BufRead, Write},
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use block2::RcBlock;
use objc2::{
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
    MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate,
    MPNowPlayingPlaybackState, MPRemoteCommandCenter, MPRemoteCommandEvent,
    MPRemoteCommandHandlerStatus,
};

use crate::{MediaCommand, NowPlaying};

/// Runs the Now Playing host on the calling (main) thread until stdin closes.
///
/// # Errors
///
/// Returns an error when called off the main thread or when the reader thread
/// cannot be spawned.
pub fn run_host() -> std::io::Result<()> {
    let Some(marker) = objc2::MainThreadMarker::new() else {
        return Err(std::io::Error::other(
            "the Now Playing host must run on the main thread",
        ));
    };

    let app = NSApplication::sharedApplication(marker);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();

    let output = Arc::new(Mutex::new(std::io::stdout()));
    let _targets = register(Arc::clone(&output));

    std::thread::Builder::new()
        .name("lekima-nowplaying-input".to_owned())
        .spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<NowPlaying>(&line) {
                    Ok(state) => apply(&state),
                    Err(error) => eprintln!("lekima media-host: bad state line: {error}"),
                }
            }
            // The daemon closed stdin: it is gone.
            std::process::exit(0);
        })?;

    app.run();
    Ok(())
}

fn register(output: Arc<Mutex<std::io::Stdout>>) -> Vec<Retained<AnyObject>> {
    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    let mut targets = Vec::new();

    let mut bind = |command: Retained<objc2_media_player::MPRemoteCommand>,
                    action: MediaCommand| {
        let output = Arc::clone(&output);
        let block = RcBlock::new(move |_event: NonNull<MPRemoteCommandEvent>| {
            send(&output, action);
            MPRemoteCommandHandlerStatus::Success
        });
        // SAFETY: MediaPlayer retains the handler target for as long as the
        // command lives, and we keep it in `targets`.
        targets.push(unsafe { command.addTargetWithHandler(&block) });
    };

    bind(unsafe { center.playCommand() }, MediaCommand::Play);
    bind(unsafe { center.pauseCommand() }, MediaCommand::Pause);
    bind(
        unsafe { center.togglePlayPauseCommand() },
        MediaCommand::Toggle,
    );
    bind(unsafe { center.stopCommand() }, MediaCommand::Stop);
    bind(unsafe { center.nextTrackCommand() }, MediaCommand::Next);
    bind(
        unsafe { center.previousTrackCommand() },
        MediaCommand::Previous,
    );

    let output_for_seek = Arc::clone(&output);
    let seek = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
        // SAFETY: the event is valid for the call; the downcast checks its type.
        let event = unsafe { event.as_ref() };
        let object: &AnyObject = unsafe { &*std::ptr::from_ref(event).cast() };
        match object.downcast_ref::<MPChangePlaybackPositionCommandEvent>() {
            Some(event) => {
                let seconds = unsafe { event.positionTime() }.max(0.0);
                send(
                    &output_for_seek,
                    MediaCommand::Seek((seconds * 1000.0) as u64),
                );
                MPRemoteCommandHandlerStatus::Success
            }
            None => MPRemoteCommandHandlerStatus::CommandFailed,
        }
    });
    targets.push(unsafe {
        center
            .changePlaybackPositionCommand()
            .addTargetWithHandler(&seek)
    });

    targets
}

fn send(output: &Mutex<std::io::Stdout>, command: MediaCommand) {
    let Ok(line) = serde_json::to_string(&command) else {
        return;
    };
    if let Ok(mut output) = output.lock() {
        let _ = writeln!(output, "{line}");
        let _ = output.flush();
    }
}

fn apply(state: &NowPlaying) {
    autoreleasepool(|_| {
        // SAFETY: the center is safe to use from any thread; the dictionary
        // outlives the call and every value is kept alive until the copy.
        unsafe {
            let center = MPNowPlayingInfoCenter::defaultCenter();
            if state.has_track {
                let info = now_playing_info(state);
                center.setNowPlayingInfo(Some(&info));
            } else {
                center.setNowPlayingInfo(None);
            }
            center.setPlaybackState(playback_state(state));
        }
    });
}

fn playback_state(state: &NowPlaying) -> MPNowPlayingPlaybackState {
    if state.playing {
        MPNowPlayingPlaybackState::Playing
    } else if state.has_track {
        MPNowPlayingPlaybackState::Paused
    } else {
        MPNowPlayingPlaybackState::Stopped
    }
}

fn now_playing_info(state: &NowPlaying) -> Retained<NSDictionary<NSString, AnyObject>> {
    let title = any(NSString::from_str(&state.title));
    let artist = any(NSString::from_str(&state.artist));
    let album = any(NSString::from_str(&state.album));
    let duration = any(NSNumber::new_f64(state.duration_ms as f64 / 1000.0));
    let position = any(NSNumber::new_f64(state.position_ms as f64 / 1000.0));
    let rate = any(NSNumber::new_f64(if state.playing { 1.0 } else { 0.0 }));

    // SAFETY: the framework provides these immutable storage keys.
    let keys = unsafe {
        [
            MPMediaItemPropertyTitle,
            MPMediaItemPropertyArtist,
            MPMediaItemPropertyAlbumTitle,
            MPMediaItemPropertyPlaybackDuration,
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            MPNowPlayingInfoPropertyPlaybackRate,
        ]
    };
    let values = [&*title, &*artist, &*album, &*duration, &*position, &*rate];
    NSDictionary::from_slices(&keys, &values)
}

/// Reinterprets an owned Objective-C object as `AnyObject`.
fn any<T: objc2::Message>(value: Retained<T>) -> Retained<AnyObject> {
    // SAFETY: every Objective-C object shares the `AnyObject` layout.
    unsafe { Retained::cast_unchecked::<AnyObject>(value) }
}

#[cfg(test)]
mod tests {
    use super::{now_playing_info, playback_state};
    use crate::NowPlaying;
    use objc2::rc::autoreleasepool;
    use objc2_media_player::MPNowPlayingPlaybackState;

    #[test]
    fn playback_state_tracks_the_snapshot() {
        let mut state = NowPlaying {
            playing: true,
            has_track: true,
            ..NowPlaying::default()
        };
        assert_eq!(playback_state(&state), MPNowPlayingPlaybackState::Playing);
        state.playing = false;
        assert_eq!(playback_state(&state), MPNowPlayingPlaybackState::Paused);
        state.has_track = false;
        assert_eq!(playback_state(&state), MPNowPlayingPlaybackState::Stopped);
    }

    #[test]
    fn info_dictionary_has_entries_for_a_playing_track() {
        let state = NowPlaying {
            track_id: 1,
            title: "Song".to_owned(),
            artist: "Artist".to_owned(),
            album: "Album".to_owned(),
            duration_ms: 180_000,
            position_ms: 42_000,
            playing: true,
            has_track: true,
            ..NowPlaying::default()
        };
        autoreleasepool(|_| {
            let info = now_playing_info(&state);
            assert_eq!(info.len(), 6);
        });
    }
}
