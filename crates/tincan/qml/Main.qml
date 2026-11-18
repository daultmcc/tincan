import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Qt.labs.platform as Platform
import org.tincan

// The main window (DESIGN.md, Main window). It is also the application root: it owns the
// controller, the tray icon and the edit windows, and stays alive while hidden.
ApplicationWindow {
    id: main

    width: app.mainWidth
    height: app.mainHeight
    minimumWidth: 640
    minimumHeight: 480
    visible: false
    title: "TinCan"

    property var loadedPresets: JSON.parse(app.loaded)
    property var allPresets: JSON.parse(app.presets)
    property var devices: JSON.parse(app.devices || "[]")
    // Open edit windows by preset id.
    property var editors: ({})
    property bool quitting: false

    AppController {
        id: app
    }

    Component.onCompleted: {
        app.start();
        if (!app.startHidden || app.grabDir !== "") showMain();
        if (app.grabDir !== "") grabTimer.start();
    }

    // Development aid (TINCAN_GRAB_DIR): render the main window and an edit window, then quit.
    Timer {
        id: grabTimer
        interval: 1500
        property int stage: 0
        onTriggered: {
            if (stage === 0) {
                main.header.grabToImage(function(r) { r.saveToFile(app.grabDir + "/main-header.png"); });
                mainContent.grabToImage(function(r) { r.saveToFile(app.grabDir + "/main.png"); });
                if (main.loadedPresets.length > 0) app.openEditor(main.loadedPresets[0].id);
                stage = 1;
                start();
            } else {
                for (var id in main.editors) {
                    var w = main.editors[id];
                    // Show the selected and hovered states too.
                    w.selectedIndex = 0;
                    w.boxHoverIndex = 1;
                    w.header.grabToImage(function(r) { r.saveToFile(app.grabDir + "/editor-header.png"); });
                    w.content.grabToImage(function(r) { r.saveToFile(app.grabDir + "/editor.png"); main.doQuit(); });
                    return;
                }
                main.doQuit();
            }
        }
    }

    onClosing: (close) => {
        // Closing only hides the window; TinCan keeps running.
        app.setWindowSize("main", width, height);
    }

    function showMain() {
        main.show();
        main.raise();
        main.requestActivate();
    }

    // Show every open window, the main one last so it ends up in front.
    function showAll() {
        for (var id in editors) {
            editors[id].show();
            editors[id].raise();
        }
        showMain();
    }

    function openEditorWindow(id) {
        var w = editors[id];
        if (w) {
            w.show();
            w.raise();
            w.requestActivate();
            return;
        }
        w = editorComponent.createObject(null, { app: app, presetId: id });
        w.finished.connect(function(pid) {
            app.closeEditor(pid);
            var e = editors;
            delete e[pid];
            editors = e;
            w.destroy();
        });
        var e = editors;
        e[id] = w;
        editors = e;
        w.show();
    }

    function requestQuit() {
        var dirty = JSON.parse(app.dirtyPresets());
        if (dirty.length === 0) {
            doQuit();
            return;
        }
        quitDialog.dirty = dirty;
        showMain();
        quitDialog.open();
    }

    function doQuit() {
        if (quitting) return;
        quitting = true;
        if (main.visible) app.setWindowSize("main", width, height);
        app.shutdown();
        Qt.quit();
    }

    Component {
        id: editorComponent
        EditWindow {}
    }

    Connections {
        target: app
        function onEditorRequested(id) { main.openEditorWindow(id); }
        function onPresetChanged(id) { if (main.editors[id]) main.editors[id].refresh(); }
        function onPresetRenamed(oldId, newId) {
            var e = main.editors;
            var w = e[oldId];
            if (!w) return;
            delete e[oldId];
            e[newId] = w;
            w.presetId = newId;
            main.editors = e;
            w.refresh();
        }
        function onEditorClosed(id) { if (main.editors[id]) main.editors[id].forceClose(); }
        function onShowRequested() { main.showAll(); }
        function onErrorOccurred(message) {
            errorDialog.message = message;
            main.showMain();
            errorDialog.open();
        }
    }

    Timer {
        interval: 200
        running: true
        repeat: true
        onTriggered: app.poll()
    }

    Platform.SystemTrayIcon {
        visible: true
        icon.source: app.clipping ? "qrc:/qt/qml/org/tincan/icons/tincan-clipped.svg"
                   : app.bypass ? "qrc:/qt/qml/org/tincan/icons/tincan-bypassed.svg"
                   : "qrc:/qt/qml/org/tincan/icons/tincan.svg"
        tooltip: "TinCan" + (app.bypass ? " (bypassed)" : "") + " — " + app.abList + (app.clipping ? " — clipping" : "")
        onActivated: (reason) => {
            if (reason === Platform.SystemTrayIcon.Trigger || reason === Platform.SystemTrayIcon.DoubleClick)
                main.showAll();
        }
        menu: Platform.Menu {
            Platform.MenuItem {
                text: app.bypass ? "Activate" : "Deactivate"
                onTriggered: app.setGlobalBypass(!app.bypass)
            }
            Platform.MenuItem {
                text: "Toggle A/B"
                onTriggered: app.toggleAb()
            }
            Platform.MenuItem {
                text: "Quit"
                onTriggered: main.requestQuit()
            }
        }
    }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 10

            Label { text: "Output" }
            ComboBox {
                id: deviceBox
                // Fills (up to 260) so it can shrink when the bar is short of room.
                Layout.fillWidth: true
                Layout.maximumWidth: 260
                Layout.minimumWidth: 120
                model: main.devices
                textRole: "description"
                displayText: {
                    var i = currentIndex;
                    if (i >= 0) return main.devices[i].description;
                    return app.outputDevice !== "" ? app.outputDevice + " (unavailable)" : "Default";
                }
                currentIndex: {
                    for (var i = 0; i < main.devices.length; i++)
                        if (main.devices[i].name === app.outputDevice) return i;
                    return -1;
                }
                onActivated: (i) => app.setOutput(main.devices[i].name)
                ToolTip.visible: hovered && app.outputStatus !== ""
                ToolTip.text: app.outputStatus
            }
            Label {
                visible: app.outputStatus !== ""
                text: "⚠"
                color: "#d08000"
                ToolTip.visible: statusHover.hovered
                ToolTip.text: app.outputStatus
                HoverHandler { id: statusHover }
            }

            ToolSeparator {}

            StateSwitch {
                labels: ["EQ active", "EQ inactive"]
                text: app.bypass ? "EQ inactive" : "EQ active"
                checked: !app.bypass
                onToggled: app.setGlobalBypass(!checked)
                ToolTip.visible: hovered
                ToolTip.delay: 800
                ToolTip.text: "Turn the whole EQ on or off (bypassed passes audio through unchanged)"
            }

            ToolSeparator {}

            // A/B: which of the two loaded-preset lists is heard.
            Row {
                Repeater {
                    model: ["A", "B"]
                    delegate: Button {
                        required property string modelData
                        text: modelData
                        implicitWidth: 34
                        checkable: true
                        checked: app.abList === modelData
                        // Clicking toggles `checked`; restore the binding to the controller.
                        onClicked: {
                            app.setAb(modelData);
                            checked = Qt.binding(function() { return app.abList === modelData; });
                        }
                        ToolTip.visible: hovered
                        ToolTip.delay: 800
                        ToolTip.text: "Hear loaded presets list " + modelData
                            + ". The two lists share presets; only which are loaded and active differs."
                    }
                }
            }

            ToolSeparator {}

            Label { text: "Preamp" }
            ComboBox {
                id: modeBox
                model: ["Match bypass", "Safe", "Manual"]
                currentIndex: app.preampMode
                onActivated: (i) => app.setPreamp(i, i === 2 ? app.preampDb : app.manualPreampDb)
                ToolTip.visible: hovered
                ToolTip.delay: 800
                ToolTip.text: ["Overall level matches pass-through, for fair A/B comparisons",
                               "Never pushes a full-scale signal above full scale",
                               "A fixed preamp"][currentIndex]
            }
            NumberField {
                Layout.preferredWidth: 90
                Layout.minimumWidth: 90
                kind: "gain"
                unit: "dB"
                value: app.preampMode === 2 ? app.manualPreampDb : app.preampDb
                readOnly: app.preampMode !== 2
                onEdited: (v) => app.setPreamp(2, v)
            }

            Item { Layout.fillWidth: true }

            Button {
                visible: app.excessDb > 0
                flat: true
                text: "⚠ +" + app.excessDb.toFixed(1) + " dB, may clip"
                palette.buttonText: "#d08000"
                ToolTip.visible: hovered
                ToolTip.text: "Switch the preamp to safe mode"
                onClicked: app.setPreamp(1, app.manualPreampDb)
            }
            Rectangle {
                visible: app.clipping
                color: "#e0645a"
                radius: 3
                implicitWidth: clipLabel.implicitWidth + 12
                implicitHeight: clipLabel.implicitHeight + 6
                Label {
                    id: clipLabel
                    anchors.centerIn: parent
                    text: "CLIP"
                    color: "white"
                    font.bold: true
                }
            }
        }
    }

    ColumnLayout {
        id: mainContent
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        ResponseGraph {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 180
            frequencies: JSON.parse(app.frequencies || "[]")
            curves: JSON.parse(app.mainCurves || "null")
        }

        RowLayout {
            Layout.fillWidth: true
            Label {
                text: "Loaded presets (" + app.abList + ")"
                font.bold: true
                Layout.fillWidth: true
            }
            Button {
                id: addButton
                text: "Add preset ▾"
                onClicked: addMenu.popup(addButton, 0, addButton.height)
                Menu {
                    id: addMenu
                    MenuItem {
                        text: "New preset"
                        onTriggered: app.createPreset()
                    }
                    MenuSeparator {}
                    Instantiator {
                        model: main.allPresets.filter(function(p) { return !p.loaded; })
                        delegate: MenuItem {
                            required property var modelData
                            text: modelData.name
                            onTriggered: app.loadPreset(modelData.id)
                        }
                        onObjectAdded: (index, object) => addMenu.insertItem(index + 2, object)
                        onObjectRemoved: (index, object) => addMenu.removeItem(object)
                    }
                }
            }
        }

        Frame {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(220, Math.max(60, loadedList.contentHeight + 4))
            padding: 1

            ListView {
                id: loadedList
                anchors.fill: parent
                clip: true
                model: main.loadedPresets
                ScrollBar.vertical: ScrollBar {}

                delegate: ItemDelegate {
                    id: row
                    required property var modelData
                    width: loadedList.width
                    onDoubleClicked: app.openEditor(modelData.id)
                    ToolTip.visible: hovered
                    ToolTip.delay: 1000
                    ToolTip.text: "Double-click to edit"

                    contentItem: RowLayout {
                        spacing: 8
                        Switch {
                            checked: row.modelData.active
                            onToggled: app.setPresetActive(row.modelData.id, checked)
                        }
                        Label {
                            Layout.fillWidth: true
                            text: row.modelData.name + (row.modelData.isNew ? "  • not saved yet" : row.modelData.dirty ? "  • unsaved changes" : row.modelData.editing ? "  (editing)" : "")
                            opacity: row.modelData.active ? 1 : 0.6
                            elide: Text.ElideRight
                        }
                        ToolButton {
                            text: "✕"
                            ToolTip.visible: hovered
                            ToolTip.text: "Unload"
                            onClicked: app.unloadPreset(row.modelData.id)
                        }
                    }
                }

                Label {
                    anchors.centerIn: parent
                    visible: main.loadedPresets.length === 0
                    text: "No presets loaded"
                    opacity: 0.6
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Button {
                text: "Manage presets…"
                onClicked: manageDialog.open()
            }
            CheckBox {
                text: "Move playing streams to TinCan"
                checked: app.moveStreams
                enabled: app.moveStreamsEditable
                onToggled: app.setStreamMoving(checked)
                ToolTip.visible: hovered
                ToolTip.delay: 800
                ToolTip.text: app.moveStreamsEditable
                    ? "Also move streams onto TinCan, including ones sent to another device. Otherwise TinCan only becomes the default output."
                    : "Set by a command-line option for this run"
            }
            Item { Layout.fillWidth: true }
            Label {
                text: app.sampleRate > 0 ? (app.sampleRate / 1000) + " kHz" : ""
                opacity: 0.6
            }
            Button {
                text: "Quit"
                onClicked: main.requestQuit()
            }
        }
    }

    ManageDialog {
        id: manageDialog
        app: app
    }

    Dialog {
        id: quitDialog
        width: 460
        property var dirty: []
        title: "Unsaved changes"
        modal: true
        anchors.centerIn: parent
        Label {
            width: parent.width
            wrapMode: Text.Wrap
            text: "These presets have unsaved changes:\n\n" + quitDialog.dirty.map(function(p) { return "  • " + p.name; }).join("\n")
        }
        footer: DialogButtonBox {
            Button {
                text: "Save all and quit"
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
                onClicked: {
                    quitDialog.close();
                    for (var i = 0; i < quitDialog.dirty.length; i++)
                        if (!app.savePreset(quitDialog.dirty[i].id)) return;
                    main.doQuit();
                }
            }
            Button {
                text: "Discard and quit"
                DialogButtonBox.buttonRole: DialogButtonBox.DestructiveRole
                onClicked: { quitDialog.close(); main.doQuit(); }
            }
            Button {
                text: "Cancel"
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: quitDialog.close()
            }
        }
    }

    Dialog {
        id: errorDialog
        width: 420
        property string message
        title: "TinCan"
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Ok
        Label { text: errorDialog.message; wrapMode: Text.Wrap; width: parent.width }
    }
}
