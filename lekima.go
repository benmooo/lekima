package main

import (
	"fmt"
	"log"
	"time"

	ui "github.com/gizak/termui/v3"
	w "github.com/gizak/termui/v3/widgets"
	// c "github.com/benmooo/lekima/components"
)

const (
	appName = "lekima"
	version = "1.0.0"
)

// const (
// 	configDir = "configDir"
// 	logDir    = "logDir"
// 	logPath   = "logPath"

// 	stderrLogger = log.New(os.Stderr, "", 0)

// 	// graphHirizaontaScale = 3
// 	helpVisible = false

// 	colorScheme    = "default"
// 	updateInterval = "everyweek"
// 	miniMode       = false
// 	aver
// )

// Config : basic configuration of the app
type config struct {
	// path of config cache
	configDir string
	logDir    string
	logPath   string

	// schema
	colorSchema string

	// show help widget or not
	helpVisible bool

	// timeof interval of app update
	// updateInterval time.Day

	// app mode
	miniMode bool

	// show desktop lyrics or not
	desktopLyric bool

	// volume
	volume int
}

type account struct {
	accountType string // phone or email
	phone       string
	email       string
	password    string
	vip         string
}

// users schema that lekima holds
type user struct {
	name  string
	level string
	account
}

// ui of the app
// type ui struct {
// 	header   *c.Header
// 	sidebar  *c.Sidebar
// 	mainPage *c.MainPage
// 	// breadCrumb *c.BreadCrumb
// 	searchBox *c.SearchBox
// 	footer    *c.Footer
// }

// Lekima : the app instance
type Lekima struct {
	// version string
	config
	user
	// logger log.Logger
}

func NewLekima() *Lekima {
	return &Lekima{
		config{
			configDir:   "configDir",
			logDir:      "configDir",
			logPath:     "configDir",
			colorSchema: "configDir",
			helpVisible: false,
			// updateInterval: "configDir",
			miniMode:     false,
			desktopLyric: false,
			volume:       5,
		},
		user{
			"unknown",
			"lv0",
			account{
				accountType: "phone",
				phone:       "1289328137",
				email:       "foo",
				password:    "jdhfajdh",
				vip:         "hejiao",
			},
		},
		// log.NewLogger(),
	}
}

// entry
func main() {
	if err := ui.Init(); err != nil {
		log.Fatalf("failed to init termui: %v", err)
	}
	defer ui.Close()

	// le := NewLekima()
	// p0 = c.NewLogo()
	l := w.NewList()
	l.Title = "list"
	l.Rows = []string{
		"[foo](fg:red)",
		"[bar](fg:red)",
		"[baz](fg:red)",
		"[jjjjjjjjj](fg:red)",
		"[0] [github.com/gizak/termui/v3](fg:red)",
		"[1] [你好，世界](fg:red)",
		"[2] [こんにちは世界](fg:red)",
		"[3] [color](fg:red)",
		"[4] [output.go](fg:red)",
		"[5] [random_out.go](fg:red)",
		"[6] [dashboard.go](fg:red)",
		"[7] [foo](fg:red)",
		"[8] [bar](fg:red)",
		"[9] [baz](fg:red)",
	}
	l.SetRect(0, 0, 50, 10)
	// p0.B= "hello world"

	ui.Render(l)

	events := ui.PollEvents()
	for {
		e := <-events
		switch e.ID {
		case "j":
			l.ScrollDown()
		case "k":
			l.ScrollUp()
		case "G":
			l.ScrollBottom()
		case "<C-c>":
			fmt.Println("program terminated.")
			time.Sleep(3000 * time.Millisecond)
			return
		}
		ui.Render(l)
	}

}
