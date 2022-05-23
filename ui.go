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

	// indicator of searchbox
	Popup *widgets.Paragraph
	// login
	Login *LoginWidget
}

func NewUI() *UI {
	layoutUI := &UI{
		Layout:      ui.NewGrid(),
		MarginTop:   0.900 / 8,
		MarginLeft:  1.500 / 12,
		HeaderRatio: 1.400 / 9,
		BodyRatio:   7.800 / 9,
		SiderRatio:  1.000 / 5,
		MainRatio:   4.000 / 5,

		// Logo:        widgets.NewParagraph(),
		// UserProfile: widgets.NewParagraph(),
		Header:    widgets.NewParagraph(),
		SearchBox: widgets.NewParagraph(),
		Comments:  widgets.NewParagraph(),
		Help:      widgets.NewParagraph(),
		Popup:     widgets.NewParagraph(),

		HeaderText: defaultHeaderText,
		HelpDoc:    defaultHelpDoc,

		Sidebar: widgets.NewTree(),
		MainContent: NewTable(),
		Tiles:       []Tile{SidebarTile, MainContentTile, SearchBoxTile, HelpTile, CommentsTile},
		Focus:       SidebarTile,
		// Ready:       make(chan bool),
		Login: NewLoginWidget(),
	}

	ui.Theme.Default.Fg = ui.ColorClear
	clearStyle := ui.NewStyle(ui.ColorClear)

	// ui settings
	layoutUI.Header.BorderStyle = clearStyle
	layoutUI.Header.TextStyle = clearStyle

	layoutUI.SearchBox.BorderStyle = clearStyle
	layoutUI.SearchBox.TextStyle = clearStyle
	layoutUI.SearchBox.TitleStyle = clearStyle

	layoutUI.Help.BorderStyle = clearStyle
	layoutUI.Help.TextStyle = clearStyle
	layoutUI.Help.TitleStyle = clearStyle

	layoutUI.Sidebar.BorderStyle = clearStyle
	layoutUI.Sidebar.TextStyle = clearStyle
	layoutUI.Sidebar.SelectedRowStyle = clearStyle

	layoutUI.MainContent.BorderStyle = clearStyle
	layoutUI.MainContent.TitleStyle = clearStyle
	layoutUI.MainContent.ShowLocation = true
	layoutUI.MainContent.ShowCursor = true
	layoutUI.MainContent.CursorColor = ui.ColorCyan

	return layoutUI
}

var (
	defaultHeaderText = "LEKIMA, Username: %s\nPlayMode: %s, Volume: %f\nSong: %s | %s"
	defaultHelpDoc    = `    Key Map
	"j": scroll down
	"k": scroll up
	"o": toggle playlists|play a song
	"?": show help
	"/": search
	"P": toggle play mode
	"m": toggle mute
	"q" "Ctrl+c": quit the program
	"<Tab>": toggle Focus
	"<Escape>": terminate current operation
	"<Space>": toggle pause | play
	"<Enter>": toggle playlists|play a song|search ..etc.
	`
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
	// u.MainContent.ShowCursor = true
	// u.MainContent.CursorColor = ui.ColorCyan
	u.MainContent.ColResizer = func() {
		u.MainContent.ColWidths = []int{
			10, 40, 30, 30, 20, 10,
		}
	}

	// help
	u.Help.Title = "Help Doc"
	u.Help.Text = u.HelpDoc
	// search box
	u.SearchBox.Title = "Search"
	// u.ResizeSearchBox()
	u.Comments.Text = "no comments avaiable."
	// login
	u.Login.Username.Title = "Phone|Email"
	u.Login.Password.Title = "Password"
	u.ResizeLogin()
	return u
}

func (u *UI) ResizeLogin() *UI {
	w, h := u.Size()
	u.Login.Username.SetRect(3*w/8, h/3, 5*w/8, h/3+3)
	u.Login.Password.SetRect(3*w/8, h/3+3, 5*w/8, h/3+6)
	return u
}

func (u *UI) RefreshHeader(name string, mode PlayMode, vol float64, song string, status uint8) *UI {
	var m, s string
	switch mode {
	case Loop:
		m = "loop"
	case SingleCycle:
		m = "singlecycle"
	case Random:
		m = "random"
	}
	if status == 0 {
		s = "paused"
	} else {
		s = "playing"
	}
	u.Header.Text = fmt.Sprintf(defaultHeaderText, name, m, vol, song, s)
	return u
}

func (u *UI) ResizeLayout() *UI {
	w, h := u.Size()
	u.Layout.SetRect(0, 0, w, h)
	return u
}

func (u *UI) ToggleHelp() *UI {
	if u.Popup.Title == "Help Doc" {
		u.Popup = widgets.NewParagraph()
	} else {
		u.ResizeHelp()
		u.Popup = u.Help
	}
	return u
}
func (u *UI) ResizeHelp() *UI {
	w, h := u.Size()
	u.Help.SetRect(3*w/8, h/3, 5*w/8, h/3+17)
	return u
}

func (u *UI) ToggleSearchBox() *UI {
	if u.Popup.Title == "Search" {
		u.Popup = widgets.NewParagraph()
	} else {
		u.ResizeSearchBox()
		u.Popup = u.SearchBox
	}
	return u
}

func (u *UI) AppendSearchText(s string) *UI {
	u.SearchBox.Text += s
	return u
}

func (u *UI) PopSearchText() *UI {
	text := u.SearchBox.Text

	r := []rune(text)
	l := len(r)

	if l > 0 {
		u.SearchBox.Text = string(r[:len(r)-1])
	}

	return u
}

func (u *UI) ClearSearchText() *UI {
	u.SearchBox.Text = ""
	return u
}

func (u *UI) ResizeSearchBox() *UI {
	w, h := u.Size()
	u.SearchBox.SetRect(3*w/8, h/3, 5*w/8, h/3+3)
	return u
}

func (u *UI) ToggleFocus(t Tile) *UI {
	u.Focus = t
	// reset the style
	u.ResetBorderStyle()
	style := ui.NewStyle(ui.ColorCyan)
	switch t {
	case SidebarTile:
		u.Sidebar.BorderStyle = style
	case MainContentTile:
		u.MainContent.BorderStyle = style
	case HelpTile:
		u.Help.BorderStyle = style
	case SearchBoxTile:
		u.SearchBox.BorderStyle = style
	}
	return u
}

func (u *UI) ResetBorderStyle() *UI {
	u.Sidebar.BorderStyle = ui.StyleClear
	u.MainContent.BorderStyle = ui.StyleClear
	u.SearchBox.BorderStyle = ui.StyleClear
	u.Help.BorderStyle = ui.StyleClear
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
	u.Render(u.Layout, u.Popup)
	return u
}

func (u *UI) Ready(ch chan<- bool) {
	ch <- true
}

func (u *UI) PollEvents() <-chan ui.Event {
	return ui.PollEvents()
}

func (u *UI) LoadSidebarContent(c *SidebarContents) *UI {
	var nodes, mylist []*widgets.TreeNode
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

type LoginWidget struct {
	Username *widgets.Paragraph
	Password *widgets.Paragraph
	Focus    *widgets.Paragraph
}

func NewLoginWidget() *LoginWidget {
	l := &LoginWidget{
		Username: widgets.NewParagraph(),
		Password: widgets.NewParagraph(),
	}
	l.Focus = l.Username
	return l
}

func (l *LoginWidget) ToggleFocus() {
	if l.Focus == l.Username {
		l.Focus = l.Password
	} else {
		l.Focus = l.Username
	}
}

func (l *LoginWidget) AppendFocusText(s string) {
	l.Focus.Text += s
}

func (l *LoginWidget) Clear() {
	l.Username.Text = ""
	l.Password.Text = ""
}

func (l *LoginWidget) PopFocusText() {
	text := l.Focus.Text
	length := len(text)
	if length > 0 {
		l.Focus.Text = text[0 : length-1]
	}
}

type nodeValue string

func (n nodeValue) String() string {
	return string(n)
}
