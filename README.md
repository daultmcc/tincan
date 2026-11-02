# TinCan

A real-time, two-channel parametric EQ for headphones on Linux (Qt, Wayland, PipeWire).
See [DESIGN.md](DESIGN.md).

## Building

Requires Rust (2024 edition), Qt 6 (base, declarative, svg) and PipeWire development files
(`libpipewire-0.3`), plus clang for bindgen.

```sh
cargo build --release
./target/release/tincan            # --hidden to start in the tray, --no-route to leave routing alone
```

### Installing

```sh
./install.sh                       # PREFIX=... to install elsewhere than ~/.local
```

installs the binary to `~/.local/bin`, plus a desktop entry and icon. The desktop entry
is what gives TinCan its icon in the taskbar on Wayland and lets it register with the
desktop portal; without it, Qt prints a harmless "Could not register app ID" warning. For
autostart, add TinCan in your desktop's autostart settings and append `--hidden`.

Presets live in `$XDG_DATA_HOME/tincan/presets/` (TOML, see DESIGN.md), application state in
`$XDG_STATE_HOME/tincan/state.toml`.

## Layout

- `crates/tincan-core`: filter math, DSP, presets and the application model (no Qt/PipeWire).
- `crates/tincan-audio`: the PipeWire backend. `cargo run --example probe --`
  runs it headless (`--output NODE`, `--bell F,G,Q`, `--bypass`, `--route`).
- `crates/tincan`: the Qt/QML application (cxx-qt).

## Development

`cargo test` covers the core. To render the windows without showing them:

```sh
QT_QPA_PLATFORM=offscreen TINCAN_GRAB_DIR=/some/dir cargo run -- --no-route
```

writes `main.png` and `editor.png` (of the first loaded preset) and quits. Point
`XDG_DATA_HOME`/`XDG_STATE_HOME` at a scratch directory to keep your real presets out of it.
