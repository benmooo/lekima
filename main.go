package main

import (
	"fmt"
	"log"
	"strconv"
)

func chk(e error) {
	if e != nil {
		log.Panic(e)
	}
}

func main() {
	lekima := NewLekima()
	// close api server and ui
	defer lekima.APIServer.Close()
	defer lekima.UI.Close()

	// channel data to communitcation
	ass := make(chan string, 4) // api server status
	uiReady := make(chan bool)  // ui initialized & 1st rendered
	apiReady := make(chan bool) // ready2serve := make(chan bool, 4)
	quit := make(chan bool)     // quit the programe

	// reqs

	// initialize & start api server
	go func() {
		lekima.
			Notify(ass).
			MarkNotify(ass, Initializing)
		lekima.Init().
			MarkNotify(ass, Starting).
			Start().
			MarkNotify(ass, Running).
			CloseNotifier(ass).
			Ready(apiReady)
	}()

	// listen 4 api server status change
	go func() {
		if <-uiReady {
			for status := range ass {
				lekima.UI.HeaderText = fmt.Sprintf("%s status: %s", defaultHeaderText, status)
				lekima.UI.RenderLayout()
			}
		}
	}()

	// dispatch req
	go func() {
		select {
		// case <-
		}
	}()

	// main event handler
	go func() {
		if <-apiReady {
			l := lekima
			// <-time.After(time.Second * 1)
			// try to login
			acc := l.ReadAccount()
			if acc.username != "" {
				if err := l.Login(acc); err != nil {
					// input to login
				}
			} else {
				// input to login && sync to local
			}

			// fetch playlists
			c := l.FetchSidebarContent()
			// render
			l.UI.LoadSidebarContent(c).RenderLayout()

			uiEvent := l.UI.PollEvents()
			// main event handler
			for {
				select {
				case e := <-uiEvent:
					// sidebar key events handler
					switch l.UI.Focus {
					case SidebarTile:
						switch e.ID {
						case "q", "<C-c>":
							l.Exit(quit)
						case "<Tab>":
							// l.UI.ToggleFocus()
						case "o", "<Enter>":
							n := l.UI.Sidebar.SelectedNode()
							if n.Nodes != nil {
								l.UI.Sidebar.ToggleExpand()
							} else {
								l.UI.ToggleFocus()
								p := n.Value.(*Playlist)
								if p.Tracks == nil {
									p = l.FetchPlaylistDetail(strconv.Itoa(p.ID))
								}
								l.UI.SetMainContent(p)
							}
						case "j":
							l.UI.Sidebar.ScrollDown()
						case "k":
							l.UI.Sidebar.ScrollUp()
						case "l":
							// l.UI.ScrollUp()
						}
					case MainContentTile:
						switch e.ID {
						case "<Tab>":
							l.UI.ToggleFocus()
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
							l.Player.TogglePlay()
						case "o", "<Enter>":
							index := l.UI.MainContent.SelectedRow
							songid := l.UI.MainContent.Rows[index][5]
							songurl := l.FetchSongURL(songid)
							l.Player.Play(songurl)
						case "m":
							l.Player.ToggleMute()
						case "=":
							l.Player.IncreaseVol()
						case "-":
							l.Player.DecreaseVol()

						}

					}

				}
				l.UI.RenderLayout()
			}

		}
	}()

	lekima.UI.Init().Prepare().RenderLayout().Ready(uiReady)
	lekima.ListenExit(quit)
}
