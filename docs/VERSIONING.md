# Versioning

Magi follows [Semantic Versioning 2.0.0](https://semver.org).

**Current version:** `0.2.0-alpha.2` (tagged; no downloadable build until M7 adds signing)

---

## What the version actually promises

Semver is defined in terms of a public API. A desktop application does not export functions, so without stating what its API *is*, "breaking change" becomes a judgement call and the version number stops meaning anything.

For Magi, the public contract is exactly these four surfaces:

| Surface | Why it counts |
|---|---|
| **`config.toml` schema** | Users hand-edit it and paste it into bug reports |
| **Tauri IPC commands** (`invoke`) | The extension point for future plugins |
| **Tauri events** (`magi://*`) | Same |
| **Documented user-facing behavior** | Hotkey semantics, capture policy, tier rules |

A change is **breaking** if it requires the user to do something: migrate a config, re-grant a permission, re-configure a provider, or relearn an interaction. Anything the app can handle silently on their behalf is not breaking.

Notably **not** part of the contract: internal Rust module structure, crate choices, the Svelte component tree, log formats.

## Increment rules

| | When |
|---|---|
| **MAJOR** | A breaking change to any of the four surfaces above |
| **MINOR** | New capability, backward compatible. New milestone shipped, new provider supported, new tier behavior |
| **PATCH** | Bug fixes and performance work with no contract change |

While the major version is `0`, minor bumps may break things — that is what `0.x` means, and it is why Magi stays there until the config schema is stable enough to defend.

## Pre-release identifiers

```
0.1.0-alpha.1  →  0.1.0-alpha.2  →  0.1.0-beta.1  →  0.1.0
```

| Stage | Meaning |
|---|---|
| `alpha` | Incomplete. Expect breakage, data loss, and config churn. Contributors only. |
| `beta` | Feature-complete for that version. Config schema frozen for the cycle. Public testing. |
| *(none)* | Stable for its major version. |

Ordering follows semver: `0.1.0-alpha.1 < 0.1.0-beta.1 < 0.1.0`. Pre-releases are published as GitHub **pre-releases** so they never surface as "latest" to someone who just wants a working build.

## Release train

Each milestone from [`TASKS.md`](TASKS.md) maps to a version. This is the plan, not a promise about dates.

| Version | Milestone | What works |
|---|---|---|
| `0.1.0-alpha.1` | M1 — Shell | Tray, global hotkey, panel and settings windows. No intelligence. |
| `0.2.0-alpha.1` | M2 — Config & providers | Configure a model, type a question, watch the answer stream back. |
| `0.2.0-alpha.2` | M3 — Pre-flight & capability tiers | See what the chosen model can actually do before relying on it. |
| `0.3.0-alpha.1` | M4 — Audio & STT | Speak, get a local transcript. First genuinely useful build. |
| `0.4.0-alpha.1` | M5 — Capture & agentic vision | The model can decide to look at your screen. |
| `0.5.0-beta.1` | M6 — Session & panel | v1 feature-complete. Config schema frozen. |
| `0.6.0-beta.1` | M7 — Packaging | Signed, notarized DMG. **First public download.** |
| `1.0.0` | — | After beta feedback settles. Config schema committed to. |
| `1.1.0` | M8 — Ambient | Wake word and TTS. |
| `1.2.0` | M9 — Agency | Computer use. |

`0.2.0` was originally planned as one release covering M2 and M3 together. It was split once M2 was working: a build that answers questions is worth putting a tag on rather than holding back until pre-flight lands, and accumulating alpha increments toward one minor is exactly what the pre-release identifiers are for.

`1.0.0` is not a quality claim. It is a promise that `config.toml` will not break under you without a major bump.

## Where the version lives

`package.json` is the single source of truth. `tauri.conf.json` points at it:

```json
{ "version": "../package.json" }
```

Tauri reads the version from `tauri.conf.json`, falling back to `src-tauri/Cargo.toml` only when unset. Pointing it at `package.json` means one file to edit and no possibility of drift between manifests.

`Cargo.toml` keeps a version for crate metadata, but it is not what ships. CI asserts the two agree so the discrepancy can never go unnoticed.

### Platform bundle formats

Platform bundle version fields are numeric-only and do not carry pre-release identifiers — Tauri's Android version code, for instance, is derived arithmetically as `major*1000000 + minor*1000 + patch`, which structurally cannot represent `-alpha.1`.

This means **`0.1.0-alpha.1` and `0.1.0-beta.3` may be indistinguishable to the OS installer** even though they are distinct releases to us. Verifying exactly how each target handles this, and deciding on a monotonic build-number scheme if needed, is an explicit M7 task. It is called out here rather than discovered during the first release attempt.

## Tagging

```bash
git tag -a v0.1.0-alpha.1 -m "Shell: tray, hotkey, windows"
git push origin v0.1.0-alpha.1
```

Tags are prefixed `v`. The release workflow builds, signs, notarizes, and attaches artifacts, marking anything with a pre-release identifier as a GitHub pre-release.

## Changelog

[`CHANGELOG.md`](../CHANGELOG.md) follows [Keep a Changelog](https://keepachangelog.com). Every user-visible change lands in `Unreleased` in the same PR that makes it — reconstructing a changelog from git history after the fact produces a list of commits, not a list of changes users care about.
