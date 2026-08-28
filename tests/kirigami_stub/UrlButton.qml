import QtQuick.Controls as QQC2

QQC2.Button {
    property url url
    text: url.toString()
}
