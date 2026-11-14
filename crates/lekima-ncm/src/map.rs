//! Provider → domain mapping.
//!
//! Everything here is pure: fixtures in, domain values out. The provider is
//! allowed to omit fields, so defaults are explicit and every field that can be
//! absent is documented.

use lekima_domain as domain;
use ncmapi as provider;

pub(crate) fn track(track: provider::Track) -> domain::Track {
    let album = if track.album.id == 0 && track.album.name.is_empty() {
        None
    } else {
        Some(domain::Album {
            id: domain::AlbumId(track.album.id),
            name: track.album.name,
            cover_url: (!track.album.pic_url.is_empty()).then_some(track.album.pic_url),
        })
    };

    domain::Track {
        id: domain::TrackId(track.id),
        name: track.name,
        artists: track
            .artists
            .into_iter()
            .map(|artist| domain::Artist {
                id: domain::ArtistId(artist.id),
                name: artist.name,
            })
            .collect(),
        album,
        duration_ms: u64::from(track.duration),
    }
}

pub(crate) fn search_results(body: provider::SearchBody) -> domain::SearchResults {
    let tracks: Vec<_> = body.result.songs.into_iter().map(track).collect();
    domain::SearchResults {
        total: u64::try_from(tracks.len()).unwrap_or(u64::MAX),
        tracks,
        playlists: Vec::new(),
        artists: Vec::new(),
    }
}

pub(crate) fn playlist(playlist: provider::Playlist) -> domain::Playlist {
    domain::Playlist {
        id: domain::PlaylistId(playlist.id),
        name: playlist.name,
        description: playlist.description.unwrap_or_default(),
        track_count: 0,
        cover_url: None,
        owner: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_complete_track() {
        let raw = serde_json::json!({
            "id": 42,
            "name": "mota",
            "artists": [{"id": 7, "name": "Zhang"}],
            "album": {"id": 9, "name": "Album", "picUrl": "https://img/9.jpg"},
            "duration": 245_000
        });
        let provider_track: provider::Track = serde_json::from_value(raw).unwrap();
        let track = track(provider_track);
        assert_eq!(track.id, domain::TrackId(42));
        assert_eq!(track.artist_names(), "Zhang");
        assert_eq!(track.album_name(), "Album");
        assert_eq!(track.duration_ms, 245_000);
        assert_eq!(
            track.album.unwrap().cover_url.as_deref(),
            Some("https://img/9.jpg")
        );
    }

    #[test]
    fn tolerates_missing_album_and_artist_fields() {
        let raw = serde_json::json!({
            "id": 1,
            "name": "nameless",
            "artists": []
        });
        let provider_track: provider::Track = serde_json::from_value(raw).unwrap();
        let track = track(provider_track);
        assert!(track.album.is_none());
        assert_eq!(track.artist_names(), "Unknown artist");
        assert_eq!(track.duration_label(), "--:--");
    }

    #[test]
    fn maps_search_payloads() {
        let raw = serde_json::json!({
            "code": 200,
            "result": {
                "songs": [{"id": 1, "name": "a", "artists": [], "duration": 1000}],
                "hasMore": true
            }
        });
        let response: provider::ApiResponse<provider::SearchBody> =
            serde_json::from_value(raw).unwrap();
        let results = search_results(response.body);
        assert_eq!(results.tracks.len(), 1);
        assert_eq!(results.total, 1);
    }

    #[test]
    fn maps_playlists_without_optional_fields() {
        let raw = serde_json::json!({"id": 5, "name": "Late night"});
        let provider_playlist: provider::Playlist = serde_json::from_value(raw).unwrap();
        let playlist = playlist(provider_playlist);
        assert_eq!(playlist.id, domain::PlaylistId(5));
        assert_eq!(playlist.track_count, 0);
        assert_eq!(playlist.description, "");
    }
}
