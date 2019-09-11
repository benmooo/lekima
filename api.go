// powered by https://github.com/Binaryify/NeteaseCloudMusicApi

package main

const (
	domain = "127.0.0.1"
	port   = 3000
)

// const url = fmt.Sprintf("%s:%d", domain, port)

type Api struct {
	Route routeMap
	Proxy string
}

type routeMap map[string]string

func NewApi() Api {
	return Api{
		Route: newRouteMap(&routeMap{
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
		}),
	}
}

func newRouteMap(m *routeMap) routeMap {
	// for k, v := range m {
	// 	m[k] = fmt.Sprintf("%s:%d%s", domain, port, v)
	// }
	return *m
}
