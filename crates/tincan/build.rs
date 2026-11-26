use std::path::{Path, PathBuf};
use std::process::Command;

use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlModule};
use qt_build_utils::QResourceFile;

fn main() {
    let shader = compile_shader("shaders/channelblend.frag");

    CxxQtBuilder::new_qml_module(QmlModule::new("org.tincan").qml_files([
        "qml/Main.qml",
        "qml/ResponseGraph.qml",
        "qml/EditWindow.qml",
        "qml/FilterBox.qml",
        "qml/NumberField.qml",
        "qml/ManageDialog.qml",
        "qml/StateSwitch.qml",
    ]))
    .qt_module("Gui")
    .qt_module("Qml")
    .qt_module("Quick")
    .qt_module("Widgets")
    .qrc_resources(["qml/Palette.js", "icons/tincan.svg", "icons/tincan-bypassed.svg", "icons/tincan-clipped.svg"])
    .qrc_resources(QResources::new().resource(
        QResource::new().file(QResourceFile::new(shader).alias("shaders/channelblend.frag.qsb")),
    ))
    .files(["src/controller.rs", "src/app.rs"])
    .cpp_file("cpp/helpers.cpp")
    .include_dir("cpp")
    .build();
}

/// Compiles a Vulkan-style GLSL shader with Qt's `qsb` into OUT_DIR, for every graphics API Qt
/// Quick may use, and returns the path of the result.
fn compile_shader(source: &str) -> PathBuf {
    println!("cargo::rerun-if-changed={source}");
    println!("cargo::rerun-if-env-changed=QSB");
    println!("cargo::rerun-if-env-changed=QMAKE");
    let name = Path::new(source).file_name().unwrap().to_string_lossy();
    let output = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(format!("{name}.qsb"));
    let qsb = find_qsb();
    let status = Command::new(&qsb)
        .args(["--glsl", "100es,120,150", "--hlsl", "50", "--msl", "12", "-o"])
        .arg(&output)
        .arg(source)
        .status()
        .unwrap_or_else(|e| panic!("could not run {} (from qt6-shadertools; set QSB to its path): {e}", qsb.display()));
    assert!(status.success(), "qsb failed on {source}");
    output
}

/// `qsb` from $QSB, else from the Qt installation's binary directories (as reported by $QMAKE,
/// qmake6 or qmake), else from PATH.
fn find_qsb() -> PathBuf {
    if let Some(qsb) = std::env::var_os("QSB") {
        return qsb.into();
    }
    let qmakes = std::env::var("QMAKE").ok().into_iter().chain(["qmake6".into(), "qmake".into()]);
    for qmake in qmakes {
        for var in ["QT_HOST_LIBEXECS", "QT_HOST_BINS"] {
            let Ok(out) = Command::new(&qmake).args(["-query", var]).output() else { continue };
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let candidate = Path::new(&dir).join("qsb");
            if out.status.success() && candidate.is_file() {
                return candidate;
            }
        }
    }
    "qsb".into()
}
