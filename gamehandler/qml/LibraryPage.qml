// The library: grid and list views, search, category filter, sorting, and
// every per-game action (play, edit, prefix tools, shortcut, remove).
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page

    title: "Library"
    padding: 0

    function focusSearch() {
        searchField.forceActiveFocus()
    }

    actions: [
        Kirigami.Action {
            text: "Add Game"
            icon.name: "list-add"
            onTriggered: root.openGameForm("")
        }
    ]

    header: QQC2.ToolBar {
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            Kirigami.SearchField {
                id: searchField
                Layout.fillWidth: true
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                placeholderText: "Search games…"
                onTextChanged: backend.searchText = text
            }

            QQC2.ComboBox {
                id: categoryBox
                model: backend.categories
                QQC2.ToolTip.text: "Filter by category"
                QQC2.ToolTip.visible: hovered
                onActivated: backend.categoryFilter = currentText
                Connections {
                    target: backend
                    function onCategoriesChanged() {
                        var current = backend.categoryFilter
                        var index = backend.categories.indexOf(current)
                        categoryBox.currentIndex = index >= 0 ? index : 0
                        if (index < 0)
                            backend.categoryFilter = "All"
                    }
                }
            }

            QQC2.ComboBox {
                id: sortBox
                textRole: "label"
                valueRole: "key"
                model: backend.sortOptions
                QQC2.ToolTip.text: "Sort library"
                QQC2.ToolTip.visible: hovered
                Component.onCompleted: currentIndex = indexOfValue(backend.sortMode)
                onActivated: backend.sortMode = currentValue
            }

            QQC2.ToolButton {
                icon.name: backend.viewMode === "list" ? "view-list-icons" : "view-list-details"
                checkable: true
                checked: backend.viewMode === "list"
                QQC2.ToolTip.text: "Toggle list view"
                QQC2.ToolTip.visible: hovered
                onToggled: backend.viewMode = checked ? "list" : "grid"
            }

            Item { Layout.fillWidth: true }
        }
    }

    // ------------------------------------------------------------ empty states

    Kirigami.PlaceholderMessage {
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 4
        visible: backend.librarySize === 0
        icon.name: "applications-games"
        text: "No games yet"
        explanation: "Add a Windows or Linux game, install a store launcher in one click, or download a Proton build to get started."

        ColumnLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.smallSpacing
            QQC2.Button {
                Layout.alignment: Qt.AlignHCenter
                text: "Add your first game"
                icon.name: "list-add"
                onClicked: root.openGameForm("")
            }
            QQC2.Button {
                Layout.alignment: Qt.AlignHCenter
                text: "Easy install"
                onClicked: root.showPage("installers")
            }
            QQC2.Button {
                Layout.alignment: Qt.AlignHCenter
                text: "Download a runner"
                onClicked: root.showPage("runners")
            }
        }
    }

    Kirigami.PlaceholderMessage {
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 4
        visible: backend.librarySize > 0 && backend.games.length === 0
        icon.name: "edit-find"
        text: "No matching games"
        explanation: "Try a different search, or clear the category filter."

        QQC2.Button {
            Layout.alignment: Qt.AlignHCenter
            text: "Clear filters"
            onClicked: {
                searchField.text = ""
                backend.categoryFilter = "All"
            }
        }
    }

    // ------------------------------------------------------------------- grid

    GridView {
        id: grid
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing
        visible: backend.viewMode === "grid" && backend.games.length > 0
        clip: true
        model: backend.viewMode === "grid" ? backend.games : []
        cellWidth: 200
        cellHeight: 300
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

        delegate: Item {
            id: cardSlot
            required property var modelData
            width: grid.cellWidth
            height: grid.cellHeight

            Rectangle {
                id: card
                anchors.fill: parent
                anchors.margins: Kirigami.Units.smallSpacing
                radius: 14
                color: cardHover.hovered
                    ? Qt.alpha(Kirigami.Theme.highlightColor, 0.12)
                    : Kirigami.Theme.alternateBackgroundColor

                HoverHandler { id: cardHover }
                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    onDoubleTapped: backend.playGame(cardSlot.modelData.gameId)
                }
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: page.openGameMenu(cardSlot.modelData, card)
                }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: Kirigami.Units.smallSpacing
                    spacing: Kirigami.Units.smallSpacing

                    CoverArt {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        coverUrl: cardSlot.modelData.coverUrl
                        coverIsIcon: cardSlot.modelData.coverIsIcon
                        initials: cardSlot.modelData.initials
                        accent: cardSlot.modelData.accent
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: cardSlot.modelData.name
                        font.bold: true
                        elide: Text.ElideRight
                        horizontalAlignment: Text.AlignHCenter
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: cardSlot.modelData.subtitle
                        opacity: 0.7
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        elide: Text.ElideRight
                        horizontalAlignment: Text.AlignHCenter
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        QQC2.Button {
                            text: "Play"
                            icon.name: "media-playback-start"
                            onClicked: backend.playGame(cardSlot.modelData.gameId)
                        }
                        QQC2.ToolButton {
                            icon.name: "view-more-symbolic"
                            QQC2.ToolTip.text: "More actions"
                            QQC2.ToolTip.visible: hovered
                            onClicked: page.openGameMenu(cardSlot.modelData, this)
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------- list

    ListView {
        id: list
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing
        visible: backend.viewMode === "list" && backend.games.length > 0
        clip: true
        model: backend.viewMode === "list" ? backend.games : []
        spacing: Kirigami.Units.smallSpacing
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

        delegate: QQC2.ItemDelegate {
            id: row
            required property var modelData
            width: ListView.view.width
            height: Kirigami.Units.gridUnit * 3.4

            onDoubleClicked: backend.playGame(row.modelData.gameId)
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: page.openGameMenu(row.modelData, row)
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.largeSpacing

                CoverArt {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 2
                    Layout.preferredHeight: Kirigami.Units.gridUnit * 2.8
                    coverUrl: row.modelData.coverUrl
                    coverIsIcon: row.modelData.coverIsIcon
                    initials: row.modelData.initials
                    accent: row.modelData.accent
                    compact: true
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: row.modelData.name
                        font.bold: true
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: row.modelData.subtitle + " · " + row.modelData.lastPlayed
                        opacity: 0.7
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        elide: Text.ElideRight
                    }
                }

                QQC2.Button {
                    text: "Play"
                    icon.name: "media-playback-start"
                    onClicked: backend.playGame(row.modelData.gameId)
                }
                QQC2.ToolButton {
                    icon.name: "view-more-symbolic"
                    onClicked: page.openGameMenu(row.modelData, this)
                }
            }
        }
    }

    // ----------------------------------------------------------- game actions

    property var menuGame: null

    function openGameMenu(game, anchorItem) {
        menuGame = game
        gameMenu.popup(anchorItem)
    }

    QQC2.Menu {
        id: gameMenu

        QQC2.MenuItem {
            text: "Play"
            icon.name: "media-playback-start"
            onTriggered: backend.playGame(page.menuGame.gameId)
        }
        QQC2.MenuItem {
            text: "Edit"
            icon.name: "edit-entry"
            onTriggered: root.openGameForm(page.menuGame.gameId)
        }
        QQC2.MenuItem {
            text: "Find cover art"
            icon.name: "viewimage"
            onTriggered: backend.fetchCover(page.menuGame.gameId)
        }
        QQC2.MenuSeparator {}
        QQC2.MenuItem {
            text: "Winecfg"
            enabled: page.menuGame !== null && !page.menuGame.isLinux
            onTriggered: backend.runPrefixTool(page.menuGame.gameId, "winecfg")
        }
        QQC2.MenuItem {
            text: "Winetricks"
            enabled: page.menuGame !== null && !page.menuGame.isLinux
            onTriggered: backend.runPrefixTool(page.menuGame.gameId, "winetricks")
        }
        QQC2.MenuItem {
            text: "Open prefix folder"
            icon.name: "folder-open"
            enabled: page.menuGame !== null && !page.menuGame.isLinux
            onTriggered: backend.openPrefix(page.menuGame.gameId)
        }
        QQC2.MenuSeparator {}
        QQC2.MenuItem {
            text: "Create desktop shortcut"
            onTriggered: backend.createShortcut(page.menuGame.gameId)
        }
        QQC2.MenuItem {
            text: "Remove from library"
            icon.name: "delete"
            onTriggered: removeDialog.open()
        }
    }

    Kirigami.PromptDialog {
        id: removeDialog
        title: page.menuGame ? "Remove “" + page.menuGame.name + "”?" : ""
        subtitle: "This removes the game from your GameHandler library. Its Wine prefix and game files are left on disk."
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                text: "Remove"
                icon.name: "delete"
                onTriggered: {
                    backend.removeGame(page.menuGame.gameId)
                    removeDialog.close()
                }
            }
        ]
    }
}
