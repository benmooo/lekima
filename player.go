package main

import (
	"net/http"
	"net/url"

	"github.com/faiface/beep/mp3"
)

type Player struct {
	List  []*Song
	*Song // current focus
	Playlist

	History []*Song

	PlayMode
	Volume float32
	Speed  Speed

	// request to the music server
	Loader
}

func NewPlayer() *Player {
	return &Player{}
}

func (p *Player) Play(s *Song) {
	u, err := url.ParseRequestURI(s.URL)
	chk(err)
	p.Req.URL = u
	resp, err := p.Client.Do(p.Req)
	chk(err)
	defer resp.Body.Close()

	streamer, format, err := mp3.Decode(resp.Body)
	chk(err)
	defer streamer.Close()
	done := make(chan bool)

}

func (p *Player) Pause() {
}

func (p *Player) PlayFocus() *Player {
	return p
}

func (p *Player) TogglePlay() *Player {
	return p
}

func (p *Player) NextSong() *Player {
	return p
}

// func (p *Player) Pause() *Player {
// 	return p
// }

// func (p *Player) Pause() *Player {
// 	return p
// }

// func (p *Player) Pause() *Player {
// 	return p
// }

type Playlist struct {
	Songs []*Song
	Index int16
}

type Song struct {
	ID   int
	Name string
	Artist
	// Alia     []string
	// Pop      byte // popular 1-100
	Album
	Duration int
	SongURL
}

func NewSong(id int, name string, ar Artist, al Album, dt int, url SongURL) *Song {
	return &Song{
		ID:       id,
		Name:     name,
		Artist:   ar,
		Album:    al,
		Duration: dt,
		SongURL:  url,
	}
}

type Artist struct {
	ID   int
	Name string
}

type Album struct {
	ID   int
	Name string
}

type SongURL struct {
	URL    string
	Expire int
}

type Speed float32

var speedMap = map[string]Speed{
	"1.0x": 1.00,
	"1.2x": 1.20,
	"1.4x": 1.40,
	"1.5x": 1.50,
	"1.6x": 1.60,
	"1.8x": 1.80,
	"2.0x": 2.00,
}

type PlayMode byte

const (
	Order PlayMode = iota
	Loop
	SingleCycle
	Random
)

type Loader struct {
	Header map[string]string
	Client *http.Client
	Req    *http.Request
}

func (l *Loader) InitReq() *Loader {
	// add header to every request
	for k, v := range l.Header {
		l.Req.Header.Add(k, v)
	}
	return l
}

func NewLoader() *Loader {
	return &Loader{
		Header: map[string]string{
			"Range":          "bytes=0-",
			"Referer":        "https://music.163.com/",
			"Sec-Fetch-Mode": "cors",
			"User-Agent":     "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/76.0.3809.100 Safari/537.36",
		},
		Client: &http.Client{},
	}
}
