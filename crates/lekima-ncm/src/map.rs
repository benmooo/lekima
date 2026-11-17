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
        Some(album(track.album))
    };

    domain::Track {
        id: domain::TrackId(track.id),
        name: text(track.name),
        artists: track
            .artists
            .into_iter()
            .map(|artist| domain::Artist {
                id: domain::ArtistId(artist.id),
                name: text(artist.name),
            })
            .collect(),
        album,
        duration_ms: u64::from(track.duration),
    }
}

/// Album metadata. `artist` and `description` are filled in only where the
/// provider supplies them; album detail derives the artist from its tracks.
pub(crate) fn album(album: provider::Album) -> domain::Album {
    domain::Album {
        id: domain::AlbumId(album.id),
        name: text(album.name),
        cover_url: non_empty(album.pic_url),
        artist: album.artist.map(|artist| domain::Artist {
            id: domain::ArtistId(artist.id),
            name: text(artist.name),
        }),
        year: year_from_timestamp(album.publish_time),
        track_count: (album.size > 0).then_some(album.size),
        description: text(album.description),
    }
}

/// A millisecond (or second) timestamp reduced to its Gregorian year.
fn year_from_timestamp(timestamp: u64) -> Option<u16> {
    if timestamp == 0 {
        return None;
    }
    // Newer payloads use milliseconds; older ones occasionally use seconds.
    let milliseconds = if timestamp < 100_000_000_000 {
        timestamp.saturating_mul(1_000)
    } else {
        timestamp
    };
    let mut days = i64::try_from(milliseconds / 86_400_000).ok()?;
    let mut year = 1970i64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let length = if leap { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
    }
    u16::try_from(year).ok()
}

pub(crate) fn artist_detail(artist: provider::ArtistDetail) -> domain::ArtistDetail {
    domain::ArtistDetail {
        id: domain::ArtistId(artist.id),
        name: text(artist.name),
        avatar_url: non_empty(artist.avatar_url),
        cover_url: non_empty(artist.cover_url),
        album_count: artist.album_size,
        track_count: artist.music_size,
        description: text(artist.brief_desc),
        aliases: artist
            .aliases
            .into_iter()
            .map(text)
            .filter(|alias| !alias.is_empty())
            .collect(),
    }
}

pub(crate) fn artist_albums(body: provider::ArtistAlbumsBody) -> domain::ArtistAlbums {
    domain::ArtistAlbums {
        albums: body.hot_albums.into_iter().map(album).collect(),
        has_more: body.more,
    }
}

pub(crate) fn album_detail(body: provider::AlbumDetailBody) -> domain::AlbumDetail {
    let tracks: Vec<domain::Track> = body.songs.into_iter().map(track).collect();
    let mut album = album(body.album);
    // The album endpoint does not repeat the artist: take it from the tracks.
    if album.artist.is_none() {
        album.artist = tracks
            .iter()
            .find_map(|track| track.artists.first().cloned());
    }
    domain::AlbumDetail { album, tracks }
}

pub(crate) fn search_results(body: provider::SearchBody) -> domain::SearchResults {
    let tracks: Vec<_> = body.result.songs.into_iter().map(track).collect();
    let playlists: Vec<_> = body.result.playlists.into_iter().map(playlist).collect();
    let artists: Vec<_> = body
        .result
        .artists
        .into_iter()
        .map(|artist| domain::Artist {
            id: domain::ArtistId(artist.id),
            name: text(artist.name),
        })
        .collect();
    let albums: Vec<_> = body.result.albums.into_iter().map(album).collect();

    let count = |len: usize| u64::try_from(len).unwrap_or(u64::MAX);
    let totals = domain::SearchTotals {
        tracks: count(tracks.len()),
        // The album search reports matches, not just the decoded page.
        albums: body.result.album_count.max(count(albums.len())),
        artists: count(artists.len()),
        playlists: count(playlists.len()),
    };
    domain::SearchResults {
        total: totals.total(),
        tracks,
        playlists,
        artists,
        albums,
        totals,
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = text(value);
    (!value.is_empty()).then_some(value)
}

fn text(value: String) -> String {
    domain::sanitize_line(&value)
}

pub(crate) fn playlist(playlist: provider::Playlist) -> domain::Playlist {
    domain::Playlist {
        id: domain::PlaylistId(playlist.id),
        name: text(playlist.name),
        description: text(playlist.description.unwrap_or_default()),
        track_count: playlist.track_count,
        cover_url: non_empty(playlist.cover_img_url),
        owner: playlist
            .creator
            .map(|creator| text(creator.nickname))
            .filter(|nickname| !nickname.is_empty()),
    }
}

/// One official chart.
pub(crate) fn toplist(entry: provider::ToplistEntry) -> domain::Toplist {
    domain::Toplist {
        id: domain::PlaylistId(entry.id),
        name: text(entry.name),
        track_count: entry.track_count,
        update_frequency: text(entry.update_frequency),
    }
}

/// A cloud upload, preferring the provider's matched full metadata.
pub(crate) fn cloud_track(entry: provider::CloudTrack) -> Option<domain::Track> {
    if let Some(song) = entry.simple_song {
        return Some(track(song));
    }
    if entry.song_id == 0 {
        return None;
    }
    Some(domain::Track {
        id: domain::TrackId(entry.song_id),
        name: text(entry.song_name),
        artists: vec![domain::Artist {
            id: domain::ArtistId(0),
            name: text(entry.artist),
        }],
        album: Some(domain::Album {
            id: domain::AlbumId(0),
            name: text(entry.album),
            cover_url: None,
            artist: None,
            year: None,
            track_count: None,
            description: String::new(),
        }),
        duration_ms: 0,
    })
}

pub(crate) fn comments(body: provider::CommentsBody) -> domain::Comments {
    domain::Comments {
        total: body.total,
        has_more: body.more,
        hot: body.hot_comments.into_iter().map(comment).collect(),
        comments: body.comments.into_iter().map(comment).collect(),
    }
}

fn comment(comment: provider::Comment) -> domain::Comment {
    let author = comment
        .user
        .as_ref()
        .map_or_else(String::new, |user| domain::sanitize_line(&user.nickname));
    let avatar_url = comment.user.as_ref().and_then(|user| {
        (!user.avatar_url.is_empty()).then(|| domain::sanitize_line(&user.avatar_url))
    });
    domain::Comment {
        id: comment.comment_id,
        author,
        avatar_url,
        content: domain::sanitize_line(&comment.content),
        timestamp_ms: comment.time,
        liked_count: comment.liked_count,
        replies: comment
            .be_replied
            .into_iter()
            .map(|reply| domain::CommentReply {
                author: reply
                    .user
                    .as_ref()
                    .map_or_else(String::new, |user| domain::sanitize_line(&user.nickname)),
                content: domain::sanitize_line(&reply.content),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_provider_text_before_display() {
        let raw = serde_json::json!({
            "id": 1,
            "name": "\u{1b}[31mowned\u{1b}[0m",
            "artists": [{"id": 2, "name": "safe\u{202e}name"}],
            "album": {"id": 3, "name": "ok\u{7}"}
        });
        let provider_track: provider::Track = serde_json::from_value(raw).unwrap();
        let track = track(provider_track);
        assert_eq!(track.name, "owned");
        assert_eq!(track.artist_names(), "safename");
        assert_eq!(track.album_name(), "ok");
    }

    #[test]
    fn maps_and_sanitizes_comments() {
        let raw = serde_json::json!({
            "code":200,"total":1,"more":false,
            "hotComments":[{
                "commentId":5,"content":"\u{1b}[31mhot\u{1b}[0m","time":10,"likedCount":3,
                "user":{"nickname":"ben","avatarUrl":"https://img/ben"},
                "beReplied":[{"content":"parent","user":{"nickname":"aka"}}]
            }],
            "comments":[]
        });
        let body: provider::CommentsBody = serde_json::from_value(raw).unwrap();
        let comments = comments(body);
        assert_eq!(comments.total, 1);
        assert_eq!(comments.hot.len(), 1);
        assert_eq!(comments.hot[0].content, "hot");
        assert_eq!(comments.hot[0].author, "ben");
        assert_eq!(comments.hot[0].liked_count, 3);
        assert_eq!(comments.hot[0].replies[0].author, "aka");
    }

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
    fn maps_album_search_results_and_counts() {
        let raw = serde_json::json!({
            "code": 200,
            "result": {
                "songs": [],
                "albums": [{
                    "id": 7,
                    "name": "Mota",
                    "picUrl": "https://img/7.jpg",
                    "publishTime": 1_704_067_200_000u64,
                    "size": 1
                }],
                "albumCount": 48,
                "artists": [],
                "playlists": []
            }
        });
        let response: provider::ApiResponse<provider::SearchBody> =
            serde_json::from_value(raw).unwrap();
        let results = search_results(response.body);
        assert_eq!(results.albums.len(), 1);
        assert_eq!(results.albums[0].name, "Mota");
        assert_eq!(results.albums[0].year, Some(2024));
        assert_eq!(results.albums[0].track_count, Some(1));
        // The provider reports matches, not just the decoded page.
        assert_eq!(results.totals.albums, 48);
        assert_eq!(results.total, 48);
    }

    #[test]
    fn maps_artist_profiles_with_null_fields() {
        let raw = serde_json::json!({
            "code": 200,
            "data": {"artist": {
                "id": 6452,
                "name": "周杰伦",
                "avatar": "https://img/a",
                "cover": null,
                "albumSize": 49,
                "musicSize": 592,
                "briefDesc": "bio",
                "alias": ["Jay Chou", "周董"],
                "identities": null
            }}
        });
        let response: provider::ApiResponse<provider::ArtistDetailBody> =
            serde_json::from_value(raw).unwrap();
        let detail = artist_detail(response.body.data.artist);
        assert_eq!(detail.id, domain::ArtistId(6452));
        assert_eq!(detail.avatar_url.as_deref(), Some("https://img/a"));
        assert_eq!(detail.cover_url, None);
        assert_eq!(detail.album_count, 49);
        assert_eq!(detail.track_count, 592);
        assert_eq!(detail.description, "bio");
        assert_eq!(detail.aliases, ["Jay Chou", "周董"]);
    }

    #[test]
    fn maps_artist_album_pages_and_tolerates_nulls() {
        let raw = serde_json::json!({
            "code": 200,
            "artist": {"id": 1, "name": "Aash"},
            "hotAlbums": [{"id": 9, "name": "Mota", "size": 2}],
            "more": true
        });
        let response: provider::ApiResponse<provider::ArtistAlbumsBody> =
            serde_json::from_value(raw).unwrap();
        let page = artist_albums(response.body);
        assert_eq!(page.albums.len(), 1);
        assert!(page.has_more);

        let raw = serde_json::json!({"code": 200, "hotAlbums": null, "more": null});
        let response: provider::ApiResponse<provider::ArtistAlbumsBody> =
            serde_json::from_value(raw).unwrap();
        let page = artist_albums(response.body);
        assert!(page.albums.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn maps_album_detail_and_derives_the_artist_from_tracks() {
        let raw = serde_json::json!({
            "code": 200,
            "album": {"id": 9, "name": "Mota", "picUrl": "", "size": 1},
            "songs": [{
                "id": 1,
                "name": "Mota",
                "ar": [{"id": 2, "name": "Aash"}],
                "al": {"id": 9, "name": "Mota"},
                "dt": 120_000
            }]
        });
        let response: provider::ApiResponse<provider::AlbumDetailBody> =
            serde_json::from_value(raw).unwrap();
        let detail = album_detail(response.body);
        assert_eq!(detail.album.id, domain::AlbumId(9));
        assert_eq!(detail.tracks.len(), 1);
        assert_eq!(detail.tracks[0].duration_ms, 120_000);
        assert_eq!(
            detail
                .album
                .artist
                .as_ref()
                .map(|artist| artist.name.as_str()),
            Some("Aash")
        );
    }

    #[test]
    fn timestamps_reduce_to_a_gregorian_year() {
        assert_eq!(year_from_timestamp(0), None);
        assert_eq!(year_from_timestamp(1_704_067_200), Some(2024));
        assert_eq!(year_from_timestamp(1_704_067_200_000), Some(2024));
        assert_eq!(year_from_timestamp(1_609_459_200_000), Some(2021));
    }

    #[test]
    fn maps_playlists_without_optional_fields() {
        let raw = serde_json::json!({"id": 5, "name": "Late night"});
        let provider_playlist: provider::Playlist = serde_json::from_value(raw).unwrap();
        let playlist = playlist(provider_playlist);
        assert_eq!(playlist.id, domain::PlaylistId(5));
        assert_eq!(playlist.track_count, 0);
        assert_eq!(playlist.description, "");
        assert_eq!(playlist.owner, None);
    }

    #[test]
    fn maps_playlist_metadata() {
        let raw = serde_json::json!({
            "id": 5,
            "name": "Focus",
            "trackCount": 42,
            "coverImgUrl": "https://img/5.jpg",
            "creator": {"nickname": "ben"}
        });
        let provider_playlist: provider::Playlist = serde_json::from_value(raw).unwrap();
        let playlist = playlist(provider_playlist);
        assert_eq!(playlist.track_count, 42);
        assert_eq!(playlist.cover_url.as_deref(), Some("https://img/5.jpg"));
        assert_eq!(playlist.owner.as_deref(), Some("ben"));
    }

    #[test]
    fn maps_toplists() {
        let raw = serde_json::json!({
            "id": 1, "name": "热歌榜", "trackCount": 200, "updateFrequency": "每天更新"
        });
        let entry: provider::ToplistEntry = serde_json::from_value(raw).unwrap();
        let chart = toplist(entry);
        assert_eq!(chart.id, domain::PlaylistId(1));
        assert_eq!(chart.name, "热歌榜");
        assert_eq!(chart.track_count, 200);
    }

    #[test]
    fn maps_cloud_uploads_with_and_without_metadata() {
        let raw = serde_json::json!({
            "songId": 9,
            "songName": "upload",
            "artist": "me",
            "album": "demos",
            "simpleSong": {"id": 9, "name": "upload", "ar": [], "al": {"id": 0, "name": ""}}
        });
        let entry: provider::CloudTrack = serde_json::from_value(raw).unwrap();
        let mapped = cloud_track(entry).unwrap();
        assert_eq!(mapped.id, domain::TrackId(9));
        assert_eq!(mapped.artist_names(), "Unknown artist");

        let raw = serde_json::json!({
            "songId": 7, "songName": "local", "artist": "me", "album": "demos"
        });
        let entry: provider::CloudTrack = serde_json::from_value(raw).unwrap();
        let mapped = cloud_track(entry).unwrap();
        assert_eq!(mapped.name, "local");
        assert_eq!(mapped.artist_names(), "me");

        let raw = serde_json::json!({"songId": 0, "songName": "unmatched"});
        let entry: provider::CloudTrack = serde_json::from_value(raw).unwrap();
        assert!(cloud_track(entry).is_none());
    }
}
