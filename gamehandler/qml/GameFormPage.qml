// Add / edit a library entry. Pushed as a layer above the main pages.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: form

    property var gameData: ({})
    property bool isNew: true
    // Injected by the caller: this page is pushed by URL, so it must not
    // reach for ids in Main.qml (they are outside its creation context).
    property var dismiss: null
    readonly property bool isLinux: kindBox.currentIndex === 1

    title: isNew ? "Add Game" : "Edit Game"

    function closeForm() {
        if (dismiss)
            dismiss()
    }

    actions: [
        Kirigami.Action {
            text: "Cancel"
            icon.name: "dialog-cancel"
            onTriggered: form.closeForm()
        },
        Kirigami.Action {
            text: form.isNew ? "Add" : "Save"
            icon.name: "dialog-ok-apply"
            enabled: nameField.text.trim().length > 0
            onTriggered: form.save()
        }
    ]

    function save() {
        gameData.name = nameField.text
        gameData.exePath = exeField.text
        gameData.arguments = argsField.text
        gameData.workingDirectory = cwdField.text
        gameData.isLinux = isLinux
        gameData.runner = runnerBox.currentValue !== undefined ? runnerBox.currentValue : gameData.runner
        gameData.prefixPath = prefixField.text
        gameData.category = categoryBox.editText.trim()
        gameData.virtualDesktopSize = desktopSizeField.text
        gameData.additionalApp = extraField.text
        gameData.environment = envField.text
        backend.saveGame(gameData)
        closeForm()
    }

    Connections {
        target: backend
        function onCoverFetched(token, hit) {
            if (token !== form.gameData.gameId)
                return
            form.gameData.coverPath = hit.coverPath
            if (hit.steamAppid)
                form.gameData.steamAppid = hit.steamAppid
            if (hit.category && hit.category !== "Uncategorized"
                    && categoryBox.editText.trim() === "Uncategorized")
                categoryBox.editText = hit.category
            coverPreview.source = backend.coverUrlFor(hit.coverPath)
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Kirigami.Separator {
                Kirigami.FormData.label: "Game"
                Kirigami.FormData.isSection: true
            }

            QQC2.TextField {
                id: nameField
                Kirigami.FormData.label: "Name:"
                text: form.gameData.name || ""
            }

            QQC2.ComboBox {
                id: kindBox
                Kirigami.FormData.label: "Type:"
                model: ["Windows (Wine / Proton)", "Linux native"]
                currentIndex: form.gameData.isLinux ? 1 : 0
            }

            RowLayout {
                Kirigami.FormData.label: "Executable:"
                QQC2.TextField {
                    id: exeField
                    Layout.fillWidth: true
                    text: form.gameData.exePath || ""
                }
                QQC2.ToolButton {
                    icon.name: "document-open"
                    QQC2.ToolTip.text: "Browse for an executable"
                    QQC2.ToolTip.visible: hovered
                    onClicked: exeDialog.open()
                }
            }

            QQC2.TextField {
                id: argsField
                Kirigami.FormData.label: "Launch arguments:"
                text: form.gameData.arguments || ""
            }

            QQC2.TextField {
                id: cwdField
                Kirigami.FormData.label: "Working directory:"
                text: form.gameData.workingDirectory || ""
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "Library"
                Kirigami.FormData.isSection: true
            }

            QQC2.ComboBox {
                id: categoryBox
                Kirigami.FormData.label: "Category:"
                editable: true
                model: backend.formCategories
                Component.onCompleted: editText = form.gameData.category || "Uncategorized"
            }

            RowLayout {
                Kirigami.FormData.label: "Cover art:"
                spacing: Kirigami.Units.smallSpacing

                Image {
                    id: coverPreview
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 2
                    Layout.preferredHeight: Kirigami.Units.gridUnit * 3
                    fillMode: Image.PreserveAspectFit
                    source: backend.coverUrlFor(form.gameData.coverPath || "")
                    visible: source.toString() !== ""
                }
                QQC2.Label {
                    visible: !coverPreview.visible
                    opacity: 0.7
                    text: "No cover yet"
                }
                QQC2.Button {
                    text: "Find cover"
                    QQC2.ToolTip.text: "Search Steam, then fall back to the executable's own icon"
                    QQC2.ToolTip.visible: hovered
                    onClicked: backend.fetchCoverForForm(
                        form.gameData.gameId, form.gameData.gameId,
                        nameField.text, form.isLinux ? "" : exeField.text)
                }
                QQC2.ToolButton {
                    icon.name: "document-open"
                    QQC2.ToolTip.text: "Choose a custom cover image"
                    QQC2.ToolTip.visible: hovered
                    onClicked: coverDialog.open()
                }
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "Compatibility tool"
                Kirigami.FormData.isSection: true
            }

            QQC2.ComboBox {
                id: runnerBox
                Kirigami.FormData.label: "Runner:"
                enabled: !form.isLinux
                textRole: "label"
                valueRole: "runnerId"
                model: backend.runnerChoices
                Component.onCompleted: {
                    var index = indexOfValue(form.gameData.runner)
                    currentIndex = index >= 0 ? index : 0
                }
            }

            QQC2.TextField {
                id: prefixField
                Kirigami.FormData.label: "Wine prefix (optional):"
                enabled: !form.isLinux
                placeholderText: "Leave empty for an isolated prefix per game"
                text: form.gameData.prefixPath || ""
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "Launch options"
                Kirigami.FormData.isSection: true
            }

            QQC2.Switch {
                Kirigami.FormData.label: "MangoHud:"
                checked: form.gameData.mangohud === true
                onToggled: form.gameData.mangohud = checked
                text: "Performance overlay when MangoHud is installed"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Feral GameMode:"
                checked: form.gameData.gamemode === true
                onToggled: form.gameData.gamemode = checked
                text: "Ask the system to boost performance while the game runs"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Prefer SDL:"
                checked: form.gameData.prefer_sdl === true
                onToggled: form.gameData.prefer_sdl = checked
                text: "Can fix controller issues in some games"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Wine Wayland driver:"
                enabled: !form.isLinux
                checked: form.gameData.wayland === true
                onToggled: form.gameData.wayland = checked
                text: "Experimental. Works best on Proton-EM and recent GE-Proton"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "HDR:"
                enabled: !form.isLinux
                checked: form.gameData.hdr === true
                onToggled: form.gameData.hdr = checked
                text: "Experimental. Requires a compatible Proton build and display"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Esync:"
                enabled: !form.isLinux
                checked: form.gameData.esync === true
                onToggled: form.gameData.esync = checked
                text: "Eventfd-based Wine sync. Disable if you hit file-descriptor limits"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Fsync:"
                enabled: !form.isLinux
                checked: form.gameData.fsync === true
                onToggled: form.gameData.fsync = checked
                text: "Futex-based Wine sync. Preferred when the kernel supports it"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Gamescope:"
                checked: form.gameData.gamescope === true
                onToggled: form.gameData.gamescope = checked
                text: "Nested compositor for scaling, a stable session, and optional HDR"
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "Compatibility"
                Kirigami.FormData.isSection: true
            }

            QQC2.Switch {
                Kirigami.FormData.label: "DXVK:"
                enabled: !form.isLinux
                checked: form.gameData.dxvk === true
                onToggled: form.gameData.dxvk = checked
                text: "Direct3D 8–11 through Vulkan. Turn off to use WineD3D instead"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "VKD3D:"
                enabled: !form.isLinux
                checked: form.gameData.vkd3d === true
                onToggled: form.gameData.vkd3d = checked
                text: "Direct3D 12 through Vulkan"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "DXVK-NVAPI / DLSS:"
                enabled: !form.isLinux
                checked: form.gameData.nvapi === true
                onToggled: form.gameData.nvapi = checked
                text: "NVIDIA NVAPI and DLSS. Needs a Proton runner through UMU"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "AMD FSR:"
                enabled: !form.isLinux
                checked: form.gameData.fsr === true
                onToggled: form.gameData.fsr = checked
                text: "Wine fullscreen FidelityFX Super Resolution"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "BattlEye runtime:"
                enabled: !form.isLinux
                checked: form.gameData.battleye === true
                onToggled: form.gameData.battleye = checked
                text: "Proton BattlEye helper for supported online games"
            }
            QQC2.Switch {
                Kirigami.FormData.label: "Easy Anti-Cheat runtime:"
                enabled: !form.isLinux
                checked: form.gameData.eac === true
                onToggled: form.gameData.eac = checked
                text: "Proton EAC helper for supported online games"
            }
            QQC2.Switch {
                id: desktopSwitch
                Kirigami.FormData.label: "Virtual desktop:"
                enabled: !form.isLinux
                checked: form.gameData.virtual_desktop === true
                onToggled: form.gameData.virtual_desktop = checked
                text: "Run the game inside a Wine desktop window"
            }
            QQC2.TextField {
                id: desktopSizeField
                Kirigami.FormData.label: "Desktop size:"
                enabled: !form.isLinux && desktopSwitch.checked
                placeholderText: "1920x1080"
                text: form.gameData.virtualDesktopSize || "1920x1080"
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "Advanced"
                Kirigami.FormData.isSection: true
            }

            QQC2.TextField {
                id: extraField
                Kirigami.FormData.label: "Additional application:"
                placeholderText: "Optional helper launched in the same prefix"
                text: form.gameData.additionalApp || ""
            }
            QQC2.TextField {
                id: envField
                Kirigami.FormData.label: "Environment variables:"
                placeholderText: "KEY=value pairs. Overrides the toggles above"
                text: form.gameData.environment || ""
            }
        }
    }

    FileDialog {
        id: exeDialog
        title: "Select an executable"
        nameFilters: ["Windows executables (*.exe *.EXE)", "All files (*)"]
        onAccepted: {
            var path = backend.urlToLocalFile(selectedFile.toString())
            exeField.text = path
            if (nameField.text.trim() === "" && path !== "") {
                var base = path.split("/").pop()
                var dot = base.lastIndexOf(".")
                nameField.text = dot > 0 ? base.substring(0, dot) : base
            }
        }
    }

    FileDialog {
        id: coverDialog
        title: "Select a cover image"
        nameFilters: ["Images (*.png *.jpg *.jpeg *.webp)"]
        onAccepted: {
            var path = backend.importCustomCover(
                form.gameData.gameId, selectedFile.toString())
            if (path !== "") {
                form.gameData.coverPath = path
                coverPreview.source = backend.coverUrlFor(path)
            }
        }
    }
}
