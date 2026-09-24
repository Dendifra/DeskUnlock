//! Packaging contracts for the Arch migration path.
//!
//! Journey: specs/journeys/JOURNEY-S-013-pam-install-helper.md

use std::{fs, path::PathBuf};

fn repo_file(path: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).expect("repository file exists")
}

#[test]
fn health_treats_pam_module_as_readable_regular_file() {
    let health = repo_file("desktop/bin/syauth-health");
    assert!(health.contains("check_file \"PAM module\""));
    assert!(!health.contains("check_exec \"PAM module\""));
}

/// DeskUnlock must never modify a PAM stack by itself.
///
/// Twice on 2026-09-23 the operator could not get back into their session with
/// the correct password because `pam_syauth.so` sat in the lock stack
/// (`/etc/pam.d/dankshell`) ahead of `pam_unix`: the module holds the
/// authentication phase while it waits for the phone, so the lock screen's own
/// prompt starves and nothing the operator types gets through. The unlock moved
/// out of band (`syauth unlock-request`), so the package now ships **no**
/// automatic PAM integration at all — and this test is what keeps it that way.
#[test]
fn arch_package_never_touches_a_pam_stack() {
    let build = repo_file("packaging/arch/PKGBUILD");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Still a normal package: launcher + icon ship as before.
    assert!(build.contains("desktop/applications/syauth.desktop"));
    assert!(build.contains("/usr/share/applications/syauth.desktop"));
    assert!(build.contains("assets/deskunlock-logo.png"));
    assert!(build.contains("/usr/share/icons/hicolor/256x256/apps/deskunlock.png"));
    assert!(!build.contains("'ghostty'"));

    assert!(
        !build.contains("install=deskunlock.install"),
        "an install hook can re-add our module to a PAM stack"
    );
    assert!(
        !build.contains("libalpm/hooks"),
        "a pacman hook re-applied the PAM integration after every plasma/pam update"
    );
    assert!(!build.contains("/etc/pam.d/"), "the package must not ship PAM service files");
    assert!(
        !root.join("packaging/arch/deskunlock.install").exists(),
        "the install script only ever ran the PAM sync"
    );
    assert!(
        !root.join("desktop/libexec/syauth-pam-sync").exists(),
        "the PAM sync wrote our module into a login stack"
    );
    assert!(
        !root.join("desktop/hooks/syauth-pam.hook").exists(),
        "the pacman hook re-applied the PAM integration after every plasma/pam update"
    );
}

/// The lock-screen adaptation may show DMS's fingerprint indicator, but it must
/// never point a PAM service at `pam_syauth.so` — and it must actively undo that
/// if an earlier adaptation left it behind. DMS's own enable/suppression gates
/// must stay intact, otherwise DMS starts the biometric context the moment the
/// lock appears and the phone is asked before the operator does anything.
#[test]
fn dms_lock_patch_only_shows_the_indicator_and_restores_pam() {
    let patch = repo_file("desktop/libexec/syauth-dms-lock-patch");

    assert!(
        patch.contains("lockPamExternallyManaged"),
        "the indicator must still light up for an externally managed lock PAM"
    );
    assert!(
        patch.contains("fprintSuppressedByPrimaryPam"),
        "DMS's own gate must stay intact so the phone is never asked on lock"
    );
    assert!(
        patch.contains("auth    required    pam_fprintd.so  max-tries=5"),
        "the stock fprintd line must be restored, not replaced"
    );
    assert!(
        patch.contains("pam/fprint ripristinato"),
        "restoring a patched fprint service must be part of the adaptation"
    );
}

#[test]
fn desktop_launcher_uses_deskunlock_branding() {
    let launcher = repo_file("desktop/applications/syauth.desktop");

    assert!(launcher.contains("Name=DeskUnlock"));
    assert!(launcher.contains("Comment=Autenticazione sicura con il telefono"));
    assert!(launcher.contains("Exec=/usr/bin/syauth-user-setup --gui"));
    assert!(launcher.contains("Icon=deskunlock"));
}

#[test]
fn dms_lock_indicator_uses_persistent_syauth_state() {
    let patch = repo_file("desktop/dms/build-dms-syauth.sh");

    assert!(patch.contains("property bool syauthAvailable: false"));
    assert!(patch.contains("root.syauthAvailable = start();"));
    assert!(patch.contains("root.syauthAvailable = false"));
    assert!(patch.contains("pam.syauthAvailable"));
    assert!(patch.contains("if (pam.syauthAvailable)"));
    assert!(patch.contains("return \"fingerprint\";"));
    assert!(!patch.contains("/usr/share/icons/hicolor/256x256/apps/deskunlock.png"));
    assert!(!patch.contains("visible: pam.syauthAvailable"));
    assert!(patch.contains("LockScreenContent.qml"));
    assert!(!patch.contains("syauthAvailable: SettingsData.lockFingerprintReady"));
}

#[test]
fn return_auth_is_transport_gated_and_not_retried_automatically() {
    let proximity = repo_file("desktop/bin/syauth-proximity");
    let dms = repo_file("desktop/dms/build-dms-syauth.sh");

    assert!(proximity.contains("READY_MARKER"));
    assert!(proximity.contains("challenge_ready_valid"));
    assert!(proximity.contains("AUTO_AUTH_SENT=1"));
    assert!(proximity.contains("request_auto_auth"));
    assert!(proximity.contains("LOCK_REASON=PROXIMITY"));
    assert!(dms.contains("syauth.abort()"));
    assert!(dms.contains("root.syauthGeneration"));
    assert!(!dms.contains("syauthStartTimer.restart()"));
}

#[test]
fn settings_gui_uses_deskunlock_branding_and_no_duplicate_phone_status() {
    let settings = repo_file("desktop/bin/syauth-settings");

    // The branding assertions tolerate the translation wrapper. The copy is a
    // string resource now, so the literal legitimately sits inside `_(...)`;
    // what this contract cares about is that the brand is still what the window
    // and the header show, not how the call is spelled.
    fn shows(source: &str, call: &str, literal: &str) -> bool {
        let plain = format!("{call}(\"{literal}\")");
        let wrapped = format!("{call}(_(\"{literal}\"))");
        source.contains(&plain) || source.contains(&wrapped)
    }

    assert!(shows(&settings, "self.setWindowTitle", "DeskUnlock"));
    assert!(shows(&settings, "QLabel", "DeskUnlock"));
    assert!(settings.contains("DESKUNLOCK_LOGO"));
    // The daemon liveness row is gone: Proximity Lock already reports the
    // service state, and the redundant row read like a crash when toggled off.
    assert!(!settings.contains("Servizio DeskUnlock"));
    assert!(settings.contains("Proximity Lock"));
    assert!(settings.contains("get_proximity_state"));
    assert!(settings.contains("Diagnostica avanzata"));
    assert!(settings.contains("get_proximity_diagnostics"));
    assert!(!settings.contains("QProcess.startDetached"));
    assert!(!settings.contains("ghostty"));
    assert!(!settings.contains("self.presence_row"));
}
