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
	logo = widgets.NewParagraph()
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
					Value: nodeValue("lll💓"),
					Nodes: nil,
				},
			},
		},
		{
			Value: nodeValue("RECOMMAND"),
			Nodes: []*widgets.TreeNode{
				{
					Value: nodeValue("💓"),
					Nodes: nil,
				},
				{
					Value: nodeValue("💓"),
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
