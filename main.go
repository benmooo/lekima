package main

import (
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
	defer ui.Close()

	pg := make([]*widgets.Paragraph, 6)
	for i := range pg {
		pg[i] = widgets.NewParagraph()
		switch i % 2 {
		case 0:
			pg[i].Border = false
		default:
			pg[i].Text = "lorem ispum ..."
			// pg[i].Title = fmt.Sprintf("para: %d", i+1)
		}
	}

	grid := ui.NewGrid()
	width, height := ui.TerminalDimensions()
	grid.SetRect(0, 0, width, height)
	grid.Set(
		ui.NewRow(1.0/8, pg[0]),
		// ui.NewRow(1.0/4,
		// 	ui.NewCol(1.0/2, pg[0]),
		// 	ui.NewCol(1.0/2, pg[1]),
		// ),
		ui.NewRow(6.0/8,
			ui.NewCol(1.0/12, pg[0]),
			ui.NewCol(3.0/12, pg[1]),
			ui.NewCol(7.0/12,
				ui.NewRow(0.8/4, pg[1]),
				ui.NewRow(3.2/4, pg[1]),
			),
			ui.NewCol(1.0/12, pg[0]),
		),
		ui.NewRow(1.0/8, pg[0]),
	)

	showSearchBox := false
	mode := "normal"

	p := widgets.NewParagraph()
	p.SetRect(0, 0, 20, 20)
	p.Title = "search"
	p.Text = "...."
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
				ui.Clear()
				ui.Render(grid)
			case "/":

				w, h := ui.TerminalDimensions()
				p.SetRect(w/3, h/4, 2*w/3, h/4+4)
				ui.Render(p)

			}
		}

	}

}
