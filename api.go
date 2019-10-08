// powered by https://github.com/Binaryify/NeteaseCloudMusicApi

package main

import (
	"fmt"
	"os/exec"
	"path/filepath"
)

const (
	Domain     = "127.0.0.1"
	Port       = 3000
	APIRepoURI = "https://github.com/Binaryify/NeteaseCloudMusicApi.git"
	APIRepo    = "/.lekima/NeteaseCloudMusicApi"
)

type APIServer struct {
	Cmd *exec.Cmd

	Repo    string
	RepoURI string

	// Routes

	Ok     chan bool
	Status string
	Routes

	// make requests
}

func NewAPIServer() *APIServer {
	homedir := getHomeDir()
	return &APIServer{
		Repo:    filepath.Join(homedir, APIRepo),
		RepoURI: filepath.Join(homedir, APIRepoURI),

		Routes: NewRoutes(),

		// Ready2Start: make(chan bool),
		Ok:     make(chan bool),
		Status: Inactive,
	}
}

const (
	Inactive       = "inactive"
	Running        = "running"
	Starting       = "starting"
	Restarting     = "restarting"
	Terminating    = "terminating"
	Updating       = "updating"
	Pulling        = "pulling from upstream master"
	Cloning        = "cloning the repositry"
	InstallingPkgs = "installing packages"
	Initializing   = "Initializing"
)

func (s *APIServer) Mark(status string) *APIServer {
	s.Status = status
	return s
}

func (s *APIServer) Notify(ch chan<- string) *APIServer {
	ch <- s.Status
	return s
}

func (s *APIServer) CloseNotifier(ch chan string) *APIServer {
	close(ch)
	return s
}

func (s *APIServer) MarkNotify(ch chan<- string, sta string) *APIServer {
	return s.Mark(sta).Notify(ch)
}

func (s *APIServer) Ready(ch chan<- bool) {
	ch <- true
}

func (s *APIServer) Start() *APIServer {
	s.Cmd = exec.Command("node", filepath.Join(s.Repo, "app.js"))
	err := s.Cmd.Start()
	chk(err)
	return s
}

func (s *APIServer) Restart() *APIServer {
	return s.Close().Start()
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

// type RouteMap map[string]string

var routemap = map[string]string{
	"login":        "/login/cellphone",
	"loginEmail":   "/login",
	"refreshLogin": "/login/refresh",
	"loginStatus":  "/login/status",
	"user":         "/user/detail", // params: userid
	"subcount":     "/user/subcount",
	"myPlaylist":   "/user/playlist", // params: userid
	"radio":        "/user/dj",       // params: userid
	// "follows": "/user/follows", // userid
	// "fans": "/user/followed",   // usreid
	"record":         "/user/record", // userid
	"subArtist":      "/artist/sub",  // artistid : id, type : 1 | 2
	"artistSubist":   "/artist/sublist",
	"playlistDetail": "/playlist/detail", //playlist id
	"song":           "/song/url",        // songid , br=999000
	"checkmusic":     "/check/music",     //songid, br=999000
	"search":         "/search",          //keywords, alt-> [limit, type, offset]
	// "searchHotList": "/search/hot",
	"searchSug":         "/search/suggest",     //keywords, alt->[type='mobile']
	"subPlaylist":       "/playlist/subscribe", // playlist id, type: 1:2
	"altPlaylist":       "/playlist/tracks",    // op: add | del, pid, songid
	"lyric":             "/lyric",              // songid
	"comments":          "/comment/music",      // songid, limit=20, offset, before( >5000)
	"songDetail":        "/song/detail",        // ids: songids[232,123,23]
	"recommendPlaylist": "/recommend/resource",
	"recommendSongs":    "/recommend/songs",
	"fm":                "/personal_fm",
	"dailyAttendance":   "/daily_signin",
	"like":              "/like",       // songid
	"fmTrash":           "/fm_trash",   // songid
	"scrobble":          "/scrobble",   // songid, playlistid
	"cloud":             "/user/cloud", // limit:20, offset=0
	"topList":           "/top/playlist",
}

type Routes map[string]string

func NewRoutes() Routes {
	var r Routes
	for name, url := range routemap {
		r[name] = fmt.Sprintf("http://%s:%d%s", Domain, Port, url)
	}
	return r
}
