package main

import (
	"fmt"
	"log"
	"net/http"
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
		lekima.Init(ass).
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
		uiEvent := l.UI.PollEvents()
		// try to login
		if err := l.Login(l.ReadAccount()); err != nil {
			l.UI.Render(l.UI.Login.Username, l.UI.Login.Password)
			l.HandleLogin(uiEvent)
		}
		// header
		p := l.FetchUserDetail(l.User.ID)
		l.UI.Header.Text = fmt.Sprintf("Lekima, ID: %d, Username: %s!", p.UserID, p.Nickname)
		c := l.FetchSidebarContent()
		// render
		l.UI.LoadSidebarContent(c).RenderLayout()
		// main event handler
		l.EventLoop(uiEvent, quit)
	}()

	lekima.ListenExit(quit)
}
