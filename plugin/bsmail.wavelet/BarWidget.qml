pragma ComponentBehavior: Bound
import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root

  moduleName: "bsmail.wavelet"
  readonly property string bin: "@WAVELET_BIN@"
  readonly property string statusPath: Quickshell.env("XDG_RUNTIME_DIR") + "/wavelet/status.json"

  property string playState: "off"
  property string kind: ""
  property string title: ""
  property string subtitle: ""
  property string stationId: ""
  property int reportedVolume: 0
  property int volume: 0
  property var stations: []
  property var upNext: []
  property bool statusLoaded: false

  property int pendingVolume: -1
  property int pendingVolumeDelta: 0
  property bool popupOpen: false
  property string tab: "stations"

  readonly property bool running: root.playState !== "off"
  readonly property bool playing: root.playState === "playing"
  readonly property bool actionBusy: actionProcess.running
  readonly property color foreground: root.bar ? root.bar.foreground : Color.foreground
  readonly property color mutedForeground: root.bar ? Qt.darker(root.bar.foreground, 1.4) : Color.foreground
  readonly property string fontFamily: root.bar ? root.bar.fontFamily : Style.font.family

  function singleLineText(value, limit) {
    return String(value || "").replace(/[\r\n\t]+/g, " ").slice(0, limit)
  }

  function safeTooltipText(value) {
    return root.singleLineText(value, 160).replace(/</g, "‹").replace(/>/g, "›")
  }

  function clampVolume(value, fallback) {
    var v = Math.round(Number(value))
    return isFinite(v) ? Math.max(0, Math.min(130, v)) : fallback
  }

  function parseStations(value) {
    if (!Array.isArray(value)) return []
    var out = []
    for (var i = 0; i < value.length && out.length < 200; i++) {
      var s = value[i]
      if (!s || typeof s !== "object") continue
      var id = root.singleLineText(s.id, 64)
      if (id) out.push({ id: id, name: root.singleLineText(s.name, 160) })
    }
    return out
  }

  function parseUpNext(value) {
    if (!Array.isArray(value)) return []
    var out = []
    for (var i = 0; i < value.length && out.length < 10; i++) {
      var e = value[i]
      if (!e || typeof e !== "object") continue
      var entry = Math.round(Number(e.entry))
      if (!isFinite(entry) || entry < 0) continue
      out.push({
        entry: entry,
        title: root.singleLineText(e.title, 160),
        subtitle: root.singleLineText(e.subtitle, 160)
      })
    }
    return out
  }

  function applyStatus(raw) {
    try {
      if (typeof raw !== "string" || raw.length > 262144) return
      var status = JSON.parse(raw || "{}")
      if (!status || typeof status !== "object") return
      var states = ["off", "idle", "loading", "playing", "paused"]
      root.playState = states.indexOf(status.state) >= 0 ? status.state : "off"
      root.kind = status.kind === "track" || status.kind === "station" ? status.kind : ""
      root.title = root.singleLineText(status.title, 160)
      root.subtitle = root.singleLineText(status.subtitle, 160)
      root.stationId = root.singleLineText(status.station_id, 64)
      root.reportedVolume = root.clampVolume(status.volume, root.reportedVolume)
      if (!root.volumeBusy()) root.volume = root.reportedVolume
      root.stations = root.parseStations(status.stations)
      root.upNext = root.parseUpNext(status.up_next)
    } catch (error) {
      return
    }
  }

  function markOff() {
    root.playState = "off"
    root.kind = ""
    root.title = ""
    root.subtitle = ""
    root.stationId = ""
    root.upNext = []
  }

  // FileView does not notice a file that appears after it failed to load,
  // so a missing status.json is retried until the daemon writes one.
  function rearmStatus() {
    if (root.statusLoaded) return
    statusView.path = ""
    statusView.path = root.statusPath
  }

  function ctl(args) {
    if (actionProcess.running) return
    button.hideOwnTooltip()
    actionProcess.command = [root.bin, "ctl"].concat(args)
    actionProcess.running = true
  }

  function volumeBusy() {
    return volumeProcess.running || root.pendingVolume >= 0 || root.pendingVolumeDelta !== 0
  }

  function changeVolume(delta) {
    var step = delta > 0 ? 5 : -5
    root.volume = Math.max(0, Math.min(130, root.volume + step))
    if (root.pendingVolume >= 0) root.pendingVolume = root.volume
    else root.pendingVolumeDelta += step
    root.flushVolume()
  }

  function setVolume(value) {
    var v = root.clampVolume(value, -1)
    if (v < 0) return
    root.pendingVolume = v
    root.pendingVolumeDelta = 0
    root.volume = v
    root.flushVolume()
  }

  function flushVolume() {
    if (volumeProcess.running) return
    var arg
    if (root.pendingVolume >= 0) {
      arg = String(root.pendingVolume)
      root.pendingVolume = -1
    } else if (root.pendingVolumeDelta !== 0) {
      arg = (root.pendingVolumeDelta > 0 ? "+" : "") + root.pendingVolumeDelta
      root.pendingVolumeDelta = 0
    } else {
      return
    }
    volumeProcess.command = [root.bin, "ctl", "volume", arg]
    volumeProcess.running = true
  }

  function openTui() {
    root.close()
    if (!root.bar) return
    root.bar.run("omarchy-launch-or-focus-tui --app-id=org.omarchy.wavelet " + root.bin)
  }

  function togglePopup() {
    button.hideOwnTooltip()
    if (!root.popupOpen) root.tab = root.kind === "track" ? "upnext" : "stations"
    root.popupOpen = !root.popupOpen
  }

  function close() {
    root.popupOpen = false
  }

  function stateText() {
    if (root.playState === "loading") return "Loading"
    if (root.playState === "paused") return "Paused"
    if (root.playState === "playing") return root.kind === "station" ? "Live" : "Playing"
    if (root.playState === "idle") return "Stopped"
    return ""
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  FileView {
    id: statusView
    path: root.statusPath
    watchChanges: true
    atomicWrites: true
    printErrors: false
    onLoaded: {
      root.statusLoaded = true
      root.applyStatus(text())
    }
    onLoadFailed: function(error) {
      root.statusLoaded = false
      root.markOff()
    }
    onFileChanged: reload()
  }

  Timer {
    interval: 3000
    repeat: true
    running: !root.statusLoaded
    onTriggered: root.rearmStatus()
  }

  Process {
    id: actionProcess
    command: []
    onExited: function(exitCode) {
      if (exitCode === 0) root.rearmStatus()
    }
  }

  Process {
    id: volumeProcess
    command: []
    onExited: function(exitCode) {
      if (exitCode === 0) root.rearmStatus()
      if (root.pendingVolume >= 0 || root.pendingVolumeDelta !== 0) {
        Qt.callLater(root.flushVolume)
        return
      }
      if (exitCode !== 0) root.volume = root.reportedVolume
    }
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: ""
    active: root.playing
    tooltipText: !root.running
      ? "wavelet is off  ·  click to open, right-click for controls"
      : (root.title
        ? root.safeTooltipText(root.title)
          + (root.subtitle ? "  ·  " + root.safeTooltipText(root.subtitle) : "")
        : "Nothing playing")
        + (root.playState === "paused" ? "  ·  paused" : root.playState === "loading" ? "  ·  loading" : "")
        + "  ·  " + root.volume + "%"

    onPressed: function(mouseButton) {
      if (!root.bar) return
      if (mouseButton === Qt.RightButton) {
        root.togglePopup()
        return
      }
      if (mouseButton === Qt.MiddleButton) {
        root.ctl(["toggle"])
        return
      }
      root.openTui()
    }

    onWheelMoved: function(delta) {
      root.changeVolume(delta)
    }
  }

  PopupCard {
    id: popup
    anchorItem: button
    bar: root.bar
    owner: root
    open: root.popupOpen
    contentWidth: popup.fittedContentWidth(Style.space(320))
    contentHeight: popup.fittedContentHeight(popupColumn.implicitHeight, Style.space(500))

    Column {
      id: popupColumn
      anchors.fill: parent
      spacing: Style.space(10)

      Text {
        width: parent.width
        textFormat: Text.PlainText
        text: root.title || (root.running ? "Nothing playing" : "wavelet is off")
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.subtitle
        font.bold: true
        elide: Text.ElideRight
      }

      Text {
        width: parent.width
        textFormat: Text.PlainText
        text: !root.running
          ? "Play something to start it"
          : [root.subtitle, root.stateText(), root.volume + "%"]
            .filter(function(part) { return part !== "" }).join("  ·  ")
        color: root.mutedForeground
        font.family: root.fontFamily
        font.pixelSize: Style.font.caption
        elide: Text.ElideRight
      }

      Row {
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Style.space(6)

        Button {
          iconText: ""
          tooltipText: "Previous"
          foreground: root.foreground
          enabled: !root.actionBusy
          opacity: enabled ? 1.0 : 0.4
          onClicked: root.ctl(["prev"])
        }

        Button {
          iconText: root.playing || root.playState === "loading" ? "" : ""
          tooltipText: root.playing || root.playState === "loading" ? "Pause" : "Play"
          foreground: root.foreground
          enabled: !root.actionBusy
          opacity: enabled ? 1.0 : 0.4
          onClicked: root.ctl(["toggle"])
        }

        Button {
          iconText: ""
          tooltipText: "Next"
          foreground: root.foreground
          enabled: !root.actionBusy
          opacity: enabled ? 1.0 : 0.4
          onClicked: root.ctl(["next"])
        }
      }

      Row {
        width: parent.width
        spacing: Style.space(8)

        Text {
          id: volumeIcon
          anchors.verticalCenter: parent.verticalCenter
          textFormat: Text.PlainText
          text: root.volume === 0 ? "" : ""
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
        }

        PanelSlider {
          anchors.verticalCenter: parent.verticalCenter
          width: parent.width - volumeIcon.width - percentLabel.width - parent.spacing * 2
          bar: root.bar
          minimum: 0
          maximum: 100
          step: 5
          integer: true
          value: Math.min(100, root.volume)
          onMoved: function(nextVolume) { root.setVolume(nextVolume) }
        }

        Text {
          id: percentLabel
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(38)
          textFormat: Text.PlainText
          text: root.volume + "%"
          color: root.mutedForeground
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          horizontalAlignment: Text.AlignRight
        }
      }

      PanelSeparator {
        foreground: root.foreground
      }

      ButtonGroup {
        options: [
          { value: "stations", label: "Stations (" + root.stations.length + ")" },
          { value: "upnext", label: "Up next (" + root.upNext.length + ")" }
        ]
        value: root.tab
        focusable: false
        foreground: root.foreground
        fontFamily: root.fontFamily
        fontSize: Style.font.caption
        onChanged: function(value) { root.tab = value }
      }

      ListView {
        width: parent.width
        visible: root.tab === "stations"
        height: visible ? Math.min(contentHeight, Style.space(208)) : 0
        model: root.stations
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: Style.space(2)

        delegate: Item {
          id: stationRow
          required property var modelData
          width: ListView.view.width
          height: Style.space(34)

          readonly property bool isPlaying: root.stationId !== ""
            && modelData.id === root.stationId

          Rectangle {
            anchors.fill: parent
            radius: Style.cornerRadius
            color: stationRow.isPlaying
              ? Style.selectedFillFor(root.foreground, Color.accent)
              : (stationMouse.containsMouse
                ? Style.hoverFillFor(root.foreground, Color.accent)
                : "transparent")
          }

          Rectangle {
            visible: stationRow.isPlaying
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 2
            color: Color.accent
          }

          Text {
            anchors.left: parent.left
            anchors.leftMargin: Style.space(10)
            anchors.right: parent.right
            anchors.rightMargin: Style.space(8)
            anchors.verticalCenter: parent.verticalCenter
            textFormat: Text.PlainText
            text: stationRow.modelData.name || "Unknown station"
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.bodySmall
            font.bold: stationRow.isPlaying
            elide: Text.ElideRight
          }

          MouseArea {
            id: stationMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.ctl(["station", stationRow.modelData.id])
          }
        }
      }

      ListView {
        width: parent.width
        visible: root.tab === "upnext"
        height: visible ? Math.min(contentHeight, Style.space(208)) : 0
        model: root.upNext
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        spacing: Style.space(2)

        delegate: Item {
          id: entryRow
          required property var modelData
          width: ListView.view.width
          height: Style.space(40)

          Rectangle {
            anchors.fill: parent
            radius: Style.cornerRadius
            color: entryMouse.containsMouse
              ? Style.hoverFillFor(root.foreground, Color.accent)
              : "transparent"
          }

          Column {
            anchors.left: parent.left
            anchors.leftMargin: Style.space(10)
            anchors.right: parent.right
            anchors.rightMargin: Style.space(8)
            anchors.verticalCenter: parent.verticalCenter

            Text {
              width: parent.width
              textFormat: Text.PlainText
              text: entryRow.modelData.title || "Unknown"
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }

            Text {
              width: parent.width
              visible: text !== ""
              textFormat: Text.PlainText
              text: entryRow.modelData.subtitle
              color: root.mutedForeground
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              elide: Text.ElideRight
            }
          }

          MouseArea {
            id: entryMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.ctl(["jump", String(entryRow.modelData.entry)])
          }
        }
      }

      Text {
        width: parent.width
        visible: root.tab === "stations" ? root.stations.length === 0 : root.upNext.length === 0
        textFormat: Text.PlainText
        text: root.tab === "stations"
          ? "No favorite stations yet. Press f on a station in wavelet."
          : "Nothing queued."
        color: root.bar ? Qt.darker(root.bar.foreground, 1.5) : Color.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.caption
        wrapMode: Text.Wrap
      }

      Button {
        width: parent.width
        leftAlign: true
        text: "Open wavelet"
        iconText: ""
        foreground: root.foreground
        onClicked: root.openTui()
      }
    }
  }
}
