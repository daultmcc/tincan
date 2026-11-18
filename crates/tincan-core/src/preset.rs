//! Presets and their TOML file format (DESIGN.md, Presets).

use crate::filter::Filter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub name: String,
    pub filters: Vec<Filter>,
}

#[derive(Serialize, Deserialize)]
struct PresetFile {
    format: u32,
    name: String,
    #[serde(default, rename = "filter")]
    filters: Vec<Filter>,
}

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("not a valid TinCan preset: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("this preset was made by a newer version of TinCan (format {0}, this version reads up to {FORMAT_VERSION})")]
    NewerFormat(u32),
    #[error("a preset named \"{0}\" already exists")]
    NameTaken(String),
    #[error("a preset name can't be empty")]
    EmptyName,
}

impl Preset {
    pub fn parse(text: &str) -> Result<Preset, PresetError> {
        // Check the version first, so a newer file is reported as such rather than as invalid.
        #[derive(Deserialize)]
        struct Version {
            format: u32,
        }
        let version: Version = toml::from_str(text)?;
        if version.format > FORMAT_VERSION {
            return Err(PresetError::NewerFormat(version.format));
        }
        let file: PresetFile = toml::from_str(text)?;
        Ok(Preset { name: file.name, filters: file.filters.into_iter().map(Filter::clamped).collect() })
    }

    pub fn to_toml(&self) -> String {
        let file = PresetFile { format: FORMAT_VERSION, name: self.name.clone(), filters: self.filters.clone() };
        toml::to_string(&file).expect("presets always serialize")
    }

    pub fn read(path: &Path) -> Result<Preset, PresetError> {
        Preset::parse(&fs::read_to_string(path)?)
    }

    pub fn write(&self, path: &Path) -> Result<(), PresetError> {
        write_atomic(path, self.to_toml().as_bytes())?;
        Ok(())
    }
}

/// Write via a temporary file and rename, so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

/// File stem for a preset name. Two names with the same slug count as the same name.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() && !name.trim().is_empty() { "preset".to_string() } else { out }
}

/// When a preset file was last changed, as far as can be seen without reading it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
    inode: u64,
}

/// The directory of saved presets. Presets are identified by their file stem (the slug of
/// their name).
#[derive(Clone, Debug)]
pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    pub fn new(dir: PathBuf) -> PresetStore {
        PresetStore { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.toml"))
    }

    pub fn exists(&self, id: &str) -> bool {
        self.path(id).exists()
    }

    /// All readable presets as (id, preset), sorted by name. Unreadable files are reported
    /// separately.
    #[allow(clippy::type_complexity)]
    pub fn list(&self) -> (Vec<(String, Preset)>, Vec<(PathBuf, PresetError)>) {
        let mut presets = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return (presets, errors);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            match Preset::read(&path) {
                Ok(p) => presets.push((id, p)),
                Err(e) => errors.push((path, e)),
            }
        }
        presets.sort_by_key(|(_, p)| p.name.to_lowercase());
        (presets, errors)
    }

    /// A stamp for every preset file, by id, to detect changes made by other programs.
    pub fn stamps(&self) -> HashMap<String, FileStamp> {
        use std::os::unix::fs::MetadataExt;
        let mut stamps = HashMap::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return stamps;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let (Some(id), Ok(meta)) = (path.file_stem().and_then(|s| s.to_str()), fs::metadata(&path)) else {
                continue;
            };
            let stamp = FileStamp { modified: meta.modified().ok(), len: meta.len(), inode: meta.ino() };
            stamps.insert(id.to_string(), stamp);
        }
        stamps
    }

    pub fn load(&self, id: &str) -> Result<Preset, PresetError> {
        Preset::read(&self.path(id))
    }

    /// Overwrite the preset stored as `id`.
    pub fn save(&self, id: &str, preset: &Preset) -> Result<(), PresetError> {
        preset.write(&self.path(id))
    }

    /// The id a new preset named `name` would get, if the name is free.
    pub fn check_new_name(&self, name: &str) -> Result<String, PresetError> {
        let id = slug(name);
        if id.is_empty() {
            return Err(PresetError::EmptyName);
        }
        if self.exists(&id) {
            return Err(PresetError::NameTaken(name.trim().to_string()));
        }
        Ok(id)
    }

    /// Store a preset under a new name. Returns its id.
    pub fn create(&self, preset: &Preset) -> Result<String, PresetError> {
        let id = self.check_new_name(&preset.name)?;
        self.save(&id, preset)?;
        Ok(id)
    }

    /// A name based on `base` that no stored preset uses.
    pub fn unused_name(&self, base: &str) -> String {
        if self.check_new_name(base).is_ok() {
            return base.to_string();
        }
        (2..).map(|n| format!("{base} {n}")).find(|n| self.check_new_name(n).is_ok()).unwrap()
    }

    /// Rename a preset. Returns its new id.
    pub fn rename(&self, id: &str, new_name: &str) -> Result<String, PresetError> {
        let mut preset = self.load(id)?;
        let new_id = slug(new_name);
        if new_id.is_empty() {
            return Err(PresetError::EmptyName);
        }
        if new_id != id && self.exists(&new_id) {
            return Err(PresetError::NameTaken(new_name.trim().to_string()));
        }
        preset.name = new_name.trim().to_string();
        self.save(&new_id, &preset)?;
        if new_id != id {
            fs::remove_file(self.path(id))?;
        }
        Ok(new_id)
    }

    pub fn delete(&self, id: &str) -> Result<(), PresetError> {
        fs::remove_file(self.path(id))?;
        Ok(())
    }

    /// Import a preset file, replacing any stored preset with the same name if `replace`.
    /// Returns the new id, or `NameTaken` so the caller can ask to rename or replace.
    pub fn import(&self, path: &Path, name: Option<&str>, replace: bool) -> Result<String, PresetError> {
        let mut preset = Preset::read(path)?;
        if let Some(name) = name {
            preset.name = name.trim().to_string();
        }
        if replace {
            let id = slug(&preset.name);
            if id.is_empty() {
                return Err(PresetError::EmptyName);
            }
            self.save(&id, &preset)?;
            return Ok(id);
        }
        self.create(&preset)
    }

    pub fn export(&self, id: &str, path: &Path) -> Result<(), PresetError> {
        fs::copy(self.path(id), path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{Channel, FilterType};

    fn sample() -> Preset {
        Preset {
            name: "Sundara resonances".into(),
            filters: vec![
                Filter { kind: FilterType::Bell, channel: Channel::L, frequency: 6150.0, gain: -4.2, q: 6.5, enabled: true },
                Filter { kind: FilterType::HighShelf, channel: Channel::LR, frequency: 9000.0, gain: 2.0, q: 0.7, enabled: false },
            ],
        }
    }

    #[test]
    fn round_trip_and_documented_format() {
        let p = sample();
        assert_eq!(Preset::parse(&p.to_toml()).unwrap(), p);
        let doc = r#"
format = 1
name = "Sundara resonances"

[[filter]]
type = "bell"
channel = "LR"
frequency = 6150.0
gain = -4.2
q = 6.5
enabled = true
future_key = 3
"#;
        let parsed = Preset::parse(doc).unwrap();
        assert_eq!(parsed.filters[0].kind, FilterType::Bell);
        assert!(matches!(Preset::parse("format = 2\nname = \"x\""), Err(PresetError::NewerFormat(2))));
    }

    #[test]
    fn store_operations() {
        let dir = tempfile::tempdir().unwrap();
        let store = PresetStore::new(dir.path().join("presets"));
        let id = store.create(&sample()).unwrap();
        assert_eq!(id, "sundara-resonances");
        assert!(matches!(store.create(&sample()), Err(PresetError::NameTaken(_))));
        assert_eq!(store.unused_name("Sundara Resonances"), "Sundara Resonances 2");
        let id2 = store.rename(&id, "Tonality").unwrap();
        assert_eq!(id2, "tonality");
        assert!(!store.exists(&id));
        assert_eq!(store.load(&id2).unwrap().name, "Tonality");
        let out = dir.path().join("out.toml");
        store.export(&id2, &out).unwrap();
        assert!(matches!(store.import(&out, None, false), Err(PresetError::NameTaken(_))));
        assert_eq!(store.import(&out, Some("Copy"), false).unwrap(), "copy");
        assert_eq!(store.import(&out, None, true).unwrap(), "tonality");
        let (list, errors) = store.list();
        assert_eq!(list.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), vec!["copy", "tonality"]);
        assert!(errors.is_empty());
        store.delete("copy").unwrap();
        assert_eq!(store.list().0.len(), 1);
    }
}
