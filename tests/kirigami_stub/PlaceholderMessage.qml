import QtQuick
import QtQuick.Layouts

ColumnLayout {
    component IconGroup: QtObject {
        property string name: ""
    }

    property string text: ""
    property string explanation: ""
    readonly property IconGroup icon: IconGroup {}
}
