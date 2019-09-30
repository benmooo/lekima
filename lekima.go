package main

import (
	"encoding/json"
	"io/ioutil"
	"os"
	"os/exec"
	"os/user"
)

// defaults
const (
	AppName = "lekima"
	Version = "1.0.0"
	ApiRepo = "https://github.com/Binaryify/NeteaseCloudMusicApi.git"

	// settingsPath = "~/.lekima/settings.json"
	// logPath      = "~/.lekima/errlog.log"
	// volume       = 0.99
)

func getHomeDir() string {
	u, err := user.Current()
	chk(err)
	return u.HomeDir
}

// Config : basic configuration of the app
// type config struct {
// 	// path of config cache
// 	configDir string
// 	logDir    string
// 	logPath   string

// 	// schema
// 	colorSchema string
// 	// show help widget or not
// 	// helpVisible bool

// 	// timeof interval of app update
// 	// updateInterval time.Day

// 	// app mode
// 	miniMode bool

// 	// show desktop lyrics or not
// 	// desktopLyric bool
// 	// volume
// 	volume int
// }

// Lekima : the app instance
type Lekima struct {
	// *user
	// ui
	HomeDir string
	CfgPath string
	ApiPath string
	*ApiServer
}

func NewLekima() *Lekima {
	homedir := getHomeDir() + "/.lekima"
	return &Lekima{
		HomeDir: homedir,
		CfgPath: homedir + "/cfg.json",
		ApiPath: homedir + "/NeteaseCloudMusicApi",
		ApiServer: &ApiServer{
			Ready:  make(chan bool),
			Status: Dead,
		},
	}
}

func (l *Lekima) run() {
	l.init()

}

func (l *Lekima) init() *Lekima {
	// channel for communication
	// apiCh := make(chan bool)

	// check prerequestes libasounds-2, git, node, npm, npx
	prerequisites := []string{"git", "node", "npm"}
	for _, v := range prerequisites {
		_, err := exec.LookPath(v)
		chk(err)
	}

	// check $USER/.lekima dir
	_, err := os.Stat(l.HomeDir)
	if os.IsNotExist(err) {
		// make dir
		err = os.Mkdir(l.HomeDir, os.ModePerm)
		chk(err)
	}

	// chk settings file
	_, err = os.Stat(l.CfgPath)
	if os.IsNotExist(err) {
		// create a new settings file
		cfg := NewCfg("", "")
		bytes, err := json.Marshal(cfg)
		chk(err)

		err = ioutil.WriteFile(l.CfgPath, bytes, os.ModePerm)
		chk(err)
	}

	// check neteasecloudmusicapi && version
	_, err = os.Stat(l.ApiPath)
	if os.IsNotExist(err) {
		// clone the repo to local
		go func() {
			l.cloneApiRepo().Ready <- true
		}()
	} else {
		// check the version
	}

	// start the apiserver

	return l
}

func (l *Lekima) startApiServer() *Lekima {
	cmd := exec.Command("node", l.ApiPath+"/app.js")
	err := cmd.Run()
	chk(err)
	return l
}

func (l *Lekima) cloneApiRepo() *Lekima {
	cmd := exec.Command("git", "clone", "--depth=1", ApiRepo, l.ApiPath)
	err := cmd.Run()
	chk(err)
	return l
}

func (l *Lekima) updateApiRepo() *Lekima {
	cmd := exec.Command("git", "-C", l.ApiPath, "pull", "--depth=1")
	err := cmd.Run()
	chk(err)
	return l
}

type ApiServer struct {
	Ready  chan bool
	Status ApiServerStatus
}

type ApiServerStatus byte

const (
	Dead ApiServerStatus = iota
	Running
	Starting
	Suspending
)

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
