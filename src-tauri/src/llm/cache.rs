//! Remembering what each model turned out to be able to do.
//!
//! Probing costs four requests and, on a metered endpoint, real money. Doing it on
//! every launch would make the app slow to start and expensive to own, so results
//! are written to `capabilities.json` beside `config.toml`.
//!
//! **Not** in `config.toml`, and that is a deliberate boundary. That file is a
//! documented contract surface (see `docs/VERSIONING.md`) and the one the user owns
//! and hand-edits. Probe results are the opposite of all three: derived,
//! disposable, and meaningless to write by hand — the design spec is explicit that
//! a tier is never set manually. Keeping them apart means deleting this file is a
//! supported way to force a re-probe, and that a change to the probes cannot
//! require a config schema migration.
//!
//! Takes a directory rather than resolving the OS config path, mirroring
//! `Config::load`, so tests never touch the real one.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::llm::capability::{assign, Capabilities, Tier};

const FILE_NAME: &str = "capabilities.json";

/// Bumped when a change to the probes makes older results untrustworthy.
///
/// Needed because the safe-looking alternative is not safe. `Capabilities` fields
/// default to `false`, so an old file missing a newly added capability would parse
/// cleanly and report that capability as absent — a stale entry that looks like a
/// real result, and a model quietly assigned a lower tier than it deserves. A
/// version means the file is discarded and re-probed instead of half-believed.
const CURRENT_VERSION: u32 = 1;

/// Probe results by provider id, then model.
///
/// Nested rather than keyed by a joined `"provider:model"` string. Both halves are
/// arbitrary text — provider ids are typed by the user, model names come from the
/// endpoint — so any separator could appear inside one of them and collide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCache {
    #[serde(default)]
    version: u32,

    #[serde(default)]
    providers: HashMap<String, HashMap<String, Capabilities>>,
}

impl CapabilityCache {
    pub fn path_in(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    /// Reads the cache, or returns an empty one.
    ///
    /// Infallible by design. A missing file, unreadable file, malformed JSON, or a
    /// version from a different build all mean the same thing — nothing has been
    /// probed yet — and that is an ordinary state rather than a failure. Returning
    /// a `Result` here would make every caller handle an error whose only sensible
    /// response is to carry on with an empty cache.
    pub fn load(dir: &Path) -> Self {
        let path = Self::path_in(dir);

        let Ok(raw) = fs::read_to_string(&path) else {
            return Self::empty();
        };

        match serde_json::from_str::<Self>(&raw) {
            Ok(cache) if cache.version == CURRENT_VERSION => cache,
            Ok(cache) => {
                tracing::info!(
                    found = cache.version,
                    expected = CURRENT_VERSION,
                    "capability cache is from a different version; re-probing"
                );
                Self::empty()
            }
            Err(error) => {
                // Logged rather than surfaced. The user did not write this file and
                // cannot fix it; the recovery is to probe again.
                // File name only — see the note in `lib.rs`. The path runs through the
                // user's home directory and the log is a file they may share.
                tracing::warn!(
                    %error,
                    file = %path.file_name().unwrap_or_default().to_string_lossy(),
                    "capability cache could not be read"
                );
                Self::empty()
            }
        }
    }

    fn empty() -> Self {
        Self {
            version: CURRENT_VERSION,
            providers: HashMap::new(),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        // Pretty-printed. Nobody is meant to edit this, but somebody will read it
        // while working out why a model was put in the tier it was.
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(Self::path_in(dir), json)
    }

    pub fn get(&self, provider: &str, model: &str) -> Option<Capabilities> {
        self.providers.get(provider)?.get(model).copied()
    }

    /// The tier for a model, or `None` if it has not been probed.
    ///
    /// Derived on read rather than stored. Writing the tier into the file would
    /// duplicate a value that `assign` already determines, and the copy would go
    /// stale the moment the assignment rules change — leaving a file full of tiers
    /// that no longer follow from the capabilities recorded next to them.
    pub fn tier(&self, provider: &str, model: &str) -> Option<Tier> {
        self.get(provider, model).map(|caps| assign(&caps))
    }

    pub fn set(&mut self, provider: &str, model: &str, capabilities: Capabilities) {
        self.version = CURRENT_VERSION;
        self.providers
            .entry(provider.to_string())
            .or_default()
            .insert(model.to_string(), capabilities);
    }

    /// Drops everything known about a provider.
    ///
    /// Called when a provider is removed or its endpoint or key changes. Results
    /// are a property of the endpoint as much as the model: the same model name
    /// behind a different URL is a different deployment, and a re-pointed provider
    /// keeping its old capabilities would report a tier that was measured somewhere
    /// else.
    pub fn forget_provider(&mut self, provider: &str) {
        self.providers.remove(provider);
    }

    /// Every model known for a provider, with its tier.
    pub fn tiers_for(&self, provider: &str) -> HashMap<String, Tier> {
        self.providers
            .get(provider)
            .map(|models| {
                models
                    .iter()
                    .map(|(model, caps)| (model.clone(), assign(caps)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temp dir must be creatable")
    }

    fn capable() -> Capabilities {
        Capabilities {
            reachable: true,
            vision: true,
            tools: true,
            structured_output: true,
        }
    }

    fn sighted_only() -> Capabilities {
        Capabilities {
            reachable: true,
            vision: true,
            tools: false,
            structured_output: false,
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_cache_not_an_error() {
        let dir = tempdir();
        let cache = CapabilityCache::load(dir.path());
        assert_eq!(cache.get("ollama", "llava"), None);
        assert_eq!(cache.tier("ollama", "llava"), None);
    }

    #[test]
    fn results_survive_a_round_trip() {
        let dir = tempdir();
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", sighted_only());
        cache.set("openai", "gpt-5", capable());
        cache.save(dir.path()).expect("save must succeed");

        let reloaded = CapabilityCache::load(dir.path());
        assert_eq!(reloaded.get("ollama", "llava"), Some(sighted_only()));
        assert_eq!(reloaded.tier("ollama", "llava"), Some(Tier::Heuristic));
        assert_eq!(reloaded.tier("openai", "gpt-5"), Some(Tier::Agentic));
    }

    #[test]
    fn one_provider_can_hold_several_models_at_different_tiers() {
        // The normal case for OpenRouter and Ollama: one endpoint, models with
        // sharply different capabilities. A per-provider tier would be wrong.
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", sighted_only());
        cache.set(
            "ollama",
            "llama3.2",
            Capabilities {
                reachable: true,
                ..Default::default()
            },
        );

        assert_eq!(cache.tier("ollama", "llava"), Some(Tier::Heuristic));
        assert_eq!(cache.tier("ollama", "llama3.2"), Some(Tier::TextOnly));
    }

    #[test]
    fn malformed_json_is_treated_as_nothing_probed_yet() {
        // The user did not write this file and cannot repair it. Failing to start,
        // or surfacing an error they can do nothing about, would both be worse than
        // probing again.
        let dir = tempdir();
        fs::write(CapabilityCache::path_in(dir.path()), "{ not json").expect("writable");

        let cache = CapabilityCache::load(dir.path());
        assert_eq!(cache.get("ollama", "llava"), None);
    }

    #[test]
    fn a_cache_from_another_version_is_discarded_rather_than_half_believed() {
        // The reason the version field exists. Capabilities default to false, so an
        // older file would parse cleanly and report any newly added capability as
        // absent — a stale entry indistinguishable from a real result.
        let dir = tempdir();
        fs::write(
            CapabilityCache::path_in(dir.path()),
            r#"{"version":0,"providers":{"ollama":{"llava":{"reachable":true,"vision":true}}}}"#,
        )
        .expect("writable");

        let cache = CapabilityCache::load(dir.path());
        assert_eq!(
            cache.get("ollama", "llava"),
            None,
            "results from another probe version must not be trusted"
        );
    }

    #[test]
    fn missing_fields_default_to_absent_within_the_current_version() {
        // Under-reporting is the safe direction: a capability read as false costs a
        // feature until the next probe, while one read as true produces wrong
        // answers.
        let dir = tempdir();
        fs::write(
            CapabilityCache::path_in(dir.path()),
            r#"{"version":1,"providers":{"ollama":{"llava":{"reachable":true,"vision":true}}}}"#,
        )
        .expect("writable");

        let cache = CapabilityCache::load(dir.path());
        let caps = cache.get("ollama", "llava").expect("present");
        assert!(caps.vision);
        assert!(!caps.tools, "an absent field must not read as capable");
        assert_eq!(cache.tier("ollama", "llava"), Some(Tier::Heuristic));
    }

    #[test]
    fn forgetting_a_provider_drops_all_its_models() {
        // Results belong to the endpoint as much as the model. A provider re-pointed
        // at a different URL that kept its old capabilities would report a tier
        // measured somewhere else entirely.
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", sighted_only());
        cache.set("ollama", "llama3.2", capable());
        cache.set("openai", "gpt-5", capable());

        cache.forget_provider("ollama");

        assert_eq!(cache.get("ollama", "llava"), None);
        assert_eq!(cache.get("ollama", "llama3.2"), None);
        assert_eq!(
            cache.get("openai", "gpt-5"),
            Some(capable()),
            "other providers are untouched"
        );
    }

    #[test]
    fn re_probing_replaces_rather_than_accumulates() {
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", capable());
        cache.set("ollama", "llava", sighted_only());

        assert_eq!(cache.get("ollama", "llava"), Some(sighted_only()));
        assert_eq!(cache.tiers_for("ollama").len(), 1);
    }

    #[test]
    fn the_tier_is_derived_on_read_not_stored() {
        // Guards against someone adding a `tier` field for convenience. A stored
        // copy would go stale the moment the assignment rules change, leaving a file
        // of tiers that no longer follow from the capabilities beside them.
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", sighted_only());
        let json = serde_json::to_string(&cache).expect("serialisable");

        assert!(
            !json.contains("tier"),
            "the tier must not be persisted: {json}"
        );
    }

    #[test]
    fn tiers_for_lists_every_probed_model() {
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", sighted_only());
        cache.set(
            "ollama",
            "llama3.2",
            Capabilities {
                reachable: true,
                ..Default::default()
            },
        );

        let tiers = cache.tiers_for("ollama");
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers.get("llava"), Some(&Tier::Heuristic));
        assert_eq!(tiers.get("llama3.2"), Some(&Tier::TextOnly));
        assert!(cache.tiers_for("nobody").is_empty());
    }

    #[test]
    fn saving_creates_the_directory_if_it_is_missing() {
        // First run: the config directory may not exist yet, and a failure to write
        // the cache would mean re-probing on every launch forever.
        let dir = tempdir();
        let nested = dir.path().join("magi").join("nested");
        let mut cache = CapabilityCache::empty();
        cache.set("ollama", "llava", sighted_only());

        cache.save(&nested).expect("save must create the path");
        assert_eq!(
            CapabilityCache::load(&nested).get("ollama", "llava"),
            Some(sighted_only())
        );
    }
}
