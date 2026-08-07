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

/// The newest released version recorded in the changelog.
///
/// Headings are `## [0.2.0-alpha.1] - 2026-08-07`. `## [Unreleased]` sits above
/// them and is skipped: it is where entries accumulate before a release, so it
/// never names a version.
fn changelog_latest_release() -> String {
    let raw = fs::read_to_string(repo_root().join("CHANGELOG.md"))
        .expect("CHANGELOG.md must exist at the repo root");

    for line in raw.lines() {
        let Some(rest) = line.strip_prefix("## [") else {
            continue;
        };
        let Some((version, _)) = rest.split_once(']') else {
            continue;
        };
        if version == "Unreleased" {
            continue;
        }
        return version.to_string();
    }

    panic!("CHANGELOG.md has no released version heading");
}

/// A version bump and its changelog entry must land together.
///
/// This replaced an assertion that the version equalled a hardcoded literal. That
/// literal was a fourth place the version lived, which is the duplication
/// `docs/VERSIONING.md` exists to prevent, and it had to be edited on every
/// release — so it only ever failed for the intended bump it was supposed to be
/// guarding.
///
/// Comparing against the changelog instead adds no new copy: it cross-checks two
/// records that already have to agree, and it enforces the project's own rule that
/// every user-visible change is written down in the release that carries it. A
/// bump with no entry fails here, and so does an entry with no bump.
#[test]
fn the_released_version_has_a_changelog_entry() {
    assert_eq!(
        package_json_version(),
        changelog_latest_release(),
        "package.json and the newest CHANGELOG.md heading disagree. Bumping the \
         version means adding its changelog section in the same commit."
    );
}
