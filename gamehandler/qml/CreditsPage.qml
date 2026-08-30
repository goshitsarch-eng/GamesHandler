// Credits: the upstream projects GameHandler runs on, and why it exists.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    title: "About & Credits"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Heading {
            level: 3
            text: "Standing on other people's work"
        }

        QQC2.Label {
            text: "Made by Gosh."
            font.bold: true
        }
        QQC2.Label {
            text: "Version " + backend.appVersion
            opacity: 0.8
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: backend.acknowledgement
            wrapMode: Text.WordWrap
            opacity: 0.85
        }

        Repeater {
            model: backend.creditSections

            delegate: ColumnLayout {
                id: section
                required property var modelData
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Kirigami.Separator { Layout.fillWidth: true }

                Kirigami.Heading {
                    level: 3
                    text: section.modelData.title
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: section.modelData.summary
                    wrapMode: Text.WordWrap
                    opacity: 0.7
                }

                Repeater {
                    model: section.modelData.entries
                    delegate: Kirigami.AbstractCard {
                        id: creditCard
                        required property var modelData
                        Layout.fillWidth: true
                        contentItem: RowLayout {
                            spacing: Kirigami.Units.largeSpacing
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: creditCard.modelData.label
                                    font.bold: true
                                    wrapMode: Text.WordWrap
                                }
                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: creditCard.modelData.license
                                        ? creditCard.modelData.role + "\nLicense: " + creditCard.modelData.license
                                        : creditCard.modelData.role
                                    wrapMode: Text.WordWrap
                                    opacity: 0.75
                                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                                }
                            }
                            Kirigami.UrlButton {
                                visible: creditCard.modelData.url !== ""
                                url: creditCard.modelData.url
                                text: "Visit"
                            }
                        }
                    }
                }
            }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading {
            level: 3
            text: "Why one app instead of assembling the stack yourself"
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: "GameHandler replaces none of these projects. It removes the assembly work between them."
            wrapMode: Text.WordWrap
            opacity: 0.7
        }

        Repeater {
            model: backend.whyAllInOne
            delegate: ColumnLayout {
                id: why
                required property var modelData
                Layout.fillWidth: true
                spacing: 2
                Kirigami.Heading { level: 4; text: why.modelData.heading }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: why.modelData.body
                    wrapMode: Text.WordWrap
                    opacity: 0.8
                }
            }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        QQC2.Label {
            Layout.fillWidth: true
            text: "GameHandler " + backend.appVersion
                + " — GPL-3.0-or-later. Proton and Wine builds are downloaded from "
                + "their maintainers at your request and remain under their own licenses."
            wrapMode: Text.WordWrap
            opacity: 0.6
            font.pointSize: Kirigami.Theme.smallFont.pointSize
        }
        Kirigami.UrlButton {
            url: "https://github.com/goshitsarch-eng/GamesHandler"
            text: "GameHandler on GitHub"
        }
    }
}
