//! The application model: loaded presets, edit sessions and the resulting EQ.
//!
//! A preset has at most two states: saved (on disk) and live (in its edit window). A loaded
//! preset is heard in its live state when it has one (DESIGN.md, Saved and live state).
//! A new preset has only a live state, under a temporary id, until it is first saved.

use crate::dsp::{EqParams, MAX_SLOTS, SlotAllocator, SlotParams};
use crate::filter::{Filter, clamp_frequency};
use crate::preset::{Preset, PresetError, PresetStore, slug};
use crate::response::{Headroom, PreampMode};
use crate::state::{AppState, LoadedEntry};
use std::collections::HashMap;
use std::path::Path;

/// Prefix of the temporary ids of unsaved new presets. Slugs never contain it.
const NEW_PREFIX: &str = "+new-";

/// A filter with a runtime id, which keeps it in the same DSP slot while it is edited, and a
/// display color index, which stays with it when filters are reordered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackedFilter {
    pub uid: u64,
    pub color: usize,
    pub filter: Filter,
}

#[derive(Clone, Debug)]
struct TrackedPreset {
    name: String,
    filters: Vec<TrackedFilter>,
}

impl TrackedPreset {
    fn to_preset(&self) -> Preset {
        Preset { name: self.name.clone(), filters: self.filters.iter().map(|f| f.filter).collect() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPreset {
    pub id: String,
    pub active: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Preset(#[from] PresetError),
    #[error("too many filters: at most {MAX_SLOTS} can be loaded at once")]
    TooManyFilters,
    #[error("no such preset")]
    NoSuchPreset,
    #[error("the preset isn't open for editing")]
    NotEditing,
    #[error("a preset name can't be empty")]
    EmptyName,
}

pub struct Session {
    store: PresetStore,
    next_uid: u64,
    /// Saved state of every preset that is loaded or being edited.
    saved: HashMap<String, TrackedPreset>,
    /// Live state of every preset with an open edit window.
    live: HashMap<String, TrackedPreset>,
    loaded: Vec<LoadedPreset>,
    pub bypass: bool,
    pub preamp_mode: PreampMode,
    pub manual_preamp_db: f64,
    pub output_device: Option<String>,
    pub move_streams: bool,
    sample_rate: f64,
    allocator: SlotAllocator,
}

impl Session {
    /// Restore a session from saved state. Presets that no longer load are skipped and
    /// returned as errors.
    pub fn new(store: PresetStore, state: &AppState) -> (Session, Vec<(String, PresetError)>) {
        let mut session = Session {
            store,
            next_uid: 1,
            saved: HashMap::new(),
            live: HashMap::new(),
            loaded: Vec::new(),
            bypass: state.bypass,
            preamp_mode: state.preamp_mode,
            manual_preamp_db: state.manual_preamp_db,
            output_device: state.output_device.clone(),
            move_streams: state.move_streams,
            sample_rate: 48_000.0,
            allocator: SlotAllocator::default(),
        };
        let mut errors = Vec::new();
        for entry in &state.loaded {
            if session.is_loaded(&entry.id) {
                continue;
            }
            match session.load(&entry.id) {
                Ok(()) => session.set_active(&entry.id, entry.active),
                Err(SessionError::Preset(e)) => errors.push((entry.id.clone(), e)),
                Err(_) => {}
            }
        }
        (session, errors)
    }

    pub fn state(&self) -> AppState {
        AppState {
            bypass: self.bypass,
            output_device: self.output_device.clone(),
            move_streams: self.move_streams,
            preamp_mode: self.preamp_mode,
            manual_preamp_db: self.manual_preamp_db,
            loaded: self
                .loaded
                .iter()
                .filter(|l| !Session::is_new(&l.id))
                .map(|l| LoadedEntry { id: l.id.clone(), active: l.active })
                .collect(),
            main_window: None,
            edit_window: None,
        }
    }

    pub fn store(&self) -> &PresetStore {
        &self.store
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
    }

    fn track(&mut self, preset: Preset) -> TrackedPreset {
        let filters = preset
            .filters
            .into_iter()
            .enumerate()
            .map(|(color, filter)| TrackedFilter { uid: self.uid(), color, filter })
            .collect();
        TrackedPreset { name: preset.name, filters }
    }

    fn uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }

    /// Whether `id` is an unsaved new preset.
    pub fn is_new(id: &str) -> bool {
        id.starts_with(NEW_PREFIX)
    }

    fn ensure_saved(&mut self, id: &str) -> Result<(), SessionError> {
        if Session::is_new(id) {
            return if self.is_editing(id) { Ok(()) } else { Err(SessionError::NoSuchPreset) };
        }
        if !self.saved.contains_key(id) {
            let preset = self.store.load(id)?;
            let tracked = self.track(preset);
            self.saved.insert(id.to_string(), tracked);
        }
        Ok(())
    }

    /// Drop the saved state of a preset that is neither loaded nor being edited.
    fn release(&mut self, id: &str) {
        if !self.is_loaded(id) && !self.is_editing(id) {
            self.saved.remove(id);
        }
    }

    /// Every loaded or edited preset: loaded ones in order, then the rest.
    pub fn open_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.loaded.iter().map(|l| l.id.clone()).collect();
        let mut editing: Vec<&String> = self.live.keys().filter(|id| !self.is_loaded(id)).collect();
        editing.sort();
        ids.extend(editing.into_iter().cloned());
        ids
    }

    // Loaded presets.

    pub fn loaded(&self) -> &[LoadedPreset] {
        &self.loaded
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.loaded.iter().any(|l| l.id == id)
    }

    fn filter_count(&self, extra: usize) -> usize {
        self.loaded.iter().map(|l| self.current(&l.id).map_or(0, |p| p.filters.len())).sum::<usize>() + extra
    }

    pub fn load(&mut self, id: &str) -> Result<(), SessionError> {
        if self.is_loaded(id) {
            return Ok(());
        }
        self.ensure_saved(id)?;
        if self.filter_count(self.current(id).unwrap().filters.len()) > MAX_SLOTS {
            self.release(id);
            return Err(SessionError::TooManyFilters);
        }
        self.loaded.push(LoadedPreset { id: id.to_string(), active: true });
        Ok(())
    }

    pub fn unload(&mut self, id: &str) {
        self.loaded.retain(|l| l.id != id);
        self.release(id);
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.loaded.iter().any(|l| l.id == id && l.active)
    }

    pub fn set_active(&mut self, id: &str, active: bool) {
        if let Some(l) = self.loaded.iter_mut().find(|l| l.id == id) {
            l.active = active;
        }
    }

    /// The name of a loaded or edited preset.
    pub fn name(&self, id: &str) -> Option<&str> {
        self.current(id).map(|p| p.name.as_str())
    }

    fn current(&self, id: &str) -> Option<&TrackedPreset> {
        self.live.get(id).or_else(|| self.saved.get(id))
    }

    // Editing.

    pub fn is_editing(&self, id: &str) -> bool {
        self.live.contains_key(id)
    }

    /// Start an edit session. Returns false if one was already open.
    pub fn open_edit(&mut self, id: &str) -> Result<bool, SessionError> {
        if self.is_editing(id) {
            return Ok(false);
        }
        if Session::is_new(id) {
            return Err(SessionError::NoSuchPreset);
        }
        self.ensure_saved(id)?;
        let live = self.saved[id].clone();
        self.live.insert(id.to_string(), live);
        Ok(true)
    }

    /// End an edit session, discarding unsaved changes. An unsaved new preset is discarded
    /// entirely.
    pub fn close_edit(&mut self, id: &str) {
        self.live.remove(id);
        if Session::is_new(id) {
            self.loaded.retain(|l| l.id != id);
        }
        self.release(id);
    }

    /// Whether the live state differs from the saved one. Unsaved new presets are always dirty.
    pub fn is_dirty(&self, id: &str) -> bool {
        match (self.live.get(id), self.saved.get(id)) {
            (Some(live), Some(saved)) => live.to_preset() != saved.to_preset(),
            (Some(_), None) => true,
            _ => false,
        }
    }

    /// Set the live state's name; it is stored, renaming the file, on the next save. Fails if
    /// the name is empty or another stored preset already has it.
    pub fn set_name(&mut self, id: &str, name: &str) -> Result<(), SessionError> {
        let name = name.trim();
        let new_id = slug(name);
        if new_id.is_empty() {
            return Err(SessionError::EmptyName);
        }
        if new_id != id && self.store.exists(&new_id) {
            return Err(PresetError::NameTaken(name.to_string()).into());
        }
        self.live_mut(id)?.name = name.to_string();
        Ok(())
    }

    pub fn live_filters(&self, id: &str) -> &[TrackedFilter] {
        self.live.get(id).map_or(&[], |p| &p.filters)
    }

    fn live_mut(&mut self, id: &str) -> Result<&mut TrackedPreset, SessionError> {
        self.live.get_mut(id).ok_or(SessionError::NotEditing)
    }

    pub fn add_filter(&mut self, id: &str, filter: Filter) -> Result<usize, SessionError> {
        if self.is_loaded(id) && self.filter_count(1) > MAX_SLOTS {
            return Err(SessionError::TooManyFilters);
        }
        let uid = self.uid();
        let live = self.live_mut(id)?;
        // The lowest color no other filter in the preset uses.
        let color = (0..).find(|c| live.filters.iter().all(|f| f.color != *c)).unwrap();
        live.filters.push(TrackedFilter { uid, color, filter: filter.clamped() });
        Ok(live.filters.len() - 1)
    }

    pub fn update_filter(&mut self, id: &str, index: usize, filter: Filter) -> Result<(), SessionError> {
        let live = self.live_mut(id)?;
        if let Some(f) = live.filters.get_mut(index) {
            f.filter = filter.clamped();
        }
        Ok(())
    }

    pub fn remove_filter(&mut self, id: &str, index: usize) -> Result<(), SessionError> {
        let live = self.live_mut(id)?;
        if index < live.filters.len() {
            live.filters.remove(index);
        }
        Ok(())
    }

    pub fn move_filter(&mut self, id: &str, from: usize, to: usize) -> Result<(), SessionError> {
        let live = self.live_mut(id)?;
        if from < live.filters.len() && to < live.filters.len() {
            let f = live.filters.remove(from);
            live.filters.insert(to, f);
        }
        Ok(())
    }

    /// Sort by frequency. Returns the new order as old indices.
    pub fn sort_filters(&mut self, id: &str) -> Result<Vec<usize>, SessionError> {
        let live = self.live_mut(id)?;
        let mut order: Vec<usize> = (0..live.filters.len()).collect();
        order.sort_by(|&a, &b| live.filters[a].filter.frequency.total_cmp(&live.filters[b].filter.frequency));
        live.filters = order.iter().map(|&i| live.filters[i]).collect();
        Ok(order)
    }

    /// Write the live state to disk. If its name changed (or the preset is new) it is stored
    /// under the new name, and the preset follows it in the loaded list and edit session.
    /// Returns the (possibly new) id.
    pub fn save(&mut self, id: &str) -> Result<String, SessionError> {
        let live = self.live.get(id).ok_or(SessionError::NotEditing)?.clone();
        let new_id = slug(&live.name);
        if new_id.is_empty() {
            return Err(SessionError::EmptyName);
        }
        if new_id == id {
            self.store.save(id, &live.to_preset())?;
            self.saved.insert(id.to_string(), live);
            return Ok(new_id);
        }
        self.store.create(&live.to_preset())?;
        if !Session::is_new(id) {
            self.store.delete(id)?;
        }
        self.saved.remove(id);
        self.live.remove(id);
        self.saved.insert(new_id.clone(), live.clone());
        self.live.insert(new_id.clone(), live);
        if let Some(l) = self.loaded.iter_mut().find(|l| l.id == id) {
            l.id = new_id.clone();
        }
        Ok(new_id)
    }

    /// Save the live state as a new preset, which the edit session then continues on. The
    /// original reverts to its saved state and, if loaded, is replaced by the new preset.
    /// Returns the new id.
    pub fn save_as(&mut self, id: &str, name: &str) -> Result<String, SessionError> {
        let mut live = self.live.get(id).ok_or(SessionError::NotEditing)?.clone();
        live.name = name.trim().to_string();
        let new_id = self.store.create(&live.to_preset())?;
        self.live.remove(id);
        self.saved.insert(new_id.clone(), live.clone());
        self.live.insert(new_id.clone(), live);
        if let Some(l) = self.loaded.iter_mut().find(|l| l.id == id) {
            l.id = new_id.clone();
        }
        self.release(id);
        Ok(new_id)
    }

    /// Create an empty preset, load it and open it for editing. It exists only as a live
    /// state until it is saved. Returns its temporary id.
    pub fn create(&mut self) -> Result<String, SessionError> {
        let taken: Vec<String> = self.live.values().map(|p| slug(&p.name)).collect();
        let name = (1..)
            .map(|n| if n == 1 { "New preset".to_string() } else { format!("New preset {n}") })
            .find(|n| self.store.check_new_name(n).is_ok() && !taken.contains(&slug(n)))
            .unwrap();
        let id = format!("{NEW_PREFIX}{}", self.uid());
        self.live.insert(id.clone(), TrackedPreset { name, filters: Vec::new() });
        if let Err(e) = self.load(&id) {
            self.live.remove(&id);
            return Err(e);
        }
        Ok(id)
    }

    /// Rename a stored preset, following it in the loaded list and edit session. Returns the
    /// new id. A live state keeps its unsaved changes but takes the new name.
    pub fn rename(&mut self, id: &str, new_name: &str) -> Result<String, SessionError> {
        let new_id = self.store.rename(id, new_name)?;
        let name = new_name.trim().to_string();
        for map in [&mut self.saved, &mut self.live] {
            if let Some(mut p) = map.remove(id) {
                p.name = name.clone();
                map.insert(new_id.clone(), p);
            }
        }
        if let Some(l) = self.loaded.iter_mut().find(|l| l.id == id) {
            l.id = new_id.clone();
        }
        Ok(new_id)
    }

    /// Delete a stored preset, unloading it and ending its edit session.
    pub fn delete(&mut self, id: &str) -> Result<(), SessionError> {
        self.store.delete(id)?;
        self.loaded.retain(|l| l.id != id);
        self.live.remove(id);
        self.saved.remove(id);
        Ok(())
    }

    pub fn import(&mut self, path: &Path, name: Option<&str>, replace: bool) -> Result<String, SessionError> {
        let id = self.store.import(path, name, replace)?;
        // A replaced preset that is loaded now has a new saved state.
        if self.saved.contains_key(&id) {
            let preset = self.store.load(&id)?;
            let tracked = self.track(preset);
            self.saved.insert(id.clone(), tracked);
        }
        Ok(id)
    }

    // The resulting EQ.

    /// Every filter that is currently heard.
    pub fn active_filters(&self) -> Vec<Filter> {
        self.loaded
            .iter()
            .filter(|l| l.active)
            .filter_map(|l| self.current(&l.id))
            .flat_map(|p| p.filters.iter().map(|f| f.filter))
            .filter(|f| f.enabled)
            .collect()
    }

    pub fn headroom(&self) -> Headroom {
        Headroom::analyse(&self.active_filters(), self.sample_rate)
    }

    pub fn preamp_db(&self) -> f64 {
        self.headroom().preamp_db(self.preamp_mode, self.manual_preamp_db)
    }

    /// The DSP parameters for the current state.
    pub fn params(&mut self) -> EqParams {
        let mut filters = Vec::new();
        for l in &self.loaded {
            if let Some(p) = self.current(&l.id) {
                filters.extend(p.filters.iter().map(|f| (f.uid, f.filter.enabled && l.active, f.filter)));
            }
        }
        let uids: Vec<u64> = filters.iter().map(|f| f.0).collect();
        let slots = self.allocator.assign(&uids);
        let mut params = EqParams { preamp_db: self.preamp_db(), bypass: self.bypass, ..EqParams::default() };
        for (slot, (_, active, f)) in slots.into_iter().zip(filters) {
            if let Some(slot) = slot {
                params.slots[slot] = SlotParams {
                    active,
                    kind: f.kind,
                    channel: f.channel,
                    frequency: clamp_frequency(f.frequency, self.sample_rate),
                    gain: f.gain,
                    q: f.q,
                };
            }
        }
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let store = PresetStore::new(dir.path().to_path_buf());
        store
            .create(&Preset { name: "Tone".into(), filters: vec![Filter { gain: 3.0, ..Filter::default() }] })
            .unwrap();
        let state = AppState {
            loaded: vec![
                LoadedEntry { id: "tone".into(), active: true },
                LoadedEntry { id: "missing".into(), active: true },
            ],
            ..AppState::default()
        };
        let (session, errors) = Session::new(store, &state);
        assert_eq!(errors.len(), 1);
        (dir, session)
    }

    fn active_slots(p: &EqParams) -> Vec<f64> {
        p.slots.iter().filter(|s| s.active).map(|s| s.gain).collect()
    }

    #[test]
    fn live_state_is_heard_and_discard_reverts() {
        let (_d, mut s) = setup();
        assert_eq!(active_slots(&s.params()), vec![3.0]);
        assert!(s.open_edit("tone").unwrap());
        assert!(!s.open_edit("tone").unwrap());
        s.update_filter("tone", 0, Filter { gain: -2.0, ..Filter::default() }).unwrap();
        assert!(s.is_dirty("tone"));
        let p = s.params();
        assert_eq!(active_slots(&p), vec![-2.0]);
        s.close_edit("tone");
        assert_eq!(active_slots(&s.params()), vec![3.0]);
        assert_eq!(s.store().load("tone").unwrap().filters[0].gain, 3.0);
    }

    #[test]
    fn edits_keep_their_slot_and_save_persists() {
        let (_d, mut s) = setup();
        let before = s.params();
        let slot = before.slots.iter().position(|x| x.active).unwrap();
        s.open_edit("tone").unwrap();
        s.update_filter("tone", 0, Filter { gain: 5.0, ..Filter::default() }).unwrap();
        assert_eq!(s.params().slots[slot].gain, 5.0);
        assert_eq!(s.save("tone").unwrap(), "tone");
        assert!(!s.is_dirty("tone"));
        s.close_edit("tone");
        // Saving keeps the filter identity, so closing the window changes nothing audible.
        assert_eq!(s.params().slots[slot].gain, 5.0);
        assert_eq!(s.store().load("tone").unwrap().filters[0].gain, 5.0);
    }

    #[test]
    fn save_as_replaces_loaded_preset() {
        let (_d, mut s) = setup();
        s.open_edit("tone").unwrap();
        s.add_filter("tone", Filter { frequency: 8000.0, gain: -6.0, ..Filter::default() }).unwrap();
        let new_id = s.save_as("tone", "Tone v2").unwrap();
        assert_eq!(new_id, "tone-v2");
        assert_eq!(s.loaded()[0].id, "tone-v2");
        assert!(s.is_editing("tone-v2") && !s.is_editing("tone"));
        assert_eq!(active_slots(&s.params()).len(), 2);
        assert_eq!(s.store().load("tone").unwrap().filters.len(), 1);
    }

    #[test]
    fn rename_delete_and_inactive() {
        let (_d, mut s) = setup();
        s.open_edit("tone").unwrap();
        let id = s.rename("tone", "Warm").unwrap();
        assert_eq!(s.name(&id), Some("Warm"));
        assert!(s.is_editing(&id) && s.is_loaded(&id));
        s.set_active(&id, false);
        assert!(active_slots(&s.params()).is_empty());
        assert_eq!(s.headroom().peak_db, 0.0);
        s.delete(&id).unwrap();
        assert!(s.loaded().is_empty() && !s.is_editing(&id));
    }

    #[test]
    fn colors_follow_filters() {
        let (_d, mut s) = setup();
        s.open_edit("tone").unwrap();
        s.add_filter("tone", Filter { frequency: 50.0, ..Filter::default() }).unwrap();
        s.add_filter("tone", Filter { frequency: 5000.0, ..Filter::default() }).unwrap();
        let colors = |s: &Session| s.live_filters("tone").iter().map(|f| f.color).collect::<Vec<_>>();
        assert_eq!(colors(&s), vec![0, 1, 2]);
        s.sort_filters("tone").unwrap();
        assert_eq!(colors(&s), vec![1, 0, 2]);
        s.remove_filter("tone", 0).unwrap();
        s.add_filter("tone", Filter::default()).unwrap();
        assert_eq!(colors(&s), vec![0, 2, 1]);
    }

    #[test]
    fn new_presets_are_ephemeral_until_saved() {
        let (_d, mut s) = setup();
        let created = s.create().unwrap();
        assert!(s.is_loaded(&created) && s.is_editing(&created) && s.is_dirty(&created));
        assert_eq!(s.name(&created), Some("New preset"));
        assert_eq!(s.store().list().0.len(), 1);
        assert!(s.state().loaded.iter().all(|l| l.id == "tone"));
        // Discarding removes it entirely.
        s.close_edit(&created);
        assert!(!s.is_loaded(&created) && s.open_ids() == vec!["tone".to_string()]);

        let created = s.create().unwrap();
        assert!(matches!(s.set_name(&created, "tone"), Err(SessionError::Preset(PresetError::NameTaken(_)))));
        assert!(matches!(s.set_name(&created, "  "), Err(SessionError::EmptyName)));
        s.set_name(&created, "Bright").unwrap();
        let id = s.save(&created).unwrap();
        assert_eq!(id, "bright");
        assert!(s.is_loaded("bright") && s.is_editing("bright") && !s.is_dirty("bright"));
        assert_eq!(s.store().load("bright").unwrap().name, "Bright");
    }

    #[test]
    fn renaming_in_the_editor_applies_on_save() {
        let (_d, mut s) = setup();
        s.open_edit("tone").unwrap();
        s.set_name("tone", "Warm").unwrap();
        assert!(s.is_dirty("tone"));
        assert!(s.store().exists("tone"));
        let id = s.save("tone").unwrap();
        assert_eq!(id, "warm");
        assert!(!s.store().exists("tone") && s.store().load("warm").unwrap().name == "Warm");
        assert_eq!(s.loaded()[0].id, "warm");
        // Discarding a rename reverts it.
        s.set_name("warm", "Cold").unwrap();
        s.close_edit("warm");
        assert_eq!(s.name("warm"), Some("Warm"));
    }
}
