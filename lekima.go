package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/ioutil"
	"net/http"
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
	Loggedin bool

	// app infomation
	*Info

	// api server
	*APIServer

	// ui
	*UI
}

func NewLekima() *Lekima {
	return &Lekima{
		Info:      NewInfo(),
		APIServer: NewAPIServer(),
		UI:        NewUI(),
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

func (l *Lekima) Login(acc Account) error {
	reg := regexp.MustCompile("^1[35789][0-9]{9}$")
	isphone := reg.MatchString(acc.username)
	params := Query{"password": acc.pwd}
	if isphone {
		params["phone"] = acc.username
	} else {
		params["email"] = acc.username
	}
	d := l.Req("login", params)
	if d["code"].(float64) != 200 {
		return errors.New("login failed")
	}
	l.Loggedin = true
	return nil
}

func (l *Lekima) User() User {
	if !l.Loggedin {
		return User{}
	}
	data := l.Req("loginStatus")
	if data["code"].(float64) != 200 {
		return User{}
	}
	return User{
		ID:       int(data["profile"].(map[string]interface{})["userId"].(float64)),
		Nickname: data["profile"].(map[string]interface{})["nickname"].(string),
	}
}

func (l *Lekima) Req(routename string, ps ...Params) Data {
	url := l.Routes[routename]
	if len(ps) > 0 {
		var query []string
		for _, p := range ps {
			query = append(query, p.Assemble())
		}
		url = fmt.Sprintf("%s?%s", url, strings.Join(query, "&"))
	}
	resp, err := http.Get(url)
	chk(err)
	defer resp.Body.Close()
	body, err := ioutil.ReadAll(resp.Body)
	chk(err)
	var data Data
	err = json.Unmarshal(body, &data)
	chk(err)
	return data
}

func (l *Lekima) FetchSongURL(s *Song) string {
	params := Query{
		"id": strconv.Itoa(s.ID),
		"br": "320000",
	}
	d := l.Req("songurl", params)
	return d["data"].([]interface{})[0].(map[string]interface{})["url"].(string)
}

// fetch playlist
func (l *Lekima) FetchPlaylists() {
	// !logged in  top play
	// var topLists = []*Playlist
	if !l.Loggedin {
		data := l.Req("topList", Query{"limit": "5"})
		if data["code"].(float64) == 200 {
			id := data["playlists"].([]map[string]interface{})[0]["id"].(float64)
		}
	}

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

type Data map[string]interface{}

type User struct {
	ID       int
	Nickname string
}
