//! Provider → domain mapping.
//!
//! Everything here is pure: fixtures in, domain values out. The provider is
//! allowed to omit fields, so defaults are explicit and every field that can be
//! absent is documented.

use lekima_domain as domain;
use ncmapi as provider;
use serde::Deserialize;

pub(crate) fn track(track: provider::Track) -> domain::Track {
    let album = if track.album.id == 0 && track.album.name.is_empty() {
        None
    } else {
        Some(domain::Album {
            id: domain::AlbumId(track.album.id),
            name: text(track.album.name),
            cover_url: (!track.album.pic_url.is_empty()).then_some(track.album.pic_url),
        })
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

pub(crate) fn search_results(body: provider::SearchBody) -> domain::SearchResults {
    let tracks: Vec<_> = body.result.songs.into_iter().map(track).collect();
    let playlists: Vec<_> = body.result.playlists.into_iter().map(playlist).collect();
    let artists: Vec<_> = body
        .result
        .artists
        .into_iter()
        .map(|artist| domain::Artist {
            id: domain::ArtistId(artist.id),
            name: artist.name,
        })
        .collect();
    let total = tracks.len() + playlists.len() + artists.len();
    domain::SearchResults {
        total: u64::try_from(total).unwrap_or(u64::MAX),
        tracks,
        playlists,
        artists,
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
        }),
        duration_ms: 0,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderComments {
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub more: bool,
    #[serde(default, rename = "hotComments")]
    pub hot_comments: Vec<ProviderComment>,
    #[serde(default)]
    pub comments: Vec<ProviderComment>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderComment {
    #[serde(default, rename = "commentId")]
    pub comment_id: u64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub time: u64,
    #[serde(default, rename = "likedCount")]
    pub liked_count: u64,
    #[serde(default)]
    pub user: Option<ProviderCommentUser>,
    #[serde(default, rename = "beReplied")]
    pub replies: Vec<ProviderReply>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderCommentUser {
    #[serde(default)]
    pub nickname: String,
    #[serde(default, rename = "avatarUrl")]
    pub avatar_url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderReply {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub user: Option<ProviderCommentUser>,
}

pub(crate) fn comments(body: ProviderComments) -> domain::Comments {
    domain::Comments {
        total: body.total,
        has_more: body.more,
        hot: body.hot_comments.into_iter().map(comment).collect(),
        comments: body.comments.into_iter().map(comment).collect(),
    }
}

fn comment(comment: ProviderComment) -> domain::Comment {
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
            .replies
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
        let body: ProviderComments = serde_json::from_value(raw).unwrap();
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
