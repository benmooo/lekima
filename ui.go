// ui components
package main

import (
	ui "github.com/gizak/termui/v3"
	"github.com/gizak/termui/v3/widgets"
)

type UI struct {
	Layout *ui.Grid

	MarginTop, MarginLeft, HeaderRatio, BodyRatio, SiderRatio, MainRatio float64

	Header, SearchBox, Comments, Help *widgets.Paragraph

	Sidebar     *widgets.Tree
	MainContent *widgets.Table

	HeaderText, HelpDoc string

	PlayListHeader []string

	// events bindings
	InitialRender chan bool // first render indication

	Tiles *[]Tile
	Focus Tile
	// Ready chan bool
}

func NewUI() *UI {
	return &UI{
		Layout:      ui.NewGrid(),
		MarginTop:   1.000 / 8,
		MarginLeft:  1.500 / 12,
		HeaderRatio: 1.000 / 9,
		BodyRatio:   8.000 / 9,
		SiderRatio:  1.000 / 5,
		MainRatio:   4.000 / 5,

		// Logo:        widgets.NewParagraph(),
		// UserProfile: widgets.NewParagraph(),
		Header:    widgets.NewParagraph(),
		SearchBox: widgets.NewParagraph(),
		Comments:  widgets.NewParagraph(),
		Help:      widgets.NewParagraph(),

		HeaderText:     defaultHeaderText,
		HelpDoc:        defaultHelpDoc,
		PlayListHeader: []string{"No", "Name", "Author", "Album", "Duration"},

		Sidebar:     widgets.NewTree(),
		MainContent: widgets.NewTable(),
		Tiles:       &[]Tile{Sidebar, MainContent, SearchBox, Help, Comments},
		Focus:       Sidebar,
		// Ready:       make(chan bool),
	}
}

const (
	defaultHeaderText = "LEKIMA,  hello [?],  press L to Login."
	defaultHelpDoc    = "help document."
)

func (u *UI) Init() *UI {
	err := ui.Init()
	chk(err)
	return u
}

func (u *UI) Close() *UI {
	ui.Close()
	return u
}

func (u *UI) Size() (int, int) {
	return ui.TerminalDimensions()
}

func (u *UI) Prepare() *UI {
	// margin block which is a paragraph of widgets with no border
	mb := widgets.NewParagraph()
	mb.Border = false
	// width, height of current terminal
	w, h := u.Size()
	u.Layout.SetRect(0, 0, w, h)
	u.Layout.Set(
		ui.NewRow(u.MarginTop, mb),
		ui.NewRow(1-2*u.MarginTop,
			ui.NewCol(u.MarginLeft, mb),
			ui.NewCol(1-2*u.MarginLeft,
				ui.NewRow(u.HeaderRatio, u.Header),
				ui.NewRow(u.BodyRatio,
					ui.NewCol(u.SiderRatio, u.Sidebar),
					ui.NewCol(u.MainRatio, u.MainContent),
				),
			),
			ui.NewCol(u.MarginLeft, mb),
		),
		ui.NewRow(u.MarginTop, mb),
	)
	//init paragraph text
	u.Header.Text = u.HeaderText
	// u.SideBar
	nodes := []*widgets.TreeNode{
		{
			Value: nodeValue("Top Playlise"),
			Nodes: []*widgets.TreeNode{},
		},
	}
	u.Sidebar.SetNodes(nodes)
	// main content
	u.MainContent.Rows = [][]string{
		u.PlayListHeader,
	}
	// help
	u.Help.Text = u.HelpDoc
	u.Comments.Text = "no comments avaiable."
	return u
}

func (u *UI) ToggleFocus() *UI {
	u.Focus ^= 1
	return u
}

func (u *UI) ChangeFocus(t Tile) *UI {
	u.Focus = t
	return u
}

func (u *UI) Render(us ...ui.Drawable) {
	ui.Render(us...)
}

func (u *UI) RenderLayout() *UI {
	u.Render(u.Layout)
	return u
}

func (u *UI) Ready(ch chan<- bool) {
	ch <- true
}

func (u *UI) PollEvents() <-chan ui.Event {
	return ui.PollEvents()
}

// type Ratio float64
type Tile byte

const (
	Sidebar Tile = iota
	MainContent
	SearchBox
	Help
	Comments
)

type nodeValue string

func (nv nodeValue) String() string {
	return string(nv)
}
