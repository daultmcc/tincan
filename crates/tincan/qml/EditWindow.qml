import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Palette.js" as P

// Editor for one preset's live state (DESIGN.md, Edit window).
ApplicationWindow {
    id: win

    required property var app
    property string presetId
    property var st: ({ name: "", loaded: false, active: false, dirty: false, isNew: false, tilt: 0, filters: [], colors: [], curves: null })
    property int selectedIndex: -1
    // Filter whose box is hovered, highlighted on the graph.
    property int boxHoverIndex: -1
    // While true, closing doesn't prompt (the preset was deleted or the user already chose).
    property bool closingConfirmed: false
    // Target index while dragging a filter box to reorder.
    property int dropIndex: -1
    property int dragFrom: -1

    signal finished(string presetId)

    width: app.editWidth
    height: app.editHeight
    minimumWidth: 640
    minimumHeight: 520
    title: (st.dirty ? "• " : "") + st.name + " — TinCan"

    function refresh() {
        var json = app.editorState(presetId);
        if (json === "") return;
        st = JSON.parse(json);
        if (selectedIndex >= st.filters.length) selectedIndex = st.filters.length - 1;
    }

    function update(index, filter) {
        app.updateFilter(presetId, index, JSON.stringify(filter));
    }

    function addFilter(frequency, gain) {
        var f = { type: "bell", channel: "LR", frequency: frequency, gain: gain, q: 1.0, enabled: true };
        var i = app.addFilter(presetId, JSON.stringify(f));
        if (i >= 0) {
            selectedIndex = i;
            filterList.positionViewAtIndex(i, ListView.Contain);
        }
    }

    function removeFilter(index) {
        app.removeFilter(presetId, index);
        if (selectedIndex === index) selectedIndex = -1;
        else if (selectedIndex > index) selectedIndex--;
    }

    // Save. A new preset is named first.
    function save(closeAfter) {
        if (st.isNew) {
            nameDialog.closeAfter = closeAfter;
            nameField.text = st.name;
            nameDialog.open();
            return;
        }
        if (app.savePreset(presetId) !== "" && closeAfter)
            forceClose();
    }

    function forceClose() {
        closingConfirmed = true;
        close();
    }

    readonly property alias content: editorContent

    Component.onCompleted: refresh()

    onClosing: (close) => {
        if (!closingConfirmed && st.dirty) {
            close.accepted = false;
            unsavedDialog.open();
            return;
        }
        app.setWindowSize("edit", width, height);
        finished(presetId);
    }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8

            // The name; double-click to edit it. The new name is stored on the next save.
            Item {
                Layout.fillWidth: true
                implicitHeight: Math.max(nameLabel.implicitHeight, nameEdit.implicitHeight)

                Label {
                    id: nameLabel
                    visible: !nameEdit.visible
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(implicitWidth, parent.width)
                    text: win.st.name + (win.st.isNew ? "  • not saved yet" : win.st.dirty ? "  • unsaved" : "")
                    font.pixelSize: 16
                    font.bold: true
                    elide: Text.ElideRight
                    HoverHandler { id: nameHover; cursorShape: Qt.IBeamCursor }
                    ToolTip.visible: nameHover.hovered
                    ToolTip.delay: 800
                    ToolTip.text: "Double-click to rename"
                    TapHandler {
                        onDoubleTapped: {
                            nameEdit.text = win.st.name;
                            nameEdit.visible = true;
                            nameEdit.forceActiveFocus();
                            nameEdit.selectAll();
                        }
                    }
                }
                TextField {
                    id: nameEdit
                    visible: false
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(360, parent.width)
                    font.pixelSize: 16
                    font.bold: true
                    selectByMouse: true
                    property bool cancelled: false
                    onEditingFinished: {
                        if (!visible) return;
                        visible = false;
                        if (!cancelled && text.trim() !== win.st.name)
                            app.setPresetName(win.presetId, text);
                        cancelled = false;
                    }
                    Keys.onEscapePressed: { cancelled = true; focus = false; visible = false; cancelled = false; }
                }
            }
            StateSwitch {
                labels: ["Active", "Inactive", "Not loaded"]
                text: !win.st.loaded ? "Not loaded" : win.st.active ? "Active" : "Inactive"
                enabled: win.st.loaded
                checked: win.st.loaded && win.st.active
                ToolTip.visible: hovered
                ToolTip.delay: win.st.loaded ? 800 : 0
                ToolTip.text: win.st.loaded ? "Whether this preset is heard (also on the main window)"
                                            : "Load this preset to hear it"
                onToggled: app.setPresetActive(win.presetId, checked)
            }
            Button {
                text: win.st.isNew ? "Save…" : "Save"
                enabled: win.st.dirty
                onClicked: win.save(false)
            }
            Button {
                text: "Save as…"
                visible: !win.st.isNew
                onClicked: { saveAsField.text = win.st.name; saveAsDialog.open(); }
            }
        }
    }

    Shortcut {
        sequences: [StandardKey.Save]
        onActivated: if (win.st.dirty) win.save(false)
    }

    ColumnLayout {
        id: editorContent
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        ResponseGraph {
            id: graph
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 200
            editable: true
            frequencies: JSON.parse(app.frequencies)
            curves: win.st.curves
            filters: win.st.filters
            colors: win.st.colors
            selectedIndex: win.selectedIndex
            linkedIndex: win.boxHoverIndex
            onFilterSelected: (i) => win.selectedIndex = i
            onFilterMoved: (i, f, g) => {
                var flt = Object.assign({}, win.st.filters[i]);
                flt.frequency = f;
                flt.gain = g;
                win.update(i, flt);
            }
            onFilterCreated: (f, g) => win.addFilter(f, g)
            onFilterDeleted: (i) => win.removeFilter(i)
            onQScrolled: (i, steps, fine) => {
                var flt = Object.assign({}, win.st.filters[i]);
                flt.q = P.stepValue("q", flt.q, steps, fine);
                win.update(i, flt);
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Button {
                text: "Add filter"
                onClicked: win.addFilter(1000, 0)
            }
            Button {
                text: "Sort by frequency"
                enabled: win.st.filters.length > 1
                onClicked: { app.sortFilters(win.presetId); win.selectedIndex = -1; }
            }
            // The preset's tilt: a constant slope across the whole band, pivoting at 1 kHz.
            Label {
                Layout.leftMargin: 12
                text: "Tilt"
                HoverHandler { id: tiltHover }
                ToolTip.visible: tiltHover.hovered
                ToolTip.delay: 800
                ToolTip.text: "A constant slope across the whole band, leaving 1 kHz unchanged. Negative is darker."
            }
            NumberField {
                kind: "tilt"
                unit: "dB/oct"
                value: win.st.tilt
                onEdited: (v) => app.setTilt(win.presetId, v)
            }
            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignRight
                opacity: 0.6
                font.pixelSize: 11
                text: "Double-click the graph to add a filter. Drag dots to move, scroll to change Q, Delete to remove."
                elide: Text.ElideRight
            }
        }

        ListView {
            id: filterList
            Layout.fillWidth: true
            Layout.preferredHeight: 330
            orientation: ListView.Horizontal
            spacing: 8
            clip: true
            model: win.st.filters.length
            boundsBehavior: Flickable.StopAtBounds
            interactive: false
            ScrollBar.horizontal: ScrollBar { policy: ScrollBar.AsNeeded }

            WheelHandler {
                // Scrolling over empty space scrolls the list; over a field it adjusts the field.
                acceptedModifiers: Qt.NoModifier
                onWheel: (event) => {
                    var dx = event.angleDelta.x !== 0 ? event.angleDelta.x : event.angleDelta.y;
                    filterList.contentX = Math.max(0, Math.min(filterList.contentWidth - filterList.width,
                                                               filterList.contentX - dx));
                }
                enabled: filterList.contentWidth > filterList.width
            }

            delegate: FilterBox {
                required property int modelData
                height: filterList.height - 14
                index: modelData
                colorIndex: win.st.colors[modelData] !== undefined ? win.st.colors[modelData] : modelData
                filter: win.st.filters[modelData] || filter
                selected: modelData === win.selectedIndex
                highlighted: modelData === graph.hoverIndex
                onBoxHoveredChanged: {
                    if (boxHovered) win.boxHoverIndex = modelData;
                    else if (win.boxHoverIndex === modelData) win.boxHoverIndex = -1;
                }
                onSelectRequested: win.selectedIndex = modelData
                onChanged: (f) => win.update(modelData, f)
                onRemoveRequested: win.removeFilter(modelData)
                onReorderDragged: (x) => {
                    win.dragFrom = modelData;
                    var listX = mapToItem(filterList.contentItem, x, 0).x;
                    win.dropIndex = Math.max(0, Math.min(win.st.filters.length - 1, Math.floor(listX / (width + filterList.spacing))));
                }
                onReorderDropped: (x) => {
                    var to = win.dropIndex;
                    win.dropIndex = -1;
                    if (to >= 0 && to !== modelData) {
                        app.moveFilter(win.presetId, modelData, to);
                        win.selectedIndex = to;
                    }
                }
            }

            Rectangle {
                // Insertion marker while reordering.
                visible: win.dropIndex >= 0
                parent: filterList.contentItem
                width: 3
                height: filterList.height - 14
                color: win.palette.highlight
                x: (win.dropIndex + (win.dropIndex > win.dragFrom ? 1 : 0)) * (150 + filterList.spacing) - filterList.spacing / 2 - 1
            }

            Label {
                anchors.centerIn: parent
                visible: win.st.filters.length === 0
                text: "No filters yet"
                opacity: 0.6
            }
        }
    }

    Dialog {
        id: unsavedDialog
        width: 460
        title: "Unsaved changes"
        modal: true
        anchors.centerIn: parent
        Label {
            width: parent.width
            wrapMode: Text.Wrap
            text: win.st.isNew ? "Save the new preset “" + win.st.name + "” before closing?"
                               : "Save changes to “" + win.st.name + "” before closing?"
        }
        footer: DialogButtonBox {
            Button {
                text: "Save"
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
                onClicked: { unsavedDialog.close(); win.save(true); }
            }
            Button {
                text: "Discard"
                DialogButtonBox.buttonRole: DialogButtonBox.DestructiveRole
                onClicked: { unsavedDialog.close(); win.forceClose(); }
            }
            Button {
                text: "Cancel"
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: unsavedDialog.close()
            }
        }
    }

    Dialog {
        id: nameDialog
        width: 360
        property bool closeAfter: false
        title: "Save preset"
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Save | Dialog.Cancel
        ColumnLayout {
            width: parent.width
            Label { text: "Preset name:" }
            TextField {
                id: nameField
                Layout.fillWidth: true
                selectByMouse: true
                onAccepted: nameDialog.accept()
            }
        }
        onOpened: { nameField.forceActiveFocus(); nameField.selectAll(); }
        onAccepted: {
            if (nameField.text.trim() !== win.st.name && !app.setPresetName(win.presetId, nameField.text))
                return;
            if (app.savePreset(win.presetId) !== "" && closeAfter)
                win.forceClose();
        }
    }

    Dialog {
        id: saveAsDialog
        width: 360
        title: "Save as"
        modal: true
        anchors.centerIn: parent
        standardButtons: Dialog.Save | Dialog.Cancel
        ColumnLayout {
            width: parent.width
            Label { text: "New preset name:" }
            TextField {
                id: saveAsField
                Layout.fillWidth: true
                selectByMouse: true
                onAccepted: saveAsDialog.accept()
            }
        }
        onOpened: { saveAsField.forceActiveFocus(); saveAsField.selectAll(); }
        onAccepted: app.savePresetAs(win.presetId, saveAsField.text)
    }
}
