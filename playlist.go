package main

import "container/list"

type Playlist struct {
	*list.List
	ID   int
	Name string
	// Author
	Focus *list.Element
}

func NewPlaylist(id int, name string) *Playlist {
	l := list.New()
	return &Playlist{
		List: list.New(),
		ID:   id,
		Name: name,
	}
}

// type Playlists
