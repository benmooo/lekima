package main

// defaults
const (
	appName = "lekima"
	version = "1.0.0"

	settingsPath = "~/.lekima/settings.json"
	logPath      = "~/.lekima/errlog.log"
	volume       = 0.99
)

// Config : basic configuration of the app
// type config struct {
// 	// path of config cache
// 	configDir string
// 	logDir    string
// 	logPath   string

// 	// schema
// 	colorSchema string
// 	// show help widget or not
// 	// helpVisible bool

// 	// timeof interval of app update
// 	// updateInterval time.Day

// 	// app mode
// 	miniMode bool

// 	// show desktop lyrics or not
// 	// desktopLyric bool
// 	// volume
// 	volume int
// }

// Lekima : the app instance
type lekima struct {
	*user
	ui
}

func newLekima() *lekima {
	return &lekima{
		cfgPath: *newConfigPath(SettingsPath, LogPath),
		user: newUser()
	}
}
