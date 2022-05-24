package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/ioutil"
	"log"
	"math/rand"
	"net/http"
	"net/http/cookiejar"
	"os"
	"os/user"
	"path/filepath"
	"strings"
	"time"

	"github.com/benmooo/ncmapi"
	// apitypes "github.com/benmooo/ncmapi/api-types"
	ui "github.com/gizak/termui/v3"
)

// defaults
const (
	AppName    = "lekima"
	AppVersion = "1.0.0"
	Home       = "/.lekima"
	ConfigFile = "/.lekima/cfg.json"
	LogFile    = "/.lekima/log"
)

func getHomeDir() string {
	u, err := user.Current()
	chk(err)
	return u.HomeDir
}

// Lekima : the app instance
type Lekima struct {
	User
	Loggedin bool

	// app infomation
	*Info

	// api
	api *ncmapi.NeteaseAPI

	// ui
	*UI
	// Client
	Client *http.Client

	// Player
	*Player
	*Playlist
	Index int
}

func NewLekima() *Lekima {
	cookieJar, _ := cookiejar.New(nil)
	return &Lekima{
		Info:     NewInfo(),
		api:      ncmapi.Default(),
		UI:       NewUI(),
		Loggedin: false,

		Client: &http.Client{Jar: cookieJar},
		Player: NewPlayer(),
		Playlist: &Playlist{
			Tracks: []*Track{
				{Name: "void"},
			},
		},
	}
}

func (l *Lekima) Init() *Lekima {
	// check $USER/.lekima dir
	_, err := os.Stat(l.HomeDir)
	if os.IsNotExist(err) {
		l.mkHomeDir()
	}
	// chk settings file
	_, err = os.Stat(l.CfgFile)
	if os.IsNotExist(err) {
		l.newCfgFile()
	}
	return l
}

func (l *Lekima) ListenExit(ch <-chan bool) {
	if <-ch {
		// l.UI.Render(l.UI.Exit)
		fmt.Println("programe exit.")
	}
}

func (l *Lekima) mkHomeDir() *Lekima {
	err := os.Mkdir(l.HomeDir, os.ModePerm)
	chk(err)
	return l
}

func (l *Lekima) newCfgFile() *Lekima {
	cfg := NewCfg("", "")
	bytes, err := json.Marshal(cfg)
	chk(err)
	err = ioutil.WriteFile(l.CfgFile, bytes, os.ModePerm)
	chk(err)
	return l
}

func (l *Lekima) WriteAccount(acc Account) {
	cfg := NewCfg(acc.username, acc.pwd)
	bytes, err := json.Marshal(cfg)
	chk(err)
	err = ioutil.WriteFile(l.CfgFile, bytes, os.ModePerm)
	chk(err)
}

func (l *Lekima) ReadCfg() *Cfg {
	f, err := os.Open(l.Info.CfgFile)
	chk(err)
	byt, err := ioutil.ReadAll(f)
	chk(err)
	var cfg Cfg
	err = json.Unmarshal(byt, &cfg)
	chk(err)
	return &cfg
}

func (l *Lekima) ReadAccount() Account {
	cfg := l.ReadCfg()
	return Account{
		username: cfg.Account,
		pwd:      cfg.Passwd,
	}
}

func (l *Lekima) Login(acc Account) error {
	if acc.username == "" {
		return errors.New("invalid account")
	}

	res, err := l.api.LoginPhone(acc.username, acc.pwd)
	if err != nil {
		return err
	}

	// get user id, name
	var resp LoginResp
	err = json.Unmarshal(res.Data, &resp)
	chk(err)

	if resp.Code == 200 && resp.Acc != nil {
		l.Loggedin = true
		l.User = User{
			ID: resp.Acc.ID,
		}
	}
	return nil
}

func (l *Lekima) CheckLoginStatus() {
	res, _ := l.api.LoginStatus()

	var s LoggedinStatusResp
	err := json.Unmarshal(res.Data, &s)
	chk(err)
	if s.Profile != nil {
		l.Loggedin = true
		l.User.ID = s.Profile.UserID
	}
}

func (l *Lekima) FetchUserDetail(id int) Profile {
	res, _ := l.api.UserDetail(id)

	var resp UserDetailResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("failed to fetch user detail")
	}
	l.User.Nickname = resp.Profile.Nickname
	return resp.Profile
}

func (l *Lekima) FetchSearch(keywords string) *Playlist {
	if len(keywords) < 1 {
		return nil
	}

	res, _ := l.api.Search(keywords)
	var resp SearchResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("failed to search with keywors " + keywords)
	}
	var ts []*Track
	for _, v := range resp.SearchResult.Songs {
		t := Track(*v)
		ts = append(ts, &t)
	}

	return &Playlist{
		Name:        "Search",
		Description: "SearchResult",
		Tracks:      ts,
	}

}

func (l *Lekima) FetchSongURL(id int) *SongURL {
	res, _ := l.api.SongUrl(id)
	var su SongURLResp
	err := json.Unmarshal(res.Data, &su)
	chk(err)
	if su.Code != 200 {
		log.Panic("failed to fetch song url")
	}
	return su.Data[0]
}
func (l *Lekima) FetchPlaylistDetail(id int) *Playlist {
	res, _ := l.api.PlaylistDetail(id)

	var resp PlaylistDetailResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("failed to fetch playlist detail")
	}
	return resp.Playlist
}

// fetch fm
func (l *Lekima) FetchFM() *Playlist {
	res, _ := l.api.PersonalFm()

	var resp FMResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("fail to fetch fm")
	}
	var ts []*Track
	for _, v := range resp.Data {
		var t Track = Track(*v)
		ts = append(ts, &t)
	}

	return &Playlist{
		Name:        "FM",
		Description: "Personal_FM",
		Tracks:      ts,
	}
}

func (l *Lekima) ExpandFMTracks(list []*Track) *Lekima {
	l.Playlist.Tracks = append(l.Playlist.Tracks, list...)
	return l
}

// fetch daily recommend songs
func (l *Lekima) FetchRecommendSongs() *Playlist {
	res, _ := l.api.RecommendSongs()
	var resp RecommendSongsResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("fail to fetch recommend songs")
	}
	var ts []*Track
	for _, v := range resp.Data.DailySongs {
		var t Track = Track(*v)
		ts = append(ts, &t)
	}
	date := time.Now().Format("02 Jan")
	return &Playlist{
		Name:        date,
		Description: "Recommend_Songs",
		Tracks:      ts,
	}
}

// fetch daily cloud
func (l *Lekima) FetchCloud() *Playlist {
	res, _ := l.api.UserCloud()
	var resp CloudResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("fail to fetch cloud")
	}
	var ts []*Track
	for _, v := range resp.Data {
		ts = append(ts, &v.SimpleSong)
	}
	return &Playlist{
		Name:        "Cloud",
		Description: "Cloud_Data",
		Tracks:      ts,
	}
}

// fetch my playlists
func (l *Lekima) FetchMyPlaylist() []*Playlist {
	res, _ := l.api.UserPlaylist(l.User.ID)

	var resp MyPlaylistResp
	err := json.Unmarshal(res.Data, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("fail to fetch myplaylist")
	}
	return resp.Playlists
}

//

// fetch playlist
func (l *Lekima) FetchSidebarContent() *SidebarContents {
	// not logged in -> top play
	if !l.Loggedin {
		return &SidebarContents{}
	}
	return &SidebarContents{
		FM:         l.FetchFM(),
		Recommend:  l.FetchRecommendSongs(),
		Cloud:      l.FetchCloud(),
		MyPlaylist: l.FetchMyPlaylist(),
	}
}

func (l *Lekima) HandleLogin(uiEvent <-chan ui.Event) {
	password := ""
	for {
		e := <-uiEvent
		switch e.ID {
		case "<Tab>":
			l.UI.Login.ToggleFocus()
		case "<Space>":
			if l.UI.Login.Focus == l.UI.Login.Username {
				l.UI.Login.AppendFocusText(" ")
			} else {
				l.UI.Login.AppendFocusText("*")
				password += " "
			}
		case "<Backspace>":
			l.UI.Login.PopFocusText()
			if l.UI.Login.Focus == l.UI.Login.Password {
				length := len(password)
				if length > 0 {
					password = password[0 : length-1]
				}
			}
		case "<Enter>":
			// check login
			acc := Account{l.UI.Login.Username.Text, password}
			if err := l.Login(acc); err != nil {
				password = ""
				l.UI.Login.Clear()
				l.UI.Login.Username.Title = "Login failed, plz try again."
			} else {
				// write username & password to local files
				l.WriteAccount(acc)
				return
			}
		case "<Escape>":
		case "Resize":
			l.UI.ResizeLogin()
		default:
			if l.UI.Login.Focus == l.UI.Login.Username {
				l.UI.Login.AppendFocusText(e.ID)
			} else {
				l.UI.Login.AppendFocusText("*")
				password += e.ID
			}
		}
		l.UI.Render(l.UI.Login.Username, l.UI.Login.Password)
	}
}

func (l *Lekima) RefreshUIHeader() {
	l.UI.RefreshHeader(
		l.User.Nickname,
		l.Player.PlayMode,
		l.Player.Volume,
		l.Playlist.Tracks[l.Index].Name,
		l.Player.Status,
	)
}

func (l *Lekima) EventLoop(uiEvent <-chan ui.Event, quit chan<- bool) {
	for {
		select {
		// next song
		case index := <-l.Player.PlayNext:
			songCount := len(l.Playlist.Tracks)
			switch l.Player.PlayMode {
			case Loop:
				index = ringNext(songCount, index)
				l.Player.Play(l.Playlist, index, l.FetchSongURL)
				l.Index = index
				l.RefreshUIHeader()
			case SingleCycle:
				l.Player.Play(l.Playlist, index, l.FetchSongURL)
			case Random:
				index = rand.Intn(songCount)
				l.Player.Play(l.Playlist, index, l.FetchSongURL)
				l.Index = index
				l.RefreshUIHeader()
			}
		case e := <-uiEvent:
			// sidebar key events handler
			switch l.UI.Focus {
			case SidebarTile:
				switch e.ID {
				// case "<MouseLeft>":
				// 	l.UI.MainContent.HandleClick(e.Payload.(ui.Mouse).X, e.Payload.(ui.Mouse).Y)
				case "q", "<C-c>":
					l.Exit(quit)
				case "<Tab>":
					l.UI.ToggleFocus(MainContentTile)
				case "o", "<Enter>":
					n := l.UI.Sidebar.SelectedNode()
					if n.Nodes != nil {
						l.UI.Sidebar.ToggleExpand()
					} else {
						l.UI.ToggleFocus(MainContentTile)
						p := n.Value.(*Playlist)
						if p.Tracks == nil {
							p = l.FetchPlaylistDetail(p.ID)
						}
						l.Playlist = p
						l.UI.SetMainContent(p)
						l.UI.MainContent.ScrollTop()
					}
					// l.Player.SetStatus(1)
				case "j":
					l.UI.Sidebar.ScrollDown()
				case "k":
					l.UI.Sidebar.ScrollUp()
				case "l":
					// l.UI.ScrollUp()
				case "/":
					l.UI.ToggleFocus(SearchBoxTile)
					l.UI.ToggleSearchBox().ClearSearchText()
				case "?":
					l.UI.ToggleFocus(HelpTile)
					l.UI.ToggleHelp()
				case "<Resize>":
					l.UI.ResizeLayout()
				case "P":
					l.Player.TogglePlayMode()
				}
			case MainContentTile:
				switch e.ID {
				case "<MouseLeft>":
					l.UI.MainContent.HandleClick(e.Payload.(ui.Mouse).X, e.Payload.(ui.Mouse).Y)
				case "<Tab>":
					l.UI.ToggleFocus(SidebarTile)
				case "q", "<C-c>":
					l.Exit(quit)
				case "g":
					l.UI.MainContent.ScrollTop()
				case "G":
					l.UI.MainContent.ScrollBottom()
				case "j":
					l.UI.MainContent.ScrollDown()
				case "k":
					l.UI.MainContent.ScrollUp()
				case "<Space>":
					l.Player.TogglePlay().ToggleStatus()
					l.RefreshUIHeader()
				case "o", "<Enter>":
					if l.Player.Streamer != nil {
						l.Player.CloseStreamer()
						l.Player.Interupt = true
					}
					// start a new play loop
					index := l.UI.MainContent.SelectedRow
					l.Player.Play(l.Playlist, index, l.FetchSongURL)
					// handler status change
					l.Index = index
					l.Player.SetStatus(1)
					l.RefreshUIHeader()
				case "m":
					l.Player.ToggleMute()
				case "=":
					l.Player.IncreaseVol()
					l.RefreshUIHeader()
				case "-":
					l.Player.DecreaseVol()
					l.RefreshUIHeader()
				case "/":
					l.UI.ToggleFocus(SearchBoxTile)
					l.UI.ToggleSearchBox().ClearSearchText()
				case "?":
					l.UI.ToggleFocus(HelpTile)
					l.UI.ToggleHelp()
				case "<Resize>":
					l.UI.ResizeLayout()
				case "P":
					l.Player.TogglePlayMode()
					l.RefreshUIHeader()
				case "r":
					p := l.FetchFM()
					l.Playlist = p
					l.UI.SetMainContent(p)
					l.UI.MainContent.ScrollTop()
				}
			case SearchBoxTile:
				switch e.Type {
				case ui.KeyboardEvent:
					switch e.ID {
					case "<Tab>", "<Space>":
						l.UI.AppendSearchText(" ")
					case "<Escape>":
						l.ToggleSearchBox()
						l.UI.ClearSearchText()
						l.UI.ToggleFocus(MainContentTile)
					case "<Enter>":
						p := l.FetchSearch(l.UI.SearchBox.Text)
						l.Playlist = p
						l.UI.SetMainContent(p)
						l.UI.MainContent.ScrollTop()
						l.ToggleSearchBox()
						l.UI.ClearSearchText()
						l.UI.ToggleFocus(MainContentTile)
					case "<C-c>":
						l.Exit(quit)
					case "<Backspace>":
						l.UI.PopSearchText()
					default:
						l.UI.AppendSearchText(e.ID)

					}
				case ui.ResizeEvent:
					l.UI.ResizeLayout()
					l.UI.ResizeSearchBox()

				case ui.MouseEvent:

				}

			case HelpTile:
				switch e.ID {
				case "<C-c>":
					l.Exit(quit)
				case "<Resize>":
					l.UI.ResizeLayout()
					l.UI.ResizeHelp()
				case "<Escape>":
					l.UI.ToggleHelp()
					l.UI.ToggleFocus(MainContentTile)
				}

			}

		}
		l.UI.RenderLayout()
	}

}

func (l *Lekima) Exit(c chan<- bool) {
	c <- true
}

type Account struct {
	username string
	pwd      string
}

//
type Info struct {
	AppName string
	Version string

	// path
	HomeDir string
	CfgFile string
	LogFile string
}

func NewInfo() *Info {
	homedir := getHomeDir()
	return &Info{
		AppName: AppName,
		Version: AppVersion,
		HomeDir: filepath.Join(homedir, Home),
		CfgFile: filepath.Join(homedir, ConfigFile),
		LogFile: filepath.Join(homedir, LogFile),
	}
}

type Cfg struct {
	Account string `json:"account"`
	Passwd  string `json:"passwd"`
	// Vol float32
}

func NewCfg(account, passwd string) *Cfg {
	return &Cfg{
		Account: account,
		Passwd:  passwd,
	}
}

type Params interface {
	Assemble() string
}

type Query map[string]string

func (qm Query) Assemble() string {
	var p []string
	for k, v := range qm {
		p = append(p, fmt.Sprintf("%s=%s", k, v))
	}
	return strings.Join(p, "&")
}

type User struct {
	ID       int
	Nickname string
}

type Check struct {
	Stop  bool
	Index int
}
