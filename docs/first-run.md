# First run

```bash
lekima doctor           # audio device, session, provider, socket
lekima login --qr       # opens the web player in Chrome
lekima tui
```

`login --qr` launches Chrome, Chromium, Brave or Edge with a dedicated profile
under `<config>/web-login` and opens <https://music.163.com>. Sign in there as
usual; `lekima` reads the session cookie over the DevTools protocol, stores it
with `0600`, closes that browser window and returns. The profile is separate
from your normal browser data and is reused on later logins, so the first run
is the one that asks for a password or QR scan.

If no supported browser is installed, or you prefer the phone flow, add
`--no-browser` to print a QR code in the terminal:

```bash
lekima login --qr --no-browser
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
