import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Bridge in the Omarchy bar: one icon with the number of agents working, lit
// when any is waiting for you. It reads `bridge-cli status --json --watch` (one
// JSON object per line) and never touches Bridge's database or socket; the
// CLI does the authenticated handshake, so this file holds no secrets.
//
// Left click: review approvals when something waits, otherwise Mission
// Control. Right click: the floating quick prompt.
BarWidget {
  id: root
  moduleName: "app.bridge.status"

  property bool running: false
  property int working: 0
  property int waiting: 0

  visible: running
  implicitWidth: running ? button.implicitWidth : 0
  implicitHeight: button.implicitHeight

  function apply(line) {
    var status
    try {
      status = JSON.parse(String(line))
    } catch (error) {
      return
    }
    root.running = status.running === true
    root.working = Number(status.working) || 0
    root.waiting = Number(status.waiting) || 0
  }

  Process {
    id: status
    command: ["bridge-cli", "status", "--json", "--watch"]
    running: true
    stdout: SplitParser { onRead: function(line) { root.apply(line) } }
    // bridged stopped, or the CLI is missing: hide, then try again.
    onExited: {
      root.running = false
      retry.start()
    }
  }

  Timer {
    id: retry
    interval: 5000
    onTriggered: status.running = true
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "󰚩"
    labelText: root.waiting > 0 ? root.working + " · " + root.waiting + " waiting" : String(root.working)
    active: root.waiting > 0
    onPressed: function(buttonCode) {
      if (!root.bar) return
      if (buttonCode === Qt.RightButton) root.bar.run("bridge-deck --quick")
      else if (root.waiting > 0) root.bar.run("bridge-deck --approvals")
      else root.bar.run("bridge-deck --mission-control")
    }
  }
}
