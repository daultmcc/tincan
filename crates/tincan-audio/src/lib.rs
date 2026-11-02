//! PipeWire backend (DESIGN.md, Audio routing).
//!
//! TinCan's sink is a pair of linked streams, as in PipeWire's own loopback and filter-chain
//! modules: a capture stream published as an `Audio/Sink` node named "tincan", and a playback
//! stream that outputs to the selected device. Both are scheduled in the same graph cycle; the
//! playback stream does the processing. Default-sink changes and stream moves go through the
//! "default" metadata, which the session manager acts on.
//!
//! The PipeWire objects live on a dedicated thread running a PipeWire main loop; the UI talks
//! to it through [`Backend`].

use pipewire as pw;
use pw::metadata::Metadata;
use pw::properties::properties;
use pw::spa;
use spa::pod::Pod;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use tincan_core::dsp::{EqParams, Processor};

pub const SINK_NAME: &str = "tincan";
const OUTPUT_NAME: &str = "tincan.output";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// PipeWire node name.
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The output devices available (every sink except TinCan's).
    Devices(Vec<Device>),
    /// The device TinCan is outputting to, and whether it is the selected one (rather than
    /// a fallback because the selected one is missing).
    Output { device: Option<String>, is_selected: bool },
    SampleRate(u32),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Make TinCan the default sink and move playing streams onto it.
    pub route: bool,
    /// Node name of the selected output device; `None` uses the default sink at startup.
    pub output: Option<String>,
}

enum Command {
    SetOutput(Option<String>),
    Quit,
}

/// Handle to the backend thread. Dropping it shuts the backend down, restoring routing.
pub struct Backend {
    commands: pw::channel::Sender<Command>,
    params: triple_buffer::Input<EqParams>,
    clipped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Start the backend. `events` is called on the backend thread.
    pub fn start(
        config: Config,
        params: &EqParams,
        events: impl Fn(Event) + Send + 'static,
    ) -> Result<Backend, String> {
        let (params_in, params_out) = triple_buffer::triple_buffer(params);
        let clipped = Arc::new(AtomicBool::new(false));
        let (commands, receiver) = pw::channel::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_clipped = clipped.clone();
        let thread = std::thread::Builder::new()
            .name("tincan-pipewire".into())
            .spawn(move || {
                let events: Rc<dyn Fn(Event)> = Rc::new(events);
                if let Err(e) = run(config, params_out, thread_clipped, receiver, events, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("the PipeWire thread exited unexpectedly".into()),
        }
        Ok(Backend { commands, params: params_in, clipped, thread: Some(thread) })
    }

    pub fn set_params(&mut self, params: &EqParams) {
        self.params.write(*params);
    }

    pub fn set_output(&self, device: Option<String>) {
        let _ = self.commands.send(Command::SetOutput(device));
    }

    /// Whether the output went above full scale since the last call.
    pub fn take_clipped(&self) -> bool {
        self.clipped.swap(false, Ordering::Relaxed)
    }

    /// Restore routing and stop.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.commands.send(Command::Quit);
            let _ = thread.join();
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Debug)]
struct NodeInfo {
    name: String,
    description: String,
    class: String,
    link_group: bool,
    dont_move: bool,
}

/// Main-loop-side state.
struct Inner {
    route: bool,
    selected: Option<String>,
    nodes: BTreeMap<u32, NodeInfo>,
    metadata: Option<Metadata>,
    _metadata_listener: Option<pw::metadata::MetadataListener>,
    /// Current values of the default metadata: (subject, key) -> (type, value).
    meta_values: HashMap<(u32, String), (Option<String>, String)>,
    /// Whether the initial registry and metadata contents have arrived.
    synced: bool,
    routed: bool,
    /// The configured default sink before TinCan replaced it (type, value), if any.
    previous_default: Option<(Option<String>, String)>,
    /// Streams moved onto TinCan, with their previous target.object metadata.
    moved: HashMap<u32, Option<(Option<String>, String)>>,
    output_target: Option<String>,
    last_output: Option<(Option<String>, bool)>,
    last_devices: Vec<Device>,
    events: Rc<dyn Fn(Event)>,
}

const DEFAULT_SINK_KEY: &str = "default.configured.audio.sink";
const TARGET_KEY: &str = "target.object";

impl Inner {
    fn our_output_id(&self) -> Option<u32> {
        self.nodes.iter().find(|(_, n)| n.name == OUTPUT_NAME).map(|(&id, _)| id)
    }

    fn sinks(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self
            .nodes
            .values()
            .filter(|n| n.class == "Audio/Sink" && n.name != SINK_NAME)
            .map(|n| Device { name: n.name.clone(), description: n.description.clone() })
            .collect();
        devices.sort_by(|a, b| a.description.cmp(&b.description));
        devices
    }

    fn has_sink(&self, name: &str) -> bool {
        name != SINK_NAME && self.nodes.values().any(|n| n.class == "Audio/Sink" && n.name == name)
    }

    fn set_meta(&mut self, subject: u32, key: &str, value: Option<(Option<String>, String)>) {
        if let Some(m) = &self.metadata {
            match &value {
                Some((type_, v)) => m.set_property(subject, key, type_.as_deref(), Some(v)),
                None => m.set_property(subject, key, None, None),
            }
        }
    }

    /// Name of the configured default sink before TinCan, if it still exists.
    fn previous_default_sink(&self) -> Option<String> {
        let value = if self.routed {
            self.previous_default.as_ref().map(|(_, v)| v.clone())
        } else {
            self.meta_values.get(&(0, DEFAULT_SINK_KEY.to_string())).map(|(_, v)| v.clone())
        };
        value.and_then(|v| json_name(&v)).filter(|n| self.has_sink(n))
    }

    /// Point our output at the selected device, or a fallback when it's missing.
    fn update_output(&mut self) {
        if !self.synced {
            return;
        }
        let selected_present = self.selected.as_deref().is_some_and(|s| self.has_sink(s));
        let target = if selected_present {
            self.selected.clone()
        } else {
            self.previous_default_sink().or_else(|| self.sinks().first().map(|d| d.name.clone()))
        };
        if target != self.output_target
            && let Some(id) = self.our_output_id() {
                let value = target.clone().map(|t| (None, t));
                self.set_meta(id, TARGET_KEY, value);
                self.output_target = target.clone();
            }
        let output = (target, selected_present);
        if self.last_output.as_ref() != Some(&output) {
            self.last_output = Some(output.clone());
            (self.events)(Event::Output { device: output.0, is_selected: output.1 });
        }
    }

    fn should_move(&self, id: u32) -> bool {
        let Some(n) = self.nodes.get(&id) else { return false };
        n.class == "Stream/Output/Audio" && n.name != OUTPUT_NAME && !n.link_group && !n.dont_move
    }

    fn move_stream(&mut self, id: u32) {
        if self.moved.contains_key(&id) || !self.should_move(id) {
            return;
        }
        let previous = self.meta_values.get(&(id, TARGET_KEY.to_string())).cloned();
        self.moved.insert(id, previous);
        self.set_meta(id, TARGET_KEY, Some((None, SINK_NAME.to_string())));
    }

    /// Make TinCan the default sink and move playing streams onto it.
    fn apply_routing(&mut self) {
        if !self.route || self.routed || self.metadata.is_none() || !self.nodes.values().any(|n| n.name == SINK_NAME) {
            return;
        }
        self.previous_default = self.meta_values.get(&(0, DEFAULT_SINK_KEY.to_string())).cloned();
        self.routed = true;
        self.set_meta(
            0,
            DEFAULT_SINK_KEY,
            Some((Some("Spa:String:JSON".into()), format!("{{ \"name\": \"{SINK_NAME}\" }}"))),
        );
        let ids: Vec<u32> = self.nodes.keys().copied().collect();
        for id in ids {
            self.move_stream(id);
        }
    }

    fn restore_routing(&mut self) {
        if !self.routed {
            return;
        }
        // Restore the default first, so streams returning to "follow the default" go there.
        let previous = self.previous_default.take();
        self.set_meta(0, DEFAULT_SINK_KEY, previous);
        let moved: Vec<_> = self.moved.drain().collect();
        for (id, target) in moved {
            // Leave streams the user has since moved elsewhere where they are.
            let still_ours =
                self.meta_values.get(&(id, TARGET_KEY.to_string())).is_some_and(|(_, v)| v == SINK_NAME);
            if self.nodes.contains_key(&id) && still_ours {
                self.set_meta(id, TARGET_KEY, target);
            }
        }
        self.routed = false;
    }

    fn emit_devices(&mut self) {
        let devices = self.sinks();
        if devices != self.last_devices {
            self.last_devices = devices.clone();
            (self.events)(Event::Devices(devices));
        }
    }
}

/// The node name in a default-sink metadata value such as `{ "name": "alsa_output.x" }`.
fn json_name(value: &str) -> Option<String> {
    let rest = &value[value.find("\"name\"")? + 6..];
    let rest = &rest[rest.find('"')? + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn audio_format_param() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32P);
    info.set_channels(2);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .expect("format pod serializes")
    .0
    .into_inner()
}

struct PlaybackData {
    capture: pw::stream::StreamRc,
    params: triple_buffer::Output<EqParams>,
    processor: Processor,
    rate: Arc<AtomicU32>,
    clipped: Arc<AtomicBool>,
    left: Vec<f32>,
    right: Vec<f32>,
}

fn playback_process(stream: &pw::stream::Stream, d: &mut PlaybackData) {
    // Take the newest captured buffer; dropping a buffer returns it to its stream.
    let mut input = None;
    while let Some(b) = d.capture.dequeue_buffer() {
        input = Some(b);
    }
    let Some(mut output) = stream.dequeue_buffer() else { return };

    let rate = d.rate.load(Ordering::Relaxed);
    if rate != 0 && rate as f64 != d.processor.sample_rate() {
        d.processor.set_sample_rate(rate as f64);
    }

    let out_datas = output.datas_mut();
    if out_datas.len() < 2 {
        return;
    }
    let capacity = out_datas.iter_mut().map(|data| data.data().map_or(0, |s| s.len() / 4)).min().unwrap_or(0);
    let mut frames = 0;
    if let Some(input) = input.as_mut() {
        let in_datas = input.datas_mut();
        if in_datas.len() >= 2 {
            for (ch, buf) in [&mut d.left, &mut d.right].into_iter().enumerate() {
                let data = &mut in_datas[ch];
                let (offset, size) = (data.chunk().offset() as usize, data.chunk().size() as usize);
                let n = (size / 4).min(capacity).min(buf.capacity());
                buf.clear();
                if let Some(bytes) = data.data() {
                    let bytes = &bytes[offset.min(bytes.len())..];
                    let n = n.min(bytes.len() / 4);
                    buf.extend(bytes[..n * 4].as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])));
                }
                frames = if ch == 0 { buf.len() } else { frames.min(buf.len()) };
            }
        }
    }
    if frames == 0 {
        // Nothing captured: output silence of the requested size.
        let requested = (output.requested() as usize).min(capacity).min(d.left.capacity());
        frames = if requested == 0 { capacity.min(d.left.capacity()).min(1024) } else { requested };
        d.left.clear();
        d.right.clear();
        d.left.resize(frames, 0.0);
        d.right.resize(frames, 0.0);
    }
    d.left.truncate(frames);
    d.right.truncate(frames);

    let params = d.params.read();
    let peak = d.processor.process(params, &mut d.left, &mut d.right);
    if peak > 1.0 {
        d.clipped.store(true, Ordering::Relaxed);
    }

    let out_datas = output.datas_mut();
    for (ch, buf) in [&d.left, &d.right].into_iter().enumerate() {
        let data = &mut out_datas[ch];
        if let Some(bytes) = data.data() {
            for (dst, s) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(buf.iter()) {
                dst.copy_from_slice(&s.to_ne_bytes());
            }
        }
        let chunk = data.chunk_mut();
        *chunk.offset_mut() = 0;
        *chunk.stride_mut() = 4;
        *chunk.size_mut() = (frames * 4) as u32;
    }
}

fn run(
    config: Config,
    params: triple_buffer::Output<EqParams>,
    clipped: Arc<AtomicBool>,
    commands: pw::channel::Receiver<Command>,
    events: Rc<dyn Fn(Event)>,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let err = |e: pw::Error| format!("PipeWire: {e}");
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(err)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(err)?;
    let core = context.connect_rc(None).map_err(|e| format!("couldn't connect to PipeWire: {e}"))?;
    let registry = core.get_registry_rc().map_err(err)?;

    let inner = Rc::new(RefCell::new(Inner {
        route: config.route,
        selected: config.output.clone(),
        nodes: BTreeMap::new(),
        metadata: None,
        _metadata_listener: None,
        meta_values: HashMap::new(),
        synced: false,
        routed: false,
        previous_default: None,
        moved: HashMap::new(),
        output_target: None,
        last_output: None,
        last_devices: Vec::new(),
        events: events.clone(),
    }));
    let rate = Arc::new(AtomicU32::new(0));

    // Streams.
    let group = format!("tincan-{}", std::process::id());
    let capture = pw::stream::StreamRc::new(
        core.clone(),
        "TinCan",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_CLASS => "Audio/Sink",
            *pw::keys::NODE_NAME => SINK_NAME,
            *pw::keys::NODE_DESCRIPTION => "TinCan",
            "node.group" => group.as_str(),
            "node.link-group" => group.as_str(),
            "audio.position" => "[ FL FR ]",
            "node.virtual" => "true",
        },
    )
    .map_err(err)?;
    let mut playback_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::NODE_NAME => OUTPUT_NAME,
        *pw::keys::NODE_DESCRIPTION => "TinCan output",
        "node.group" => group.as_str(),
        "node.link-group" => group.as_str(),
        "audio.position" => "[ FL FR ]",
        "stream.dont-remix" => "true",
        "node.dont-fallback" => "true",
    };
    if let Some(output) = &config.output {
        playback_props.insert(*pw::keys::TARGET_OBJECT, output.as_str());
    }
    let playback = pw::stream::StreamRc::new(core.clone(), "TinCan output", playback_props).map_err(err)?;

    let capture_rate = rate.clone();
    let rate_events = events.clone();
    let weak_playback = playback.downgrade();
    let _capture_listener = capture
        .add_local_listener_with_user_data(())
        .param_changed(move |_, _, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let mut info = spa::param::audio::AudioInfoRaw::new();
            if info.parse(param).is_ok() && info.rate() != 0 {
                capture_rate.store(info.rate(), Ordering::Relaxed);
                rate_events(Event::SampleRate(info.rate()));
            }
        })
        .process(move |_, _| {
            if let Some(p) = weak_playback.upgrade() {
                let _ = p.trigger_process();
            }
        })
        .register()
        .map_err(err)?;

    let data = PlaybackData {
        capture: capture.clone(),
        params,
        processor: Processor::new(48_000.0),
        rate: rate.clone(),
        clipped,
        left: Vec::with_capacity(16_384),
        right: Vec::with_capacity(16_384),
    };
    let _playback_listener = playback
        .add_local_listener_with_user_data(data)
        .process(playback_process)
        .register()
        .map_err(err)?;

    // As in PipeWire's module-loopback: the playback stream is connected first, so it is active
    // before the capture stream triggers it, and the capture stream is ASYNC because its buffers
    // are dequeued from the playback stream's process callback.
    let format = audio_format_param();
    let flags = pw::stream::StreamFlags::AUTOCONNECT
        | pw::stream::StreamFlags::MAP_BUFFERS
        | pw::stream::StreamFlags::RT_PROCESS;
    // Not exposed by pipewire-rs.
    let async_flag = pw::stream::StreamFlags::from_bits_retain(pw::sys::pw_stream_flags_PW_STREAM_FLAG_ASYNC);
    let mut format_params = [Pod::from_bytes(&format).unwrap()];
    playback
        .connect(spa::utils::Direction::Output, None, flags | pw::stream::StreamFlags::TRIGGER, &mut format_params)
        .map_err(err)?;
    let mut format_params = [Pod::from_bytes(&format).unwrap()];
    capture.connect(spa::utils::Direction::Input, None, flags | async_flag, &mut format_params).map_err(err)?;

    // Registry: nodes and the default metadata.
    let reg_inner = inner.clone();
    let reg_weak = registry.downgrade();
    let sync_core = core.clone();
    let pending_sync = Rc::new(RefCell::new(None));
    let pending_for_registry = pending_sync.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let Some(props) = global.props else { return };
            match global.type_ {
                pw::types::ObjectType::Node => {
                    let get = |k: &str| props.get(k).unwrap_or("").to_string();
                    let info = NodeInfo {
                        name: get("node.name"),
                        description: props
                            .get("node.description")
                            .or(props.get("node.nick"))
                            .or(props.get("node.name"))
                            .unwrap_or("")
                            .to_string(),
                        class: get("media.class"),
                        link_group: props.get("node.link-group").is_some(),
                        dont_move: props.get("node.dont-move") == Some("true"),
                    };
                    let mut inner = reg_inner.borrow_mut();
                    let is_sink = info.class == "Audio/Sink";
                    let is_ours = info.name == SINK_NAME || info.name == OUTPUT_NAME;
                    inner.nodes.insert(global.id, info);
                    if inner.synced {
                        if is_sink || is_ours {
                            inner.emit_devices();
                            inner.update_output();
                            inner.apply_routing();
                        }
                        if inner.routed {
                            inner.move_stream(global.id);
                        }
                    }
                }
                pw::types::ObjectType::Metadata => {
                    if props.get("metadata.name") != Some("default") {
                        return;
                    }
                    let Some(registry) = reg_weak.upgrade() else { return };
                    let Ok(metadata) = registry.bind::<Metadata, _>(global) else { return };
                    let meta_inner = Rc::downgrade(&reg_inner);
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, type_, value| {
                            let Some(inner) = meta_inner.upgrade() else { return 0 };
                            let Ok(mut inner) = inner.try_borrow_mut() else { return 0 };
                            match (key, value) {
                                (Some(k), Some(v)) => {
                                    inner.meta_values.insert((subject, k.to_string()), (type_.map(str::to_string), v.to_string()));
                                }
                                (Some(k), None) => {
                                    inner.meta_values.remove(&(subject, k.to_string()));
                                }
                                (None, _) => inner.meta_values.retain(|(s, _), _| *s != subject),
                            }
                            0
                        })
                        .register();
                    let mut inner = reg_inner.borrow_mut();
                    inner.metadata = Some(metadata);
                    inner._metadata_listener = Some(listener);
                    // Wait for the metadata's contents before acting on them.
                    *pending_for_registry.borrow_mut() = sync_core.sync(0).ok();
                }
                _ => {}
            }
        })
        .global_remove({
            let inner = inner.clone();
            move |id| {
                let mut inner = inner.borrow_mut();
                let Some(node) = inner.nodes.remove(&id) else { return };
                inner.moved.remove(&id);
                inner.meta_values.retain(|(s, _), _| *s != id);
                if node.class == "Audio/Sink" {
                    inner.emit_devices();
                    if inner.output_target.as_deref() == Some(node.name.as_str()) {
                        inner.output_target = None;
                    }
                    inner.update_output();
                }
            }
        })
        .register();

    *pending_sync.borrow_mut() = core.sync(0).ok();
    let done_inner = inner.clone();
    let done_pending = pending_sync.clone();
    let quit_pending: Rc<RefCell<Option<pw::spa::utils::result::AsyncSeq>>> = Rc::new(RefCell::new(None));
    let done_quit = quit_pending.clone();
    let done_loop = mainloop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id != pw::core::PW_ID_CORE {
                return;
            }
            if Some(seq) == *done_quit.borrow() {
                if let Some(l) = done_loop.upgrade() {
                    l.quit();
                }
                return;
            }
            if Some(seq) != *done_pending.borrow() {
                return;
            }
            let mut inner = done_inner.borrow_mut();
            inner.synced = true;
            inner.emit_devices();
            if inner.selected.is_none() {
                inner.selected = inner.previous_default_sink();
            }
            inner.update_output();
            inner.apply_routing();
        })
        .error({
            let events = events.clone();
            move |_, _, res, message| {
                events(Event::Error(format!("PipeWire error {res}: {message}")));
            }
        })
        .register();

    let cmd_inner = inner.clone();
    let cmd_core = core.clone();
    let cmd_capture = capture.clone();
    let cmd_playback = playback.clone();
    let _receiver = commands.attach(mainloop.loop_(), move |command| match command {
        Command::SetOutput(device) => {
            let mut inner = cmd_inner.borrow_mut();
            inner.selected = device;
            inner.update_output();
        }
        Command::Quit => {
            cmd_inner.borrow_mut().restore_routing();
            let _ = cmd_capture.disconnect();
            let _ = cmd_playback.disconnect();
            // Quit once the server has processed the restore.
            *quit_pending.borrow_mut() = cmd_core.sync(0).ok();
        }
    });

    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_default_sink_json() {
        assert_eq!(super::json_name("{ \"name\": \"alsa_output.usb-DX1\" }").as_deref(), Some("alsa_output.usb-DX1"));
        assert_eq!(super::json_name("{\"name\":\"eq-sink\"}").as_deref(), Some("eq-sink"));
        assert_eq!(super::json_name("{}"), None);
    }
}
