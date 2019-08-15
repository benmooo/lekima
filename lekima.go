package main

import (
	"log"
	"os"

	c "github.com/benmooo/lekima/components"
)

const (
	appName = "lekima"
	version = "1.0.0"
)

const (
	configDir = "configDir"
	logDir    = "logDir"
	logPath   = "logPath"

	stderrLogger = log.New(os.Stderr, "", 0)

	// graphHirizaontaScale = 3
	helpVisible = false

	colorScheme    = "default"
	updateInterval = "everyweek"
	miniMode       = false
	aver
)

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
	updateInterval time.Day

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
type ui struct {
	header   *c.Header
	sidebar  *c.Sidebar
	mainPage *c.MainPage
	// breadCrumb *c.BreadCrumb
	searchBox *c.SearchBox
	footer    *c.Footer
}

// Lekima : the app instance
type Lekima struct {
	// version string
	config
	user
	logger log.Logger
	ui
}
