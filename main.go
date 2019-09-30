package main

import (
	"log"
)

func chk(e error) {
	if e != nil {
		log.Fatal(e)
	}
}

func main() {
	lekima := NewLekima()

	lekima.run()
}
