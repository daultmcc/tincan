import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

// "Manage presets": delete, rename, edit, import and export (DESIGN.md, Main window).
Dialog {
    id: root

    required property var app
    property var presets: JSON.parse(app.presets)
    property int current: -1
    readonly property var currentPreset: current >= 0 && current < presets.length ? presets[current] : null
    // Import waiting on a name decision.
    property string pendingImport: ""

    title: "Manage presets"
    modal: true
    width: Math.min(560, parent ? parent.width - 40 : 560)
    height: Math.min(520, parent ? parent.height - 40 : 520)
    anchors.centerIn: parent
    standardButtons: Dialog.Close

    onPresetsChanged: if (current >= presets.length) current = presets.length - 1

    function doImport(path, name, replace) {
        var result = JSON.parse(app.importPreset(path, name, replace));
        if (result.nameTaken !== undefined) {
            pendingImport = path;
            conflictName.text = result.nameTaken;
            conflictDialog.existing = result.nameTaken;
            conflictDialog.open();
        } else if (result.error !== undefined) {
            errorText.text = result.error;
            errorDialog.open();
        } else {
            pendingImport = "";
        }
    }

    contentItem: ColumnLayout {
        spacing: 8

        Frame {
            Layout.fillWidth: true
            Layout.fillHeight: true
            padding: 1

            ListView {
                id: list
                anchors.fill: parent
                clip: true
                model: root.presets
                currentIndex: root.current
                ScrollBar.vertical: ScrollBar {}

                delegate: ItemDelegate {
                    required property var modelData
                    required property int index
                    width: list.width
                    highlighted: index === root.current
                    text: modelData.name + (modelData.loaded ? "   (loaded)" : "")
                    onClicked: root.current = index
                    onDoubleClicked: root.app.openEditor(modelData.id)
                }

                Label {
                    anchors.centerIn: parent
                    visible: root.presets.length === 0
                    text: "No saved presets"
                    opacity: 0.6
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Button {
                text: "Edit"
                enabled: root.currentPreset !== null
                onClicked: root.app.openEditor(root.currentPreset.id)
            }
            Button {
                text: "Rename…"
                enabled: root.currentPreset !== null
                onClicked: { renameField.text = root.currentPreset.name; renameDialog.open(); }
            }
            Button {
                text: "Delete…"
                enabled: root.currentPreset !== null
                onClicked: deleteDialog.open()
            }
            Item { Layout.fillWidth: true }
            Button {
                text: "Import…"
                onClicked: importFileDialog.open()
            }
            Button {
                text: "Export…"
                enabled: root.currentPreset !== null
                onClicked: {
                    exportFileDialog.selectedFile = "file://" + encodeURI(root.currentPreset.id) + ".toml";
                    exportFileDialog.open();
                }
            }
        }
    }

    Dialog {
        id: renameDialog
        width: 360
        title: "Rename preset"
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok | Dialog.Cancel
        TextField {
            id: renameField
            width: parent.width
            selectByMouse: true
            onAccepted: renameDialog.accept()
        }
        onOpened: { renameField.forceActiveFocus(); renameField.selectAll(); }
        onAccepted: root.app.renamePreset(root.currentPreset.id, renameField.text)
    }

    Dialog {
        id: deleteDialog
        width: 420
        title: "Delete preset"
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Yes | Dialog.No
        Label {
            width: parent.width
            wrapMode: Text.Wrap
            text: {
                var p = root.currentPreset;
                if (!p) return "";
                var s = "Delete “" + p.name + "”? This can't be undone.";
                if (p.loaded && p.editing)
                    s += "\n\nIt will be unloaded and its edit window closed.";
                else if (p.loaded)
                    s += "\n\nIt will be unloaded.";
                else if (p.editing)
                    s += "\n\nIts edit window will be closed.";
                return s;
            }
        }
        onAccepted: root.app.deletePreset(root.currentPreset.id)
    }

    Dialog {
        id: conflictDialog
        width: 460
        property string existing
        title: "Preset already exists"
        modal: true
        anchors.centerIn: parent
        ColumnLayout {
            width: parent.width
            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: "A preset named “" + conflictDialog.existing + "” already exists. Import it under another name, or replace the existing one?"
            }
            TextField {
                id: conflictName
                Layout.fillWidth: true
                selectByMouse: true
            }
        }
        footer: DialogButtonBox {
            Button {
                text: "Import as new name"
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
                onClicked: { conflictDialog.close(); root.doImport(root.pendingImport, conflictName.text, false); }
            }
            Button {
                text: "Replace"
                DialogButtonBox.buttonRole: DialogButtonBox.DestructiveRole
                onClicked: { conflictDialog.close(); root.doImport(root.pendingImport, "", true); }
            }
            Button {
                text: "Cancel"
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: { conflictDialog.close(); root.pendingImport = ""; }
            }
        }
    }

    Dialog {
        id: errorDialog
        width: 420
        title: "Couldn't import"
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok
        Label { id: errorText; width: parent.width; wrapMode: Text.Wrap }
    }

    FileDialog {
        id: importFileDialog
        title: "Import preset"
        fileMode: FileDialog.OpenFile
        nameFilters: ["TinCan presets (*.toml)", "All files (*)"]
        onAccepted: root.doImport(selectedFile.toString(), "", false)
    }

    FileDialog {
        id: exportFileDialog
        title: "Export preset"
        fileMode: FileDialog.SaveFile
        defaultSuffix: "toml"
        nameFilters: ["TinCan presets (*.toml)"]
        onAccepted: root.app.exportPreset(root.currentPreset.id, selectedFile.toString())
    }
}
