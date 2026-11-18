//! TinCan: a headphone EQ for Linux.

mod app;
mod controller;

use controller::qobject::AppController;
use cxx_qt::CxxQtThread;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Don't make TinCan the default sink or move streams (for development).
    pub no_route: bool,
    /// Move streams onto TinCan for this run, whatever the saved setting.
    pub move_streams: bool,
    /// Start without showing the main window.
    pub hidden: bool,
}

pub static OPTIONS: OnceLock<Options> = OnceLock::new();
static CONTROLLER: Mutex<Option<CxxQtThread<AppController>>> = Mutex::new(None);

/// Give the instance listener and signal handler a way to reach the controller.
pub fn set_show_handler(thread: CxxQtThread<AppController>) {
    *CONTROLLER.lock().unwrap() = Some(thread);
}

fn with_controller(f: impl FnOnce(std::pin::Pin<&mut AppController>) + Send + 'static) {
    if let Some(thread) = CONTROLLER.lock().unwrap().as_ref() {
        let _ = thread.queue(f);
    }
}

fn usage() -> ! {
    println!("Usage: tincan [--hidden] [--move-streams | --no-route]");
    println!("  --hidden        start in the background without showing the main window");
    println!("  --move-streams  also move playing streams onto TinCan, even ones sent to another device");
    println!("  --no-route      don't make TinCan the default output or move playing streams to it");
    std::process::exit(0);
}

/// Hand off to a running instance, if there is one. Otherwise listen for later launches.
fn claim_instance() -> Option<UnixListener> {
    let path = tincan_core::state::instance_socket();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(b"show\n");
        return None;
    }
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => Some(listener),
        Err(e) => {
            eprintln!("tincan: couldn't create {}: {e}", path.display());
            // Run anyway; only single-instance handling is lost.
            UnixListener::bind(std::env::temp_dir().join(format!("tincan-{}.sock", std::process::id()))).ok()
        }
    }
}

fn main() {
    let mut options = Options::default();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--no-route" => options.no_route = true,
            "--move-streams" => options.move_streams = true,
            "--hidden" => options.hidden = true,
            "-h" | "--help" => usage(),
            other => {
                eprintln!("tincan: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(listener) = claim_instance() else {
        println!("TinCan is already running; showing its window.");
        return;
    };
    OPTIONS.set(options).unwrap();

    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
            with_controller(|c| c.show_requested());
        }
    });

    // Quit cleanly on SIGINT/SIGTERM, so routing is restored.
    if let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM]) {
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                with_controller(|c| {
                    c.shutdown();
                    app::ffi::quit_application();
                });
            }
        });
    }

    let status = app::ffi::run_application("qrc:/qt/qml/org/tincan/qml/Main.qml");
    let _ = std::fs::remove_file(tincan_core::state::instance_socket());
    std::process::exit(status);
}
