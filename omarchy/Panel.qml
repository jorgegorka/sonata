import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.jorgegorka.sonata"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null

  readonly property color foreground: barForeground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  function open() {
    install.refresh()
    root.controller.show()
  }

  function close() {
    root.controller.hide()
  }

  Component.onCompleted: install.refresh()

  function switchPanel(direction) {
    if (root.bar && typeof root.bar.switchPanelFrom === "function")
      return root.bar.switchPanelFrom(root.hostWidget || root, direction)
    return false
  }

  Commons.Translations {
    id: strings
    directory: Qt.resolvedUrl("i18n")
  }

  // ~/.local/bin/sonata is only ours when the installer wrote its marker.
  // Another program may already own that command name.
  QtObject {
    id: install

    readonly property string debPath: "/usr/bin/sonata"

    property string home: ""
    property bool launcherExecutable: false
    property bool debExecutable: false
    property bool appImageExecutable: false
    property bool probed: false
    property bool busy: false
    property bool again: false
    property int pending: 0

    readonly property string launcherPath: home + "/.local/bin/sonata"
    readonly property string appImagePath: home + "/.local/opt/sonata/AppRun"
    readonly property bool launcherOwned: markerPresent(launcherView.text())
    readonly property string binary: {
      if (!probed) return ""
      if (launcherOwned && launcherExecutable) return launcherPath
      if (debExecutable) return debPath
      if (appImageExecutable) return appImagePath
      return ""
    }

    function markerPresent(raw) {
      var lines = String(raw || "").split("\n")
      var limit = Math.min(lines.length, 8)
      for (var i = 0; i < limit; i++) {
        if (lines[i] === "# Sonata installer") return true
      }
      return false
    }

    function safeHome() {
      var value = String(Quickshell.env("HOME") || "")
      if (value.charAt(0) !== "/") return ""
      if (value.indexOf("\n") >= 0 || value.indexOf("\r") >= 0) return ""
      return value
    }

    function refresh() {
      if (busy) {
        again = true
        return
      }
      busy = true
      again = false
      probed = false
      home = safeHome()
      launcherExecutable = false
      debExecutable = false
      appImageExecutable = false
      pending = 0
      if (home !== "") {
        start(launcherProbe, launcherPath)
        start(appImageProbe, appImagePath)
      }
      start(debProbe, debPath)
      if (pending === 0) finish()
    }

    function start(proc, path) {
      pending += 1
      proc.command = ["test", "-x", path]
      proc.running = true
    }

    function note(kind, ok) {
      if (!busy) return
      if (kind === "launcher") launcherExecutable = ok
      else if (kind === "deb") debExecutable = ok
      else if (kind === "appimage") appImageExecutable = ok
      pending -= 1
      if (pending <= 0) finish()
    }

    function finish() {
      busy = false
      probed = true
      if (again) refresh()
    }

    function openApp() {
      if (binary === "" || clientsProc.running) return
      clientsProc.command = ["hyprctl", "clients", "-j"]
      clientsProc.running = true
    }

    function handleClients(exitCode, raw) {
      var foundClass = false
      var foundAppId = false
      var foundInitial = false
      if (exitCode === 0) {
        try {
          var clients = JSON.parse(raw || "[]")
          if (!Array.isArray(clients)) return
          for (var i = 0; i < clients.length; i++) {
            var client = clients[i] || {}
            var klass = String(client["class"] || "")
            var initial = String(client.initialClass || "")
            if (klass === "Sonata" || initial === "Sonata") foundClass = true
            if (klass === "io.sonata.desktop") foundAppId = true
            if (initial === "io.sonata.desktop") foundInitial = true
          }
        } catch (e) {
          return
        }
      }
      // A second process exits with the already-running dialog, so an
      // existing window is focused with a fixed matcher before any launch.
      if (foundClass) {
        Quickshell.execDetached(["hyprctl", "dispatch", "focuswindow", "class:^(Sonata)$"])
        return
      }
      if (foundAppId) {
        Quickshell.execDetached(["hyprctl", "dispatch", "focuswindow", "class:^(io\\.sonata\\.desktop)$"])
        return
      }
      if (foundInitial) {
        Quickshell.execDetached(["hyprctl", "dispatch", "focuswindow", "initialclass:^(io\\.sonata\\.desktop)$"])
        return
      }
      if (binary !== "") Quickshell.execDetached([binary])
    }

    function installApp() {
      var dir = String(Qt.resolvedUrl("."))
      if (dir.indexOf("file://") === 0) dir = dir.slice(7)
      try { dir = decodeURIComponent(dir) } catch (e) {}
      if (dir.slice(-1) === "/") dir = dir.slice(0, -1)
      var script = dir + "/install-app.sh"
      if (script.indexOf("\n") >= 0 || script.indexOf("'") >= 0) return
      Quickshell.execDetached([
        "omarchy-launch-floating-terminal-with-presentation",
        "bash '" + script + "'"
      ])
    }
  }

  FileView {
    id: launcherView
    path: install.home === "" ? "" : install.launcherPath
    watchChanges: true
    printErrors: false
    blockLoading: true
  }

  Process {
    id: launcherProbe
    running: false
    command: ["test", "-x", "/usr/bin/sonata"]
    onExited: function(code) { install.note("launcher", code === 0) }
  }

  Process {
    id: debProbe
    running: false
    command: ["test", "-x", "/usr/bin/sonata"]
    onExited: function(code) { install.note("deb", code === 0) }
  }

  Process {
    id: appImageProbe
    running: false
    command: ["test", "-x", "/usr/bin/sonata"]
    onExited: function(code) { install.note("appimage", code === 0) }
  }

  Process {
    id: clientsProc
    running: false
    command: ["hyprctl", "clients", "-j"]
    stdout: StdioCollector {
      id: clientsOut
      waitForEnd: true
    }
    onExited: function(code) {
      install.handleClients(code, String(clientsOut.text || ""))
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.hostWidget || root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(420))
    contentHeight: panel.fittedContentHeight(content.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: content
        width: parent.width
        spacing: Style.space(8)

        Text {
          width: parent.width
          textFormat: Text.PlainText
          text: strings.t("panel.title")
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.subtitle
          font.bold: true
          wrapMode: Text.WordWrap
        }

        Button {
          visible: install.probed && install.binary !== ""
          width: parent.width
          text: strings.t("panel.open")
          fontFamily: root.fontFamily
          foreground: root.foreground
          onClicked: install.openApp()
        }

        Text {
          visible: install.probed && install.binary === ""
          width: parent.width
          textFormat: Text.PlainText
          text: strings.t("panel.missing")
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
          wrapMode: Text.WordWrap
        }

        Button {
          visible: install.probed && install.binary === ""
          width: parent.width
          text: strings.t("panel.install")
          fontFamily: root.fontFamily
          foreground: root.foreground
          onClicked: install.installApp()
        }
      }
    }
  }
}
