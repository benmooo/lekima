//! `MusicService` implemented over `ncmapi`.

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use lekima_domain::{
    self as domain, DomainError, Lyrics, MusicService, PlaylistId, QrLogin, QrStatus, Quality,
    SearchKind, Toplist, TrackId, UserId, UserProfile,
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
            "the music provider returned code {code}{}",
            response
                .message
                .as_deref()
                .map_or_else(String::new, |message| format!(": {message}"))
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
        const PAGE: u16 = 500;
        const MAX_PAGES: u32 = 40;

        let mut tracks = Vec::new();
        let mut offset = 0u32;
        for _ in 0..MAX_PAGES {
            let response = self
                .client
                .library()
                .playlist_tracks(id.0, PAGE, offset)
                .await
                .map_err(provider_error)?;
            let response = ensure_success(response)?;
            let received = response.body.songs.len();
            if received == 0 {
                break;
            }
            tracks.extend(response.body.songs.into_iter().map(map::track));
            if received < usize::from(PAGE) {
                break;
            }
            offset = offset.saturating_add(u32::try_from(received).unwrap_or(0));
        }

        if tracks.is_empty() {
            return Err(DomainError::NotFound(format!(
                "playlist {id} has no tracks"
            )));
        }
        Ok(tracks)
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

    async fn lyrics(&self, id: TrackId) -> domain::Result<Lyrics> {
        let response = self
            .client
            .library()
            .lyrics(id.0)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        // An empty document is a valid answer: the TUI renders its empty state.
        let translation = response.body.translation().map(str::to_owned);
        Ok(lekima_domain::parse_lrc_with_translation(
            response.body.text(),
            translation.as_deref(),
        ))
    }

    async fn daily(&self) -> domain::Result<Vec<domain::Track>> {
        let response = self
            .client
            .discovery()
            .daily()
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response
            .body
            .data
            .daily_songs
            .into_iter()
            .map(map::track)
            .collect())
    }

    async fn toplists(&self) -> domain::Result<Vec<Toplist>> {
        let response = self
            .client
            .discovery()
            .toplists()
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response.body.list.into_iter().map(map::toplist).collect())
    }

    async fn fm_batch(&self) -> domain::Result<Vec<domain::Track>> {
        let response = self
            .client
            .radio()
            .fm_batch()
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response.body.data.into_iter().map(map::track).collect())
    }

    async fn fm_trash(&self, id: TrackId) -> domain::Result<()> {
        let response = self
            .client
            .radio()
            .trash(ncmapi::TrackId(id.0))
            .await
            .map_err(provider_error)?;
        ensure_success(response)?;
        Ok(())
    }

    async fn set_like(&self, id: TrackId, liked: bool) -> domain::Result<()> {
        let response = self
            .client
            .radio()
            .set_like(ncmapi::TrackId(id.0), liked)
            .await
            .map_err(provider_error)?;
        ensure_success(response)?;
        Ok(())
    }

    async fn liked_ids(&self) -> domain::Result<Vec<TrackId>> {
        let profile = self.current_user().await?.ok_or_else(|| {
            DomainError::AuthenticationRequired("sign in to read liked tracks".to_owned())
        })?;
        let response = self
            .client
            .radio()
            .liked_ids(ncmapi::UserId(profile.id.0))
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response.body.ids.into_iter().map(domain::TrackId).collect())
    }

    async fn cloud(&self, limit: u32, offset: u32) -> domain::Result<Vec<domain::Track>> {
        let response = self
            .client
            .library()
            .cloud(limit.clamp(1, 500), offset)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(response
            .body
            .data
            .into_iter()
            .filter_map(map::cloud_track)
            .collect())
    }

    async fn qr_start(&self) -> domain::Result<QrLogin> {
        let response = self
            .client
            .auth()
            .qr_start()
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        if response.body.unikey.is_empty() {
            return Err(DomainError::Unavailable(
                "the provider returned an empty QR key".to_owned(),
            ));
        }
        Ok(QrLogin {
            key: response.body.unikey.clone(),
            url: response.body.login_url(),
        })
    }

    async fn qr_poll(&self, key: &str) -> domain::Result<QrStatus> {
        let response = self
            .client
            .auth()
            .qr_poll(key)
            .await
            .map_err(provider_error)?;
        tracing::debug!(code = response.code, "qr poll");
        match response.code {
            800 => Ok(QrStatus::Expired),
            801 => Ok(QrStatus::Waiting),
            802 => Ok(QrStatus::Scanned),
            803 => Ok(QrStatus::Confirmed {
                nickname: response.body.nickname,
            }),
            8821 => Err(DomainError::AuthenticationRequired(format!(
                "NetEase requires behavioral verification for this login (code 8821{})\n\
                 use `lekima login --cookie-file -` with a session cookie from your browser, \
                 or retry later without a VPN/proxy",
                response
                    .body
                    .message
                    .as_deref()
                    .map_or_else(String::new, |message| format!(": {message}"))
            ))),
            code => Err(DomainError::Unavailable(format!(
                "the provider returned code {code} while polling the QR login{}",
                response
                    .body
                    .message
                    .as_deref()
                    .map_or_else(String::new, |message| format!(": {message}"))
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lekima_domain::MusicService;

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
    async fn blank_qr_keys_are_rejected_before_any_network_call() {
        let service = service();
        let error = service.qr_poll(" ").await.unwrap_err();
        assert!(matches!(error, lekima_domain::DomainError::InvalidInput(_)));
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
