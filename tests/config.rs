use ma_tui::config;

#[test]
fn parses_server_without_storing_a_token() {
    let cfg =
        config::Config::parse("server = 'https://ma.example:8095'\nplayer_id = 'ma-tui-test'\n")
            .unwrap();
    assert_eq!(cfg.server, "https://ma.example:8095");
    assert_eq!(cfg.player_id, "ma-tui-test");
    assert!(!cfg.local_playback);
    assert_eq!(cfg.volume, 50);
    assert_eq!(cfg.output_buffer_frames, None);
    assert_eq!(config::Config::default().output_buffer_frames, None);
    assert!(cfg.mpris);
    assert!(!cfg.notifications);
    let saved = toml::to_string_pretty(&cfg).unwrap();
    assert!(!saved.contains("output_buffer_frames"));
    assert_eq!(
        config::Config::parse(&saved).unwrap().output_buffer_frames,
        None,
        "saving an existing config leaves output buffering at the backend default"
    );
}

#[test]
fn rejects_unsafe_server_urls_and_empty_identity() {
    for server in [
        "ftp://host",
        "http://user:password@host",
        "http://host/?token=secret",
        "http://host/#secret",
    ] {
        assert!(
            config::Config::parse(&format!("server = '{server}'\nplayer_id = 'test'\n")).is_err()
        );
    }
    assert!(config::Config::parse("player_id = ''").is_err());
}

#[test]
fn loading_a_config_must_not_generate_a_new_player_identity() {
    assert!(config::Config::parse("server = 'http://localhost:8095'").is_err());
}

#[test]
fn explicit_output_buffer_frames_accept_supported_bounds_and_round_trip() {
    for frames in [256, 1024, 2048, 8192] {
        let cfg = config::Config::parse(&format!(
            "player_id = 'test'\noutput_buffer_frames = {frames}\n"
        ))
        .unwrap();
        assert_eq!(cfg.output_buffer_frames, Some(frames));
        let saved = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(
            config::Config::parse(&saved).unwrap().output_buffer_frames,
            Some(frames),
            "saving settings preserves the advanced buffer setting"
        );
    }
}

#[test]
fn invalid_output_buffer_frames_are_rejected() {
    for frames in ["0", "255", "8193", "4294967295", "-1", "1024.5"] {
        assert!(
            config::Config::parse(&format!(
                "player_id = 'test'\noutput_buffer_frames = {frames}\n"
            ))
            .is_err(),
            "invalid buffer size {frames} should not reach the audio backend"
        );
    }
}
