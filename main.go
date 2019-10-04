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
	defer lekima.APIServer.Close()
	defer lekima.UI.Close()

	// init
	go func() {
		lekima.
			Init().
			Mark(Initializing)
	}()

	// start api server
	go func() {
		if <-lekima.Initiated {
			lekima.
				Start()
		}

		// update api server
		go func() {
		}()
	}()

	// ui
	go func() {
		lekima.UI.
			Init().
			Prepare().
			FirstRender()
	}()

	// action handler
	go func() {
		if <-lekima.UI.InitialRender {
			uiEvents := lekima.UI.PollEvents()
			for {
				select {
				// api status change
				case status := <-lekima.Status:
					fmt.Println(status)
				case e := <-uiEvents:
					fmt.Println(e.ID)

				}
			}
		}
	}()

}
