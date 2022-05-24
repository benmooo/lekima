package main

import (
	"log"
)

func chk(e error) {
	if e != nil {
		log.Panic(e)
	}
}

func main() {
	lekima := NewLekima()
	// close ui
	defer lekima.UI.Close()
	lekima.Init()
	quit := make(chan bool) // quit the programe

	lekima.UI.Init().Prepare().RenderLayout()

	// event loop
	go func() {
		l := lekima
		uiEvent := l.UI.PollEvents()

		// try to login
		l.CheckLoginStatus()
		if !l.Loggedin {
			if err := l.Login(l.ReadAccount()); err != nil {
				l.UI.Render(l.UI.Login.Username, l.UI.Login.Password)
				l.HandleLogin(uiEvent)
			}
		}

		// header
		l.FetchUserDetail(l.User.ID)
		l.RefreshUIHeader()
		c := l.FetchSidebarContent()
		// render
		l.UI.LoadSidebarContent(c).RenderLayout()
		// main event handler
		l.EventLoop(uiEvent, quit)
	}()

	lekima.ListenExit(quit)
}
