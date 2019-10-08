package main

import (
	"net/http"
	"net/url"
	"time"

	browser "github.com/EDDYCJY/fake-useragent"
	"github.com/faiface/beep"
	"github.com/faiface/beep/effects"
	"github.com/faiface/beep/mp3"
	"github.com/faiface/beep/speaker"
)

type Player struct {
	Playlists []*Playlist
	Playlist  Playlist

	// music stream instance
	MusicInstance

	CurrentList
	// Playlist       []*Song
	CurrentFocus   *Song // current focus
	CurrentPlaying *Song // current focus

	History []*Song

	PlayMode
	Volume float64
	Speed  beep.SampleRate

	SpeakerInitiated bool

	// request to the music server
	Loader
}

// type Playlist struct {
// 	ID   int
// 	Name string
// 	*list.List
// }

func NewPlayer() *Player {
	return &Player{}
}

func (p *Player) Init() *Player {
	// init loader
	p.InitReq()
	// another things

	return p
}

func (p *Player) InitSpeaker(sr beep.SampleRate, bufsize int) *Player {
	speaker.Init(sr, bufsize)
	return p
}

func (p *Player) prepare(s *Song) *Player {
	// check if song url has expired or not
	// if s.IsExpired() {
	// 	s.URL = p.FetchSongURL(s)
	// }
	// parse song url : *url.URL
	songurl, err := url.ParseRequestURI(s.URL)
	chk(err)
	// update url of request of player loader
	p.Loader.Req.URL = songurl
	resp, err := p.Loader.Client.Do(p.Req)
	chk(err)
	// decode response body which is an io.ReadCloser interfce
	// defalt decoder mp3 -> tobe improved
	streamer, f, err := mp3.Decode(resp.Body)
	// defer streamer.Close()
	// check if speaker initialized
	if !p.SpeakerInitiated {
		p.InitSpeaker(f.SampleRate*p.Speed, f.SampleRate.N(time.Second/20))
		p.SpeakerInitiated = true
	}
	// assign music instance
	ctrl := &beep.Ctrl{Streamer: streamer, Paused: false}
	p.MusicInstance = MusicInstance{
		Streamer: streamer,
		Ctrl:     ctrl,
		Vol:      &effects.Volume{Streamer: ctrl, Base: 2, Volume: p.Volume, Silent: false},
	}
	return p
}

func (p *Player) Play(s *Song) {
	// prepare
	done := make(chan bool)
	p.prepare(s)
	defer p.CloseStreamer()
	speaker.Play(beep.Seq(p.Vol), beep.Callback(func() {
		done <- true
	}))
	<-done
}

func (p *Player) CloseStreamer() *Player {
	p.MusicInstance.Streamer.Close()
	return p
}

func (p *Player) IncreaseVol() *Player {
	p.Volume += 0.2
	speaker.Lock()
	p.Vol.Volume = p.Volume
	speaker.Unlock()
	return p
}

func (p *Player) DecreaseVol() *Player {
	p.Volume -= 0.2
	speaker.Lock()
	p.Vol.Volume = p.Volume
	speaker.Unlock()
	return p
}

func (p *Player) Mute() *Player {
	speaker.Lock()
	p.Vol.Silent = true
	speaker.Unlock()
	return p
}

func (p *Player) Unmute() *Player {
	speaker.Lock()
	p.Vol.Silent = false
	speaker.Unlock()
	return p
}

func (p *Player) ToggleMute() *Player {
	speaker.Lock()
	p.Vol.Silent = !p.Vol.Silent
	speaker.Unlock()
	return p
}

func (p *Player) Pause() *Player {
	speaker.Lock()
	p.Ctrl.Paused = true
	speaker.Unlock()
	return p
}

func (p *Player) Resume() *Player {
	speaker.Lock()
	p.Ctrl.Paused = false
	speaker.Unlock()
	return p
}

func (p *Player) TogglePlay() *Player {
	speaker.Lock()
	p.Ctrl.Paused = !p.Ctrl.Paused
	speaker.Unlock()
	return p
}

func (p *Player) PlayFocus() {
	p.Play(p.CurrentFocus)
}

func (p *Player) PrevSong() *Song {
	l := len(p.CurrentList.Songs)
	i := p.CurrentList.Index
	if l < 1 {
		return nil
	}
	if i == 0 {
		return p.CurrentList.Songs[l-1]
	}
	return p.CurrentList.Songs[i-1]
}

func (p *Player) NextSong() *Song {
	l := len(p.CurrentList.Songs)
	i := p.CurrentList.Index
	if l < 1 {
		return nil
	}
	if i == l-1 {
		return p.CurrentList.Songs[0]
	}
	return p.CurrentList.Songs[i+1]
}

func (p *Player) UpdateFocus(s *Song) *Player {
	p.CurrentFocus = s
	return p
}

func (p *Player) FocusPrevSong() *Player {
	l := len(p.CurrentList.Songs)
	i := p.CurrentList.Index
	if l > 0 {
		if i == 0 {
			p.CurrentList.Index = l - 1

		} else {
			p.CurrentList.Index = l - 1
		}
	}
	return p
}

func (p *Player) FocusNextSong() *Player {
	l := len(p.CurrentList.Songs)
	i := p.CurrentList.Index
	if l > 0 {
		if i == l-1 {
			p.CurrentList.Index = 0

		} else {
			p.CurrentList.Index = 0
		}
	}
	return p
}

func (p *Player) FocusTop() *Player {
	l := len(p.CurrentList.Songs)
	if l > 0 {
		p.CurrentList.Index = 0
	}
	return p
}

func (p *Player) FocusBottom() *Player {
	l := len(p.CurrentList.Songs)
	if l > 0 {
		p.CurrentList.Index = l - 1
	}
	return p
}

func (p *Player) AddToHistory(s *Song) *Player {
	p.History = append(p.History, s)
	return p
}

func (p *Player) ClearHistory() *Player {
	p.History = []*Song{}
	return p
}

// func (p *Player)

type CurrentList struct {
	Songs []*Song
	Index int
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

func (s *Song) IsExpired() bool {
	return s.URL == ""
}

var speedMap = map[string]float64{
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

func (l *Loader) InitReq() {
	// add header to every request
	for k, v := range l.Header {
		l.Req.Header.Add(k, v)
	}
}

func NewLoader() *Loader {
	return &Loader{
		Header: map[string]string{
			"Range":          "bytes=0-",
			"Referer":        "https://music.163.com/",
			"Sec-Fetch-Mode": "cors",
			"User-Agent":     browser.Chrome(),
		},
		Client: &http.Client{},
	}
}

type MusicInstance struct {
	Streamer beep.StreamSeekCloser
	Ctrl     *beep.Ctrl
	Vol      *effects.Volume
}
