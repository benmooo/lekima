# First run

```bash
lekima doctor           # audio device, session, provider, socket
lekima login --qr       # scan with the NetEase Cloud Music app
lekima tui
```

If QR login is blocked by NetEase risk control (`8821`), import a browser
session instead:

1. Open <https://music.163.com> in Chrome and sign in.
2. DevTools → Network → refresh → any request → Request Headers → **Cookie**
   (`MUSIC_U` is HttpOnly, so `document.cookie` will not show it).
3. Copy the header value and pipe it in:

```bash
pbpaste | lekima login --cookie-file -
```

The session is stored with `0600`; `lekima logout` removes it.

## Keys

`j/k` move · `Enter` play · `Space` pause · `n/p` next/prev · `d` daily ·
`F` FM · `t` charts · `c` cloud · `f` like · `L` lyrics · `S` UI settings ·
`?` help · `q` quit. `Ctrl-o`/`Ctrl-i` navigate history.

## Configuration

`~/Library/Application Support/dev.lekima.lekima/config.toml` (macOS) or the
XDG config dir elsewhere:

```toml
[playback]
quality = "exhigh"
volume = 0.7

[ui]
theme = "cyber"
glyphs = "unicode"
density = "compact"
lyrics_translation = false
```
