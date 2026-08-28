// Plugins: detect optional launch helpers and offer to install them.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    title: "Plugins"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Heading {
            level: 3
            text: "Host plugins"
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: backend.pluginsIntro
            opacity: 0.7
            wrapMode: Text.WordWrap
        }

        Repeater {
            model: backend.plugins
            delegate: Kirigami.AbstractCard {
                id: pluginCard
                required property var modelData
                Layout.fillWidth: true
                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        QQC2.Label { text: pluginCard.modelData.name; font.bold: true }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: pluginCard.modelData.subtitle
                            wrapMode: Text.WordWrap
                            opacity: 0.75
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }
                    }
                    QQC2.Button {
                        text: pluginCard.modelData.state === "installed"
                            ? "Installed"
                            : pluginCard.modelData.state === "missing" ? "Install" : "Unavailable"
                        enabled: pluginCard.modelData.state === "missing"
                        onClicked: backend.installPlugin(pluginCard.modelData.pluginId)
                    }
                }
            }
        }
    }
}
