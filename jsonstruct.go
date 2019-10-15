package main

type LoginStatusResp struct {
	Code int `json:"code"`
}

type LoginResp struct {
	Code int `json:"code"`
	Acc  `json:"account"`
}

type Acc struct {
	ID       int    `json:"id"`
	UserName string `json:"userName"`
}

type LoggedinStatusResp struct {
	Code    int `json:"code"`
	Profile `json:"profile,omitempty"`
}

type Profile struct {
	UserID   int    `json:"userId"`
	Nickname string `json:"nickname"`
}

type StatusCode struct {
	Code int `json:"code"`
}

type TopPlaylistsResp struct {
	Playlists []*Playlist `json:"playlists"`
	Code      int         `json:"code"`
	Total     int         `json:"total"`
	Category  string      `json:"cat"`
}

type SearchResp struct {
	Code         int `json:"code"`
	SearchResult `json:"result"`
}

type SongURLResp struct {
	Code int        `json:"code"`
	Data []*SongURL `json:"data"`
}

type PlaylistDetailResp struct {
	Code      int `json:"code"`
	*Playlist `json:"playlist"`
}

type FMResp struct {
	Code int       `json:"code"`
	Data []*Track2 `json:"data"`
}

type CloudResp struct {
	Code int           `json:"code"`
	Data []*CloudTrack `json:"data"`
}

type DJResp struct {
	Code     int   `json:"code"`
	DJRadios []*DJ `json:"djRadios"`
}

type CloudTrack struct {
	SimpleSong Track `json:"simpleSong"`
	SongID     int   `json:"songId"`
}

type RecommendSongsResp struct {
	Code      int       `json:"code"`
	Recommend []*Track2 `json:"recommend"`
}

type MyPlaylistResp struct {
	Code      int         `json:"code"`
	Playlists []*Playlist `json:"playlist"`
}

// recommend sonds | fm | search  --> results
// recommend sonds | fm | search
type Track2 struct {
	ID       int      `json:"id"`
	Name     string   `json:"name"`
	Artists  []Artist `json:"artists"`
	Pop      int      `json:"popularity,omitempty"`
	Album    `json:"album"`
	Duration int `json:"duration"`
}

type Playlist struct {
	Name        string   `json:"name"`
	ID          int      `json:"id"`
	TrackCount  int      `json:"trackCount"`
	Description string   `json:"description"`
	Tracks      []*Track `json:"tracks"`
}

func (p *Playlist) String() string {
	return p.Name
}

type Track struct {
	ID       int      `json:"id"`
	Name     string   `json:"name"`
	Artists  []Artist `json:"ar"`
	Pop      int      `json:"pop,omitempty"`
	Album    `json:"al"`
	Duration int `json:"dt"`
}
type Artist struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

type Album struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

type SearchResult struct {
	Songs []*Track2
}

// crumb to be improved
type SongURL struct {
	ID     int    `json:"id"`
	URL    string `json:"url"`
	Expire int    `json:"expi"`
}

type DJ struct {
	ID          int    `json:"id"`
	Name        string `json:"name"`
	Description string `json:"desc"`
}

type SidebarContents struct {
	FM         *Playlist
	Recommend  *Playlist
	MyPlaylist []*Playlist
	Cloud      *Playlist
	Top        []*Playlist
	// DJs            []*DJ
}
