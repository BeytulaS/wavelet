import QtQuick
import qs.Commons
import qs.Ui

BarWidget {
  id: root

  moduleName: "bsmail.wavelet"
  readonly property string bin: "@WAVELET_BIN@"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: ""
  }
}
