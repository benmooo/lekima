// ui components
package main

import (
	"fmt"
	"strconv"

	ui "github.com/gizak/termui/v3"
	"github.com/gizak/termui/v3/widgets"
)

type UI struct {
	Layout *ui.Grid

	MarginTop, MarginLeft, HeaderRatio, BodyRatio, SiderRatio, MainRatio float64

	Header, SearchBox, Comments, Help *widgets.Paragraph

	Sidebar     *widgets.Tree
	MainContent *Table

	HeaderText, HelpDoc string

	// events bindings
	// InitialRender chan bool // first render indication

	Tiles []Tile
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

		HeaderText: defaultHeaderText,
		HelpDoc:    defaultHelpDoc,

		Sidebar:     widgets.NewTree(),
		MainContent: NewTable(),
		Tiles:       []Tile{SidebarTile, MainContentTile, SearchBoxTile, HelpTile, CommentsTile},
		Focus:       SidebarTile,
		// Ready:       make(chan bool),
	}
}

var (
	defaultHeaderText     = "LEKIMA,  hello [?],  press L to Login."
	defaultHelpDoc        = "help document."
	defaultPlaylistHeader = []string{"No", "Name", "Author", "Album", "Duration", "ID"}
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

func (u *UI) Clear() *UI {
	ui.Clear()
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
	u.Sidebar.TextStyle = ui.NewStyle(ui.ColorCyan)
	u.Sidebar.SetNodes([]*widgets.TreeNode{})
	// main content
	u.MainContent.Header = defaultPlaylistHeader
	// u.MainContent.ColWidths = []int{5, 5, 5, 5, 5}
	u.MainContent.ShowCursor = true
	u.MainContent.CursorColor = ui.ColorCyan
	u.MainContent.ColResizer = func() {
		u.MainContent.ColWidths = []int{
			10, 40, 30, 30, 20, 10,
		}
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

func (u *UI) LoadSidebarContent(c *SidebarContents) *UI {
	var nodes, top5, mylist []*widgets.TreeNode
	if c.FM != nil {
		nodes = append(nodes, &widgets.TreeNode{
			Value: c.FM,
			Nodes: nil,
		})
	}
	if c.Recommend != nil {
		nodes = append(nodes, &widgets.TreeNode{
			Value: c.Recommend,
			Nodes: nil,
		})
	}
	if c.Cloud != nil {
		nodes = append(nodes, &widgets.TreeNode{
			Value: c.Cloud,
			Nodes: nil,
		})
	}

	// top5 playlist
	for _, p := range c.Top {
		top5 = append(top5, &widgets.TreeNode{
			Value: p,
			Nodes: nil,
		})
	}
	// my playlist
	for _, p := range c.MyPlaylist {
		mylist = append(mylist, &widgets.TreeNode{
			Value: p,
			Nodes: nil,
		})
	}
	nodes = append(nodes,
		&widgets.TreeNode{
			Value: nodeValue("mylist"),
			Nodes: mylist,
		},
		&widgets.TreeNode{
			Value: nodeValue("top5"),
			Nodes: top5,
		},
	)
	u.Sidebar.SetNodes(nodes)
	return u
}

func (u *UI) SetMainContent(p *Playlist) *UI {
	var rows [][]string
	for i, t := range p.Tracks {
		seconds := int(1.0 * t.Duration / 1000)
		dt := fmt.Sprintf("%d:%02d", seconds/60, seconds%60)
		rows = append(rows, []string{
			strconv.Itoa(i + 1), t.Name, t.Artists[0].Name, t.Album.Name, dt, strconv.Itoa(t.ID),
		})
	}
	u.MainContent.Rows = rows
	return u
}

// type Ratio float64
type Tile byte

const (
	SidebarTile Tile = iota
	MainContentTile
	SearchBoxTile
	HelpTile
	CommentsTile
)

type nodeValue string

func (n nodeValue) String() string {
	return string(n)
}
