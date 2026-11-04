import QtQuick
import QtQuick.Controls as QQC
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui
import "I18n.js" as I18n

// Layoutless bar widget: the flag of the current layout (dimmed while
// auto-correction is paused; a keyboard icon while the daemon is away) that
// switches the layout on click, and a panel on right click: the auto and
// per-window toggles and buttons that open the learned words and the ignored
// apps, each in a window in the middle of the screen. All state
// lives in the layoutless daemon; this widget only talks to it through the
// `layoutless` CLI: `subscribe` for live state, `ctl` for edits. Commands go
// to `ctl` on stdin, never as arguments: words in a command line are visible
// to every local user through `ps`.
Panel {
  id: root
  moduleName: "kladafox.layoutless"
  ipcTarget: "kladafox.layoutless"

  // Interface text in the system's language (I18n.js).
  readonly property var tr: I18n.translator(I18n.textLanguage(function(name) { return Quickshell.env(name) }))

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // From the daemon. `daemonUp` is false while `layoutless subscribe` isn't
  // connected (daemon stopped or restarting).
  property bool daemonUp: false
  property bool auto: false
  // "en" or "ru", the current layout.
  property string layout: ""
  // Each window keeps its own layout (`per_window` in the config).
  property bool perWindow: false
  property int ruleCount: 0
  property string flipKey: "ALT + SPACE"
  // The focused window's class (the panel itself never takes it) and the
  // apps where auto-correction is off.
  property string app: ""
  property bool appExcluded: false
  property var excludeApps: []
  property int learnThreshold: 2
  // [{keys, en, ru, lang, created_at}], newest first.
  property var rules: []
  // The add field doubles as a filter: the list shows the rules where either
  // spelling contains what's typed (so `ghbdtn` finds `привет` too).
  readonly property string filter: addField.text.trim().toLowerCase()
  readonly property var shownRules: filter === ""
    ? rules
    : rules.filter(function(r) {
        return r.en.toLowerCase().indexOf(filter) !== -1 || r.ru.toLowerCase().indexOf(filter) !== -1
      })
  onFilterChanged: cursorIndex = -1
  property string lastError: ""
  // The list window that's open: "words", "apps", or "" for none.
  property string list: ""

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property color dim: Qt.darker(foreground, 1.55)

  // Row cursor, per the CursorSurface contract: hover moves it.
  property int cursorIndex: -1

  // The word as it belongs, and how it gets typed by mistake.
  function wordOf(rule) { return rule.lang === "en" ? rule.en : rule.ru }
  function otherOf(rule) { return rule.lang === "en" ? rule.ru : rule.en }

  function refresh() {
    if (!listProc.running) listProc.running = true
  }

  function run(cmd) {
    if (actionProc.running) return
    actionProc.pendingCommand = cmd
    actionProc.running = true
  }

  function addWord() {
    var word = addField.text.trim()
    if (word === "") return
    run("rules add " + word)
    addField.text = ""
  }

  // The list windows replace the panel, like the panel replaces the bar icon.
  function showList(name) {
    root.close()
    root.list = name
    if (name === "words") root.refresh()
  }

  function closeList() {
    root.list = ""
    addField.text = ""
    root.cursorIndex = -1
  }

  // Esc step by step: clear the field, then close.
  function back() {
    if (root.list === "words" && addField.text !== "") addField.text = ""
    else root.closeList()
  }

  onOpenedChanged: if (opened) Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  onListChanged: {
    if (list === "words") Qt.callLater(function() { addField.forceActiveFocus() })
    else if (list === "apps") Qt.callLater(function() { listKeys.forceActiveFocus() })
  }

  IpcHandler {
    target: "kladafox.layoutless-lists"
    function words(): void { root.showList("words") }
    function apps(): void { root.showList("apps") }
    function close(): void { root.closeList() }
  }

  Process {
    id: watchProc
    command: ["setpriv", "--pdeathsig", "TERM", "layoutless", "subscribe"]
    onRunningChanged: if (!running) root.daemonUp = false
    onExited: watchRestart.restart()
    stdout: SplitParser {
      onRead: function(line) {
        var ev
        try { ev = JSON.parse(line) } catch (e) { return }
        root.daemonUp = true
        if (ev.event === "state") {
          root.auto = ev.auto === true
          root.layout = ev.layout || ""
          root.perWindow = ev.per_window === true
          if (ev.flip_key) root.flipKey = ev.flip_key
          if (ev.learn_threshold) root.learnThreshold = ev.learn_threshold
          root.app = ev.app || ""
          root.appExcluded = ev.app_excluded === true
          root.excludeApps = ev.exclude_apps || []
          if (ev.rules !== root.ruleCount && root.list === "words") root.refresh()
          root.ruleCount = ev.rules || 0
        }
      }
    }
  }

  Timer {
    id: watchRestart
    interval: 2000
    onTriggered: if (!watchProc.running) watchProc.running = true
  }

  Component.onCompleted: watchProc.running = true

  Process {
    id: listProc
    command: ["layoutless", "rules", "list", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var list
        try { list = JSON.parse(text || "{}").rules || [] } catch (e) { return }
        list.sort(function(a, b) { return b.created_at - a.created_at })
        root.rules = list
      }
    }
  }

  Process {
    id: actionProc
    property string pendingCommand: ""
    command: ["layoutless", "ctl"]
    stdinEnabled: true
    onStarted: write(pendingCommand + "\n")
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var out = String(text || "").trim()
        root.lastError = out.indexOf("error") === 0 ? out.replace(/^error:\s*/, "") : ""
      }
    }
    onExited: root.refresh()
  }

  readonly property var flags: ({ en: "🇺🇸", ru: "🇷🇺" })
  readonly property string flag: root.daemonUp ? (root.flags[root.layout] || "") : ""
  readonly property string langName: root.layout === "ru" ? root.tr("Russian") : root.tr("English")

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.flag || "󰌌"
    // Flags read too small at body size, so give them room.
    fontSize: root.flag ? Math.round(Style.font.body * 1.17) : Style.font.body
    horizontalMargin: Style.spacing.rowPaddingX / 2
    dimmed: !root.daemonUp || !root.auto
    tooltipText: !root.daemonUp
      ? root.tr("Layoutless: daemon not running")
      : (root.auto ? root.tr("Layout: %1, auto-correction on", root.langName) : root.tr("Layout: %1, auto-correction paused", root.langName))
        + "\n" + root.tr("Click to switch the layout, right click for settings")
    onPressed: function(b) {
      if (b === Qt.RightButton || !root.daemonUp) root.toggle()
      else root.run("layout toggle")
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(340))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(900))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        width: parent.width
        spacing: Style.space(10)

        Text {
          visible: !root.daemonUp
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: root.tr("The layoutless daemon is not running.\nStart it with: systemctl --user start layoutless\n(installed with --system: sudo systemctl start layoutless@$USER)")
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
        }

        // The flag of the current layout, the state, and the auto-correction
        // switch at the right edge, like the other panels.
        PanelHero {
          title: root.tr("Keyboard layout")
          meta: !root.daemonUp ? root.tr("Daemon not running")
            : root.auto ? root.tr("Auto-correction on") : root.tr("Auto-correction paused")
          foreground: root.foreground
          fontFamily: root.fontFamily
          iconComponent: Text {
            text: root.flag || "󰌌"
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.display
          }
          trailingControl: ToggleSwitch {
            visible: root.daemonUp
            checked: root.auto
            foreground: root.foreground
            onToggled: root.run("auto toggle")
            PanelToolTip { visible: parent.containsMouse; text: root.auto ? root.tr("Pause auto-correction") : root.tr("Turn auto-correction on") }
          }
        }

        PanelSeparator { foreground: root.foreground }

        Toggle {
          visible: root.daemonUp
          width: parent.width
          label: root.tr("Layout per window")
          description: root.tr("A window gets its layout back on focus, new ones start on English")
          checked: root.perWindow
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.run("per-window toggle")
        }

        Button {
          visible: root.daemonUp
          width: parent.width
          leftAlign: true
          bordered: true
          iconText: "󰗊"
          text: root.tr("Learned words") + " · " + root.ruleCount
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.showList("words")
        }

        Button {
          visible: root.daemonUp
          width: parent.width
          leftAlign: true
          bordered: true
          iconText: "󰜺"
          text: root.tr("Ignored apps") + " · " + root.excludeApps.length
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.showList("apps")
        }

        Text {
          visible: root.lastError !== ""
          width: parent.width
          wrapMode: Text.Wrap
          textFormat: Text.PlainText
          text: root.lastError
          color: root.bar ? root.bar.urgent : Color.urgent
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }
      }
    }
  }

  // The learned words and the ignored apps: a modal in the middle of the
  // screen, like predmaxim.todo. A click on the dimmed screen or Esc closes it.
  PanelWindow {
    id: modal
    screen: root.QsWindow.window ? root.QsWindow.window.screen : null
    visible: root.list !== ""
    color: Color.menu.scrim
    exclusionMode: ExclusionMode.Ignore
    anchors { top: true; bottom: true; left: true; right: true }
    WlrLayershell.namespace: "layoutless"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    MouseArea { anchors.fill: parent; onClicked: root.closeList() }

    BorderSurface {
      id: card
      anchors.centerIn: parent
      width: Math.min(Style.space(480), modal.width - Style.space(80))
      height: Math.min(listColumn.implicitHeight + card.contentTopInset + card.contentBottomInset, modal.height * 0.85)
      color: Color.popups.background
      borderSpec: Border.surfaceSpec("popups", "border", Color.popups.border, Math.max(1, Style.space(2)))
      padding: Style.spacing.panelPadding
      radius: Style.cornerRadius

      MouseArea { anchors.fill: parent }   // clicks on the card stay on it

      PanelKeyCatcher {
        id: listKeys
        anchors.fill: parent
        anchors.topMargin: card.contentTopInset
        anchors.rightMargin: card.contentRightInset
        anchors.bottomMargin: card.contentBottomInset
        anchors.leftMargin: card.contentLeftInset
        onCloseRequested: root.back()

        Column {
          id: listColumn
          anchors.fill: parent
          spacing: Style.space(14)

          PanelHero {
            title: root.list === "words" ? root.tr("Learned words") : root.tr("Ignored apps")
            meta: root.list === "words" ? root.tr("Retyped in the other layout") : root.tr("Words aren't corrected there")
            foreground: root.foreground
            fontFamily: root.fontFamily
            iconComponent: Text {
              text: root.list === "words" ? "󰗊" : "󰜺"
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.display
            }
            trailingControl: Text {
              text: root.list === "words" ? root.rules.length : root.excludeApps.length
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.displayLarge
              font.bold: true
            }
          }

          Toggle {
            visible: root.list === "apps" && root.app !== ""
            width: parent.width
            label: root.tr("Don't correct in %1", root.app)
            description: root.tr("%1 still works there", root.flipKey)
            checked: root.appExcluded
            foreground: root.foreground
            fontFamily: root.fontFamily
            onClicked: root.run("exclude " + (root.appExcluded ? "rm " : "add ") + root.app)
          }

          Row {
            visible: root.list === "words"
            width: parent.width
            spacing: Style.space(8)

            TextField {
              id: addField
              width: parent.width - addButton.width - parent.spacing
              placeholderText: root.tr("Find or add a word: next, привет")
              foreground: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.heading
              onAccepted: root.addWord()
              // Tab does nothing in a centred modal.
              Keys.onTabPressed: {}
            }

            Button {
              id: addButton
              anchors.verticalCenter: parent.verticalCenter
              text: root.tr("Add")
              iconText: "󰐕"
              bordered: true
              fontSize: Style.font.title
              iconSize: Style.font.title
              foreground: root.foreground
              fontFamily: root.fontFamily
              onClicked: root.addWord()
            }
          }

          Text {
            visible: root.lastError !== ""
            width: parent.width
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: root.lastError
            color: root.bar ? root.bar.urgent : Color.urgent
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }

          Text {
            visible: root.list === "words" && root.rules.length === 0
            width: parent.width
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: root.learnThreshold === 1
              ? root.tr("No learned words yet. Flip the same word with %1 and it's remembered.", root.flipKey)
              : root.tr("No learned words yet. Flip the same word with %1 %2 times and it's remembered.", root.flipKey, root.learnThreshold)
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }

          Text {
            visible: root.list === "words" && root.rules.length > 0 && root.shownRules.length === 0
            width: parent.width
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: root.tr("Nothing matches. Press Enter to add it.")
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }

          // A ListView that only scrolls when the rows don't fit, like the
          // Bluetooth panel: a Flickable here kept nudging the content under a
          // still pointer, so the row buttons flickered between hovered and not.
          ListView {
            id: rulesView
            visible: root.list === "words" && root.shownRules.length > 0
            width: parent.width
            height: Math.min(contentHeight, modal.height * 0.5)
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentHeight > height
            model: root.shownRules

            // Always shown while the list scrolls, so it's clear there's more
            // (AsNeeded only shows it during a scroll).
            QQC.ScrollBar.vertical: QQC.ScrollBar {
              policy: rulesView.interactive ? QQC.ScrollBar.AlwaysOn : QQC.ScrollBar.AlwaysOff
            }

            delegate: CursorSurface {
              id: row
              required property int index
              required property var modelData
              // Leave room for the scrollbar when the list scrolls.
              width: ListView.view.width - (rulesView.interactive ? Style.space(10) : 0)
              height: Style.space(44)
              foreground: root.foreground
              hasCursor: root.cursorIndex === row.index

              // The row counts as hovered while the pointer is on it or on one
              // of its buttons: the row's MouseArea reports leaving as soon as
              // the pointer moves onto a button.
              property int hotButtons: 0
              readonly property bool hot: rowMouse.containsMouse || hotButtons > 0
              onHotChanged: {
                if (hot) root.cursorIndex = row.index
                else if (root.cursorIndex === row.index) root.cursorIndex = -1
              }

              MouseArea {
                id: rowMouse
                anchors.fill: parent
                hoverEnabled: true
              }

              Text {
                anchors.left: parent.left
                anchors.leftMargin: Style.space(8)
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                text: root.wordOf(row.modelData)
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.heading
              }

              Text {
                anchors.left: parent.left
                anchors.leftMargin: Style.space(170)
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                text: "← " + root.otherOf(row.modelData)
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.body
              }

              Row {
                anchors.right: parent.right
                anchors.rightMargin: Style.space(6)
                anchors.verticalCenter: parent.verticalCenter
                spacing: Style.space(4)
                // Always there, faint until the row is hovered.
                opacity: row.hasCursor ? 1 : 0.35

                PanelActionButton {
                  iconText: "󰓡"
                  foreground: root.dim
                  fontFamily: root.fontFamily
                  onHovered: function(on) { row.hotButtons += on ? 1 : -1 }
                  onClicked: root.run("rules set " + row.modelData.keys + " " + (row.modelData.lang === "en" ? "ru" : "en"))
                }

                PanelActionButton {
                  iconText: "󰆴"
                  foreground: root.dim
                  hoverColor: root.bar ? root.bar.urgent : Color.urgent
                  fontFamily: root.fontFamily
                  onHovered: function(on) { row.hotButtons += on ? 1 : -1 }
                  onClicked: root.run("rules rm " + root.wordOf(row.modelData))
                }
              }
            }
          }

          Text {
            visible: root.list === "words" && root.rules.length > 0
            width: parent.width
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: root.tr("󰓡 keep the other spelling instead   󰆴 forget")
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }

          Text {
            visible: root.list === "apps" && root.excludeApps.length === 0
            width: parent.width
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: root.tr("No ignored apps yet.")
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
          }

          ListView {
            id: appsView
            visible: root.list === "apps" && root.excludeApps.length > 0
            width: parent.width
            height: Math.min(contentHeight, modal.height * 0.5)
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentHeight > height
            model: root.excludeApps

            QQC.ScrollBar.vertical: QQC.ScrollBar {
              policy: appsView.interactive ? QQC.ScrollBar.AlwaysOn : QQC.ScrollBar.AlwaysOff
            }

            delegate: Item {
              id: appRow
              required property var modelData
              width: ListView.view.width - (appsView.interactive ? Style.space(10) : 0)
              height: Style.space(40)

              Text {
                anchors.left: parent.left
                anchors.leftMargin: Style.space(8)
                anchors.right: removeApp.left
                anchors.verticalCenter: parent.verticalCenter
                elide: Text.ElideRight
                textFormat: Text.PlainText
                text: appRow.modelData
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.heading
              }

              PanelActionButton {
                id: removeApp
                anchors.right: parent.right
                anchors.rightMargin: Style.space(6)
                anchors.verticalCenter: parent.verticalCenter
                iconText: "󰆴"
                foreground: root.dim
                hoverColor: root.bar ? root.bar.urgent : Color.urgent
                fontFamily: root.fontFamily
                onClicked: root.run("exclude rm " + appRow.modelData)
              }
            }
          }
        }
      }
    }
  }
}
