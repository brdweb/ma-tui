# MA-TUI

A Linux terminal controller and local speaker for [Music Assistant](https://www.music-assistant.io/).
Browse music, manage queues, control network speakers, or play audio on the
computer running MA-TUI. Built with Rust + Ratatui and embedded **sendspin-rs**;
no companion player process is required.

- Browse and search library and provider content; favourite items, add provider
  items to the library, and choose where to play them.
- Control transport, volume, groups, sources and queues from the terminal, media
  keys or desktop bars.
- Start a dynamic radio playlist from a track, album, artist or playlist; add
  items to editable playlists or save the current queue as one.
- Follow podcasts and audiobooks: continue listening, unplayed episodes, resume
  points, and marking episodes played.
- See album art and a spectrum of MA-TUI's own output while it plays.
- Save credentials in the desktop keyring; follow live Omarchy theme changes.

MA-TUI targets **Music Assistant 2.10.2**. Its 1.0 scope is complete and stable,
but other server versions and audio devices may behave differently.
See [Known limitations](#known-limitations).

![MA-TUI showing the player, speaker list and music browser](docs/images/screenshot.png)

## Installation

Download the Flatpak bundle, Arch package or Linux x86-64 archive along with
`SHA256SUMS` from the [latest release](https://github.com/brdweb/ma-tui/releases/latest),
then verify them in the download directory:

```sh
sha256sum --ignore-missing -c SHA256SUMS
```

```sh
# Flatpak (any distribution)
flatpak install --user ./ma-tui-*.flatpak
flatpak run io.github.brdweb.MaTui

# Arch Linux / Omarchy
sudo pacman -U ./ma-tui-*.pkg.tar.zst
ma-tui
```

Checksums verify file integrity; packages are not signed. Each download bundles
installation instructions and third-party notices, and release notes are in
[CHANGELOG.md](CHANGELOG.md). The Flatpak keeps a separate profile from a native
install — see [Flatpak setup and permissions](packaging/flatpak/README.md). On
other distributions use the Flatpak, the x86-64 archive, or
[build from source](#build-and-local-install). Run MA-TUI as your normal desktop
user, not with `sudo`.

## Getting started

```sh
ma-tui                 # Opens connection setup when there is no saved login
ma-tui --setup         # Edit connection/login and local speaker settings
ma-tui --demo          # Offline preview; never connects or opens audio
```

Press **F2** for connection settings and **? / F1** for playback controls. Set the
server URL, speaker name and audio output, then select **Test connection and save
login**. The settings screen takes either a Music Assistant username/password or a
profile access token; Home Assistant/OAuth users create a profile token in Music
Assistant and paste it in the masked token field. Use the direct Music Assistant
base URL, not a Home Assistant dashboard or ingress link.

Passwords are never saved; the resulting token goes to the desktop Secret Service
keyring via `secret-tool` (`libsecret` on Arch). Unlock the keyring if saving
fails. `MA_TUI_TOKEN` works as a temporary environment override.

New setup enables **Expose this computer as a speaker** by default: MA-TUI
registers its persistent Sendspin identity and selects it when it appears.
Registration issues no play command, but Music Assistant can send audio to the
endpoint. Opening settings disconnects it until you return; quitting stops local
audio. `--remote-only` and `--local` override registration.

Local audio follows the desktop's ALSA/PipeWire routing; an explicitly selected
missing device fails visibly rather than falling back, and `--list-devices` lists
the options. Local means the computer running MA-TUI — over SSH, audio does not
follow you to the SSH client.

Local startup volume defaults to 50 percent; a saved `volume` setting overrides
it. Isolated ALSA underruns and a known transient timing-query error can recover
without reconnecting if output callbacks resume. Repeated errors, stalled
callbacks or other output stream
failures trigger up to three delayed recovery attempts, preserving the current
volume, mute and delay while discarding buffered audio. If recovery fails,
**F2**, then **Esc**, restarts the local speaker.
The optional `output_buffer_frames` setting requests 256–8192 output frames per
channel; omit it to keep the audio backend's default buffer size. Larger buffers
can help underruns at the cost of added latency.
For output or realtime-scheduling problems, see
[local audio troubleshooting](docs/audio-troubleshooting.md). RTKit is optional
host support, installed separately when needed; it is not bundled as a daemon.

Non-secret settings live at `$XDG_CONFIG_HOME/ma-tui/config.toml` (mode 0600);
`--config PATH` selects another file and `--init` creates one without overwriting
an existing file. Keep `player_id` unchanged to retain the same Music Assistant
speaker identity; `player_name` is what this computer advertises. `spectrum`
(`braille` or `blocks`) and `album_art` (`auto`, `sixel`, `blocks` or `off`) choose
how the player draws. `mpris = true` enables desktop player integration;
`notifications = true` enables track notifications. Configurations and keyring
entries from the former `local-matui`/`matui` names are still read while no current
one exists. Do not put tokens in TOML or Git; prefer HTTPS outside a trusted LAN.

## Playback and player controls

The player sits on top; the speakers and queue fill the left column and the music
browser, search results or a menu the right, so the queue stays visible while you
browse. The bottom two lines list the keys for the focused pane and the transport.

Select a speaker with Enter, then browse the **Music** pane. Home opens with
**Continue listening**, **Unplayed podcasts**, **Recently added**, **Recently
played**, favorite tracks, albums, artists, playlists and radio, then the
libraries and provider browsing. Enter opens a collection or folder; on a track it
offers **Play now (replace queue)**, **Play next**, or **Add to queue**, naming the
destination speaker. Its **Start radio** entry replaces the queue with a radio
playlist. Its Library section adds or removes favourites, adds a provider item to
the library, and opens **Add to playlist…** for editable playlists. Press **P** on
an album, playlist or podcast to choose playback for the whole collection, or to
mark an episode or audiobook played. Browsing does not start playback.

| Key | Action |
| --- | --- |
| Tab / Shift-Tab | Cycle players, queue, music and search panes |
| Up/Down or j/k | Move highlighted row |
| Enter in players | Select a player without starting playback |
| Space or p | Play/pause selected player's MA queue |
| < / > | Previous / next queue item (n also moves to the next) |
| s / m | Stop / mute selected player |
| z / l | Toggle shuffle / cycle repeat off→all→one |
| + / - | Volume up/down (the server chooses the step) |
| Left / Right | Seek backward/forward 10 seconds |
| / | Search; Enter submits, Esc cancels |
| b / F3 | Open music browser |
| Enter in music/search | Open collection or choose playback for an item |
| P in music/search | Choose playback for the whole item, or mark it played |
| f in music/search | Toggle favourite on the highlighted item |
| F | Toggle favourite on the now-playing item |
| a / N in music/search | Add to queue / play next |
| Backspace in music | Go back, restoring the previous selection |
| [ / ] in music | Previous / next library page (100 items per page) |
| o in music | Cycle the library sort |
| Ctrl-F in music | Filter the current library listing; Enter applies, Esc cancels |
| F4 | Focus the queue pane |
| Esc | Leave search, go back in the browser, or close a menu |
| Enter in queue | Play highlighted existing queue item |
| Delete in queue | Remove highlighted item |
| Shift-J / Shift-K in queue | Move item down/up |
| ? / F1 | Open playback/player controls |
| F2 | Connection settings (temporarily disconnects local speaker) |
| r | Reload the music listing, or refresh player/queue state |
| q / Ctrl-C | Quit and restore terminal |

The controls menu covers transport and external sources, mute, power,
individual/group volume, absolute seek, sleep timers, grouping, sound modes,
writable player options, queue shuffle/repeat, autoplay/crossfade,
play/remove/reorder/clear, **Save queue as playlist…**, playback transfer,
audiobook/podcast speed, and media URIs. Entries are grouped under headings and
**/** filters them.

Search covers tracks, albums, artists, playlists, radio, audiobooks and podcasts
(up to 50 results per type). Album and playlist results open their tracks; artists
open albums and a top-tracks folder. Provider folders can expose music outside
your saved library.

Controls target the selected player. Group queue ownership is resolved separately
from player volume, and queue edits are rejected if grouping changed the queue
identity that was displayed. Failed mutations are never replayed, and commands
delayed more than three seconds are dropped.

### Spectrum and album art

The spectrum is part of the player whenever local audio is on and the terminal is
tall enough. It is drawn in braille; set `spectrum = "blocks"` for a font without
braille coverage. The bars analyze the audio MA-TUI itself is playing, aligned to
when each sample is scheduled to be emitted — device buffering and acoustic
latency are not measured. A remote speaker's audio never passes through this
computer, so the strip names the playing speaker instead of inventing a display.

Album art sits beside it. `album_art = "auto"` asks the terminal whether it draws
sixel and falls back to colour half blocks; `sixel` and `blocks` settle it
outright. Run `ma-tui --check-art` to see what your terminal reports.

### Desktop integration

While connected, except in `--demo`, MA-TUI registers
`org.mpris.MediaPlayer2.ma_tui` on the session D-Bus. Media keys, `playerctl` and
the Waybar/Omarchy bar can show the title, artists, album, cover URL, length,
position, status, volume, shuffle and repeat. They can play, pause, stop, move
between queue items, seek, and change volume, shuffle or repeat on the selected
player. `mpris = true` is the default; set it to `false` to disable this
integration. If another MA-TUI instance already owns the name, this one continues
without MPRIS and shows a notice.

`notifications = false` is the default. Set it to `true` to show a desktop
notification for each new track while playing, replacing the previous one.

## Themes

MA-TUI rereads Omarchy's `colors.toml` every 500 ms, supporting both the current
`~/.local/state/omarchy/current/theme/` layout and the older
`~/.config/omarchy/current/theme/` one, and keeps the last valid palette while a
theme directory is replaced. No Omarchy files are changed. Without a palette it
uses terminal colors; `NO_COLOR` disables color entirely.

## Build and local install

Use stable Rust, a C compiler, pkg-config and ALSA headers — `base-devel pkgconf
alsa-lib` on Arch, `build-essential pkg-config libasound2-dev` on Debian/Ubuntu.
Login persistence also needs `secret-tool` (`libsecret` / `libsecret-tools`) and a
running Secret Service.

```sh
git clone https://github.com/brdweb/ma-tui.git
cd ma-tui
cargo build --release --locked
install -Dm755 target/release/ma-tui ~/.local/bin/ma-tui
install -Dm644 packaging/ma-tui.desktop ~/.local/share/applications/ma-tui.desktop
ma-tui --demo
```

Ensure `~/.local/bin` is on your desktop PATH. Minimum terminal size is 50×16;
110×30 is recommended. Source builds may include changes not present in the latest
release. Package build instructions are in
[packaging/arch/README.md](packaging/arch/README.md) and
[packaging/flatpak/README.md](packaging/flatpak/README.md).

Checks for a change:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
uv run --with pyte python tests/terminal_smoke.py target/release/ma-tui
uv run --with pyte python tests/connected_smoke.py target/release/ma-tui
uv run --with pyte python tests/settings_smoke.py target/release/ma-tui
```

Python/uv/pyte are test tools, not runtime dependencies. The suite runs against
local HTTP/WebSocket fixtures and null audio output; opt-in tests needing real
audio devices or an unlocked keyring, along with validation evidence, are in
[architecture notes](docs/architecture.md), and packaging gates in
[release checks](docs/releasing.md).

## Known limitations

Validation has covered login, browsing, podcast progress, the event stream, local
speaker registration and local/remote playback against Music Assistant 2.10.2 —
not every device or server version, and not acoustic latency or multi-room
synchronization.

State arrives on Music Assistant's event stream, with polling as a fallback; very
large queues still cost extra requests when their contents change, and the
unplayed-podcast list costs one request per subscribed show because the server has
no filter for it. Output-device changes restart the connection, and runtime
volume/mute/delay changes are not persisted across restarts.

Album art needs a terminal that draws sixel to look sharp — foot does, Alacritty
has no image protocol at all — and a multiplexer will generally not forward
either.

MA-TUI does not administer users, providers, DSP or the MA server. Audiobooks
have no chapter navigation, because Music Assistant 2.10.2 has no chapter model.
Player support varies; server rejections appear as command errors. Playlist
additions finish in the background on the server, and radio uses Music
Assistant's dynamic radio playlists. Prolonged playback, broader hardware and
codec coverage, restart recovery and multi-room synchronization need further
testing.

## Contributing

Bug reports and focused pull requests are welcome. Include the MA-TUI and Music
Assistant versions, Linux distribution, installation method and steps to
reproduce. Do not include tokens, passwords, private server addresses or personal
media data, and run the checks above before opening a pull request.

## License

MA-TUI is licensed under the [MIT License](LICENSE). Third-party dependencies
retain their own licenses; packaged distributions include their notices.
