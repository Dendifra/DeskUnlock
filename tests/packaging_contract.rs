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

#[test]
fn arch_package_reapplies_pam_on_install_and_upgrade() {
    let build = repo_file("packaging/arch/PKGBUILD");
    let install = repo_file("packaging/arch/deskunlock.install");

    assert!(build.contains("install=deskunlock.install"));
    assert!(build.contains("desktop/applications/syauth.desktop"));
    assert!(build.contains("/usr/share/applications/syauth.desktop"));
    assert!(build.contains("assets/deskunlock-logo.png"));
    assert!(build.contains("/usr/share/icons/hicolor/256x256/apps/deskunlock.png"));
    assert!(install.contains("post_install()"));
    assert!(install.contains("post_upgrade()"));
    assert!(install.contains("/usr/lib/syauth/syauth-pam-sync install"));
    assert!(!install.contains("post_remove()"));
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
    assert!(patch.contains("if (start())"));
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
    assert!(proximity.contains("saved_ready_token"));
    assert!(proximity.contains("readiness_is_new"));
    assert!(dms.contains("syauth.abort()"));
    assert!(dms.contains("root.syauthGeneration"));
    assert!(!dms.contains("syauthStartTimer.restart()"));
}

#[test]
fn settings_gui_uses_deskunlock_branding_and_no_duplicate_phone_status() {
    let settings = repo_file("desktop/bin/syauth-settings");

    assert!(settings.contains("self.setWindowTitle(\"DeskUnlock\")"));
    assert!(settings.contains("title = QLabel(\"DeskUnlock\")"));
    assert!(settings.contains("DESKUNLOCK_LOGO"));
    assert!(settings.contains("Servizio DeskUnlock"));
    assert!(!settings.contains("self.presence_row"));
}
