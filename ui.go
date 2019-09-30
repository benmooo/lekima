// ui components
package main

import (
	ui "github.com/gizak/termui/v3"
	"github.com/gizak/termui/v3/widgets"
)

// logo which is a image or LEKIMA
// type Logo struct {
// }

type nodeValue string

func (n nodeValue) String() string {
	return string(n)
}

// the ui container which is a grid
func newUI() *ui.Grid {
	// the ui contaner
	var container = ui.NewGrid()

	// logo
	logo := widgets.NewParagraph()
	logo.Text = "#LEKIMA#"

	// user profile
	profile := widgets.NewParagraph()
	profile.Text = "user profile tbd......"

	// sider bar which is a node tree
	nodes := []*widgets.TreeNode{
		{
			Value: nodeValue("DP"),
			Nodes: nil,
		},
		{
			Value: nodeValue("FM"),
			Nodes: nil,
		},
		{
			Value: nodeValue("CLOUD"),
			Nodes: nil,
		},
		{
			Value: nodeValue("FAVORITES"),
			Nodes: []*widgets.TreeNode{
				{
					Value: nodeValue("💓"),
					Nodes: nil,
				},
				{
					Value: nodeValue("another"),
					Nodes: nil,
				},
			},
		},
		{
			Value: nodeValue("RECOMMAND"),
			Nodes: []*widgets.TreeNode{
				{
					Value: nodeValue("foo"),
					Nodes: nil,
				},
				{
					Value: nodeValue("bar"),
					Nodes: nil,
				},
			},
		},
	}
	sidebar := widgets.NewTree()
	sidebar.TextStyle = ui.NewStyle(ui.ColorYellow)
	sidebar.SetNodes(nodes)

	playlist := widgets.NewList()

	return container
}

func initUI() {
	// init ui
	err := ui.Init()
	chk(err)
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

	// showSearchBox := false
	modes := []string{"normal", "search"}
	currentMode := modes[0]

	searchBox := widgets.NewParagraph()
	searchBox.SetRect(width/3, height/4, 2*width/3, height/4+3)
	searchBox.Title = "search"
	searchBox.Text = ""
	ui.Render(grid)
	uiEvents := ui.PollEvents()

	for {
		switch currentMode {
		case "normal":
			select {
			case e := <-uiEvents:
				switch e.ID {
				case "q":
					return
				case "<Resize>":
					payload := e.Payload.(ui.Resize)
					width, height := payload.Width, payload.Height
					grid.SetRect(0, 0, width, height)
					searchBox.SetRect(width/3, height/4, 2*width/3, height/4+3)
					ui.Clear()
					ui.Render(grid)
				case "/":
					currentMode = "search"
					ui.Render(searchBox)
				}
			}
		case "search":
			select {
			case e := <-uiEvents:
				switch e.ID {
				case "<Resize>":
					payload := e.Payload.(ui.Resize)
					width, height := payload.Width, payload.Height
					grid.SetRect(0, 0, width, height)
					searchBox.SetRect(width/3, height/4, 2*width/3, height/4+3)
					ui.Clear()
					ui.Render(grid, searchBox)

				case "<Enter>", "<Escape>":
					currentMode = "normal"
					searchBox.Text = ""
					ui.Clear()
					ui.Render(grid)

				case "<Backspace>":
					l := len(searchBox.Text)
					if l > 0 {
						searchBox.Text = searchBox.Text[:l-1]
						ui.Render(searchBox)
					}
				case "<Space>":
					searchBox.Text += " "

				default:
					searchBox.Text += e.ID
					ui.Render(searchBox)
				}
			}
		}

	}
}
