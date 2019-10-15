package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/ioutil"
	"log"
	"net/http"
	"net/http/cookiejar"
	"net/url"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
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

	// api server
	*APIServer

	// ui
	*UI
	// Client
	Client *http.Client

	// Player
	*Player
}

func NewLekima() *Lekima {
	cookieJar, _ := cookiejar.New(nil)
	return &Lekima{
		Info:      NewInfo(),
		APIServer: NewAPIServer(),
		UI:        NewUI(),
		Loggedin:  false,

		Client: &http.Client{Jar: cookieJar},
		Player: NewPlayer(),
	}
}

func (l *Lekima) Init() *Lekima {
	// check prerequestes libasounds-2, git, node, npm, npx
	prerequisites := []string{"git", "node", "npm"}
	for _, v := range prerequisites {
		_, err := exec.LookPath(v)
		chk(err)
	}
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
	// check neteasecloudmusicapi && version -> 4 update
	_, err = os.Stat(l.Repo)
	if os.IsNotExist(err) {
		l.
			Clone().
			InstallPackages()
	}
	// init player
	// l.Player.Init()
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

func (l *Lekima) Login() error {
	acc := l.ReadAccount()
	// check if is valid account
	if acc.username == "" {
		return errors.New("invalid account")
	}
	reg := regexp.MustCompile("^1[35789][0-9]{9}$")
	isphone := reg.MatchString(acc.username)
	params := Query{"password": acc.pwd}
	if isphone {
		params["phone"] = acc.username
	} else {
		params["email"] = acc.username
	}
	byt := l.Req("login", params)
	var s StatusCode
	err := json.Unmarshal(byt, &s)
	chk(err)
	if s.Code != 200 {
		return errors.New("login failed")
	}
	l.Loggedin = true
	// get user id, name
	var resp LoginResp
	err = json.Unmarshal(byt, &resp)
	chk(err)
	// attach user to l
	l.User = User{
		ID: resp.Acc.ID,
	}
	return nil
}

func (l *Lekima) LoginStatus() StatusCode {
	byt := l.Req("loginStatus")
	var s StatusCode
	err := json.Unmarshal(byt, &s)
	chk(err)
	if s.Code == 200 {
		l.Loggedin = true
	}
	return s
}

// func (l *Lekima) User() User {
// return l.User
// if !l.Loggedin {
// 	return User{}
// }
// byt := l.Req("loginStatus")
// if data["code"].(float64) != 200 {
// 	return User{}
// }
// return User{
// 	ID:       int(data["profile"].(map[string]interface{})["userId"].(float64)),
// 	Nickname: data["profile"].(map[string]interface{})["nickname"].(string),
// }
// }

func (l *Lekima) Req(routename string, ps ...Params) []byte {
	url := l.Routes[routename]
	if len(ps) > 0 {
		var query []string
		for _, p := range ps {
			query = append(query, p.Assemble())
		}
		url = fmt.Sprintf("%s?%s", url, strings.Join(query, "&"))
	}
	req, _ := http.NewRequest("GET", url, nil)
	resp, err := l.Client.Do(req)
	chk(err)
	defer resp.Body.Close()
	byt, err := ioutil.ReadAll(resp.Body)
	chk(err)
	// var data Data
	// err = json.Unmarshal(body, &data)
	// chk(err)
	return byt
}

func (l *Lekima) FetchSearch(keywords string) *Playlist {
	params := Query{"keywords": url.QueryEscape(keywords)}
	byt := l.Req("search", params)
	var resp SearchResp
	err := json.Unmarshal(byt, &resp)
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

func (l *Lekima) FetchSongURL(id string) *SongURL {
	params := Query{
		"id": id,
		"br": "320000",
	}
	byt := l.Req("song", params)
	var su SongURLResp
	err := json.Unmarshal(byt, &su)
	chk(err)
	if su.Code != 200 {
		log.Panic("failed to fetch song url")
	}
	return su.Data[0]
}
func (l *Lekima) FetchPlaylistDetail(id string) *Playlist {
	params := Query{"id": id}
	byt := l.Req("playlistDetail", params)
	var resp PlaylistDetailResp
	err := json.Unmarshal(byt, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("failed to fetch playlist detail")
	}
	return resp.Playlist
}

// fetch top playlists
func (l *Lekima) FetchTop(limit int) []*Playlist {
	bytes := l.Req("topList", Query{"limit": strconv.Itoa(limit)})
	var resp TopPlaylistsResp
	err := json.Unmarshal(bytes, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("fail to fetch top playlists")
	}
	return resp.Playlists
}

// fetch fm
func (l *Lekima) FetchFM() *Playlist {
	bytes := l.Req("fm")
	var resp FMResp
	err := json.Unmarshal(bytes, &resp)
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

// fetch daily recommend songs
func (l *Lekima) FetchRecommendSongs() *Playlist {
	bytes := l.Req("recommendSongs")
	var resp RecommendSongsResp
	err := json.Unmarshal(bytes, &resp)
	chk(err)
	if resp.Code != 200 {
		log.Panic("fail to fetch recommend songs")
	}
	var ts []*Track
	for _, v := range resp.Recommend {
		var t Track = Track(*v)
		ts = append(ts, &t)
	}
	return &Playlist{
		Name:        "Recommend",
		Description: "Recommend_Songs",
		Tracks:      ts,
	}
}

// fetch daily cloud
func (l *Lekima) FetchCloud() *Playlist {
	bytes := l.Req("cloud")
	var resp CloudResp
	err := json.Unmarshal(bytes, &resp)
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
	params := Query{"uid": strconv.Itoa(l.User.ID)}
	bytes := l.Req("myPlaylist", params)
	var resp MyPlaylistResp
	err := json.Unmarshal(bytes, &resp)
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
		return &SidebarContents{
			Top: l.FetchTop(5),
		}
	}
	return &SidebarContents{
		Top:        l.FetchTop(5),
		FM:         l.FetchFM(),
		Recommend:  l.FetchRecommendSongs(),
		Cloud:      l.FetchCloud(),
		MyPlaylist: l.FetchMyPlaylist(),
	}
}

func (l *Lekima) EventLoop() {
	uiEvent := l.UI.PollEvents()
	for {
		select {
		case e := <-uiEvent:
			switch l.UI.Focus {
			case SidebarTile:
				switch e.ID {
				case "q", "<C-c>":
					return
				case "<Tab>":
					l.UI.ToggleFocus(MainContentTile)
				case "o", "<Enter>":
					// n := l.UI.Sidebar.SelectedNode()
				case "j":
					l.UI.Sidebar.ScrollDown()
				case "k":
					l.UI.Sidebar.ScrollUp()
				}
			case MainContentTile:
				switch e.ID {
				}

			}
		}

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
