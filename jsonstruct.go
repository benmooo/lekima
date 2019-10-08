package main

type LoginStatusResp struct {
	Code int `json:"code"`
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

type Playlist struct {
	Name        string   `json:"name"`
	ID          int      `json:"id"`
	TrackCount  int      `json:"trackCount"`
	Description string   `json:"description"`
	Tracks      []*Track `json:"tracks"`
}

type Track struct {
	ID       int      `json:"id"`
	Name     string   `json:"name"`
	Artists  []Artist `json:"ar"`
	Pop      int      `json:"pop"`
	Album    `json:"al"`
	Duration int `json:"dt"`
}

type PlaylistDetailResp struct {
	Code     int `json:"code"`
	Playlist `json:"playlist"`
}
type Artist struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

type Album struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

type SongURL struct {
	URL    string
	Expire int
}
