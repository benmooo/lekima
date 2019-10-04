// powered by https://github.com/Binaryify/NeteaseCloudMusicApi

package main

import (
	"fmt"
	"io/ioutil"
	"net/http"
	"os/exec"
	"path/filepath"
	"strings"
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

	Routes

	Ok     chan bool
	Status chan APIServerStatus

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
		Status: make(chan APIServerStatus, 4),
	}
}

type APIServerStatus string

const (
	Inactive       APIServerStatus = "inactive"
	Running        APIServerStatus = "running"
	Starting       APIServerStatus = "starting"
	Restarting     APIServerStatus = "restarting"
	Terminating    APIServerStatus = "terminating"
	Updating       APIServerStatus = "updating"
	Pulling        APIServerStatus = "pulling from upstream master"
	Cloning        APIServerStatus = "cloning the repositry"
	InstallingPkgs APIServerStatus = "installing packages"
	Initializing   APIServerStatus = "Initializing"
)

type Routes map[string]string

func NewRoutes() Routes {
	var r Routes
	for name, url := range routemap {
		r[name] = fmt.Sprintf("http://%s:%d%s", Domain, Port, url)
	}
	return r
}

func (s *APIServer) Start() *APIServer {
	s.Mark(Starting)
	s.Cmd = exec.Command("node", filepath.Join(s.Repo, "app.js"))
	err := s.Cmd.Start()
	chk(err)
	return s
}

func (s *APIServer) Restart() *APIServer {
	s.Mark(Restarting)
	return s.Close().Start()
}

func (s *APIServer) Mark(status APIServerStatus) *APIServer {
	s.Status <- status
	return s
}

func (s *APIServer) Close() *APIServer {
	s.Mark(Terminating)
	err := s.Cmd.Process.Kill()
	chk(err)
	return s
}

func (s *APIServer) Pull() *APIServer {
	s.Mark(Pulling)
	cmd := exec.Command("git", "-C", s.Repo, "pull", "--depth=1")
	err := cmd.Run()
	chk(err)
	return s
}

func (s *APIServer) Clone() *APIServer {
	s.Mark(Cloning)
	cmd := exec.Command("git", "clone", "--depth=1", s.RepoURI, s.Repo)
	err := cmd.Run()
	chk(err)
	return s
}

func (s *APIServer) InstallPackages() *APIServer {
	s.Mark(InstallingPkgs)
	cmd := exec.Command("npm", "i", "--prefix", s.Repo)
	err := cmd.Run()
	chk(err)
	return s
}

func (s *APIServer) Update() *APIServer {
	s.Mark(Updating)
	return s.Pull().InstallPackages()
}

// make requests to the apiserver
func (s *APIServer) Req(routename string, ps ...Params) string {
	url := s.Routes[routename]
	if len(ps) > 0 {
		url = fmt.Sprintf("%s?%s", url, ps[0].Assemble())
	}
	resp, err := http.Get(url)
	chk(err)
	defer resp.Body.Close()
	body, err := ioutil.ReadAll(resp.Body)
	chk(err)
	return string(body)
}

type Params interface {
	Assemble() string
}

type QueryMap map[string]string

func (qm QueryMap) Assemble() string {
	var p []string
	for k, v := range qm {
		p = append(p, fmt.Sprintf("%s=%s", k, v))
	}
	return strings.Join(p, "&")
}

// type RouteMap map[string]string

var routemap = map[string]string{
	"login":        "/login/cellphone",
	"loginEmail":   "/login",
	"refreshLogin": "/login/refresh",
	"loginStatus":  "/login/status",
	"user":         "/user/detail", // params: userid
	"subcount":     "/user/subcount",
	"playlist":     "/playlist",
	"radio":        "/user/dj", // params: userid
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
	"searchSug":       "/search/suggest",     //keywords, alt->[type='mobile']
	"subPlaylist":     "/playlist/subscribe", // playlist id, type: 1:2
	"altPlaylist":     "/playlist/tracks",    // op: add | del, pid, songid
	"lyric":           "/lyric",              // songid
	"comments":        "/comment/music",      // songid, limit=20, offset, before( >5000)
	"songDetail":      "/song/detail",        // ids: songids[232,123,23]
	"dailyPlaylists":  "/recommend/resource",
	"dailySongs":      "/recommend/songs",
	"fm":              "/personal_fm",
	"dailyAttendance": "/daily_signin",
	"like":            "/like",       // songid
	"fmTrash":         "/fm_trash",   // songid
	"scrobble":        "/scrobble",   // songid, playlistid
	"cloud":           "/user/cloud", // limit:20, offset=0
}
