import QtQuick
Item {
    id: lockScreenUi
    readonly property bool softwareRendering: GraphicsInfo.api === GraphicsInfo.Software
    Connections {
        target: authenticator
        function onFailed(kind) {
            graceLockTimer.restart();
        }
    }
    MouseArea {
        property bool uiVisible: false
        property bool seenPositionChange: false
        onPressed: uiVisible = true;
        onPositionChanged: {
            uiVisible = seenPositionChange;
            seenPositionChange = true;
        }
        onUiVisibleChanged: {
            authenticator.startAuthenticating();
        }
        Keys.onPressed: event => {
            uiVisible = true;
            event.accepted = false;
        }
        Timer {
            id: graceLockTimer
            interval: 3000
            onTriggered: {
                root.clearPassword();
                authenticator.startAuthenticating();
            }
        }
    }
}
