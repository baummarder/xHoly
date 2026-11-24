//! Built-in default config (TOML baseline for [`super::config_loader`]).

/// Default config as TOML text — the lowest-precedence layer.
pub const DEFAULTS_TOML: &str = r#"
[save]
auto_save = true
auto_save_interval_secs = 300
compress = true

[mod_system]
hot_reload = false

[cheat_engine]
enabled = false
show_overlay = true

[benchmark]
default_duration_ms = 5000

[debug]
hotkey = "F1"
show_fps = true
show_memory = true

[cloud]
enabled = false
token_env = "XHOLY_CLOUD_TOKEN"

[discord]
enabled = true
application_id = ""
show_game_name = true
show_generation = true
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn defaults_parse() {
        let v: toml::Value = toml::from_str(super::DEFAULTS_TOML).unwrap();
        assert_eq!(
            v["benchmark"]["default_duration_ms"].as_integer(),
            Some(5000)
        );
    }
}
