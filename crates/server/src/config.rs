//! The local setup: `bid.toml` in the games folder (`$BID_GAMES`, default `games`), so it lives
//! in the user's own games repository, not in this one. `bid.example.toml` lists the keys. An
//! environment variable, where one is named, overrides its key.

use std::path::PathBuf;
use std::sync::OnceLock;

static CONFIG: OnceLock<toml::Table> = OnceLock::new();

pub fn games_dir() -> PathBuf {
    PathBuf::from(std::env::var("BID_GAMES").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "games".into()))
}

fn table() -> &'static toml::Table {
    CONFIG.get_or_init(|| {
        let path = games_dir().join("bid.toml");
        let Ok(text) = std::fs::read_to_string(&path) else { return toml::Table::new() };
        text.parse().unwrap_or_else(|e| {
            eprintln!("{}: {e}; using the defaults", path.display());
            toml::Table::new()
        })
    })
}

/// `[server] key`, unless the environment variable `env` is set.
pub fn server(env: &str, key: &str) -> Option<String> {
    std::env::var(env).ok().filter(|v| !v.is_empty()).or_else(|| table().get("server")?.get(key)?.as_str().map(String::from))
}

/// `[server] key` as a list of strings, unless `env` is set (then comma-separated).
pub fn server_list(env: &str, key: &str) -> Vec<String> {
    if let Some(v) = std::env::var(env).ok().filter(|v| !v.is_empty()) {
        return v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    let Some(list) = table().get("server").and_then(|s| s.get(key)).and_then(|v| v.as_array()) else { return vec![] };
    list.iter().filter_map(|v| v.as_str().map(String::from)).collect()
}
