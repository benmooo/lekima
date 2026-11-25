//! `MusicService` implemented over `ncmapi`.

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use lekima_domain::{
    self as domain, Comments, DomainError, Lyrics, MusicService, Playlist, PlaylistId, QrLogin,
    QrStatus, Quality, SearchKind, Toplist, TrackId, UserId, UserProfile,
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
    /// Stable anonymous device id, reused across processes.
    pub device_id_path: PathBuf,
    /// `User-Agent` sent with provider requests.
    pub user_agent: String,
    /// Enables the SDK's short-lived response cache.
    pub cache: bool,
    pub cache_ttl: Duration,
}

impl NcmOptions {
    #[must_use]
    pub fn new(cookie_path: impl Into<PathBuf>) -> Self {
        let cookie_path = cookie_path.into();
        let device_id_path = cookie_path
            .parent()
            .map_or_else(|| PathBuf::from("device.id"), |dir| dir.join("device.id"));
        Self {
            cookie_path,
            device_id_path,
            user_agent: BROWSER_USER_AGENT.to_owned(),
            cache: true,
            cache_ttl: Duration::from_secs(180),
        }
    }
}

/// Device ids and user agents are provider-facing client identity.
///
/// A real browser string is a closer match than the SDK default, and it is
/// paired with a device id that survives restarts instead of looking like a
/// new device on every command.
#[cfg(target_os = "macos")]
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36";
#[cfg(not(target_os = "macos"))]
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36";

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
            .device_id_path(options.device_id_path)
            .user_agent(options.user_agent)
            .build()
            .map_err(provider_error)?;
        Ok(Self { client })
    }

    async fn run_search(&self, query: SearchQuery) -> domain::Result<domain::SearchResults> {
        let response = self
            .client
            .discovery()
            .search(query)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(map::search_results(response.body))
    }

    /// `SearchKind::All` fans out to the four provider kinds and merges them.
    async fn search_all(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> domain::Result<domain::SearchResults> {
        let (limit, offset) = map_page(limit.clamp(1, ALL_PER_KIND), offset)?;
        let queries = [
            search_query(query, SearchKind::Track, limit, offset)?,
            search_query(query, SearchKind::Album, limit, offset)?,
            search_query(query, SearchKind::Artist, limit, offset)?,
            search_query(query, SearchKind::Playlist, limit, offset)?,
        ];
        // Sequential on purpose: the adapter has no async-combinator
        // dependency and the SDK cache absorbs repeated fan-outs.
        let mut merged = domain::SearchResults::default();
        for query in queries {
            let page = self.run_search(query).await?;
            merged.tracks.extend(page.tracks);
            merged.albums.extend(page.albums);
            merged.artists.extend(page.artists);
            merged.playlists.extend(page.playlists);
            merged.totals.tracks += page.totals.tracks;
            merged.totals.albums += page.totals.albums;
            merged.totals.artists += page.totals.artists;
            merged.totals.playlists += page.totals.playlists;
        }
        merged.total = merged.totals.total();
        Ok(merged)
    }
}

fn ensure_success<T>(response: ApiResponse<T>) -> domain::Result<ApiResponse<T>> {
    match response.code {
        200 => Ok(response),
        301 => Err(DomainError::AuthenticationRequired(
            "the music provider rejected the session; run `lekima login`".to_owned(),
        )),
        404 => Err(DomainError::NotFound(format!(
            "the music provider could not find the requested item{}",
            response
                .message
                .as_deref()
                .map_or_else(String::new, |message| format!(": {message}"))
        ))),
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
        // `All` is an adapter fan-out (ADR-001), never a provider kind.
        SearchKind::All | SearchKind::Track => ProviderSearchKind::Song,
        SearchKind::Album => ProviderSearchKind::Album,
        SearchKind::Artist => ProviderSearchKind::Artist,
        SearchKind::Playlist => ProviderSearchKind::Playlist,
    }
}

fn search_query(
    query: &str,
    kind: SearchKind,
    limit: u16,
    offset: u32,
) -> domain::Result<SearchQuery> {
    SearchQuery::new(query)
        .map_err(provider_error)?
        .kind(provider_kind(kind))
        .page(limit, offset)
        .map_err(provider_error)
}

/// Per-kind result cap for a merged `All` search.
const ALL_PER_KIND: u32 = 10;

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
        if kind == SearchKind::All {
            return self.search_all(query, limit, offset).await;
        }
        let (limit, offset) = map_page(limit, offset)?;
        let query = search_query(query, kind, limit, offset)?;
        self.run_search(query).await
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

    async fn playlist_create(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> domain::Result<Playlist> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DomainError::InvalidInput(
                "playlist name must not be empty".to_owned(),
            ));
        }

        let response = self
            .client
            .library()
            .playlist_create(name, private)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        let id = response.body.id;
        if id == 0 {
            return Err(DomainError::Unavailable(
                "the provider did not return a playlist id".to_owned(),
            ));
        }

        // Create does not take a description; apply it as a follow-up update so
        // the returned playlist matches what the caller asked for.
        if !description.trim().is_empty() {
            self.playlist_update(PlaylistId(id), None, Some(description))
                .await?;
        }

        Ok(Playlist {
            id: PlaylistId(id),
            name: name.to_owned(),
            description: description.to_owned(),
            track_count: 0,
            cover_url: None,
            owner: None,
        })
    }

    async fn playlist_update(
        &self,
        id: PlaylistId,
        name: Option<&str>,
        description: Option<&str>,
    ) -> domain::Result<()> {
        let name = name.map(str::trim);
        if name.is_some_and(str::is_empty) {
            return Err(DomainError::InvalidInput(
                "playlist name must not be empty".to_owned(),
            ));
        }
        if name.is_none() && description.is_none() {
            return Err(DomainError::InvalidInput(
                "nothing to change: pass a name or a description".to_owned(),
            ));
        }

        let response = self
            .client
            .library()
            .playlist_update(id.0, name, description)
            .await
            .map_err(provider_error)?;
        ensure_success(response)?;
        Ok(())
    }

    async fn playlist_add(&self, id: PlaylistId, tracks: &[TrackId]) -> domain::Result<()> {
        if tracks.is_empty() {
            return Err(DomainError::InvalidInput(
                "track list must not be empty".to_owned(),
            ));
        }
        let ids = tracks
            .iter()
            .map(|track| ProviderTrackId(track.0))
            .collect::<Vec<_>>();
        let response = self
            .client
            .library()
            .playlist_add_tracks(id.0, &ids)
            .await
            .map_err(provider_error)?;
        ensure_success(response)?;
        Ok(())
    }

    async fn playlist_remove(&self, id: PlaylistId, tracks: &[TrackId]) -> domain::Result<()> {
        if tracks.is_empty() {
            return Err(DomainError::InvalidInput(
                "track list must not be empty".to_owned(),
            ));
        }
        let ids = tracks
            .iter()
            .map(|track| ProviderTrackId(track.0))
            .collect::<Vec<_>>();
        let response = self
            .client
            .library()
            .playlist_remove_tracks(id.0, &ids)
            .await
            .map_err(provider_error)?;
        ensure_success(response)?;
        Ok(())
    }

    async fn playlist_delete(&self, id: PlaylistId) -> domain::Result<()> {
        let response = self
            .client
            .library()
            .playlist_delete(id.0)
            .await
            .map_err(provider_error)?;
        ensure_success(response)?;
        Ok(())
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

    async fn artist(&self, id: domain::ArtistId) -> domain::Result<domain::ArtistDetail> {
        if id.0 == 0 {
            return Err(DomainError::InvalidInput(
                "artist id must be greater than zero".to_owned(),
            ));
        }
        let response = self
            .client
            .discovery()
            .artist(id.0)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        let detail = map::artist_detail(response.body.data.artist);
        if detail.id.0 == 0 && detail.name.is_empty() {
            return Err(DomainError::NotFound(format!("artist {id}")));
        }
        Ok(detail)
    }

    async fn artist_albums(
        &self,
        id: domain::ArtistId,
        limit: u32,
        offset: u32,
    ) -> domain::Result<domain::ArtistAlbums> {
        if id.0 == 0 {
            return Err(DomainError::InvalidInput(
                "artist id must be greater than zero".to_owned(),
            ));
        }
        let (limit, offset) = map_page(limit, offset)?;
        let response = self
            .client
            .discovery()
            .artist_albums(id.0, limit, offset)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(map::artist_albums(response.body))
    }

    async fn album(&self, id: domain::AlbumId) -> domain::Result<domain::AlbumDetail> {
        if id.0 == 0 {
            return Err(DomainError::InvalidInput(
                "album id must be greater than zero".to_owned(),
            ));
        }
        let response = self
            .client
            .discovery()
            .album(id.0)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        let detail = map::album_detail(response.body);
        if detail.album.id.0 == 0 && detail.album.name.is_empty() {
            return Err(DomainError::NotFound(format!("album {id}")));
        }
        Ok(detail)
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
            nickname: domain::sanitize_line(&profile.nickname),
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

    async fn comments(&self, id: TrackId, limit: u16, offset: u32) -> domain::Result<Comments> {
        if limit == 0 {
            return Err(DomainError::InvalidInput(
                "comment limit must be greater than zero".to_owned(),
            ));
        }
        let response = self
            .client
            .library()
            .comments(id.0, limit, offset)
            .await
            .map_err(provider_error)?;
        let response = ensure_success(response)?;
        Ok(map::comments(response.body))
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
                nickname: response
                    .body
                    .nickname
                    .map(|nickname| domain::sanitize_line(&nickname)),
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
    use ncmapi::ApiResponse;

    use super::{NcmMusicService, NcmOptions, ensure_success};

    fn service() -> NcmMusicService {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut options = NcmOptions::new(dir.path().join("session.cookies"));
        options.cache = false;
        options.cache_ttl = Duration::from_secs(1);
        NcmMusicService::new(options).expect("client construction must not touch the network")
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

    #[test]
    fn provider_404_is_not_found() {
        let response = ApiResponse {
            code: 404,
            message: Some("missing".to_owned()),
            body: (),
        };
        assert!(matches!(
            ensure_success(response),
            Err(lekima_domain::DomainError::NotFound(_))
        ));
    }
}
