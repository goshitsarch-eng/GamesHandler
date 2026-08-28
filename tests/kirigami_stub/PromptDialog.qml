import QtQuick

QtObject {
    property string title: ""
    property string subtitle: ""
    property int standardButtons: 0
    property list<QtObject> customFooterActions
    signal accepted()
    signal rejected()
    function open() {}
    function close() {}
}
