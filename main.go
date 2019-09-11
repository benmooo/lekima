package main

import (
	"fmt"
	"log"

	ui "github.com/gizak/termui/v3"
	"github.com/gizak/termui/v3/widgets"
)

// func chk(e error) {
// 	if e != nil {
// 		log.Fatal(e)
// 	}
// }

func main() {
	err := ui.Init()
	if err != nil {
		log.Fatal(err)
	}

	pg := make([]*widgets.Paragraph, 6)
	for i := range pg {
		pg[i] = widgets.NewParagraph()
		pg[i].Text = "hello"
		pg[i].Title = fmt.Sprintf("para: %d", i+1)
	}

	grid := ui.NewGrid()
	width, height := ui.TerminalDimensions()
	grid.SetRect(0, 0, width, height)
	grid.Set(
		ui.NewRow(1.0/4,
			ui.NewCol(1.0/2, pg[0]),
			ui.NewCol(1.0/2, pg[1]),
		),
		ui.NewRow(1.0/2,
			ui.NewCol(1.0/2, pg[2]),
			ui.NewCol(1.0/2,
				ui.NewRow(1.0/3, pg[3]),
				ui.NewRow(2.0/3, pg[4]),
			),
		),
		ui.NewRow(1.0/4, pg[5]),
	)

	// p := widgets.NewParagraph()
	ui.Render(grid)
	uiEvents := ui.PollEvents()

	for {
		select {
		case e := <-uiEvents:
			switch e.ID {
			case "q":
				return
			case "<Resize>":
				payload := e.Payload.(ui.Resize)
				grid.SetRect(0, 0, payload.Width, payload.Height)
				// ui.Clear()
				ui.Render(grid)
			}
		}

	}

}
