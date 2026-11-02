use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("org.tincan").qml_files([
        "qml/Main.qml",
        "qml/ResponseGraph.qml",
        "qml/EditWindow.qml",
        "qml/FilterBox.qml",
        "qml/NumberField.qml",
        "qml/ManageDialog.qml",
    ]))
    .qt_module("Gui")
    .qt_module("Qml")
    .qt_module("Quick")
    .qt_module("Widgets")
    .qrc_resources(["qml/Palette.js", "icons/tincan.svg", "icons/tincan-bypassed.svg", "icons/tincan-clipped.svg"])
    .files(["src/controller.rs", "src/app.rs"])
    .cpp_file("cpp/helpers.cpp")
    .include_dir("cpp")
    .build();
}
