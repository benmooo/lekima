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
	AppName    = "lekima"
	AppVersion = "1.0.0"
	APIRepoURI = "https://github.com/Binaryify/NeteaseCloudMusicApi.git"
	Home       = "/.lekima"
	ConfigFile = "/.lekima/cfg.json"
	LogFile    = "/.lekima/log"
	APIRepo    = "/.lekima/NeteaseCloudMusicApi"
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
		// l.init()
	}()

	// start api server
	go func() {
		// l.startAPIServer()
	}()

	// init
	go func() {
		// l.init()
	}()

	// init
	go func() {
		// l.init()
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
		l.Mark(Cloning).
			Clone().
			Mark(InstallingPkgs).
			InstallPackages()
	}
	return l
}

// func (l *Lekima)

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

type APIServer struct {
	Cmd     *exec.Cmd
	Repo    string
	RepoURI string

	Ready2Start, Ok chan bool
	Status          chan APIServerStatus
}

func NewAPIServer() *APIServer {
	return &APIServer{
		Repo:    APIRepo,
		RepoURI: APIRepoURI,

		Ready2Start: make(chan bool),
		Ok:          make(chan bool),
		Status:      make(chan APIServerStatus, 4),
	}
}

func (s *APIServer) Start() *APIServer {
	s.Cmd = exec.Command("node", s.Repo+"/app.js")
	err := s.Cmd.Start()
	chk(err)
	return s
}

func (s *APIServer) Restart() *APIServer {
	return s.Close().Start()
}

func (s *APIServer) Mark(status APIServerStatus) *APIServer {
	s.Status <- status
	return s
}

func (s *APIServer) Close() *APIServer {
	err := s.Cmd.Process.Kill()
	chk(err)
	return s
}

func (s *APIServer) Pull() *APIServer {
	cmd := exec.Command("git", "-C", s.Repo, "pull", "--depth=1")
	err := cmd.Run()
	chk(err)
	return s
}

func (s *APIServer) Clone() *APIServer {
	cmd := exec.Command("git", "clone", "--depth=1", s.RepoURI, s.Repo)
	err := cmd.Run()
	chk(err)
	return s
}

func (s *APIServer) InstallPackages() *APIServer {
	cmd := exec.Command("npm", "i", "--prefix", s.Repo)
	err := cmd.Run()
	chk(err)
	return s
}

func (s *APIServer) Update() *APIServer {
	return s.Pull().InstallPackages()
}

type APIServerStatus string

const (
	Inactive       APIServerStatus = "inactive"
	Running        APIServerStatus = "running"
	Starting       APIServerStatus = "starting"
	Terminating    APIServerStatus = "starting"
	Updating       APIServerStatus = "updating"
	Pulling        APIServerStatus = "pulling from master"
	Restarting     APIServerStatus = "restarting"
	Cloning        APIServerStatus = "cloning the repositry"
	InstallingPkgs APIServerStatus = "installing packages"
)

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
	userHomedir := getHomeDir()
	return &Info{
		AppName: AppName,
		Version: AppVersion,
		HomeDir: userHomedir + Home,
		CfgFile: userHomedir + ConfigFile,
		LogFile: userHomedir + LogFile,
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
