# Changelog

## 1.1.0 — 2026-09-25

### Queue and playback

- Clear the active queue directly from Queue with `c`, stopping playback.
- Select individual playable tracks with `x` in Music or Search, then use `A`
  to choose **Replace queue** or **Add to queue** in displayed order. `a` still
  immediately adds a highlighted track; for an album, playlist or provider
  folder it now offers those same two choices. Albums and playlists go to Music
  Assistant by their own URI; folders use only their immediately listed
  available, playable children. Multi-track operations stop on the first
  failure without rolling back earlier changes.

### Local audio and desktop packaging

- Fix false `Local audio · ready` status: a connection or stream announcement
  alone is not ready; ready requires decoded audio accepted by the local output
  path. Connected/buffering can still await audio, while recovering reports
  transient output recovery.
- Accept Music Assistant's cached post-`stream/start` audio burst while local
  output opens. The worker handoff is bounded by 1024 items and 2 MiB encoded
  audio; a real bound violation now fails visibly instead of reconnecting
  indefinitely.
- Show concise `Local audio · <state>` and short safe connection/output reasons,
  optionally followed by the current queue item's file codec, bitrate or compact
  resolution. Raw server, device and timing diagnostics remain hidden.
- Add an MA-TUI application icon to native, Arch and Flatpak packages. The
  Flatpak clears inherited `NO_COLOR` so the Omarchy palette remains visible.

### Interface

- Spell out bottom key hints for playback menus, queue edits, paging, sort,
  filter, transport and controls instead of ambiguous abbreviations.

## 1.0.0 — 2026-09-23

First 1.0 release. The stated scope is complete and stable; the README's Known
limitations still apply.

### Desktop integration

- Register `org.mpris.MediaPlayer2.ma_tui` on the session bus while connected,
  so media keys, `playerctl` and compatible desktop bars can show and control
  the selected player. `mpris = true` is the default; a second instance keeps
  running without MPRIS if the name is already owned.
- Add optional desktop track notifications. `notifications = false` remains the
  default; when enabled, each new playing track replaces the prior notification.
- Add Flatpak permissions to own the MPRIS name and talk to
  `org.freedesktop.Notifications`, without granting full session-bus access.

- Show the version in the interface header and settings title.

### Favourites and library

- Add `f` for the highlighted Music/Search item and `F` for the playing item to
  toggle favourites, with a heart marker in listings. The item menu can also add
  provider items to the library; Music Assistant makes favouriting a library add.
- Add Favorite albums, artists, playlists and radio shelves after Favorite
  tracks. Library and favourite listings update when the server reports media
  item additions, changes or deletions.

### Browsing

- Add Recently played to Home, using up to 50 fully played items from Music
  Assistant's play log.
- Add previous-page `[` alongside `]`, sort cycling with `o`, and server-side
  filtering of the current library listing with Ctrl-F.

### Radio and playlists

- Add **Start radio** to the item menu, replacing the selected queue with a
  dynamic radio playlist seeded from a track, album, artist or playlist.
- Add **Add to playlist…** with editable playlist selection or creation, and
  **Save queue as playlist…** in the controls menu. Playlist additions run in
  the background on Music Assistant.

## 0.9.3 — 2026-09-17

- Recover from local audio output stream failures automatically, with up to
  three delayed retries. Each attempt reopens the configured output and
  reconnects the local speaker, discards stale audio, and preserves the current
  volume, mute and delay. Persistent failures remain visible.
- Keep the stream connected after isolated recovered output underruns, and
  monitor callback progress so a stalled output enters bounded recovery.
- Allow the exact transient ALSA `snd_pcm_avail_delay` I/O error a bounded
  recovery period, requiring fresh callbacks before treating it as recovered.
  Repeated errors cannot extend the deadline; unknown I/O errors remain fatal
  to the current stream.
- Show local output format, callback frames and gaps, underrun counts and
  timing-error recovery counts, backend and pending recovery in diagnostics.
- Add optional `output_buffer_frames` configuration (256–8192 frames per
  channel), retaining the audio backend's default when omitted.
- Keep polling player and queue state every two seconds until the event stream
  connects and authenticates. A socket that never opens no longer leaves the
  interface waiting thirty seconds between updates.
- Report an incomplete **Unplayed podcasts** list when any show's episodes
  cannot be loaded, instead of silently displaying an empty or partial result.
- Refresh **Unplayed podcasts** when listening progress changes, including
  episodes marked played or unplayed in another client.
- Default local startup volume to 50 percent. Explicitly saved volume settings
  are preserved.
- Declare RTKit as an optional Arch dependency and document host audio
  scheduling support for native and Flatpak installations.
- Build the optimized release executable on GitHub Actions and verify its
  source, lockfile and binary identity before packaging. Release build metadata
  records the actual compiler and GitHub run.

## 0.9.2 — 2026-09-17

- Add a screenshot to the README.
- Trim the README's install steps to read the same across releases instead of
  hardcoding a version.
- Drop the stale "(Flatpak Beta)" label from the Flatpak desktop entry's name;
  0.9.1 was already the first release that was not a beta.
- Draw a rule between the player list and the queue instead of leaving blank
  space, matching every other section boundary in the layout.
- Show the actual reason a local audio stream failed instead of always the
  same generic message. `sendspin`'s stream-error text is now read and
  displayed (control-character-stripped, capped at 512 characters); previously
  it was captured but never read, so a real failure and an unknown one looked
  identical. No logger was added: `sendspin`'s own protocol logging can embed
  raw server payload text as low as debug/trace (and in one case even at
  error level), which this module's existing sanitize-only policy exists to
  avoid.

## 0.9.1 — 2026-09-16

- Update rustls to 0.23.45 for RUSTSEC-2026-0285, which 0.9.0 shipped. Rustls
  accepted TLS 1.3 handshake messages sent at the wrong encryption level. The
  handshake transcript stays authenticated, so this could not be used to alter
  or complete a handshake; the effect is that a peer could send messages in
  plaintext that should have been encrypted without the connection being
  refused. Update if you connect to Music Assistant over HTTPS.
- Update uuid to 1.26.1.
- The advisory audit now runs on main, on demand and weekly rather than on every
  pull request, where an advisory published after a branch was opened failed it
  for reasons unrelated to its contents.
- Correct the README, which still described a visualizer with a `v` key, a
  spectrum panel and a full-screen view. None of those exist: the spectrum is
  part of the player. Album art was undocumented.

## 0.9.0 — 2026-09-16

First release under the name **ma-tui**, and the first that is not a beta. It is
pre-1.0: the scope below is complete and exercised, but the limitations at the
end of the README are real and unchanged.

### The interface learns about changes instead of asking

- Music Assistant's `/ws` event stream replaces polling. State changes arrive
  when they happen rather than up to two seconds later, and the playback
  position comes straight from the server's own clock, costing no request at
  all. Polling is kept as a fallback — two seconds with no stream, thirty with
  one — so a socket that dies quietly cannot leave the interface stale. The
  header says which of the two is in use.
- The position on screen runs between updates instead of freezing and jumping.
- The interface redraws only when something changed, rather than twenty times a
  second regardless, and reading the library no longer delays a transport key.

### Podcasts and audiobooks

- Podcast and audiobook libraries, a podcast's episodes, and the shelves the
  server maintains: **Continue listening** and **Recently added**.
- **Unplayed podcasts**, assembled here because the server has no filter for it.
- Episodes and audiobooks show their resume point or that they are finished, and
  can be marked played or unplayed. Marking needs no speaker: it is a library
  edit. Progress set elsewhere — in Audiobookshelf or the web interface —
  appears without a refresh.
- A row says what its item belongs to: the show for an episode, the artists and
  album for a track, the authors for an audiobook.

### The player leads

- The layout is built around the player: what is playing, where, how far in, and
  what it sounds like. Panes are separated by rules rather than boxes, the queue
  is a table with a state column, and the focused pane's heading is filled so it
  is findable at a glance.
- The spectrum is part of the player rather than a mode, drawn in braille for
  four times the vertical resolution of block characters. `spectrum = "blocks"`
  restores the block ramp for a font without braille coverage.
- Album art, drawn as sixel where the terminal draws it and as colour half
  blocks everywhere else. `album_art` selects between them, or turns it off.

### Renamed

- The project is **ma-tui**, displayed as **MA-TUI**. The former name shared a
  binary with an existing Matrix TUI also called `matui`. Every earlier name is
  still read where one exists — the `local-matui` and `matui` configuration
  directories and keyring entries, and the `LOCAL_MATUI_TOKEN` and `MATUI_TOKEN`
  overrides — so an existing installation keeps working without re-entering
  credentials. Nothing is copied, moved or deleted, and a saved `player_id` and
  `player_name` are untouched, so Music Assistant sees the same speaker.

## 0.1.0-beta.2 — 2026-09-08

- Rename the project to **local-matui** to avoid clashing with other projects:
  the command, crate, Arch package, desktop entry and repository are now
  `local-matui`, the Flatpak application ID is `io.github.brdweb.LocalMatui`,
  settings live in `local-matui/config.toml` and the token override is
  `LOCAL_MATUI_TOKEN`. Existing installations keep working without re-entering
  credentials: a pre-rename `matui/config.toml`, keyring entry and `MATUI_TOKEN`
  are still read while no current equivalent exists. Nothing is copied, moved or
  deleted, and Music Assistant sees a new speaker identity only if you create
  one. This release replaces v0.1.0-beta.1, which was withdrawn: it was
  published under the former name only hours earlier and is superseded here.

- Rework the controls and layout: the queue stays beside the music browser, the
  header carries transport state, volume, mute and shuffle/repeat, and the hint
  lines follow the focused pane. **p** is now play/pause with tracks on
  **<**/**>**, **z**/**l** toggle shuffle and cycle repeat, **s** stops, and Esc
  cancels or steps back instead of switching panes.
- Volume keys use the server's `volume_up`/`volume_down` commands instead of
  reading the level first, and seeking resolves the target from the displayed
  position, so neither issues a request per keystroke.
- Group the controls menu under headings and add **/** to filter it.

- Add a spectrum visualizer over Local Matui's own local playback, as a panel or a
  full-screen view, cycled with **v**. Analysis uses only decoded samples this
  process is scheduled to emit; a remote speaker, disabled local audio or muted
  output shows the reason rather than invented motion.

## 0.1.0-beta.1 — 2026-09-08

First private beta for Omarchy/Arch Linux x86-64, targeting Music Assistant 2.10.2.

- Browse library and provider music, open collections, search, and choose play-now,
  play-next or add-to-queue actions for individual items or whole collections.
- Control remote speakers and expose this computer through embedded Sendspin 0.3.7.
- Configure Music Assistant using a password or profile token, with masked input,
  terminal paste support and desktop keyring persistence.
- Follow live Omarchy theme changes and expose playback, volume, grouping, source,
  sleep-timer and queue controls.
- Correct local prebuffer limits and automatically select universal-player wrappers.
- Ship a native Linux archive, Arch package, Flatpak bundle, source archive, build information,
  checksums and bundled third-party notices.

Local and remote playback are confirmed on the development laptop. Prolonged
playback, other hardware/server versions, live codec changes, server restart
during streaming, acoustic latency and multi-room synchronization need further
testing. This beta does not administer Music Assistant users, providers or DSP.
