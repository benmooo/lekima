//! Catalogue entities.
//!
//! Identifiers are newtypes rather than raw integers so that a track id can
//! never be passed where a playlist id is expected. All types are serde
//! round-trippable because the same shapes are used for IPC and for the JSON
//! output agents consume.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(
                Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
            )]
            #[serde(transparent)]
            pub struct $name(pub u64);

            impl $name {
                pub const fn new(raw: u64) -> Self {
                    Self(raw)
                }

                pub const fn get(self) -> u64 {
                    self.0
                }
            }

            impl std::fmt::Display for $name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{}", self.0)
                }
            }

            impl From<u64> for $name {
                fn from(raw: u64) -> Self {
                    Self(raw)
                }
            }

            impl std::str::FromStr for $name {
                type Err = crate::DomainError;

                fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                    value
                        .trim()
                        .parse::<u64>()
                        .map(Self)
                        .map_err(|_| crate::DomainError::InvalidInput(format!(
                            "`{value}` is not a valid {}",
                            stringify!($name)
                        )))
                }
            }
        )+
    };
}

id_type!(TrackId, AlbumId, ArtistId, PlaylistId, UserId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub id: AlbumId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Primary artist, when the provider attaches one to the album.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<Artist>,
    /// Release year. Provider payloads carry a timestamp; only the year is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl Album {
    /// Artist label for tables, or an empty string when the provider omitted it.
    #[must_use]
    pub fn artist_name(&self) -> &str {
        self.artist
            .as_ref()
            .map_or("", |artist| artist.name.as_str())
    }
}

/// A full artist profile page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistDetail {
    pub id: ArtistId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub album_count: u64,
    #[serde(default)]
    pub track_count: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// One page of an artist's albums.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistAlbums {
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub has_more: bool,
}

/// An album with its ordered track list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub album: Album,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Album>,
    #[serde(default)]
    pub duration_ms: u64,
}

impl Track {
    /// Human readable "Artist A, Artist B" label.
    #[must_use]
    pub fn artist_names(&self) -> String {
        if self.artists.is_empty() {
            "Unknown artist".to_owned()
        } else {
            self.artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    #[must_use]
    pub fn album_name(&self) -> &str {
        self.album.as_ref().map_or("", |album| album.name.as_str())
    }

    /// Duration rendered as `m:ss`, or `--:--` when unknown.
    #[must_use]
    pub fn duration_label(&self) -> String {
        format_duration_ms(self.duration_ms)
    }
}

#[must_use]
pub fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms == 0 {
        return "--:--".to_owned();
    }
    let total_seconds = duration_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes >= 60 {
        format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds)
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub track_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// An official chart entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toplist {
    pub id: PlaylistId,
    pub name: String,
    #[serde(default)]
    pub track_count: u64,
    #[serde(default)]
    pub update_frequency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: UserId,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// What a search query may target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    /// Every catalogue kind, merged by the adapter.
    #[default]
    All,
    Track,
    Album,
    Artist,
    Playlist,
}

impl SearchKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }
}

/// Number of matches per catalogue kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTotals {
    #[serde(default)]
    pub tracks: u64,
    #[serde(default)]
    pub albums: u64,
    #[serde(default)]
    pub artists: u64,
    #[serde(default)]
    pub playlists: u64,
}

impl SearchTotals {
    #[must_use]
    pub const fn total(self) -> u64 {
        self.tracks + self.albums + self.artists + self.playlists
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub totals: SearchTotals,
    /// Sum of [`SearchTotals`], kept for clients written before per-kind counts.
    #[serde(default)]
    pub total: u64,
}

impl SearchResults {
    /// True when every kind came back empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.tracks.is_empty()
            && self.playlists.is_empty()
            && self.artists.is_empty()
            && self.albums.is_empty()
    }
}

/// A started QR login: `key` is polled, `url` is encoded as a QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrLogin {
    pub key: String,
    pub url: String,
}

/// Progress of a QR login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum QrStatus {
    Expired,
    Waiting,
    Scanned,
    Confirmed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nickname: Option<String>,
    },
}

/// Requested streaming bitrate class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Standard,
    Higher,
    #[default]
    Exhigh,
    Lossless,
}

impl Quality {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Higher => "higher",
            Self::Exhigh => "exhigh",
            Self::Lossless => "lossless",
        }
    }

    #[must_use]
    pub const fn bitrate(self) -> u32 {
        match self {
            Self::Standard => 128_000,
            Self::Higher => 192_000,
            Self::Exhigh => 320_000,
            Self::Lossless => 999_000,
        }
    }
}

/// One public track comment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub content: String,
    pub timestamp_ms: u64,
    pub liked_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<CommentReply>,
}

/// A short quoted reply attached to a comment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommentReply {
    pub author: String,
    pub content: String,
}

/// A page of track comments, with hot entries separated.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Comments {
    pub total: u64,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hot: Vec<Comment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
}

#[cfg(test)]
mod tests {
    use super::{
        Album, AlbumDetail, ArtistDetail, SearchKind, SearchResults, SearchTotals, Track,
        format_duration_ms,
    };

    #[test]
    fn duration_formatting_handles_unknown_and_hours() {
        assert_eq!(format_duration_ms(0), "--:--");
        assert_eq!(format_duration_ms(5_000), "0:05");
        assert_eq!(format_duration_ms(245_000), "4:05");
        assert_eq!(format_duration_ms(3_725_000), "1:02:05");
    }

    #[test]
    fn track_artist_label_is_never_empty() {
        let track = Track {
            id: 1.into(),
            name: "mota".to_owned(),
            artists: Vec::new(),
            album: None,
            duration_ms: 0,
        };
        assert_eq!(track.artist_names(), "Unknown artist");
    }

    #[test]
    fn search_kind_all_is_lowercase_on_the_wire() {
        assert_eq!(SearchKind::All.as_str(), "all");
        assert_eq!(serde_json::to_string(&SearchKind::All).unwrap(), "\"all\"");
        assert_eq!(
            serde_json::from_str::<SearchKind>("\"all\"").unwrap(),
            SearchKind::All
        );
    }

    #[test]
    fn search_totals_sum_every_kind() {
        let totals = SearchTotals {
            tracks: 8,
            albums: 4,
            artists: 2,
            playlists: 3,
        };
        assert_eq!(totals.total(), 17);
        let results = SearchResults {
            totals,
            total: totals.total(),
            ..SearchResults::default()
        };
        assert!(results.is_empty());
    }

    #[test]
    fn old_search_payloads_decode_with_empty_new_fields() {
        let results: SearchResults =
            serde_json::from_str(r#"{"tracks":[],"playlists":[],"artists":[],"total":0}"#).unwrap();
        assert!(results.albums.is_empty());
        assert_eq!(results.totals, SearchTotals::default());
    }

    #[test]
    fn album_catalogue_models_round_trip() {
        let album = Album {
            id: 9.into(),
            name: "Mota".to_owned(),
            cover_url: Some("https://img/9.jpg".to_owned()),
            artist: Some(super::Artist {
                id: 2.into(),
                name: "Aash".to_owned(),
            }),
            year: Some(2024),
            track_count: Some(1),
            description: "debut".to_owned(),
        };
        assert_eq!(album.artist_name(), "Aash");

        let detail = AlbumDetail {
            album,
            tracks: Vec::new(),
        };
        let encoded = serde_json::to_string(&detail).unwrap();
        let decoded: AlbumDetail = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, detail);

        let artist = ArtistDetail {
            id: 6452.into(),
            name: "周杰伦".to_owned(),
            avatar_url: None,
            cover_url: None,
            album_count: 49,
            track_count: 592,
            description: "bio".to_owned(),
            aliases: vec!["Jay Chou".to_owned()],
        };
        let decoded: ArtistDetail =
            serde_json::from_str(&serde_json::to_string(&artist).unwrap()).unwrap();
        assert_eq!(decoded, artist);
    }

    #[test]
    fn old_album_payloads_still_decode() {
        let album: Album = serde_json::from_str(r#"{"id":1,"name":"x"}"#).unwrap();
        assert!(album.artist.is_none());
        assert!(album.year.is_none());
        assert!(album.track_count.is_none());
        assert!(album.description.is_empty());
        assert_eq!(album.artist_name(), "");
    }
}
