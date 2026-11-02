//! Persistent application state and standard locations (DESIGN.md, Storage, Persistence).

use crate::preset::write_atomic;
use crate::response::PreampMode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadedEntry {
    pub id: String,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub bypass: bool,
    /// PipeWire node name of the selected output device.
    pub output_device: Option<String>,
    pub preamp_mode: PreampMode,
    pub manual_preamp_db: f64,
    #[serde(rename = "loaded")]
    pub loaded: Vec<LoadedEntry>,
    pub main_window: Option<WindowSize>,
    pub edit_window: Option<WindowSize>,
}

impl AppState {
    /// Read the state file; a missing or unreadable file gives the default state.
    pub fn read(path: &Path) -> AppState {
        std::fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, toml::to_string(self).expect("state always serializes").as_bytes())
    }
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
}

pub fn presets_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("tincan/presets")
}

pub fn state_file() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("tincan/state.toml")
}

/// Socket used to enforce a single instance.
pub fn instance_socket() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("tincan.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s/state.toml");
        assert_eq!(AppState::read(&path), AppState::default());
        let s = AppState {
            bypass: true,
            output_device: Some("alsa_output.usb".into()),
            preamp_mode: PreampMode::Manual,
            manual_preamp_db: -3.5,
            loaded: vec![LoadedEntry { id: "a".into(), active: false }],
            main_window: Some(WindowSize { width: 900, height: 700 }),
            edit_window: None,
        };
        s.write(&path).unwrap();
        assert_eq!(AppState::read(&path), s);
        let partial: AppState = toml::from_str("bypass = true").unwrap();
        assert!(partial.bypass && partial.loaded.is_empty());
    }
}
