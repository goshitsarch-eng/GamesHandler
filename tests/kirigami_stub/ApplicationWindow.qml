import QtQuick
import QtQuick.Controls as QQC2

QQC2.ApplicationWindow {
    id: win

    component LayersApi: QtObject {
        property Item host
        property var items: []
        property int depth: 1
        function push(source, props) {
            var comp = Qt.createComponent(source)
            if (comp.status === Component.Error) {
                console.error("LAYER-ERROR:", comp.errorString())
                return null
            }
            var obj = comp.createObject(host, props || {})
            if (obj === null) {
                console.error("LAYER-ERROR: createObject returned null for", source)
                return null
            }
            var copy = items.slice(); copy.push(obj); items = copy
            depth = items.length + 1
            return obj
        }
        function pop() {
            var copy = items.slice()
            var top = copy.pop()
            items = copy
            depth = items.length + 1
            if (top) top.destroy()
            return top
        }
    }

    component PageStackApi: QtObject {
        property Item host
        property var initialPage: null
        onInitialPageChanged: if (initialPage) push(initialPage)
        property var pages: []
        readonly property LayersApi layers: LayersApi { host: win.contentItem }
        function push(page) {
            if (page && page.parent !== undefined)
                page.parent = host
            var copy = pages.slice(); copy.push(page); pages = copy
        }
        function clear() { pages = [] }
    }

    property var globalDrawer: null
    property string lastNotification: ""

    function showPassiveNotification(message, timeout, actionText, callBack) {
        lastNotification = message
    }

    readonly property PageStackApi pageStack: PageStackApi { host: win.contentItem }
}
