//! The QML-facing controller: a thin layer over [`Session`] and the audio [`Backend`].
//!
//! Structured state is passed to QML as JSON strings, which QML parses; this keeps the bridge
//! small. Invokables that can fail report through the `errorOccurred` signal.

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value, json};
use std::pin::Pin;
use std::time::{Duration, Instant};
use tincan_audio::{Backend, Config, Event};
use tincan_core::filter::Filter;
use tincan_core::preset::PresetStore;
use tincan_core::response::{PreampMode, channel_response, filter_response, frequency_grid};
use tincan_core::session::{DiskChanges, Session};
use tincan_core::state::{AbList, AppState, WindowSize, presets_dir, state_file};
use tincan_core::tilt::tilt_response;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, devices)]
        #[qproperty(QString, output_device)]
        #[qproperty(QString, output_status)]
        #[qproperty(bool, bypass)]
        #[qproperty(i32, preamp_mode)]
        #[qproperty(f64, preamp_db)]
        #[qproperty(f64, manual_preamp_db)]
        #[qproperty(f64, excess_db)]
        #[qproperty(bool, clipping)]
        #[qproperty(QString, loaded)]
        /// The loaded list being heard: "A" or "B".
        #[qproperty(QString, ab_list)]
        #[qproperty(QString, presets)]
        #[qproperty(QString, main_curves)]
        #[qproperty(QString, frequencies)]
        #[qproperty(i32, sample_rate)]
        #[qproperty(bool, start_hidden)]
        /// Whether streams are moved onto TinCan (the setting, or forced by --move-streams).
        #[qproperty(bool, move_streams)]
        /// Whether the move-streams setting can be changed (not with --move-streams or --no-route).
        #[qproperty(bool, move_streams_editable)]
        /// Development aid: when set (from TINCAN_GRAB_DIR), QML renders its windows to PNGs there.
        #[qproperty(QString, grab_dir)]
        #[qproperty(i32, main_width)]
        #[qproperty(i32, main_height)]
        #[qproperty(i32, edit_width)]
        #[qproperty(i32, edit_height)]
        type AppController = super::AppControllerRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        /// Start the session and audio backend. Called once from QML.
        #[qinvokable]
        fn start(self: Pin<&mut AppController>);
        /// Periodic housekeeping (clip indicator, preset files changed on disk).
        #[qinvokable]
        fn poll(self: Pin<&mut AppController>);

        #[qinvokable]
        fn set_output(self: Pin<&mut AppController>, name: &QString);
        #[qinvokable]
        fn set_stream_moving(self: Pin<&mut AppController>, enabled: bool);
        #[qinvokable]
        fn set_global_bypass(self: Pin<&mut AppController>, bypass: bool);
        #[qinvokable]
        fn set_preamp(self: Pin<&mut AppController>, mode: i32, manual_db: f64);

        /// Switch to loaded list "A" or "B".
        #[qinvokable]
        fn set_ab(self: Pin<&mut AppController>, list: &QString);
        #[qinvokable]
        fn toggle_ab(self: Pin<&mut AppController>);

        #[qinvokable]
        fn create_preset(self: Pin<&mut AppController>) -> QString;
        #[qinvokable]
        fn load_preset(self: Pin<&mut AppController>, id: &QString);
        #[qinvokable]
        fn unload_preset(self: Pin<&mut AppController>, id: &QString);
        #[qinvokable]
        fn set_preset_active(self: Pin<&mut AppController>, id: &QString, active: bool);
        #[qinvokable]
        fn rename_preset(self: Pin<&mut AppController>, id: &QString, name: &QString) -> QString;
        #[qinvokable]
        fn delete_preset(self: Pin<&mut AppController>, id: &QString) -> bool;
        /// Returns JSON `{ "id": .. }`, `{ "nameTaken": name }` or `{ "error": .. }`.
        #[qinvokable]
        fn import_preset(self: Pin<&mut AppController>, path: &QString, name: &QString, replace: bool) -> QString;
        #[qinvokable]
        fn export_preset(self: Pin<&mut AppController>, id: &QString, path: &QString) -> bool;

        /// Start editing. Emits `editorRequested` either way; QML focuses an existing window.
        #[qinvokable]
        fn open_editor(self: Pin<&mut AppController>, id: &QString);
        /// End an edit session, discarding unsaved changes.
        #[qinvokable]
        fn close_editor(self: Pin<&mut AppController>, id: &QString);
        /// JSON description of a preset's live state, for its edit window.
        #[qinvokable]
        fn editor_state(self: &AppController, id: &QString) -> QString;
        #[qinvokable]
        fn add_filter(self: Pin<&mut AppController>, id: &QString, filter_json: &QString) -> i32;
        #[qinvokable]
        fn update_filter(self: Pin<&mut AppController>, id: &QString, index: i32, filter_json: &QString);
        #[qinvokable]
        fn remove_filter(self: Pin<&mut AppController>, id: &QString, index: i32);
        #[qinvokable]
        fn move_filter(self: Pin<&mut AppController>, id: &QString, from: i32, to: i32);
        #[qinvokable]
        fn sort_filters(self: Pin<&mut AppController>, id: &QString);
        /// Set a preset's tilt, in dB/octave.
        #[qinvokable]
        fn set_tilt(self: Pin<&mut AppController>, id: &QString, tilt: f64);
        /// Set the live state's name (stored on the next save). Returns false if it's refused.
        #[qinvokable]
        fn set_preset_name(self: Pin<&mut AppController>, id: &QString, name: &QString) -> bool;
        /// Returns the preset's id afterwards (it changes with the name), or an empty string on
        /// failure.
        #[qinvokable]
        fn save_preset(self: Pin<&mut AppController>, id: &QString) -> QString;
        /// Returns the new id, or an empty string on failure.
        #[qinvokable]
        fn save_preset_as(self: Pin<&mut AppController>, id: &QString, name: &QString) -> QString;
        /// JSON list of `{ id, name }` for presets with unsaved changes.
        #[qinvokable]
        fn dirty_presets(self: &AppController) -> QString;

        #[qinvokable]
        fn set_window_size(self: Pin<&mut AppController>, window: &QString, width: i32, height: i32);
        /// Restore routing, save state and stop audio. QML quits the application afterwards.
        #[qinvokable]
        fn shutdown(self: Pin<&mut AppController>);

        #[qsignal]
        fn error_occurred(self: Pin<&mut AppController>, message: QString);
        /// A preset's edit window should be shown (opened or focused).
        #[qsignal]
        fn editor_requested(self: Pin<&mut AppController>, id: QString);
        /// A preset's state changed; its edit window should refresh.
        #[qsignal]
        fn preset_changed(self: Pin<&mut AppController>, id: QString);
        /// A preset now has a different id (rename or save as); its edit window follows it.
        #[qsignal]
        fn preset_renamed(self: Pin<&mut AppController>, old_id: QString, new_id: QString);
        /// A preset's edit window must close without prompting (the preset was deleted).
        #[qsignal]
        fn editor_closed(self: Pin<&mut AppController>, id: QString);
        /// Another launch of TinCan asked for the main window.
        #[qsignal]
        fn show_requested(self: Pin<&mut AppController>);
    }

    impl cxx_qt::Threading for AppController {}
}

use qobject::AppController;

/// Number of points in plotted curves.
const PLOT_POINTS: usize = 800;
/// How long the clip indicator stays lit.
const CLIP_HOLD: Duration = Duration::from_secs(3);
/// How often the presets folder is checked for changes made by other programs.
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(1);

struct Core {
    session: Session,
    backend: Option<Backend>,
    clip_until: Option<Instant>,
    last_disk_check: Instant,
    state_path: std::path::PathBuf,
}

#[derive(Default)]
pub struct AppControllerRust {
    devices: QString,
    output_device: QString,
    output_status: QString,
    bypass: bool,
    preamp_mode: i32,
    preamp_db: f64,
    manual_preamp_db: f64,
    excess_db: f64,
    clipping: bool,
    loaded: QString,
    ab_list: QString,
    presets: QString,
    main_curves: QString,
    frequencies: QString,
    sample_rate: i32,
    start_hidden: bool,
    move_streams: bool,
    move_streams_editable: bool,
    grab_dir: QString,
    main_width: i32,
    main_height: i32,
    edit_width: i32,
    edit_height: i32,
    core: Option<Core>,
}

fn qs(s: impl AsRef<str>) -> QString {
    QString::from(s.as_ref())
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn curve(values: Vec<f64>) -> Value {
    Value::from(values.into_iter().map(round2).collect::<Vec<_>>())
}

fn mode_from_index(i: i32) -> PreampMode {
    PreampMode::ALL.get(i as usize).copied().unwrap_or_default()
}

fn mode_index(mode: PreampMode) -> i32 {
    PreampMode::ALL.iter().position(|&m| m == mode).unwrap_or(0) as i32
}

impl AppController {
    fn core(&self) -> &Core {
        self.rust().core.as_ref().expect("controller started")
    }

    fn core_mut(self: Pin<&mut Self>) -> &mut Core {
        // Core is plain Rust data that is never pinned itself.
        unsafe { self.rust_mut().get_unchecked_mut() }.core.as_mut().expect("controller started")
    }

    fn session_mut(self: Pin<&mut Self>) -> &mut Session {
        &mut self.core_mut().session
    }

    fn started(&self) -> bool {
        self.rust().core.is_some()
    }

    fn error(self: Pin<&mut Self>, message: impl std::fmt::Display) {
        self.error_occurred(qs(message.to_string()));
    }

    pub fn start(mut self: Pin<&mut Self>) {
        if self.started() {
            return;
        }
        let options = crate::OPTIONS.get().cloned().unwrap_or_default();
        let state_path = state_file();
        let state = AppState::read(&state_path);
        let (session, errors) = Session::new(PresetStore::new(presets_dir()), &state);
        let output = session.output_device.clone();
        let move_streams = options.move_streams || session.move_streams;
        let size = |w: Option<WindowSize>, d: (i32, i32)| w.map_or(d, |w| (w.width, w.height));
        let (mw, mh) = size(state.main_window, (900, 640));
        let (ew, eh) = size(state.edit_window, (1000, 680));
        self.as_mut().set_main_width(mw);
        self.as_mut().set_main_height(mh);
        self.as_mut().set_edit_width(ew);
        self.as_mut().set_edit_height(eh);
        self.as_mut().set_start_hidden(options.hidden);
        self.as_mut().set_move_streams(move_streams && !options.no_route);
        self.as_mut().set_move_streams_editable(!options.move_streams && !options.no_route);
        self.as_mut().set_grab_dir(qs(std::env::var("TINCAN_GRAB_DIR").unwrap_or_default()));
        let grid: Vec<f64> = frequency_grid(PLOT_POINTS).into_iter().map(round2).collect();
        self.as_mut().set_frequencies(qs(json!(grid).to_string()));
        unsafe { self.as_mut().rust_mut().get_unchecked_mut() }.core =
            Some(Core { session, backend: None, clip_until: None, last_disk_check: Instant::now(), state_path });
        for (id, e) in errors {
            self.as_mut().error(format!("Couldn't load preset \"{id}\": {e}"));
        }

        let thread = self.qt_thread();
        crate::set_show_handler(self.qt_thread());
        let params = self.as_mut().session_mut().params();
        let config = Config { route: !options.no_route, move_streams, output };
        match Backend::start(config, &params, move |event| {
            let _ = thread.queue(move |obj| obj.on_backend_event(event));
        }) {
            Ok(backend) => self.as_mut().core_mut().backend = Some(backend),
            Err(e) => self.as_mut().error(format!("Couldn't start audio: {e}")),
        }
        self.as_mut().refresh_all();
    }

    fn on_backend_event(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::Devices(devices) => {
                let list: Vec<Value> =
                    devices.iter().map(|d| json!({ "name": d.name, "description": d.description })).collect();
                self.set_devices(qs(Value::from(list).to_string()));
            }
            Event::Output { device, is_selected } => {
                let status = match (&device, is_selected) {
                    (_, true) => String::new(),
                    (Some(d), false) => format!("Selected device unavailable, using {d}"),
                    (None, false) => "No output device available".into(),
                };
                let selected = self.core().session.output_device.clone();
                let shown = if is_selected { device.clone() } else { selected.or(device) };
                self.as_mut().set_output_device(qs(shown.unwrap_or_default()));
                self.as_mut().set_output_status(qs(status));
            }
            Event::SampleRate(rate) => {
                self.as_mut().session_mut().set_sample_rate(rate as f64);
                self.as_mut().set_sample_rate(rate as i32);
                self.as_mut().apply();
                let ids: Vec<String> = self.loaded_and_editing_ids();
                for id in ids {
                    self.as_mut().preset_changed(qs(id));
                }
            }
            Event::Error(e) => self.error(e),
        }
    }

    fn loaded_and_editing_ids(&self) -> Vec<String> {
        self.core().session.open_ids()
    }

    pub fn poll(mut self: Pin<&mut Self>) {
        if !self.started() {
            return;
        }
        let now = Instant::now();
        let core = self.as_mut().core_mut();
        if core.backend.as_ref().is_some_and(|b| b.take_clipped()) {
            core.clip_until = Some(now + CLIP_HOLD);
        }
        let clipping = core.clip_until.is_some_and(|t| t > now);
        let check_disk = now.duration_since(core.last_disk_check) >= DISK_CHECK_INTERVAL;
        if check_disk {
            core.last_disk_check = now;
        }
        if clipping != *self.clipping() {
            self.as_mut().set_clipping(clipping);
        }
        if check_disk && let Some(changes) = self.as_mut().session_mut().check_disk() {
            self.on_disk_changes(changes);
        }
    }

    /// Follow preset files that other programs changed (DESIGN.md, Storage).
    fn on_disk_changes(mut self: Pin<&mut Self>, changes: DiskChanges) {
        if !changes.removed.is_empty() || !changes.reloaded.is_empty() {
            // Reloaded presets may be loaded in either list, so always reapply.
            self.as_mut().changed();
        } else {
            self.as_mut().refresh_lists();
        }
        for id in &changes.reloaded {
            self.as_mut().preset_changed(qs(id));
        }
        let quote = |names: &[String]| names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
        let was = |names: &[String]| if names.len() == 1 { "was" } else { "were" };
        let mut messages = Vec::new();
        let c = &changes.conflicts;
        if !c.is_empty() {
            messages.push(format!(
                "{} changed on disk while you had unsaved changes. Saving will replace the version on disk; \
                 closing without saving switches to it.",
                quote(c)
            ));
        }
        let r = &changes.removed;
        if !r.is_empty() {
            messages.push(format!("{} {} deleted from the presets folder and unloaded.", quote(r), was(r)));
        }
        let r = &changes.removed_while_editing;
        if !r.is_empty() {
            messages.push(format!(
                "{} {} deleted from the presets folder while open for editing. Save to keep it.",
                quote(r),
                was(r)
            ));
        }
        for (name, e) in &changes.errors {
            messages.push(format!("\"{name}\" changed on disk but couldn't be read, so the previous version is still used: {e}"));
        }
        if !messages.is_empty() {
            self.error(messages.join("\n\n"));
        }
    }

    /// Push the EQ to the backend and refresh the global display.
    fn apply(mut self: Pin<&mut Self>) {
        let core = self.as_mut().core_mut();
        let params = core.session.params();
        if let Some(b) = core.backend.as_mut() {
            b.set_params(&params);
        }
        let s = &self.core().session;
        let headroom = s.headroom();
        let preamp = headroom.preamp_db(s.preamp_mode, s.manual_preamp_db);
        let excess = headroom.excess_db(preamp).unwrap_or(0.0);
        let (filters, tilt) = (s.active_filters(), s.active_tilt());
        let grid = frequency_grid(PLOT_POINTS);
        let left = channel_response(&filters, tilt, 0, &grid, s.sample_rate());
        let right = channel_response(&filters, tilt, 1, &grid, s.sample_rate());
        let curves = json!({ "same": left == right, "left": curve(left), "right": curve(right) });
        let bypass = s.bypass;
        self.as_mut().set_preamp_db(round2(preamp));
        self.as_mut().set_excess_db(round2(excess));
        self.as_mut().set_bypass(bypass);
        self.as_mut().set_main_curves(qs(curves.to_string()));
    }

    fn refresh_lists(mut self: Pin<&mut Self>) {
        let s = &self.core().session;
        let loaded: Vec<Value> = s
            .loaded()
            .iter()
            .map(|l| {
                json!({
                    "id": l.id,
                    "name": s.name(&l.id).unwrap_or(&l.id),
                    "active": l.active,
                    "editing": s.is_editing(&l.id),
                    "dirty": s.is_dirty(&l.id),
                    "isNew": Session::is_new(&l.id),
                })
            })
            .collect();
        let (all, _) = s.store().list();
        let presets: Vec<Value> = all
            .iter()
            .map(|(id, p)| json!({ "id": id, "name": p.name, "loaded": s.is_loaded(id), "editing": s.is_editing(id) }))
            .collect();
        let (mode, manual) = (mode_index(s.preamp_mode), s.manual_preamp_db);
        let ab = s.ab().label();
        self.as_mut().set_loaded(qs(Value::from(loaded).to_string()));
        self.as_mut().set_ab_list(qs(ab));
        self.as_mut().set_presets(qs(Value::from(presets).to_string()));
        self.as_mut().set_preamp_mode(mode);
        self.as_mut().set_manual_preamp_db(manual);
    }

    fn refresh_all(mut self: Pin<&mut Self>) {
        self.as_mut().apply();
        self.as_mut().refresh_lists();
    }

    fn persist(mut self: Pin<&mut Self>) {
        let mut state = self.core().session.state();
        let size = |w: i32, h: i32| (w > 0 && h > 0).then_some(WindowSize { width: w, height: h });
        state.main_window = size(*self.main_width(), *self.main_height());
        state.edit_window = size(*self.edit_width(), *self.edit_height());
        if let Err(e) = state.write(&self.core().state_path) {
            self.as_mut().error(format!("Couldn't save settings: {e}"));
        }
    }

    /// After a change to loaded presets or global settings.
    fn changed(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_all();
        self.persist();
    }

    /// After an edit to a preset's live state.
    fn edited(mut self: Pin<&mut Self>, id: &str) {
        if self.core().session.is_loaded(id) {
            self.as_mut().apply();
        }
        self.as_mut().refresh_lists();
        self.preset_changed(qs(id));
    }

    pub fn set_output(mut self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string();
        let device = (!name.is_empty()).then_some(name);
        self.as_mut().session_mut().output_device = device.clone();
        if let Some(b) = &self.core().backend {
            b.set_output(device);
        }
        self.persist();
    }

    pub fn set_stream_moving(mut self: Pin<&mut Self>, enabled: bool) {
        if !*self.move_streams_editable() {
            return;
        }
        self.as_mut().session_mut().move_streams = enabled;
        self.as_mut().set_move_streams(enabled);
        if let Some(b) = &self.core().backend {
            b.set_move_streams(enabled);
        }
        self.persist();
    }

    pub fn set_global_bypass(mut self: Pin<&mut Self>, bypass: bool) {
        self.as_mut().session_mut().bypass = bypass;
        self.changed();
    }

    pub fn set_preamp(mut self: Pin<&mut Self>, mode: i32, manual_db: f64) {
        let s = self.as_mut().session_mut();
        s.preamp_mode = mode_from_index(mode);
        if manual_db.is_finite() {
            s.manual_preamp_db = manual_db.clamp(-48.0, 24.0);
        }
        self.changed();
    }

    pub fn set_ab(mut self: Pin<&mut Self>, list: &QString) {
        let list = if list.to_string() == "B" { AbList::B } else { AbList::A };
        if list == self.core().session.ab() {
            return;
        }
        self.as_mut().session_mut().set_ab(list);
        self.as_mut().changed();
        // Edit windows show whether their preset is loaded and active in this list.
        for id in self.core().session.open_ids() {
            self.as_mut().preset_changed(qs(id));
        }
    }

    pub fn toggle_ab(self: Pin<&mut Self>) {
        let other = self.core().session.ab().other();
        self.set_ab(&qs(other.label()));
    }

    pub fn create_preset(mut self: Pin<&mut Self>) -> QString {
        match self.as_mut().session_mut().create() {
            Ok(id) => {
                self.as_mut().changed();
                self.as_mut().editor_requested(qs(&id));
                qs(id)
            }
            Err(e) => {
                self.error(e);
                QString::default()
            }
        }
    }

    pub fn load_preset(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        if let Err(e) = self.as_mut().session_mut().load(&id) {
            self.as_mut().error(e);
        }
        self.as_mut().changed();
        self.preset_changed(qs(id));
    }

    pub fn unload_preset(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        self.as_mut().session_mut().unload(&id);
        self.as_mut().changed();
        self.preset_changed(qs(id));
    }

    pub fn set_preset_active(mut self: Pin<&mut Self>, id: &QString, active: bool) {
        let id = id.to_string();
        self.as_mut().session_mut().set_active(&id, active);
        self.as_mut().changed();
        self.preset_changed(qs(id));
    }

    pub fn rename_preset(mut self: Pin<&mut Self>, id: &QString, name: &QString) -> QString {
        let id = id.to_string();
        match self.as_mut().session_mut().rename(&id, &name.to_string()) {
            Ok(new_id) => {
                self.as_mut().changed();
                if new_id != id {
                    self.as_mut().preset_renamed(qs(&id), qs(&new_id));
                }
                self.as_mut().preset_changed(qs(&new_id));
                qs(new_id)
            }
            Err(e) => {
                self.error(e);
                QString::default()
            }
        }
    }

    pub fn delete_preset(mut self: Pin<&mut Self>, id: &QString) -> bool {
        let id = id.to_string();
        match self.as_mut().session_mut().delete(&id) {
            Ok(()) => {
                self.as_mut().changed();
                self.editor_closed(qs(id));
                true
            }
            Err(e) => {
                self.error(e);
                false
            }
        }
    }

    pub fn import_preset(mut self: Pin<&mut Self>, path: &QString, name: &QString, replace: bool) -> QString {
        let path = std::path::PathBuf::from(local_path(&path.to_string()));
        let name = name.to_string();
        let name = (!name.trim().is_empty()).then_some(name);
        let result = self.as_mut().session_mut().import(&path, name.as_deref(), replace);
        let reply = match result {
            Ok(id) => {
                self.as_mut().changed();
                self.as_mut().preset_changed(qs(&id));
                json!({ "id": id })
            }
            Err(tincan_core::session::SessionError::Preset(tincan_core::preset::PresetError::NameTaken(n))) => {
                json!({ "nameTaken": n })
            }
            Err(e) => json!({ "error": e.to_string() }),
        };
        qs(reply.to_string())
    }

    pub fn export_preset(self: Pin<&mut Self>, id: &QString, path: &QString) -> bool {
        let path = std::path::PathBuf::from(local_path(&path.to_string()));
        match self.core().session.store().export(&id.to_string(), &path) {
            Ok(()) => true,
            Err(e) => {
                self.error(format!("Couldn't export: {e}"));
                false
            }
        }
    }

    pub fn open_editor(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        match self.as_mut().session_mut().open_edit(&id) {
            Ok(_) => {
                self.as_mut().refresh_lists();
                self.editor_requested(qs(id));
            }
            Err(e) => self.error(e),
        }
    }

    pub fn close_editor(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        let was_loaded = self.core().session.is_loaded(&id);
        self.as_mut().session_mut().close_edit(&id);
        if was_loaded {
            self.as_mut().apply();
        }
        self.refresh_lists();
    }

    pub fn editor_state(&self, id: &QString) -> QString {
        let id = id.to_string();
        let s = &self.core().session;
        if !s.is_editing(&id) {
            return QString::default();
        }
        let filters: Vec<Filter> = s.live_filters(&id).iter().map(|f| f.filter).collect();
        let colors: Vec<usize> = s.live_filters(&id).iter().map(|f| f.color).collect();
        let tilt = s.live_tilt(&id);
        let grid = frequency_grid(PLOT_POINTS);
        let rate = s.sample_rate();
        let left = channel_response(&filters, tilt, 0, &grid, rate);
        let right = channel_response(&filters, tilt, 1, &grid, rate);
        let individual: Vec<Value> = filters.iter().map(|f| curve(filter_response(f, &grid, rate))).collect();
        let tilt_curve = if tilt == 0.0 { Value::Null } else { curve(tilt_response(tilt, &grid, rate)) };
        let state = json!({
            "id": id,
            "name": s.name(&id).unwrap_or(&id),
            "loaded": s.is_loaded(&id),
            "active": s.is_active(&id),
            "dirty": s.is_dirty(&id),
            "isNew": Session::is_new(&id),
            "tilt": tilt,
            "filters": filters,
            "colors": colors,
            "curves": {
                "same": left == right,
                "left": curve(left),
                "right": curve(right),
                "filters": individual,
                "tilt": tilt_curve,
            },
        });
        qs(state.to_string())
    }

    fn parse_filter(self: Pin<&mut Self>, json: &QString) -> Option<Filter> {
        match serde_json::from_str::<Filter>(&json.to_string()) {
            Ok(f) => Some(f),
            Err(e) => {
                self.error(format!("Invalid filter: {e}"));
                None
            }
        }
    }

    pub fn add_filter(mut self: Pin<&mut Self>, id: &QString, filter_json: &QString) -> i32 {
        let Some(filter) = self.as_mut().parse_filter(filter_json) else { return -1 };
        let id = id.to_string();
        match self.as_mut().session_mut().add_filter(&id, filter) {
            Ok(index) => {
                self.edited(&id);
                index as i32
            }
            Err(e) => {
                self.error(e);
                -1
            }
        }
    }

    pub fn update_filter(mut self: Pin<&mut Self>, id: &QString, index: i32, filter_json: &QString) {
        let Some(filter) = self.as_mut().parse_filter(filter_json) else { return };
        let id = id.to_string();
        if let Err(e) = self.as_mut().session_mut().update_filter(&id, index.max(0) as usize, filter) {
            return self.error(e);
        }
        self.edited(&id);
    }

    pub fn remove_filter(mut self: Pin<&mut Self>, id: &QString, index: i32) {
        let id = id.to_string();
        if let Err(e) = self.as_mut().session_mut().remove_filter(&id, index.max(0) as usize) {
            return self.error(e);
        }
        self.edited(&id);
    }

    pub fn move_filter(mut self: Pin<&mut Self>, id: &QString, from: i32, to: i32) {
        let id = id.to_string();
        if let Err(e) = self.as_mut().session_mut().move_filter(&id, from.max(0) as usize, to.max(0) as usize) {
            return self.error(e);
        }
        self.edited(&id);
    }

    pub fn sort_filters(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        if let Err(e) = self.as_mut().session_mut().sort_filters(&id) {
            return self.error(e);
        }
        self.edited(&id);
    }

    pub fn set_tilt(mut self: Pin<&mut Self>, id: &QString, tilt: f64) {
        let id = id.to_string();
        if let Err(e) = self.as_mut().session_mut().set_tilt(&id, tilt) {
            return self.error(e);
        }
        self.edited(&id);
    }

    pub fn set_preset_name(mut self: Pin<&mut Self>, id: &QString, name: &QString) -> bool {
        let id = id.to_string();
        match self.as_mut().session_mut().set_name(&id, &name.to_string()) {
            Ok(()) => {
                self.edited(&id);
                true
            }
            Err(e) => {
                self.error(format!("Couldn't rename: {e}"));
                false
            }
        }
    }

    pub fn save_preset(mut self: Pin<&mut Self>, id: &QString) -> QString {
        let id = id.to_string();
        match self.as_mut().session_mut().save(&id) {
            Ok(new_id) => {
                self.as_mut().changed();
                if new_id != id {
                    self.as_mut().preset_renamed(qs(&id), qs(&new_id));
                }
                self.as_mut().preset_changed(qs(&new_id));
                qs(new_id)
            }
            Err(e) => {
                self.error(format!("Couldn't save: {e}"));
                QString::default()
            }
        }
    }

    pub fn save_preset_as(mut self: Pin<&mut Self>, id: &QString, name: &QString) -> QString {
        let id = id.to_string();
        match self.as_mut().session_mut().save_as(&id, &name.to_string()) {
            Ok(new_id) => {
                self.as_mut().changed();
                self.as_mut().preset_renamed(qs(&id), qs(&new_id));
                self.as_mut().preset_changed(qs(&new_id));
                qs(new_id)
            }
            Err(e) => {
                self.error(format!("Couldn't save: {e}"));
                QString::default()
            }
        }
    }

    pub fn dirty_presets(&self) -> QString {
        let s = &self.core().session;
        let dirty: Vec<Value> = self
            .loaded_and_editing_ids()
            .into_iter()
            .filter(|id| s.is_dirty(id))
            .map(|id| json!({ "id": id, "name": s.name(&id).unwrap_or(&id) }))
            .collect();
        qs(Value::from(dirty).to_string())
    }

    pub fn set_window_size(mut self: Pin<&mut Self>, window: &QString, width: i32, height: i32) {
        if window.to_string() == "main" {
            self.as_mut().set_main_width(width);
            self.as_mut().set_main_height(height);
        } else {
            self.as_mut().set_edit_width(width);
            self.as_mut().set_edit_height(height);
        }
    }

    pub fn shutdown(mut self: Pin<&mut Self>) {
        if !self.started() {
            return;
        }
        self.as_mut().persist();
        if let Some(backend) = self.as_mut().core_mut().backend.take() {
            backend.shutdown();
        }
    }
}

/// Accept either a path or a `file://` URL (as QML file dialogs give).
fn local_path(s: &str) -> String {
    match s.strip_prefix("file://") {
        Some(rest) => percent_decode(rest),
        None => s.to_string(),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_paths() {
        assert_eq!(super::local_path("file:///home/a%20b/x.toml"), "/home/a b/x.toml");
        assert_eq!(super::local_path("/tmp/x.toml"), "/tmp/x.toml");
    }
}
