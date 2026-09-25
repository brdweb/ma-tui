# Local audio troubleshooting

MA-TUI plays through the selected ALSA output. On a desktop that usually routes
to PipeWire or PulseAudio. Check the selected output with `ma-tui --list-devices`
and in **F2** settings; an explicitly selected missing device remains an error.
The Flatpak uses the host's PulseAudio-compatible service for its default output.

If local audio remains failed after automatic recovery, press **F2**, then
**Esc**, to recreate the local speaker connection. Preserve the visible status
when reporting a problem, along with the selected output and whether the app is
native or Flatpak. `reconnecting` reports a Sendspin session retry; a terminal
`worker queue overflow` means MA-TUI could not accept the server's bounded
cached-audio burst and stops rather than retrying the same failure indefinitely.

## Recovery and diagnostics

An ALSA underrun means the device exhausted its available audio buffer. When the
backend recovers and output callbacks resume promptly, MA-TUI keeps the stream
connected. Repeated underruns or callbacks that stop advancing trigger bounded
output recovery. A short startup grace avoids treating stream initialization as
a stall. Recovery discards stale buffered audio and preserves volume, mute and
delay; a persistent fault remains visible after the retry limit.

The exact ALSA `snd_pcm_avail_delay` I/O error can also be transient while the
PulseAudio adapter waits for timing information. MA-TUI gives that specific
report a bounded grace period and requires new callbacks after the reports
stop. Repeated reports do not extend the deadline. Three distinct timing-error
episodes within ten seconds trigger rebuilding; other I/O errors do not get
this exception. A silent test on the affected output reproduced recovery in
10–20 ms without recreating the stream; this is not a long-duration playback
guarantee.

The footer shows the local-audio state and, for known failures, a short fixed
connection or output reason. It never exposes raw server, device or timing
diagnostics. A callback gap measures progress in the application's output
callback; it does not measure acoustic latency.

## Optional output buffer

Leave `output_buffer_frames` out of the configuration to use the audio backend's
default. If playback repeatedly underruns, you can request a larger buffer in
the active configuration file, for example:

```toml
output_buffer_frames = 1024
```

The allowed range is 256–8192 frames per channel. This is an output-device buffer
request, not the network prebuffer; the audio backend's supported sizes still
apply. Larger buffers can trade more latency for additional scheduling margin.
At 48 kHz, 1024 frames represent about 21 ms of audio, not the total end-to-end
latency. Restart MA-TUI after editing its configuration. Remove the setting to
return to the backend default.

Native settings normally live at `$XDG_CONFIG_HOME/ma-tui/config.toml` (usually
`~/.config/ma-tui/config.toml`); `--config` overrides that location. Flatpak
settings are separate at
`~/.var/app/io.github.brdweb.MaTui/config/ma-tui/config.toml`.

## RTKit belongs on the host

RTKit is a D-Bus system service that grants realtime scheduling under a host
policy. Its distribution package supplies the daemon, system service and D-Bus
and PolicyKit integration. Shipping a copy inside the Flatpak or native archive
would not install that host integration. MA-TUI's Arch package therefore lists
`rtkit` as an optional dependency. [RTKit manual](https://man.archlinux.org/man/rtkitctl.8.en),
[Arch package files](https://archlinux.org/packages/extra/x86_64/rtkit/files/).

PipeWire can use the user's configured realtime limits and falls back to the
Realtime portal or RTKit when those limits do not suffice. A system with working
realtime scheduling does not need a second mechanism just for MA-TUI.
[PipeWire realtime module](https://docs.pipewire.org/page_module_rt.html).

On Arch/Omarchy, these commands inspect the installed package and recent audio
service diagnostics:

```sh
pacman -Q rtkit
systemctl status rtkit-daemon.service --no-pager
journalctl --user -b -u pipewire -u pipewire-pulse --no-pager -n 80
```

If the package is absent and the audio logs report unavailable realtime
scheduling, install it separately with `sudo pacman -S --needed rtkit`. RTKit is
activated on demand by D-Bus. An inactive service alone does not show that audio
scheduling has failed; check the audio service's actual diagnostics. Installing
RTKit may improve scheduling availability but does not establish that it fixes
a particular stream error.

The Flatpak uses host audio services. Where a client supports it, the Realtime
portal maps sandbox process IDs and forwards requests to host RTKit. The bundle
does not include an RTKit daemon or add full system-bus access.
[Realtime portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Realtime.html),
[Flatpak D-Bus permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html#d-bus-access).
