//! Guards against the version drifting between manifests.
//!
//! `package.json` is the single source of truth (see `docs/VERSIONING.md`), and
//! `tauri.conf.json` points at it. `Cargo.toml` cannot reference another file, so
//! it carries its own copy and can silently fall behind. This test is the only
//! thing that notices — without it the two diverge quietly and the mismatch
//! surfaces at release time, which is the worst moment to find it.

use std::fs;
use std::path::{Path, PathBuf};

/// Tests run with the crate root as the working directory, so the repo root is
/// its parent. Resolved from `CARGO_MANIFEST_DIR` rather than a relative path so
/// the test does not depend on how it was invoked.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a parent directory")
        .to_path_buf()
}

fn package_json_version() -> String {
    let path = repo_root().join("package.json");
    let raw = fs::read_to_string(&path).expect("package.json must exist at the repo root");
    let json: serde_json::Value =
        serde_json::from_str(&raw).expect("package.json must be valid JSON");
    json["version"]
        .as_str()
        .expect("package.json must have a string `version`")
        .to_string()
}

fn cargo_toml_version() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let raw = fs::read_to_string(&path).expect("Cargo.toml must exist");
    let manifest: toml::Value = toml::from_str(&raw).expect("Cargo.toml must be valid TOML");
    manifest["package"]["version"]
        .as_str()
        .expect("Cargo.toml must have a `package.version`")
        .to_string()
}

/// `tauri.conf.json` should delegate to `package.json` rather than repeat the
/// version. Tauri accepts either a literal semver string or a path to a
/// `package.json`; a literal here would be a third copy to keep in sync.
fn tauri_conf_version_field() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw = fs::read_to_string(&path).expect("tauri.conf.json must exist");
    let json: serde_json::Value =
        serde_json::from_str(&raw).expect("tauri.conf.json must be valid JSON");
    json["version"]
        .as_str()
        .expect("tauri.conf.json must have a string `version`")
        .to_string()
}

#[test]
fn cargo_and_package_json_versions_agree() {
    assert_eq!(
        cargo_toml_version(),
        package_json_version(),
        "Cargo.toml and package.json versions have drifted. \
         package.json is the source of truth — see docs/VERSIONING.md."
    );
}

#[test]
fn tauri_conf_delegates_to_package_json() {
    assert_eq!(
        tauri_conf_version_field(),
        "../package.json",
        "tauri.conf.json should point at package.json, not carry its own version literal."
    );
}

#[test]
fn version_is_the_expected_release() {
    assert_eq!(package_json_version(), "0.1.0-alpha.1");
}
