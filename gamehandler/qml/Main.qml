// Application shell: navigation drawer, page switching, notifications, and
// the app-wide keyboard shortcuts.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import org.kde.kirigami as Kirigami

Kirigami.ApplicationWindow {
    id: root

    title: "GameHandler"
    width: 1180
    height: 760
    minimumWidth: 420
    minimumHeight: 480

    property string currentPage: "library"

    readonly property LibraryPage libraryPage: LibraryPage {}
    readonly property InstallersPage installersPage: InstallersPage {}
    readonly property RunnersPage runnersPage: RunnersPage {}
    readonly property PluginsPage pluginsPage: PluginsPage {}
    readonly property CreditsPage creditsPage: CreditsPage {}
    readonly property SettingsPage settingsPage: SettingsPage {}

    function pageFor(name) {
        switch (name) {
        case "installers": return installersPage
        case "runners": return runnersPage
        case "plugins": return pluginsPage
        case "credits": return creditsPage
        case "settings": return settingsPage
        default: return libraryPage
        }
    }

    function showPage(name) {
        currentPage = name
        while (pageStack.layers.depth > 1)
            pageStack.layers.pop()
        pageStack.clear()
        pageStack.push(pageFor(name))
        if (name === "plugins")
            backend.refreshPlugins()
    }

    function openGameForm(gameId) {
        var data = gameId ? backend.getGame(gameId) : backend.newGameTemplate()
        pageStack.layers.push(Qt.resolvedUrl("GameFormPage.qml"), {
            gameData: data,
            isNew: !gameId,
            dismiss: function() { root.pageStack.layers.pop() }
        })
    }

    pageStack.initialPage: libraryPage

    globalDrawer: Kirigami.GlobalDrawer {
        id: drawer
        modal: false
        collapsible: true
        title: "GameHandler"

        actions: [
            Kirigami.Action {
                text: "Library"
                icon.name: "applications-games"
                checkable: true
                checked: root.currentPage === "library"
                onTriggered: root.showPage("library")
            },
            Kirigami.Action {
                text: "Installers"
                icon.name: "run-install"
                checkable: true
                checked: root.currentPage === "installers"
                onTriggered: root.showPage("installers")
            },
            Kirigami.Action {
                text: "Runners"
                icon.name: "folder-download"
                checkable: true
                checked: root.currentPage === "runners"
                onTriggered: root.showPage("runners")
            },
            Kirigami.Action {
                text: "Plugins"
                icon.name: "plugins"
                checkable: true
                checked: root.currentPage === "plugins"
                onTriggered: root.showPage("plugins")
            },
            Kirigami.Action {
                text: "About & Credits"
                icon.name: "help-about"
                checkable: true
                checked: root.currentPage === "credits"
                onTriggered: root.showPage("credits")
            },
            Kirigami.Action {
                text: "Settings"
                icon.name: "configure"
                checkable: true
                checked: root.currentPage === "settings"
                onTriggered: root.showPage("settings")
            }
        ]

        // Lands in the drawer's content area, below the navigation actions.
        QQC2.ToolButton {
            text: "Powered by Wine, Proton & DXVK"
            font.pointSize: Kirigami.Theme.smallFont.pointSize
            opacity: 0.7
            visible: !drawer.collapsed
            onClicked: root.showPage("credits")
        }
    }

    // ------------------------------------------------------------ shortcuts

    Shortcut {
        sequence: "Ctrl+N"
        context: Qt.ApplicationShortcut
        onActivated: root.openGameForm("")
    }
    Shortcut {
        sequence: "Ctrl+F"
        context: Qt.ApplicationShortcut
        onActivated: {
            root.showPage("library")
            root.libraryPage.focusSearch()
        }
    }
    Shortcut {
        sequence: "Ctrl+,"
        context: Qt.ApplicationShortcut
        onActivated: root.showPage("settings")
    }
    Shortcut {
        sequence: "Ctrl+Q"
        context: Qt.ApplicationShortcut
        onActivated: backend.quit()
    }

    // -------------------------------------------------- backend integration

    Connections {
        target: backend

        function onNotify(message) {
            root.showPassiveNotification(message)
        }

        function onGameInstalled(gameId, message) {
            root.showPage("library")
            // Installing a store launcher is only half the job — the user
            // still has to open it and sign in, so offer that right here.
            root.showPassiveNotification(message, "long", "Play", function() {
                backend.playGame(gameId)
            })
        }

        function onEasyInstallNeedsExe(token, name, startUrl) {
            locateDialog.token = token
            locateDialog.title = "Locate " + name
            locateDialog.currentFolder = startUrl
            locateDialog.open()
        }

        function onRequestHide() {
            root.visible = false
        }

        function onRequestShow() {
            // A hidden window cannot show the "stopped right away" message.
            root.visible = true
            root.raise()
            root.requestActivate()
        }
    }

    FileDialog {
        id: locateDialog
        property string token: ""
        nameFilters: ["Windows executables (*.exe *.EXE)", "All files (*)"]
        onAccepted: backend.completeEasyInstall(token, selectedFile.toString())
        onRejected: backend.cancelEasyInstall(token)
    }
}
