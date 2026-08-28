// Installers: curated one-click setup for store launchers and apps.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    title: "Installers"

    header: QQC2.ToolBar {
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing
            Kirigami.SearchField {
                Layout.fillWidth: true
                Layout.maximumWidth: Kirigami.Units.gridUnit * 22
                placeholderText: "Search installers…"
                onTextChanged: backend.installerSearch = text
            }
            QQC2.ComboBox {
                model: backend.installerCategories
                onActivated: backend.installerCategory = currentText
            }
            Item { Layout.fillWidth: true }
        }
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: true
            text: "GameHandler downloads the vendor's official Windows installer, runs it in a fresh isolated Wine prefix, then adds the result to your library. You complete the vendor's own wizard — silent-install flags are unreliable under Wine."
        }

        QQC2.ProgressBar {
            Layout.fillWidth: true
            visible: backend.busy && backend.progress >= 0
            from: 0; to: 1
            value: Math.max(0, backend.progress)
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            QQC2.ComboBox {
                id: runnerBox
                Kirigami.FormData.label: "Runner for new installs:"
                textRole: "label"
                valueRole: "runnerId"
                model: backend.runnerChoices
                Component.onCompleted: {
                    var index = indexOfValue(backend.defaultRunner)
                    currentIndex = index >= 0 ? index : 0
                }
                Connections {
                    target: backend
                    function onRunnersChanged() {
                        var index = runnerBox.indexOfValue(backend.defaultRunner)
                        runnerBox.currentIndex = index >= 0 ? index : 0
                    }
                }
            }
            QQC2.Label {
                Kirigami.FormData.label: ""
                text: "Each install gets its own prefix under your data directory. Official vendor downloads only — no game or launcher files are redistributed."
                opacity: 0.7
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }

        Kirigami.Heading {
            level: 3
            text: "Catalog"
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            visible: backend.installers.length === 0
            text: "No matching installers"
            explanation: "Try a different search, or switch the filter back to All."
        }

        Repeater {
            model: backend.installers

            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                Layout.fillWidth: true

                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        RowLayout {
                            spacing: Kirigami.Units.smallSpacing
                            Kirigami.Heading {
                                level: 4
                                text: card.modelData.name
                            }
                            Kirigami.Chip {
                                text: card.modelData.category
                                closable: false
                                checkable: false
                            }
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: card.modelData.subtitle
                            wrapMode: Text.WordWrap
                            opacity: 0.8
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }
                    }

                    QQC2.Button {
                        text: "Install"
                        icon.name: "run-install"
                        enabled: !backend.busy
                        QQC2.ToolTip.text: "Download and run the official " + card.modelData.name + " installer"
                        QQC2.ToolTip.visible: hovered
                        onClicked: backend.installEasy(
                            card.modelData.installerId,
                            runnerBox.currentValue !== undefined ? runnerBox.currentValue : "")
                    }
                }
            }
        }
    }
}
