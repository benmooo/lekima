package main

type configPath struct {
	settings string
	log      string
	// cache    string
}

func newConfigPath(s, l string) *configPath {
	return &configPath{s, l}
}
