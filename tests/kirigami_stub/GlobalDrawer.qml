import QtQuick

Item {
    property string title: ""
    property bool modal: false
    property bool collapsible: false
    property bool collapsed: false
    property bool showHeaderWhenCollapsed: false
    property list<QtObject> actions
    property Item footer
    property Item header
}
