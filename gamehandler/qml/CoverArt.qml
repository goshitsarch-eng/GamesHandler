// A game's cover, or a coloured initials plate when it has none. Artwork
// taken from an executable's own icon is letterboxed on the plate instead of
// cropped: an app icon is square and mostly transparent, and cropping it to a
// portrait tile cuts the logo in half.
import QtQuick
import org.kde.kirigami as Kirigami

Rectangle {
    id: art

    property string coverUrl: ""
    property bool coverIsIcon: false
    property string initials: "?"
    property int accent: 0
    property bool compact: false

    // The same eight gradients the tiles have always used, so a game keeps
    // its shade across sessions.
    readonly property var gradients: [
        ["#3f6fd8", "#23407f"], ["#8a4fd6", "#4b2380"],
        ["#1f8f78", "#10513f"], ["#c1533f", "#6f2a20"],
        ["#b8862c", "#6d4a12"], ["#2c7fa8", "#164b64"],
        ["#a4406e", "#5d1f3c"], ["#4c6b8a", "#2a3c50"]
    ]
    readonly property bool showPlate: coverUrl === "" || coverIsIcon
    readonly property var shade: gradients[Math.max(0, accent) % gradients.length]

    radius: compact ? 6 : 10
    clip: true
    color: showPlate ? "transparent" : Kirigami.Theme.alternateBackgroundColor
    gradient: showPlate ? plateGradient : null

    Gradient {
        id: plateGradient
        GradientStop { position: 0.0; color: art.shade[0] }
        GradientStop { position: 1.0; color: art.shade[1] }
    }

    Text {
        anchors.centerIn: parent
        visible: art.coverUrl === ""
        text: art.initials
        color: "#ffffff"
        opacity: 0.92
        font.bold: true
        font.pixelSize: art.compact
            ? Math.round(art.height * 0.42)
            : Math.round(Math.min(art.width, art.height) * 0.34)
        style: Text.Raised
        styleColor: "#66000000"
    }

    Image {
        anchors.fill: parent
        anchors.margins: art.coverIsIcon ? Kirigami.Units.largeSpacing : 0
        visible: art.coverUrl !== ""
        source: art.coverUrl
        // Portrait store art fills the tile; a transparent app icon sits on
        // the plate uncropped.
        fillMode: art.coverIsIcon ? Image.PreserveAspectFit : Image.PreserveAspectCrop
        asynchronous: true
        cache: true
        sourceSize.width: 512
        sourceSize.height: 512
        smooth: true
    }
}
