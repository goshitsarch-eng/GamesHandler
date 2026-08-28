// Settings: appearance, defaults for new games, and behavior.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    title: "Settings"

    readonly property var defaultRows: [
        { key: "mangohud", label: "Enable MangoHud by default", subtitle: "" },
        { key: "gamemode", label: "Enable GameMode by default", subtitle: "" },
        { key: "prefer_sdl", label: "Prefer SDL by default", subtitle: "" },
        { key: "esync", label: "Enable Esync by default", subtitle: "Eventfd-based Wine sync. Usually leave this on." },
        { key: "fsync", label: "Enable Fsync by default", subtitle: "Futex-based Wine sync. Preferred when the kernel supports it." },
        { key: "dxvk", label: "Enable DXVK by default", subtitle: "Direct3D 8–11 through Vulkan." },
        { key: "vkd3d", label: "Enable VKD3D by default", subtitle: "Direct3D 12 through Vulkan." },
        { key: "nvapi", label: "Enable DXVK-NVAPI / DLSS by default", subtitle: "Only needed for some NVIDIA / DLSS titles." },
        { key: "fsr", label: "Enable AMD FSR by default", subtitle: "" },
        { key: "battleye", label: "Enable BattlEye runtime by default", subtitle: "" },
        { key: "eac", label: "Enable Easy Anti-Cheat runtime by default", subtitle: "" },
        { key: "gamescope", label: "Enable Gamescope by default", subtitle: "" },
        { key: "virtual_desktop", label: "Enable virtual desktop by default", subtitle: "" }
    ]

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Kirigami.Separator {
                Kirigami.FormData.label: "Appearance"
                Kirigami.FormData.isSection: true
            }

            QQC2.ComboBox {
                id: schemeBox
                Kirigami.FormData.label: "Color scheme:"
                textRole: "label"
                valueRole: "key"
                model: [
                    { key: "system", label: "Match system" },
                    { key: "light", label: "Light" },
                    { key: "dark", label: "Dark" }
                ]
                Component.onCompleted: currentIndex = indexOfValue(backend.colorScheme)
                onActivated: backend.colorScheme = currentValue
            }

            QQC2.ComboBox {
                id: viewBox
                Kirigami.FormData.label: "Library layout:"
                textRole: "label"
                valueRole: "key"
                model: [
                    { key: "grid", label: "Grid" },
                    { key: "list", label: "List" }
                ]
                Component.onCompleted: currentIndex = indexOfValue(backend.viewMode)
                onActivated: backend.viewMode = currentValue
                Connections {
                    target: backend
                    function onSettingsChanged() {
                        viewBox.currentIndex = viewBox.indexOfValue(backend.viewMode)
                    }
                }
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "New games"
                Kirigami.FormData.isSection: true
            }

            QQC2.ComboBox {
                id: runnerBox
                Kirigami.FormData.label: "Default runner:"
                textRole: "label"
                valueRole: "runnerId"
                model: backend.runnerChoices
                Component.onCompleted: {
                    var index = indexOfValue(backend.defaultRunner)
                    currentIndex = index >= 0 ? index : 0
                }
                onActivated: backend.defaultRunner = currentValue
                Connections {
                    target: backend
                    function onRunnersChanged() {
                        var index = runnerBox.indexOfValue(backend.defaultRunner)
                        runnerBox.currentIndex = index >= 0 ? index : 0
                    }
                }
            }
        }

        Repeater {
            model: page.defaultRows
            delegate: QQC2.Switch {
                id: defaultSwitch
                required property var modelData
                Layout.fillWidth: true
                text: modelData.subtitle
                    ? modelData.label + " — " + modelData.subtitle
                    : modelData.label
                checked: backend.defaultToggles[modelData.key] === true
                onToggled: backend.setDefaultToggle(modelData.key, checked)
            }
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Kirigami.Separator {
                Kirigami.FormData.label: "Behavior"
                Kirigami.FormData.isSection: true
            }

            QQC2.Switch {
                Kirigami.FormData.label: "Hide window when launching:"
                text: "Keeps the launcher out of the way while a game starts"
                checked: backend.closeOnLaunch
                onToggled: backend.closeOnLaunch = checked
            }

            Kirigami.Separator {
                Kirigami.FormData.label: "Keyboard shortcuts"
                Kirigami.FormData.isSection: true
            }

            QQC2.Label { Kirigami.FormData.label: "Ctrl+N:"; text: "Add a game" }
            QQC2.Label { Kirigami.FormData.label: "Ctrl+F:"; text: "Search the library" }
            QQC2.Label { Kirigami.FormData.label: "Ctrl+,:"; text: "Settings" }
            QQC2.Label { Kirigami.FormData.label: "Ctrl+Q:"; text: "Quit" }
        }
    }
}
