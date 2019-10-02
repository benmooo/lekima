package main

import (
	"encoding/json"
	"io/ioutil"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
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
	// initialized
	Initiated chan bool

	// app infomation
	*Info

	// api server
	*APIServer
}

func NewLekima() *Lekima {
	return &Lekima{
		Initiated: make(chan bool),

		Info:      NewInfo(),
		APIServer: NewAPIServer(),
	}
}

func (l *Lekima) run() {
	// defer l.Close()

	// init
	go func() {
		l.Init().Initialized()
	}()

	// start api server
	go func() {
		// check if lekima initialized
		if <-l.Initiated {
			l.Mark(Starting).Start().Mark(Running)
		}
	}()

	// render ui
	go func() {

	}()

	go func() {
	}()
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
		l.MkHomeDir()
	}

	// chk settings file
	_, err = os.Stat(l.CfgFile)
	if os.IsNotExist(err) {
		l.NewCfgFile()
	}

	// check neteasecloudmusicapi && version -> 4 update
	_, err = os.Stat(l.Repo)
	if os.IsNotExist(err) {
		l.
			Mark(Cloning).
			Clone().
			Mark(InstallingPkgs).
			InstallPackages()
	}
	return l
}

func (l *Lekima) Initialized() *Lekima {
	l.Initiated <- true
	return l
}

func (l *Lekima) MkHomeDir() *Lekima {
	err := os.Mkdir(l.HomeDir, os.ModePerm)
	chk(err)
	return l
}

func (l *Lekima) NewCfgFile() *Lekima {
	cfg := NewCfg("", "")
	bytes, err := json.Marshal(cfg)
	chk(err)

	err = ioutil.WriteFile(l.CfgFile, bytes, os.ModePerm)
	chk(err)
	return l
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
