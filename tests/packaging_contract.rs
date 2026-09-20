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
    assert!(install.contains("post_install()"));
    assert!(install.contains("post_upgrade()"));
    assert!(install.contains("/usr/lib/syauth/syauth-pam-sync install"));
    assert!(!install.contains("post_remove()"));
}
