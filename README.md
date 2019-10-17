<div align="center">

<center><h1>LEKIMA</h1></center>
<br><br>

Terminal based Netease Cloud Music player for vimer? | vimor?, named after the typhoon lekima.

<br><br>
<img src="./assets/screenshot.png" width="96%" />

</div>

## Prerequisites

### Api server

git, node, npm

### Sound
refer to [hajimehoshi/oto](https://github.com/hajimehoshi/oto)


## Installation

**Note**: Tested only on Linux witch alacritty. Beside, it takes seconds when first time to run the program since clone the [api](https://github.com/Binaryify/NeteaseCloudMusicApi) repo and install its dependencies.

### Source

Need go1.12.+ installed.
```bash
go get github.com/benmooo/lekima
```

### Binary
```bash
curl -sLO https://github.com/benmooo/lekima/blob/master/assets/lekima?raw=true
```


## Usage

### Keybinds

  - `q` or `<C-c>`: quit
  - `k`: up
  - `j`: down
  - `g`: jump to top
  - `G` : jump to bottom
  - `<Tab>`: toggle focus 
  - `<Space>`: toggle play | pause
  - `<Escape>`: go back or terminate current operation
  - `o`: toggle playlists tree | play a music
  - `<Enter>`: toggle playlists tree | play a music | search ..etc.
  - `-`: decrease volume
  - `=`: increase volume
  - `m`: toggle mute
  - `P`: toggle play mode
  - `?`: toggle help

## Many thanks2

- [Binaryify/NeteaseCloudMusicApi](https://github.com/Binaryify/NeteaseCloudMusicApi)
- [darknessomi/musicbox](https://github.com/darknessomi/musicbox)
- [gizak/termui](https://github.com/gizak/termui)
  - [nsf/termbox](https://github.com/nsf/termbox-go)
- [faiface/beep](https://github.com/faiface/beep)
  - [hajimehoshi/oto](https://github.com/hajimehoshi/oto)
- [cjbassi/gotop](https://github.com/cjbassi/gotop)
- [golang.org](https://golang.org)

