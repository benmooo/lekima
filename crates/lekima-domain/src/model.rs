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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl SearchKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
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
    pub total: u64,
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

#[cfg(test)]
mod tests {
    use super::{Track, format_duration_ms};

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
}
