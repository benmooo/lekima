// Package ui implements main ui components for lekima
package ui

import (
	w "github.com/gizak/termui/v3/widgets"
)

// Logo ..
type Logo struct {
	*w.Paragraph
}

// NewLogo ..
func NewLogo() *Logo {
	return &Logo{
		w.NewParagraph(),
	}
}

// Avatar .
type Avatar struct {
	*w.Image
}

// func NewAvatar() *Avatar {
// 	return &Avatar{
// 		w.NewImage(),
// 	}
// }

// Header ..
type Header struct {
	Logo
	Avatar
}

// Footer ..
type Footer struct {
}

// Mainpage ..
type Mainpage struct {
}

// Sidebar..
type Sidebar struct {
}

// SearchBox ..
type SearchBox struct {
	w.Paragraph
}
