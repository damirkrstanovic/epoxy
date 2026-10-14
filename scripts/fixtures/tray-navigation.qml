//@ pragma UseQApplication
import QtQuick
import Quickshell
import Quickshell.Services.SystemTray
ShellRoot {
    id: root
    property var trayItem: SystemTray.items.values.length ? SystemTray.items.values[0] : null
    property int stage: 0
    // Navigation pattern reproduced from DMS v1.6.2 SystemTrayBar.qml.
    // See LICENSE.dms for attribution. No real shell instance is used.
    property int attempts: 0
    ListModel { id: entryStack }
    function topEntry() { return entryStack.count ? entryStack.get(entryStack.count - 1).handle : null; }
    function showSubMenu(entry) {
        console.log("NAV", entry.text, "hasChildren", entry.hasChildren, "menuType", typeof entry.menu);
        if (!entry || !entry.hasChildren) return;
        entryStack.append({handle: entry});
        const h = entry.menu || entry;
        if (h && typeof h.updateLayout === "function") h.updateLayout();
        submenuHydrator.menu = h;
        submenuHydrator.open();
        Qt.callLater(() => submenuHydrator.close());
    }
    FloatingWindow {
        id: probeWindow; visible: true; implicitWidth: 340; implicitHeight: 260
        Column {
            anchors.fill: parent
            Repeater {
                id: entries
                model: entryStack.count ? (subOpener.children ? subOpener.children : (root.topEntry()?.children || [])) : rootOpener.children
                Rectangle {
                    property var menuEntry: modelData
                    property alias hit: hit
                    width: 340; height: 30
                    Text { text: menuEntry?.text ?? "missing" }
                    MouseArea {
                        id: hit; anchors.fill: parent
                        enabled: !menuEntry?.isSeparator && (menuEntry?.enabled !== false)
                        onClicked: {
                            console.log("CLICK",menuEntry?.text);
                            if (!menuEntry || menuEntry.isSeparator) return;
                            if (menuEntry.hasChildren) { root.showSubMenu(menuEntry); return; }
                            if (typeof menuEntry.activate === "function") menuEntry.activate();
                            else if (typeof menuEntry.triggered === "function") menuEntry.triggered();
                        }
                    }
                }
            }
        }
    }
    QsMenuAnchor { id: submenuHydrator; anchor.window: probeWindow }
    QsMenuOpener { id: rootOpener; menu: root.trayItem ? root.trayItem.menu : null }
    QsMenuOpener { id: subOpener; menu: { const e = root.topEntry(); return e ? (e.menu || e) : null; } }
    Timer {
        interval: 200; running: true; repeat: true
        onTriggered: {
            root.attempts++;
            const children = root.stage === 0 ? rootOpener.children.values : subOpener.children.values;
            console.log("STEP",root.stage,"stack",entryStack.count,"children",children.map(x=>x.text).join(" | "));
            if (root.stage === 3 && root.attempts > 25) { Qt.quit(); return; }
            if (root.attempts > 30) { console.log("FAIL timeout"); Qt.quit(); return; }
            if (root.stage === 0 && children.length) { entries.itemAt(0).hit.clicked(null); root.stage=1; }
            else if (root.stage === 1 && children.some(x=>x.text === "Selected certificate")) { entries.itemAt(children.findIndex(x=>x.text === "Selected certificate")).hit.clicked(null); root.stage=2; }
            else if (root.stage === 2 && children.some(x=>x.text === "Used for signing")) { console.log("PASS nested navigation"); root.stage=3; }
        }
    }
}
