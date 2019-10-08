package main

import (
	"fmt"
	"log"
)

func chk(e error) {
	if e != nil {
		log.Fatal(e)
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
				lekima.UI.Render(lekima.UI.Header)
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

			// try to log in
			acc := l.ReadAccount()
			if acc.username != "" {
				err := l.Login(acc)
				chk(err)
			}
			// fetch playlists
			l.FetchPlaylists()

			uiEvent := l.UI.PollEvents()
			// main event handler
			for {
				select {
				case e := <-uiEvent:
					// sidebar key events handler
					switch l.UI.Focus {
					case Sidebar:
						switch e.ID {
						case "<Tab>":
							l.UI.ToggleFocus()
						case "o", "<Enter>":
							// if l.UI.
							l.UI.ToggleFocus().SetMainContent(l.Playlist)
						case "j":
							l.UI.ScrollDown()
						case "k":
							l.UI.ScrollUp()
						case "k":
							l.UI.ScrollUp()
						}
					case MainContent:
						switch e.ID {
						case "<Tab>":
							l.UI.ToggleFocus()
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
