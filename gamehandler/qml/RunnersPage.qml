// Runners: download, switch, and remove Proton/Wine builds.
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page

    title: "Runners"

    property string selectedFamilyId: "proton-ge"
    property var pendingRemove: null

    function familyAt(index) {
        return backend.runnerFamilies[index]
    }

    Component.onCompleted: backend.fetchReleases(selectedFamilyId)

    actions: [
        Kirigami.Action {
            text: "Refresh"
            icon.name: "view-refresh"
            enabled: backend.releasesStatus !== "loading"
            onTriggered: backend.fetchReleases(page.selectedFamilyId)
        }
    ]

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        QQC2.ProgressBar {
            Layout.fillWidth: true
            visible: backend.busy && backend.progress >= 0
            from: 0; to: 1
            value: Math.max(0, backend.progress)
        }

        Kirigami.Heading {
            level: 3
            text: "Installed"
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: "Available for launching and for the per-game runner picker."
            opacity: 0.7
            wrapMode: Text.WordWrap
        }

        Repeater {
            model: backend.installedRunners
            delegate: Kirigami.AbstractCard {
                id: installedCard
                required property var modelData
                Layout.fillWidth: true
                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    Kirigami.Icon {
                        source: installedCard.modelData.available ? "emblem-checked" : "data-warning"
                        Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                        Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        QQC2.Label { text: installedCard.modelData.name; font.bold: true }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: installedCard.modelData.detail
                            opacity: 0.7
                            elide: Text.ElideRight
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }
                    }
                    QQC2.ToolButton {
                        visible: installedCard.modelData.removable
                        icon.name: "delete"
                        QQC2.ToolTip.text: "Remove " + installedCard.modelData.name
                        QQC2.ToolTip.visible: hovered
                        onClicked: {
                            page.pendingRemove = installedCard.modelData
                            removeRunnerDialog.open()
                        }
                    }
                }
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: backend.installedRunners.length <= 1
            text: "No downloaded runners yet. Install a Proton or Wine build below, then assign it to a game."
            opacity: 0.7
            wrapMode: Text.WordWrap
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading {
            level: 3
            text: "Download a build"
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: "GameHandler fetches these archives from each maintainer's own release page, the same upstream sources ProtonPlus uses. Nothing is bundled or re-hosted here."
            opacity: 0.7
            wrapMode: Text.WordWrap
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            QQC2.ComboBox {
                id: familyBox
                Kirigami.FormData.label: "Family:"
                textRole: "name"
                valueRole: "familyId"
                model: backend.runnerFamilies
                onActivated: {
                    page.selectedFamilyId = currentValue
                    backend.fetchReleases(currentValue)
                }
            }

            QQC2.Label {
                Kirigami.FormData.label: ""
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                opacity: 0.8
                text: {
                    var family = backend.runnerFamilies[familyBox.currentIndex]
                    if (!family)
                        return ""
                    var line = family.description
                    if (family.maintainer)
                        line += "\nMaintained by " + family.maintainer
                    return line
                }
            }

            Kirigami.UrlButton {
                Kirigami.FormData.label: "Project:"
                url: {
                    var family = backend.runnerFamilies[familyBox.currentIndex]
                    return family ? family.homepage : ""
                }
            }
        }

        Kirigami.Heading {
            level: 3
            text: "Available versions"
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: backend.releasesStatus !== "ready" || backend.releases.length === 0
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: backend.releasesStatus === "loading"
                ? "Fetching the latest builds…"
                : backend.releasesStatus.indexOf("error:") === 0
                    ? "Could not fetch builds — " + backend.releasesStatus.substring(7)
                    : "No builds found for this family."
        }

        Repeater {
            model: backend.releases
            delegate: Kirigami.AbstractCard {
                id: releaseCard
                required property var modelData
                Layout.fillWidth: true
                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        QQC2.Label { text: releaseCard.modelData.tag; font.bold: true }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: releaseCard.modelData.familyName + " · "
                                  + releaseCard.modelData.assetName + " · "
                                  + releaseCard.modelData.sizeMb + " MB"
                            opacity: 0.7
                            elide: Text.ElideRight
                            font.pointSize: Kirigami.Theme.smallFont.pointSize
                        }
                    }
                    Kirigami.Chip {
                        visible: releaseCard.modelData.installed
                        text: "Installed"
                        closable: false
                        checkable: false
                    }
                    QQC2.Button {
                        visible: !releaseCard.modelData.installed
                        text: "Install"
                        icon.name: "download"
                        enabled: !backend.busy
                        onClicked: backend.installRelease(releaseCard.modelData.tag)
                    }
                }
            }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading {
            level: 3
            text: "Which runner should I use?"
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: "Proton builds are the usual choice for Windows games; standalone Wine is lighter and better for some older titles. Each entry links to the project that maintains it."
            opacity: 0.7
            wrapMode: Text.WordWrap
        }

        Repeater {
            model: backend.runnerGuide
            delegate: Kirigami.AbstractCard {
                id: guideCard
                required property var modelData
                Layout.fillWidth: true
                contentItem: ColumnLayout {
                    spacing: 2
                    RowLayout {
                        Layout.fillWidth: true
                        Kirigami.Heading { level: 4; text: guideCard.modelData.title }
                        Item { Layout.fillWidth: true }
                        Kirigami.UrlButton {
                            visible: guideCard.modelData.homepage !== ""
                            url: guideCard.modelData.homepage
                            text: "Visit project"
                        }
                    }
                    QQC2.Label {
                        text: guideCard.modelData.maintainer
                            ? guideCard.modelData.kind + " · maintained by " + guideCard.modelData.maintainer
                            : guideCard.modelData.kind
                        opacity: 0.6
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: guideCard.modelData.advice
                        wrapMode: Text.WordWrap
                        opacity: 0.85
                    }
                }
            }
        }
    }

    Kirigami.PromptDialog {
        id: removeRunnerDialog
        title: page.pendingRemove ? "Remove " + page.pendingRemove.name + "?" : ""
        subtitle: "The downloaded build is deleted from disk. Games using it fall back to System Wine until you pick another runner."
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                text: "Remove"
                icon.name: "delete"
                onTriggered: {
                    backend.uninstallRunner(page.pendingRemove.runnerId)
                    removeRunnerDialog.close()
                }
            }
        ]
    }
}
