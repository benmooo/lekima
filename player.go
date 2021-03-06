package main

import (
	"fmt"
	"net/http"
	"os"
	"strconv"
	"time"

	browser "github.com/EDDYCJY/fake-useragent"
	"github.com/faiface/beep"
	"github.com/faiface/beep/effects"
	"github.com/faiface/beep/mp3"
	"github.com/faiface/beep/speaker"
)

type Player struct {
	SpeakerInitiated bool
	MusicInstance

	Status uint8 // 0 -> paused 1-> playing
	PlayMode
	Volume float64
	Speed  beep.SampleRate

	// request to the music server
	Client   *http.Client
	PlayNext chan int
	Interupt bool
}

// type Playlist struct {
// 	ID   int
// 	Name string
// 	*list.List
// }

func NewPlayer() *Player {
	return &Player{
		Client:   &http.Client{},
		Speed:    1.0,
		PlayMode: Loop,
		PlayNext: make(chan int),
		Interupt: false,
		Status:   0,
	}
}

func (p *Player) InitSpeaker(sr beep.SampleRate, bufsize int) *Player {
	speaker.Init(sr, bufsize)
	return p
}

func (p *Player) ToggleStatus() *Player {
	p.Status ^= 1
	return p
}

func (p *Player) SetStatus(i uint8) {
	p.Status = i
}

func (p *Player) prepare(s *SongURL) *Player {
	url := s.URL
	if url == "" {
		return p
	}
	req, _ := http.NewRequest("GET", url, nil)
	req.Header = defaultRequestHeader
	resp, err := p.Client.Do(req)
	chk(err)
	// decode response body which is an io.ReadCloser interfce
	// defalt decoder mp3 -> tobe improved
	streamer, f, err := mp3.Decode(resp.Body)
	if err != nil {
		// is not mp3 file
		fmt.Println("is not mp3 file!!!")
		os.Exit(1)
	}

	// defer streamer.Close()
	// check if speaker initialized
	if !p.SpeakerInitiated {
		p.InitSpeaker(f.SampleRate*1.0, f.SampleRate.N(time.Second/20))
		p.SpeakerInitiated = true
	}
	// assign music instance
	ctrl := &beep.Ctrl{Streamer: streamer}
	p.MusicInstance = MusicInstance{
		Streamer: streamer,
		Ctrl:     ctrl,
		Vol:      &effects.Volume{Streamer: ctrl, Base: 2, Volume: p.Volume},
	}
	return p
}

func (p *Player) Play(list *Playlist, index int, fetcher func(string) *SongURL) {
	p.prepare(fetcher(strconv.Itoa(list.Tracks[index].ID)))
	// speaker.Play(p.Vol)
	speaker.Play(beep.Seq(p.Vol, beep.Callback(func() {
		if p.Interupt {
			p.Interupt = false
		} else {
			p.CloseStreamer()
			p.PlayNext <- index
		}
	})))
}

// func (p *Player) Loop(p *Playlist, index int, fetcher func(string) *SongURL) {
// 	done := make(chan bool)
// 	songCount := len(p.Tracks)
// 	// current songurl
// 	songurl := fetcher(strconv.Itoa(p.Tracks[index].ID))
// 	p.Play(songurl, done)
// }

func ringNext(sum, index int) int {
	if sum-1 <= index {
		return 0
	}
	index++
	return index
}

func (p *Player) CloseStreamer() error {
	return p.MusicInstance.Streamer.Close()
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

func (p *Player) TogglePlayMode() {
	if p.PlayMode == Random {
		p.PlayMode = Loop
	} else {
		p.PlayMode++
	}
}

// type Song struct {
// 	ID   int
// 	Name string
// 	Artist
// 	// Alia     []string
// 	// Pop      byte // popular 1-100
// 	Album
// 	Duration int
// 	SongURL
// }

// func NewSong(id int, name string, ar Artist, al Album, dt int, url SongURL) *Song {
// 	return &Song{
// 		ID:       id,
// 		Name:     name,
// 		Artist:   ar,
// 		Album:    al,
// 		Duration: dt,
// 		SongURL:  url,
// 	}
// }

// var speedMap = map[string]float64{
// 	"1.0x": 1.00,
// 	"1.2x": 1.20,
// 	"1.4x": 1.40,
// 	"1.5x": 1.50,
// 	"1.6x": 1.60,
// 	"1.8x": 1.80,
// 	"2.0x": 2.00,
// }

type PlayMode uint8

const (
	Loop PlayMode = iota
	SingleCycle
	Random
)

type MusicInstance struct {
	Streamer beep.StreamSeekCloser
	Ctrl     *beep.Ctrl
	Vol      *effects.Volume
}

var defaultRequestHeader = http.Header{
	"Range":          []string{"bytes=0-"},
	"Referer":        []string{"https://music.163.com/"},
	"Sec-Fetch-Mode": []string{"cors"},
	"User-Agent":     []string{browser.Chrome()},
}
