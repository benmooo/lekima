package main

import (
	"fmt"
	"log"
	"net/http"
	"strconv"
	"sync"
	"time"
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
	// uiReady := make(chan bool)  // ui initialized & 1st rendered
	// apiReady := make(chan bool) // ready2serve := make(chan bool, 4)
	quit := make(chan bool) // quit the programe

	// reqs
	var wg1, wg2 sync.WaitGroup
	wg1.Add(2)
	wg2.Add(1)

	// initialize & start api server
	go func() {
		lekima.
			Notify(ass).
			MarkNotify(ass, Initializing)
		lekima.Init().
			MarkNotify(ass, Starting).
			Start().
			MarkNotify(ass, Running).
			CloseNotifier(ass)
		// Ready(apiReady)
		ticker := time.NewTicker(time.Millisecond * 100)
		defer ticker.Stop()
		for {
			<-ticker.C
			_, err := http.Get(fmt.Sprintf("http://%s:%d", Domain, Port))
			if err == nil {
				break
			}
		}
		wg1.Done()
	}()

	// initialize UI
	go func() {
		lekima.UI.Init().Prepare().RenderLayout()
		wg1.Done()
		wg2.Done()
	}()

	// listen and handle api server status change
	go func() {
		wg2.Wait()
		for status := range ass {
			lekima.UI.Header.Text = fmt.Sprintf("%s status: %s", defaultHeaderText, status)
			lekima.UI.RenderLayout()
		}
	}()

	// event loop
	go func() {
		wg1.Wait()
		l := lekima
		// try to login
		if err := l.Login(); err != nil {
			// try to relogin
		}

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
						l.UI.ToggleFocus(MainContentTile)
					case "o", "<Enter>":
						n := l.UI.Sidebar.SelectedNode()
						if n.Nodes != nil {
							l.UI.Sidebar.ToggleExpand()
						} else {
							l.UI.ToggleFocus(MainContentTile)
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
					case "/":
						l.UI.ToggleFocus(SearchBoxTile)
						l.UI.ToggleSearchBox().ClearSearchText()
					case "?":
						l.UI.ToggleFocus(HelpTile)
						l.UI.ToggleHelp()
					case "<Resize>":
						l.UI.ResizeLayout()
					}
				case MainContentTile:
					switch e.ID {
					case "<Tab>":
						l.UI.ToggleFocus(SidebarTile)
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
					case "/":
						l.UI.ToggleFocus(SearchBoxTile)
						l.UI.ToggleSearchBox().ClearSearchText()
					case "?":
						l.UI.ToggleFocus(HelpTile)
						l.UI.ToggleHelp()
					case "<Resize>":
						l.UI.ResizeLayout()
					}
				case SearchBoxTile:
					switch e.ID {
					case "<Tab>", "<Space>":
						l.UI.AppendSearchText(" ")
					case "<Escape>":
						l.ToggleSearchBox()
						l.UI.ClearSearchText()
						l.UI.ToggleFocus(MainContentTile)
					case "<Enter>":
						p := l.FetchSearch(l.UI.SearchBox.Text)
						l.UI.SetMainContent(p)
						l.ToggleSearchBox()
						l.UI.ClearSearchText()
						l.UI.ToggleFocus(MainContentTile)
					case "<C-c>":
						l.Exit(quit)
					case "<Backspace>":
						l.UI.PopSearchText()
					case "<Resize>":
						l.UI.ResizeLayout()
						l.UI.ResizeSearchBox()
					default:
						l.UI.AppendSearchText(e.ID)

					}
				case HelpTile:
					switch e.ID {
					case "q", "<C-c>":
						l.Exit(quit)
					case "<Resize>":
						l.UI.ResizeLayout()
						l.UI.ResizeHelp()
					case "<Escape>":
						l.UI.ToggleHelp()
						l.UI.ToggleFocus(MainContentTile)
					}

				}

			}
			l.UI.RenderLayout()
		}

		// }
	}()

	lekima.ListenExit(quit)
}

func eventLoop() {
	// for {

	// }
}
