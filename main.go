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
	l := NewLekima()
	// close ui
	defer l.UI.Close()
	l.Init()
	quit := make(chan bool) // quit the programe

	l.UI.Init().Prepare().RenderLayout()
	uiEvent := l.UI.PollEvents()
	// try to login
	l.CheckLoginStatus()
	if !l.Loggedin {
		l.Render(l.UI.Login.Username, l.UI.Login.Password)
		l.HandleLogin(uiEvent)
	}

	// event loop
  // time.Sleep(time.Second)
	go l.Loop(uiEvent, quit)

	l.ListenExit(quit)
}
