# MA-TUI architecture

## Accepted scope

Rust + Ratatui Linux TUI; Music Assistant 2.10.2 is the initial integration
target. Local playback is included from the first implementation, not a later
phase. Embed `sendspin = "=0.3.7"`; do not introduce a companion player process.
Commit Cargo.lock when changes are reviewed. MA-TUI is MIT licensed; third-party
dependencies retain their own licenses. Release gates are in `docs/releasing.md`.

## Boundaries

- TUI: rendering, focus, text entry, user intent. Never wait for network requests
  on the rendering path. Distinguish offline fixtures from live server data.
- MA API: authenticated HTTP commands, player selection, active queue resolution,
  track search, transport/volume/seek, queue insertion and play requests.
- Local audio: authenticated WebSocket connection followed by Sendspin, decoding,
  synchronized CPAL output, server volume/mute, stream lifecycle, reconnection.
- Configuration: non-secret TOML, stable local player identity and device ID.
  Token stored in Secret Service or supplied using MA_TUI_TOKEN, never a command-line argument.

Selecting a remote player does not move playback or start local audio. Enabling
local audio registers an endpoint but does not issue a play command; MA may send
audio to that endpoint independently, including when a prior queue resumes.
No automatic fallback from missing explicit headphones/DAC to system speakers.

The pinned CPAL ALSA backend prepares its output after reporting an XRUN. An
isolated exact XRUN diagnostic therefore keeps the stream connected while
MA-TUI waits for a subsequent callback. The exact pinned ALSA
`snd_pcm_avail_delay` EIO diagnostic also gets a bounded grace period: the
Pulse ALSA adapter can report it while timing information is unavailable. Each
new report refreshes the callback baseline, never the first-error deadline;
only a later poll without a new error and with an advanced callback count proves
recovery. Three distinct timing-error episodes within ten seconds trigger
rebuilding. All other errors remain failures. Three
observed XRUNs within ten seconds, absent startup callbacks, or callbacks that
stop advancing escalate to output recovery. The watchdog allows one second
between callbacks and twice that for startup, extending those bounds for large
requested or observed callback periods. Atomic callback telemetry also reports
the backend, format, frame bounds, largest gap and observed/recovered XRUN and
timing-error episode counts in the
ready status. It measures output callback progress, not acoustic output.

Local device-stream failures that require rebuilding end the current transport
and worker, then retry up to three times after 250 ms, 500 ms and one second.
Each attempt recreates the
same configured output on a new worker, rereads its supported formats and
renegotiates Sendspin with fresh channels. Volume, mute and static delay survive
recovery; decoded samples and decoder state do not. The retry budget resets after
30 seconds of initialized worker operation. Decoder errors, queue-bound failures
and worker panics remain fatal. Shutdown interrupts retry delays, and an output
whose worker cannot finish within two seconds is not reopened concurrently.

## Versioned integration references

- MA HTTP handler: https://github.com/music-assistant/server/tree/2.10.2/music_assistant/controllers/webserver
- MA Sendspin authentication: https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/webserver/sendspin_proxy.py
- Sendspin 0.3.7: https://github.com/Sendspin/sendspin-rs/tree/v0.3.7
- Audio output: https://github.com/Sendspin/sendspin-rs/blob/v0.3.7/src/audio/synced_player.rs
- Desktop integration reference (not a dependency): https://github.com/music-assistant/desktop-app/blob/main/src-tauri/src/sendspin/mod.rs

MA proxy authentication is separate from the ordinary Sendspin handshake: send
an auth message with token and client_id, require auth_ok, then hand the socket
to ProtocolClientBuilder. Do not copy the upstream player example wholesale:
the application must manage decoder replacement, volume/mute, format validation,
cancellation, audio-thread lifetime and reconnects.

The MA 2.10.2 HTTP `/api` response is the bare JSON command result, not the
WebSocket response envelope. Resolve a selected player's active queue before
queue commands; volume commands still target the player itself. Submit explicit
`replace` or `add` options for play-media requests rather than relying on defaults.

## Event stream (2026-09-16)

Commands stay on HTTP; state changes arrive on the `/ws` socket instead of being
polled for. The socket carries the same command envelope as `/api`. The server
greets a new connection with its own information before answering anything, so
the authentication result has to be matched by `message_id` rather than assumed
to be the first message. The first command must be `auth`; once it succeeds the
server subscribes the connection itself, so there is no subscribe command to
send. Events then arrive unprompted as `{"event", "object_id", "data"}`.

Only the event name and `object_id` are trusted for routing. The single payload
read is `queue_time_updated`, whose `data` is the elapsed seconds itself
(`player_queues/controller.py` signals `data=queue.elapsed_time`), so the
position costs no request at all. `queue_updated` and `queue_items_updated` also
carry a whole queue object, but it is deliberately ignored: the HTTP reads stay
the only place a player or queue is parsed, so there is one shape to keep
correct rather than two. Events for a queue that is not on screen are dropped,
and a burst is coalesced into one read.

Polling is retained as the fallback, at two seconds with no stream and thirty
with one, which also covers a socket that dies without closing. Authentication
failures back off rather than reconnecting in a loop and never report the stream
as online. Stream failures are not surfaced as errors: they carry peer input,
and polling covers the outage.

Sources: `controllers/webserver/websocket_client.py` and
`controllers/player_queues/controller.py` at server tag 2.10.2, and the official
client's `connect()` ordering in `music_assistant_client/client.py`.

Verified against local WebSocket fixtures only. Fixture success does not
establish live compatibility; the handshake, event names and payload shapes need
confirming against an actual 2.10.2 server with explicit authorization.

Sendspin 0.3.7's protocol router has unbounded internal receivers and SyncedPlayer
callbacks use locks. Application-level channel bounds do not fix those upstream
properties. Split inbound receivers lack ordering IDs, so drain/discard pending
audio on stream boundaries rather than decoding stale bytes in a new format.
Do not enable upstream payload tracing where it might disclose protocol data.

Check audio chunk continuity in server timestamp/sample coordinates, not local
Instants converted under changing clock-sync estimates. Local deadline order can
change after synchronization corrections; occupancy is an enqueue-time estimate,
not measured device consumption. Keep those concerns separate.

Apply delay changes based on the real player's configured delay, not the budget
cache: a stream begin or stale-clock reset can reset accounting independently of
the output. Test nonzero-to-zero resets before the first buffer as well as after
clock invalidation on an actual null-output player.

Music Assistant 2.10.2 pins aiosendspin 9.1.1. Its player buffer tracker accounts
encoded bytes and permits a 30-second duration horizon. MA-TUI advertises 2 MiB
of encoded capacity, so a two-second/2 MiB decoded limit is incompatible: 48 kHz
stereo PCM16 can legitimately fill almost 11 seconds and expands to i32 samples.
The decoded queue therefore permits 32 MiB, 4096 chunks and a 35-second scheduling
horizon (30 seconds plus timing margin). This covers the largest advertised
96 kHz stereo format. Per-chunk size/duration and overlap checks remain bounded;
the protocol-to-worker handoff accepts 1024 items while reserving at most 2 MiB
of encoded audio. These are software buffer bounds, not a claim of measured
device latency or global memory bounds.

The local playback regression reproduced rejection at PCM chunk 76 with a
500 ms initial lead, before the advertised encoded capacity was reached. Tests
cover a full PCM buffer, the compressed-audio duration horizon, memory/count
bounds and deadline reordering. A synchronized real CPAL null-output test feeds
the full PCM buffer without failure; a separate default-output fixture opens a
silent stream without sending audio frames. Worker shutdown preserves fixed,
sanitized failure details instead of overwriting them with "Audio worker stopped".

Sources: the versioned server's
`music_assistant/providers/sendspin/manifest.json` and the official aiosendspin
9.1.1 distribution's `server/audio.py` and `server/roles/player/v1.py`.

Live testing showed Music Assistant wrapping this Sendspin endpoint in a universal
player. Automatic selection matches the persistent endpoint against
`output_protocols[].output_protocol_id` as well as a direct player ID, then uses
the public wrapper ID for player/queue controls. Display names are not identity
matches, and an existing user selection is preserved. Source:
https://github.com/music-assistant/server/blob/2.10.2/music_assistant/providers/universal_player/player.py

With explicit user authorization, the installed build resumed only the existing
ungrouped local queue, remained available/playing throughout two brief tests,
and was paused afterward. The final run produced a non-silent signal measured
from MA-TUI's own PipeWire/PulseAudio sink-input monitor on the configured desktop
output. Samples stayed in memory and were discarded; no media recording was
saved. Remote player state/control snapshots were unchanged. A restart also
verified automatic selection of the universal wrapper. Acoustic latency and
multi-room synchronization were not measured.

## Controls and layout (2026-09-08)

The content area is two columns: players above the queue on the left, browser or
search on the right, so adding music never hides the queue. Chrome is eleven
rows — two header, four now playing, three status, two hints — and the now
playing block carries transport state, volume, mute and the queue's
shuffle/repeat rather than leaving them to other panes. The hint lines are the
focused pane's keys plus a fixed transport line kept under 102 columns.

`p` is play/pause, matching other players, with previous/next on `<`/`>` (`,`
and `.` as unshifted aliases) and `n` retained for next. Following the
terminal-player convention, `s` stops, `z` toggles shuffle and `l` cycles
repeat, so no binding is the shifted form of an unrelated one. Queue-mode keys carry the displayed queue ID and
therefore keep `check_queue`'s ownership comparison; a dynamic queue reports no
shuffle or repeat and the key says so instead of guessing. Esc closes the
visualizer, leaves search for the browser, or steps back in the browser.

Volume keys send `players/cmd/volume_up`/`volume_down`, so the server owns the
step and rapid presses cannot race a read-modify-write; `Control::Volume` is
gone because `playback_command` already routes player commands. Seeking resolves
an absolute position from the displayed elapsed time and updates it optimistically,
so holding the key accumulates locally instead of issuing a queue request per
keystroke; the next two-second poll corrects it.

Controls menu entries carry a heading and are sorted into heading order with a
stable sort, preserving order within each group. `/` filters by label or
heading; the cursor indexes the filtered entries, headings are unselectable
rows, and Enter on a filter matching nothing keeps the menu open.

## Visualizer (2026-09-08)

The spectrum display analyses only MA-TUI's own output. `DeviceOutput::write`
hands each accepted buffer to an `audio::SampleSink` — a trait declared in the
audio module so it does not depend on presentation code — tagged with the
instant the player is scheduled to emit it, which is
`server_to_local_instant(timestamp)` minus the configured static delay, the same
instant `QueueBudget` measures. Sendspin emits each sample `static_delay_ms`
early to compensate downstream latency; the two must stay in step. That instant
is a schedule, not a measurement of device buffering or acoustic output, and
nothing here claims otherwise. Fixture outputs have no sink, so a remote speaker
or a disabled local endpoint produces no frames at all.

Analysis runs on the audio worker thread — the thread that already decodes and
allocates, never the CPAL callback — so retained state is bounded no matter how
far ahead the server streams: a 2048-point window every 21 ms of audio at any sample
rate, kept as 64 single-byte bands with their deadline, capped at 1600 frames
(about 34 seconds, covering the decoded queue's 35-second horizon). The
interface copies one frame under a short lock and renders afterwards; smoothing
and peak fall are presentation state kept out of the captured bands. Mute is
honoured because muted output is silent, while the ramped per-player volume
curve is applied downstream inside Sendspin and is not reproduced.

Frequency mapping is logarithmic over 40 Hz to 16 kHz with a 3 dB/octave
presentation tilt above 200 Hz and a 66 dB displayed range. A real ALSA null
output test asserts every accepted buffer reaches the sink with emission
instants advancing with the audio; analysis, timing, bounds, rendering and the
empty-state explanations are covered without hardware.

## Podcasts and audiobooks (2026-09-16)

Music Assistant already keeps the listening state; the client only has to read
and show it. Library browsing uses the same `music/<type>s/library_items`
command as every other type, because MA derives that base from the media type
(`api_base = f"{media_type}s"`), so podcasts and audiobooks need no special
casing. `music/in_progress_items` and `music/recently_added_tracks` are the
"continue listening" and "latest" shelves: the server maintains both, they take
a limit and no offset, and the interface only renders them.

`PodcastEpisode` and `Audiobook` carry `fully_played` and `resume_position_ms`,
hydrated per user from the playlog table and falling back to the provider's own
state. Both are null when the provider does not report progress, which is not
the same as "not played": nothing is shown then rather than claiming unplayed.

A podcast opens into `music/podcasts/podcast_episodes`. An audiobook does not
open into anything: 2.10.2 has no chapter model at all, only
`audiobook_versions`, so an audiobook is one playable item with a resume point.
Do not build a chapter listing against this server version.

There is no server-side filter for unplayed episodes: `library_items` takes
`played_only`, which selects the opposite, and has no unplayed equivalent. The
unplayed list is therefore assembled here — the shows are listed, then each is
asked for its episodes and filtered — which is one request per subscription.
They overlap a few at a time rather than running in a loop, because the server
answering them is the one also serving the audio, and the result is capped so a
large subscription list cannot become an unbounded read.

`music/mark_played` and `music/mark_unplayed` take the item itself rather than a
URI, so `Media` keeps `item_id` and `provider` and hands back the four fields
`ItemMapping` requires. Marking is a library edit and deliberately needs no
speaker, which is why the controls menu permits a player-free action and the
item menu opens without a selection. `playlog_updated` re-reads a listing that
would show progress, throttled, because a playing audiobook produces those
events continuously.

Sources: `controllers/music/controller.py`, `controllers/music/media/base.py`,
`media/podcasts.py` and `media/audiobooks.py` at server tag 2.10.2, and
`media_items/media_item.py` in music-assistant/models.

## Player-first layout (2026-09-16)

The focused pane is marked by filling its heading rather than enlarging it: a
cell grid has one type size, and a bar of colour is findable at a glance in a
way a colour change alone is not. Every heading is bold, so they read as
headings whether focused or not.

The interface has no boxes. A pane is a dim uppercase label and the space around
it, which reads quieter than a border and returns two columns and two rows per
pane to the lists. The spectrum is part of the player and nothing else: there is no mode and no
key for it. Both views it used to have showed the same thing with more of the
interface taken away. The strip costs five rows and appears only when this run has
local audio to analyse and the terminal is at least 26 rows, because the lists
matter more than the strip on a short screen. Its empty state is unchanged: a
flat baseline and the reason, never motion that means nothing.

Bars are lit and unlit half-block segments rather than a smooth eighth-block
ramp, one column wide at every width. A column therefore has as many steps as it
has rows, which is coarser than before; the display indicates level and the
frequency ruler already says it is approximate.

The now-playing title scrolls when it does not fit, holding at each end. The
step count is advanced by the render loop rather than read from a clock inside
`draw`, so drawing stays a function of state and a title that fits costs no
redraws at all. The queue is a table — number, title over artist, and a state
column naming the playing item or giving the item's length — and the transport
row shows all four controls with the current state filled, so it reads without
pressing anything.

## Album art (2026-09-16)

`/imageproxy/<proxy_id>?size=&fmt=` serves covers without credentials and
resizes server-side, so the client asks for a small image rather than fetching a
full cover to discard most of it. The served sizes are fixed by the server
(0, 80, 160, 256, 512, 1024) and `fmt=jpg` is requested so only one decoder is
needed: `zune-jpeg`, two crates, chosen over the `image` crate's twenty-four
because nothing here needs the rest of them. The `proxy_id` is generated during
serialization and appears on `MediaItemImage`, which a queue item may carry at
its media item's metadata or as a mapping; all the places it turns up are tried.
The id goes into a URL path, so it is validated as alphanumeric rather than
trusted.

Covers draw as sixel where the terminal will take it and as half blocks
everywhere else. `album_art` selects between them; `auto` asks the terminal, with
Primary Device Attributes, which lists 4 when sixel is supported. TERM does not
answer the question and guessing from it was wrong: foot draws sixel and is
commonly configured to report `xterm-256color`, which is also what a terminal
that cannot draw sixel reports. The reply is read once, before the interface
starts, so it cannot be mistaken for a keystroke. A multiplexer short-circuits
the question: tmux and zellij sit between this program and the terminal drawing
the pixels and mostly do not forward them. foot draws sixel; Alacritty has no
image protocol at all.

Sixel writes pixels the cell renderer knows nothing about, so it is emitted
after the cells are flushed, into a region the layout claims but leaves blank —
a blank region gives a later diff nothing to paint back over the image. It is
re-emitted only when the cover or its region changes, and on a resize, which
repaints everything. Those pixels also outlive the cells they sit in, so nothing is drawn over them:
the controls and playback menus take the browser's column rather than the
screen, which leaves the player, the speakers and the queue where they are.
Not covering the cover is worth more than repairing it afterwards, and it also
means what is playing stays visible while choosing what to play next. The screen
is still repainted in full when the cover appears, moves or goes, for the cases
the layout cannot avoid. Colours quantize to a fixed 6x6x6 cube: 216 colours is
enough for a cover and avoids deriving a palette per image.

Half blocks are the fallback: the upper half takes the foreground colour and
the lower half the background, so a cell carries two pixels and a panel `n`
columns wide is `n` pixels wide. That is coarse — a panel `n` columns wide is `n` pixels wide — but they are
ordinary styled cells, so they compose with the diffing renderer, survive a
resize and cost nothing to redraw.

A row says what its item belongs to, which is a different question per media
type: the artists and album for a track, the show for a podcast episode, the
authors and narrators for an audiobook. With none of those the media type is
said readably; a provider instance id is never shown, because it means nothing
to anyone reading it.

The cover is fetched when the playing item changes, not on every queue read, and
a superseded fetch is cancelled. It needs ten rows of its own: it must not
depend on the spectrum being present, and the four-row player leaves it too
small to recognise. `album_art = false` turns it off entirely.

Sources: `controllers/metadata/images.py` and `constants.py` at server tag
2.10.2, and `media_items/media_item.py` in music-assistant/models.

## Desktop integration (2026-09-23)

`src/mpris.rs` uses zbus 5 on the existing Tokio runtime to register
`org.mpris.MediaPlayer2.ma_tui` while connected, never in `--demo`. Each UI tick
derives an MPRIS snapshot from `App` and publishes it through a watch channel
only when it changed. D-Bus transport, seek, volume, shuffle and repeat commands
become existing `ui::Action`s and follow the same dispatch path as keys, so they
retain selected-player and queue checks. Position is estimated between snapshots;
a position jump emits `Seeked`. Artwork is the unauthenticated MA image-proxy URL,
not an authenticated image fetch.

The name is optional: a name already owned by another instance leaves the TUI
connected and reports a notice rather than replacing its owner. Notifications are
also optional and replace the prior notification for each newly playing track.

## Library edits, radio and playlists (2026-09-23)

The MA 2.10.2 favourites commands are `music/favorites/add_item` with
`{item: uri}` and `music/favorites/remove_item` with `{media_type,
library_item_id}`. Removal first resolves the library ID with
`music/item_by_uri`; adding an item to the library uses `music/library/add_item`.
`music/recently_played_items` supplies the Home shelf. Library listings use
`library_items` search and `order_by` arguments. `media_item_updated`,
`media_item_added` and `media_item_deleted` events update an already displayed
listing in place.

Radio uses `player_queues/play_media` with
`radio_playlist://playlist/<uri>` and `option: "replace"`. MA 2.10.2 deprecates
`radio_mode`; the dynamic radio playlist URI is the supported representation.
The editable-playlist picker reads `music/playlists/library_items` and keeps only
`is_editable` rows. It creates with `music/playlists/create_playlist`, then adds
the URI through `music/playlists/add_playlist_tracks`; that command returns a
background task, so completion is server-side. `player_queues/save_as_playlist`
saves the selected queue.

Sources: [music controller](https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/controller.py),
[music media base](https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/base.py),
[playlist media](https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/playlists.py),
[player queues](https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/player_queues/controller.py),
and [the client radio playlist URI](https://github.com/music-assistant/client/blob/main/music_assistant_client/player_queues.py).
Verification used local fixtures and the session bus only, not a live server.

## Engineering safeguards

- Treat successful command submission separately from confirmed player state.
  Refresh after commands; do not blindly retry mutations after a timeout.
- Keep the active queue's identity separate from player identity (grouping).
- Reject base URLs containing credentials, query strings or fragments; never
  include credentials, response bodies or raw authentication frames in errors.
- Test with local HTTP/WebSocket fixtures, not the user's live server.
- A hardware-free test or null sink does not demonstrate audible playback.
- Treat Linux output backends and device enumeration as a runtime capability;
  fail visibly when unavailable. Do not claim bit-perfect output or sync accuracy
  until measured on actual hardware.

## Laptop controller iteration (2026-09-08)

- Retain Rust/Ratatui and the exact Sendspin 0.3.7 pin. The laptop endpoint is
  embedded in MA-TUI, active while the application runs; no background service.
- The settings screen is limited to connection/login and the local endpoint.
  Built-in login uses POST `/auth/login` with `provider_id`, `credentials`, and
  `device_name`, then reads `token`. It uses the returned session token rather
  than creating another long-lived token on each settings save. Profile tokens
  support users whose login provider is Home Assistant/OAuth.
- Read `/info` without credentials, then authenticate `auth/me` and list players before saving. Preserve
  URL prefixes, disable redirects, bound network/keyring operations, and omit
  response bodies from errors. Never put a password/token in process arguments.
- `secret-tool` passes tokens via pipes into Secret Service, keyed by server URL
  and persistent player identity. MA_TUI_TOKEN overrides lookup for that run.
  Config updates are private, atomic file replacements. Settings/network work
  runs on runtime workers while the terminal remains responsive.
- Theme updates reopen the palette path every 500 ms. This laptop's installed
  `/usr/share/omarchy/bin/omarchy-theme-set` uses
  `$HOME/.local/state/omarchy/current/theme/colors.toml`; the older config path
  is a fallback. No packaged Omarchy files, theme hooks or desktop settings are
  edited. Transient missing/invalid palettes retain the last valid colors.
- Playback menus expose player transport separately from MA queue transport,
  group/source/sound-mode choices, runtime player options, sleep timers and
  queue operations. They exclude server/provider/user administration. Queue
  edits carry the displayed queue ID and compare it with fresh active-queue
  ownership before mutation. Numeric entry rejects NaN, infinity, fractions for
  integer controls and out-of-range values.
- Search aggregates playable media types, with 50 results per type. Player
  capabilities and server command failures remain visible.

Additional official integration sources inspected:

- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/webserver/controller.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/webserver/auth.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/players/controller.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/player_queues/controller.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/controller.py
- https://github.com/music-assistant/client/blob/main/music_assistant_client/players.py
- https://github.com/music-assistant/models/blob/main/music_assistant_models/player.py
- https://www.music-assistant.io/player-support/sendspin/

The unversioned client/model references supplement the versioned server handlers;
server tag 2.10.2 defines command compatibility. Fixture success does not establish
compatibility with an unknown live server version.

Terminal input enables bracketed paste and disables it on normal exit, signals
and panic cleanup. Paste events insert only into the active text field, excluding
control characters without turning them into shortcuts or submissions. Oversized
pastes are rejected atomically so URLs and credentials are not silently truncated.
PTY tests cover long prefixed URLs, masked password paste, search paste and paste
mode restoration on normal quit and SIGTERM.

Failed connection tests retain masked form credentials for retry; only the
worker receives a clone. A token-only PTY fixture returns HTTP 405 once and
verifies that retry succeeds without repasting or calling the password-login
endpoint. Server-info preflight rejects HTML/dashboard URLs before API token
submission. HTTP 405 errors identify the rejected API method and advise checking
the base URL/proxy route, without echoing response bodies.

## Music selection (2026-09-08)

The default content pane is a read-only music browser. Library categories use
`music/{media_type}s/library_items` with 100-item offset pages and optional
favorite filtering. Provider browsing follows server-returned folder paths.
Album/playlist rows open track listings; artist rows open albums with a top-tracks
folder. Search results retain item/provider identity and use the same navigation.
History restores the prior cursor; generation IDs discard replies after back or
new navigation. Loading, empty, failure and retry states remain inside the pane.

Enter on a playable leaf, or P on a collection, opens a menu showing the speaker
and explicit replace/next/add queue options. Browse requests need no selected
player. Playback requests retain the chosen player, check its availability at
submission, and resolve its active group queue before sending `play_media`.
Navigation never issues playback commands. The demo uses fictional catalog data.

Official 2.10.2 command/argument references:

- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/base.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/albums.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/artists.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/playlists.py
- https://github.com/music-assistant/server/blob/2.10.2/music_assistant/controllers/music/media/radio.py

Read-only checks against the configured server confirmed version 2.10.2/schema 65
and library/provider listing endpoints. No credentials or returned personal media
are retained in test fixtures. A live `--remote-only` PTY check verified album
listing, album tracks, back navigation and provider listing using the saved login;
radio and favorite-track endpoints also succeeded in read-only checks. The connected PTY fixture checks browse before
speaker selection, playlist drill-down, track and whole-collection queue actions,
search, and group queue routing. Audible/live playback is a separate manual check.
