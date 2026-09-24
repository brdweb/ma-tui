use anyhow::{bail, Context, Result};
use clap::Parser;
use ma_tui::{
    cli::Args,
    ui::{self, App, PlayerView, TrackView},
};
use std::io::IsTerminal;

fn dispatch(
    app: &mut App,
    action: ui::Action,
    requests: &tokio::sync::mpsc::Sender<ma_tui::controller::Request>,
    selection: &tokio::sync::watch::Sender<Option<String>>,
) {
    if let ui::Action::Select(id) = action {
        if selection.send(Some(id)).is_err() {
            app.status = "API worker stopped".into();
        }
    } else {
        let searching = matches!(action, ui::Action::Search(_));
        let browsing = matches!(action, ui::Action::Browse { .. });
        if requests
            .try_send(ma_tui::controller::Request::new(
                app.selected_id.clone(),
                action,
            ))
            .is_err()
        {
            app.status = "Busy: command not sent; try again".into();
            if browsing {
                app.music.apply(
                    app.music.generation,
                    Err("Busy: press r to retry loading music".into()),
                );
            }
        } else if searching {
            app.results.clear();
            app.status = "Searching…".into();
        } else if browsing {
            app.status = "Browsing music · Enter opens collections; P chooses playback".into();
        } else {
            app.status = "Command pending…".into();
        }
    }
}

fn demo() -> App {
    App {
        demo: true,
        connected: true,
        title: "Sample track — offline preview".into(),
        artist: "Fictional artist · no audio or network".into(),
        status: "Offline demo: controls do not affect any server".into(),
        audio_status: "Sendspin 0.3.7 · disabled in demo".into(),
        selected_id: Some("demo".into()),
        players: vec![PlayerView {
            details: serde_json::Value::Null,
            id: "demo".into(),
            name: "This computer (demo)".into(),
            available: true,
            state: "paused".into(),
            volume: Some(30),
        }],
        queue: vec![TrackView {
            title: "Sample track — offline preview".into(),
            artist: "Fictional artist".into(),
            duration: 240.0,
            ..Default::default()
        }],
        elapsed: 72.0,
        duration: 240.0,
        ..App::default()
    }
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.init {
        let path = ma_tui::cli::config_path(args.config)?;
        ma_tui::cli::initialize(&path)?;
        println!(
            "Created {}. Run ma-tui --setup to configure the server, login and local speaker.",
            path.display()
        );
        return Ok(());
    }
    if args.demo && args.snapshot {
        let mut app = demo();
        ma_tui::theme::reload(&mut app.palette, &ma_tui::theme::paths());
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30))?;
        terminal.draw(|f| ui::draw(f, &mut app))?;
        for row in terminal.backend().buffer().content.chunks(110) {
            println!("{}", row.iter().map(|c| c.symbol()).collect::<String>());
        }
        return Ok(());
    }
    if args.demo {
        return ma_tui::terminal_ui::run(
            demo(),
            |_| false,
            |app, action| {
                if let ui::Action::Search(query) = action {
                    app.results = app
                        .queue
                        .iter()
                        .filter(|t| t.title.to_lowercase().contains(&query.to_lowercase()))
                        .cloned()
                        .collect();
                    app.status = "Demo search complete (fictional offline data)".into();
                } else if let ui::Action::Browse { generation, target } = action {
                    app.music
                        .apply(generation, Ok((ma_tui::music::demo_listing(&target), None)));
                } else {
                    app.status = "Offline demo: no command was sent".into();
                }
            },
        )
        .map(|_| ());
    }
    if args.check_art {
        use ma_tui::artwork;
        let cells = artwork::cell_pixels();
        println!(
            "TERM          {}",
            std::env::var("TERM").unwrap_or_default()
        );
        println!(
            "TERM_PROGRAM  {}",
            std::env::var("TERM_PROGRAM").unwrap_or_default()
        );
        println!(
            "cell size     {}",
            match cells {
                Some((w, h)) => format!("{w}x{h} pixels"),
                None => "not reported — sixel is not possible".into(),
            }
        );
        println!(
            "TMUX          {}",
            if std::env::var_os("TMUX").is_some() {
                "yes — a multiplexer mostly will not forward pixels"
            } else {
                "no"
            }
        );
        println!(
            "ZELLIJ        {}",
            if std::env::var_os("ZELLIJ").is_some() {
                "yes — a multiplexer mostly will not forward pixels"
            } else {
                "no"
            }
        );
        for setting in [
            ma_tui::config::AlbumArt::Auto,
            ma_tui::config::AlbumArt::Sixel,
            ma_tui::config::AlbumArt::Blocks,
        ] {
            println!(
                "album_art = {:<8} draws {}",
                format!("{setting:?}").to_lowercase(),
                artwork::renderer(setting).1
            );
        }
        let art = artwork::test_pattern(96);
        println!("\nHalf blocks (should work in any terminal):");
        for line in art.half_blocks(24, 12) {
            // Re-emit the styled cells as plain ANSI: no interface is running.
            let mut out = String::new();
            for span in line.spans {
                if let (
                    Some(ratatui::style::Color::Rgb(r, g, b)),
                    Some(ratatui::style::Color::Rgb(br, bg, bb)),
                ) = (span.style.fg, span.style.bg)
                {
                    out.push_str(&format!("\x1b[38;2;{r};{g};{b}m\x1b[48;2;{br};{bg};{bb}m▀"));
                }
            }
            println!("{out}\x1b[0m");
        }
        if let Some((w, h)) = cells {
            println!("\nSixel (a square with four quadrants and a white diagonal):");
            println!("{}", art.sixel(24 * w, 12 * h));
        }
        println!("\nIf the sixel square is missing, stretched or striped, set");
        println!("album_art = \"blocks\" and tell me which of those it was.");
        return Ok(());
    }
    if args.list_devices {
        let devices = ma_tui::audio::devices()?;
        if devices.is_empty() {
            println!("No usable audio output devices found");
        }
        for device in devices {
            println!("{}\n  {}", device.id, device.name);
        }
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("An interactive terminal is required; use --demo --snapshot for plain output");
    }
    let path = ma_tui::cli::config_path(args.config)?;
    let mut config = if path.exists() {
        ma_tui::config::Config::parse(
            &std::fs::read_to_string(&path).context("Cannot read configuration")?,
        )?
    } else {
        ma_tui::config::Config {
            local_playback: true,
            ..Default::default()
        }
    };
    // Overrides used before each rename stay readable, newest first, so an
    // existing setup keeps working without being reconfigured.
    let mut token = std::env::var("MA_TUI_TOKEN")
        .or_else(|_| std::env::var("LOCAL_MATUI_TOKEN"))
        .or_else(|_| std::env::var("MATUI_TOKEN"))
        .ok();
    if token.is_none() && path.exists() {
        token = ma_tui::credentials::load(&config.server, &config.player_id)
            .await
            .ok();
    }
    let mut setup = args.setup || token.is_none() || !path.exists();
    loop {
        if setup {
            if let Some((next_config, next_token)) =
                ma_tui::settings::run(config.clone(), token.clone(), &path)?
            {
                config = next_config;
                token = Some(next_token);
            } else if token.is_none() {
                return Ok(());
            }
        }
        let Some(token_value) = token.as_ref() else {
            return Ok(());
        };
        let api = ma_tui::api::ApiClient::new(&config.server, token_value)?;
        // The visualizer analyzes only what this endpoint plays.
        let spectrum = ma_tui::visualizer::Analyzer::new();
        let audio = if (args.local || config.local_playback) && !args.remote_only {
            Some(ma_tui::audio::start(
                ma_tui::audio::AudioConfig {
                    server: config.server.clone(),
                    token: token_value.clone(),
                    player_id: config.player_id.clone(),
                    player_name: config.player_name.clone(),
                    device_id: config.device_id.clone(),
                    output_buffer_frames: config.output_buffer_frames,
                    volume: config.volume,
                    muted: false,
                },
                Some(std::sync::Arc::new(spectrum.clone())),
            )?)
        } else {
            None
        };
        // The server says when something changed; polling is the safety net.
        let (events, stream) = match ma_tui::events::Events::start(&config.server, token_value) {
            Ok((events, stream)) => (Some(events), Some(stream)),
            Err(_) => (None, None),
        };
        let mut controller =
            ma_tui::controller::Controller::start(api, stream, config.album_art.enabled());
        let requests = controller.requests.clone();
        let refresh = controller.requests.clone();
        let selection = controller.selection.clone();
        let audio_status = audio.as_ref().map(|a| a.status.clone());
        let local_id = if audio.is_some() {
            Some(config.player_id.as_str())
        } else {
            None
        };
        let mut app = App {
            // Only real device output produces samples; a remote speaker
            // never routes audio through this machine.
            spectrum: audio.as_ref().map(|_| spectrum.clone()),
            spectrum_style: config.spectrum,
            sixel: ma_tui::artwork::use_sixel(config.album_art),
            local_endpoint: local_id.map(str::to_owned),
            ..App::default()
        };
        let server_base = config.server.trim_end_matches('/').to_owned();
        let (mut mpris_state, mut mpris_commands, mpris) = if config.mpris {
            let (state, receiver) =
                tokio::sync::watch::channel(ma_tui::mpris::NowPlaying::default());
            let (commands, actions) = tokio::sync::mpsc::unbounded_channel();
            match ma_tui::mpris::Mpris::start(commands, receiver, config.notifications).await {
                Ok(mpris) => (Some(state), Some(actions), Some(mpris)),
                Err(error) => {
                    app.status = format!("Media keys unavailable: {error}");
                    (None, None, None)
                }
            }
        } else {
            (None, None, None)
        };
        let result = ma_tui::terminal_ui::run(
            app,
            |app| {
                let mut changed = false;
                while let Ok(update) = controller.updates.try_recv() {
                    if let Some(action) = ma_tui::presentation::apply(app, update) {
                        let _ = refresh.try_send(ma_tui::controller::Request::new(
                            app.selected_id.clone(),
                            action,
                        ));
                    }
                    changed = true;
                }
                if let Some(endpoint) = local_id {
                    if let Some(id) = ma_tui::presentation::select_local(app, endpoint) {
                        let _ = selection.send(Some(id));
                        changed = true;
                    }
                }
                if let Some(status) = &audio_status {
                    let status = status.borrow();
                    let line = format!("Local audio · {} · {}", status.state, status.detail);
                    if line != app.audio_status {
                        app.audio_status = line;
                        changed = true;
                    }
                }
                if let (Some(state), Some(commands)) =
                    (mpris_state.as_mut(), mpris_commands.as_mut())
                {
                    let snapshot = ma_tui::mpris::snapshot(app, &server_base);
                    let _ = state.send_if_modified(|current| {
                        if current == &snapshot {
                            false
                        } else {
                            *current = snapshot;
                            true
                        }
                    });
                    let now = state.borrow();
                    while let Ok(command) = commands.try_recv() {
                        if let Some(action) = ma_tui::mpris::action(command, &now, &app.queue_id) {
                            dispatch(app, action, &requests, &selection);
                            changed = true;
                        }
                    }
                }
                changed
            },
            |app, action| dispatch(app, action, &requests, &selection),
        );
        if let Some(mpris) = mpris {
            mpris.shutdown().await;
        }
        controller.shutdown().await;
        if let Some(events) = events {
            events.shutdown().await;
        }
        if let Some(audio) = audio {
            audio.shutdown().await;
        }
        match result? {
            ui::Action::OpenSettings => setup = true,
            _ => return Ok(()),
        }
    }
}
