package main

import (
	browser "github.com/EDDYCJY/fake-useragent"
)

type Header map[string]string

func NewHeader() Header {
	return Header{
		"Range":          "bytes=0-",
		"Referer":        "https://music.163.com/",
		"Sec-Fetch-Mode": "cors",
		"User-Agent":     browser.Chrome(),
	}
}
