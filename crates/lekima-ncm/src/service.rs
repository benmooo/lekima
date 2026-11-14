//! `MusicService` implemented over `ncmapi`.

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use lekima_domain::{
    self as domain, DomainError, MusicService, PlaylistId, Quality, SearchKind, TrackId, UserId,
    UserProfile,
};
use ncmapi::{
    ApiResponse, NcmClient, Page, SearchKind as ProviderSearchKind, SearchQuery,
    TrackId as ProviderTrackId, UserId as ProviderUserId,
};

use crate::map;

/// Construction inputs for [`NcmMusicService`].
#[derive(Debug, Clone)]
pub struct NcmOptions {
    /// Where the session cookie header is read from and written to.
    pub cookie_path: PathBuf,
    /// Enables the SDK's short-lived response cache.
    pub cache: bool,
    pub cache_ttl: Duration,
}

impl NcmOptions {
    #[must_use]
    pub fn new(cookie_path: impl Into<PathBuf>) -> Self {
        Self {
            cookie_path: cookie_path.into(),
            cache: true,
            cache_ttl: Duration::from_secs(180),
        }
    }
}

/// NetEase-backed catalogue and account service.
pub struct NcmMusicService {
    client: NcmClient,
}

impl NcmMusicService {
    /// Builds the service. Fails eagerly on invalid local configuration.
    ///
    /// # Errors
    ///
    /// Returns a domain error when the SDK cannot be constructed (for example a
    /// cookie file with invalid permissions content).
    pub fn new(options: NcmOptions) -> domain::Result<Self> {
        let client = NcmClient::builder()
            .cache(options.cache)
            .cache_ttl(options.cache_ttl)
            .cookie_path(options.cookie_path)
            .build()
            .map_err(provider_error)?;
        Ok(Self { client })
    }
}

fn ensure_success<T>(response: ApiResponse<T>) -> domain::Result<ApiResponse<T>> {
    match response.code {
        200 => Ok(response),
        301 => Err(DomainError::AuthenticationRequired(
            "the music provider rejected the session; run `lekima login`".to_owned(),
        )),
        code => Err(DomainError::Unavailable(format!(
            "the music provider returned code {code}"
        ))),
    }
}

fn provider_error(error: ncmapi::Error) -> DomainError {
    use ncmapi::Error;
    match error {
        Error::InvalidInput { field, reason } => {
            DomainError::InvalidInput(format!("{field}: {reason}"))
        }
        Error::Http(source) => {
            DomainError::Unavailable(format!("provider request failed: {source}"))
        }
        Error::CookiePersistence { operation, source } => DomainError::Unavailable(format!(
            "session storage failed while {operation}: {source}"
        )),
        other => DomainError::Unavailable(format!("provider protocol error: {other}")),
    }
}

fn provider_kind(kind: SearchKind) -> ProviderSearchKind {
    match kind {
        SearchKind::Track => ProviderSearchKind::Song,
        SearchKind::Album => ProviderSearchKind::Album,
        SearchKind::Artist => ProviderSearchKind::Artist,
        SearchKind::Playlist => ProviderSearchKind::Playlist,
    }
}

fn map_page(limit: u32, offset: u32) -> domain::Result<(u16, u32)> {
    if limit == 0 {
        return Err(DomainError::InvalidInput(
            "search limit must be greater than zero".to_owned(),
        ));
    }
    let limit = u16::try_from(limit.min(100))
        .map_err(|_| DomainError::InvalidInput("search limit is too large".to_owned()))?;
    Ok((limit, offset))
}

#[async_trait]
impl MusicService for NcmMusicService {
    async fn search(
        &self,
        query: &str,
        kind: SearchKind,
        limit: u32,
        offset: u32,
    ) -> domain::Result<domain::SearchResults> {
        let (limit, offset) = map_page(limit, offset)?;
        let query = SearchQuery::new(query)
            .map_err(provider_error)?
            .kind(provider_kind(kind))
            .page(limit, offset)
            .map_err(provider_error)?;

        let response = self
            .client
            .discovery()
            .search(query)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(map::search_results(response.body))
    }

    async fn tracks(&self, ids: &[TrackId]) -> domain::Result<Vec<domain::Track>> {
        let provider_ids: Vec<_> = ids.iter().map(|id| ProviderTrackId(id.0)).collect();
        let response = self
            .client
            .library()
            .tracks(&provider_ids)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response.body.songs.into_iter().map(map::track).collect())
    }

    async fn stream_url(&self, id: TrackId, _quality: Quality) -> domain::Result<String> {
        let response = self
            .client
            .library()
            .stream_urls(&[ProviderTrackId(id.0)])
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        response
            .body
            .data
            .into_iter()
            .find_map(|stream| stream.url)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                DomainError::Unsupported(format!(
                    "track {id} has no playable stream (VIP, region, or removed)"
                ))
            })
    }

    async fn playlists(&self, user: Option<UserId>) -> domain::Result<Vec<domain::Playlist>> {
        let user_id = match user {
            Some(user) => user,
            None => {
                let profile = self.current_user().await?.ok_or_else(|| {
                    DomainError::AuthenticationRequired("sign in to list your playlists".to_owned())
                })?;
                profile.id
            }
        };

        let page = Page::first(200).map_err(provider_error)?;
        let response = self
            .client
            .profile()
            .playlists(ProviderUserId(user_id.0), page)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response
            .body
            .playlist
            .into_iter()
            .map(map::playlist)
            .collect())
    }

    async fn playlist_tracks(&self, id: PlaylistId) -> domain::Result<Vec<domain::Track>> {
        let _ = id;
        Err(DomainError::Unsupported(
            "playlist tracks require ncmapi 1.1 (`/api/v6/playlist/detail`)".to_owned(),
        ))
    }

    async fn current_user(&self) -> domain::Result<Option<UserProfile>> {
        let response = self
            .client
            .profile()
            .current()
            .await
            .map_err(provider_error)?;
        if response.code == 301 {
            return Ok(None);
        }
        let response = ensure_success(response)?;
        Ok(response.body.profile.map(|profile| UserProfile {
            id: UserId(profile.user_id),
            nickname: profile.nickname,
            avatar_url: None,
        }))
    }

    async fn lyrics(&self, id: TrackId) -> domain::Result<String> {
        let _ = id;
        Err(DomainError::Unsupported(
            "lyrics require ncmapi 1.1 (`/api/song/lyric`)".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lekima_domain::{MusicService, PlaylistId, TrackId};

    use super::{NcmMusicService, NcmOptions};

    fn service() -> NcmMusicService {
        NcmMusicService::new(NcmOptions {
            cookie_path: std::env::temp_dir().join("lekima-test-cookies"),
            cache: false,
            cache_ttl: Duration::from_secs(1),
        })
        .expect("client construction must not touch the network")
    }

    #[tokio::test]
    async fn unimplemented_provider_features_report_unsupported() {
        let service = service();
        assert!(matches!(
            service.playlist_tracks(PlaylistId(1)).await,
            Err(lekima_domain::DomainError::Unsupported(_))
        ));
        assert!(matches!(
            service.lyrics(TrackId(1)).await,
            Err(lekima_domain::DomainError::Unsupported(_))
        ));
    }

    #[tokio::test]
    async fn blank_search_is_rejected_before_any_network_call() {
        let service = service();
        let error = service
            .search(" ", lekima_domain::SearchKind::Track, 20, 0)
            .await
            .unwrap_err();
        assert!(matches!(error, lekima_domain::DomainError::InvalidInput(_)));
    }
}
