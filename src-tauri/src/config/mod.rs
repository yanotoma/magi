//! User configuration: a human-editable TOML file plus its migration path.
//!
//! Lives in the OS config directory as `config.toml`. It is meant to be opened
//! in an editor and pasted into bug reports, which is why API keys are not in it
//! — see [`secrets`](crate::config::secrets).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped whenever a change to this file requires the user to act.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const FILE_NAME: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config.toml could not be parsed: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("config.toml could not be written: {0}")]
    Serialise(#[from] toml::ser::Error),

    #[error("could not read or write config.toml: {0}")]
    Io(#[from] std::io::Error),

    #[error(
        "config.toml declares schema version {0}, which this build of Magi does not \
         understand. It was probably written by a newer version — upgrade Magi, or \
         move the file aside to start from defaults."
    )]
    UnsupportedSchema(u32),

    #[error(
        "two providers share the id '{0}'. Ids must be unique: they key the keychain \
         entry holding each provider's API key, so duplicates would share one secret."
    )]
    DuplicateProviderId(String),
}

/// How a provider speaks, which decides which implementation handles it.
///
/// This is not cosmetic. "OpenAI-compatible" is a family of endpoints, and
/// Anthropic's API is outside it — different auth header, system prompt
/// placement, tool schema, image encoding, and required fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenaiCompatible,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Stable identifier. Also the keychain account name, so changing it orphans
    /// the stored key.
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,

    /// Whether this provider needs a key at all. Ollama and LM Studio do not.
    #[serde(default)]
    pub requires_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyConfig {
    pub toggle: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle: crate::hotkey::DEFAULT_SHORTCUT.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    #[serde(default)]
    pub hotkey: HotkeyConfig,

    /// `[[provider]]` in the file; plural in code.
    #[serde(default, rename = "provider")]
    pub providers: Vec<ProviderConfig>,
}

fn default_schema_version() -> u32 {
    // A hand-written file with no version is treated as the first schema rather
    // than rejected. Being liberal costs nothing here.
    1
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            hotkey: HotkeyConfig::default(),
            providers: Vec::new(),
        }
    }
}

impl Config {
    /// Parses and validates, without touching the filesystem.
    ///
    /// Two passes, and the order matters. `deny_unknown_fields` and the schema
    /// check compete: a file written by a newer Magi would fail strict
    /// deserialisation with "unknown field", blaming a field when the real cause
    /// is that the file comes from the future. So the version is read leniently
    /// first and decided on, and only then is the whole file parsed strictly.
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        #[derive(Deserialize)]
        struct VersionProbe {
            #[serde(default = "default_schema_version")]
            schema_version: u32,
        }

        let probe: VersionProbe = toml::from_str(source)?;
        if probe.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchema(probe.schema_version));
        }

        let config: Config = toml::from_str(source)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        for provider in &self.providers {
            if !seen.insert(provider.id.as_str()) {
                return Err(ConfigError::DuplicateProviderId(provider.id.clone()));
            }
        }
        Ok(())
    }

    /// Reads `config.toml` from `dir`. A missing file yields defaults.
    ///
    /// Takes a directory rather than resolving the OS config path itself, so
    /// tests can point at a temporary directory and never touch the real one.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        let path = Self::path_in(dir);
        match fs::read_to_string(&path) {
            Ok(source) => Self::from_toml(&source),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), ConfigError> {
        fs::create_dir_all(dir)?;
        fs::write(Self::path_in(dir), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temp dir must be creatable")
    }

    #[test]
    fn defaults_are_usable_without_a_file() {
        let config = Config::default();
        assert_eq!(config.hotkey.toggle, "Alt+Space");
        assert!(config.providers.is_empty());
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Config::default();
        let rendered = toml::to_string_pretty(&original).expect("default config must serialise");
        let parsed: Config = toml::from_str(&rendered).expect("its own output must parse");
        assert_eq!(original, parsed);
    }

    #[test]
    fn a_provider_without_a_base_url_is_rejected() {
        let result = Config::from_toml(
            r#"
            [[provider]]
            id = "local"
            kind = "openai-compatible"
            model = "qwen2.5"
            "#,
        );
        assert!(result.is_err(), "base_url is required");
    }

    #[test]
    fn an_api_key_in_the_file_is_a_hard_error() {
        // Keys belong in the OS keychain. Ignoring one silently would leave the
        // user believing a secret is configured when nothing reads it, and would
        // leak it into any bug report that pastes the config — the exact failure
        // keeping keys out of this file is meant to prevent.
        let result = Config::from_toml(
            r#"
            [[provider]]
            id = "openai"
            kind = "openai-compatible"
            base_url = "https://api.openai.com/v1"
            model = "gpt-4o"
            api_key = "sk-oops"
            "#,
        );
        assert!(result.is_err(), "api_key must be rejected, not ignored");
    }

    #[test]
    fn a_misspelled_field_is_rejected_rather_than_defaulted() {
        // Without deny_unknown_fields this parses, `base_url` falls back to a
        // default, and the user gets a connection error pointing somewhere they
        // never configured.
        let result = Config::from_toml(
            r#"
            [[provider]]
            id = "local"
            kind = "openai-compatible"
            base_ur = "http://localhost:11434/v1"
            model = "qwen2.5"
            "#,
        );
        assert!(result.is_err(), "base_ur is a typo, not a new field");
    }

    #[test]
    fn a_newer_schema_is_reported_not_guessed() {
        let result = Config::from_toml("schema_version = 999\n");
        assert!(
            matches!(result, Err(ConfigError::UnsupportedSchema(999))),
            "a config written by a newer Magi must say so, not be half-read"
        );
    }

    #[test]
    fn a_file_with_no_schema_version_is_treated_as_version_one() {
        // Being liberal here costs nothing and covers hand-written configs.
        let config = Config::from_toml(
            r#"
            [hotkey]
            toggle = "Alt+M"
            "#,
        )
        .expect("a config without schema_version should still load");
        assert_eq!(config.hotkey.toggle, "Alt+M");
    }

    #[test]
    fn duplicate_provider_ids_are_rejected() {
        // Provider ids key the keychain entries. Two providers sharing one id
        // would silently share one secret.
        let result = Config::from_toml(
            r#"
            [[provider]]
            id = "local"
            kind = "openai-compatible"
            base_url = "http://localhost:11434/v1"
            model = "a"

            [[provider]]
            id = "local"
            kind = "openai-compatible"
            base_url = "http://localhost:1234/v1"
            model = "b"
            "#,
        );
        assert!(matches!(result, Err(ConfigError::DuplicateProviderId(_))));
    }

    #[test]
    fn loading_from_a_directory_without_a_config_returns_defaults() {
        let dir = tempdir();
        let config = Config::load(dir.path()).expect("a missing file is not an error");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn saving_then_loading_preserves_the_config() {
        let dir = tempdir();
        let mut config = Config::default();
        config.hotkey.toggle = "CmdOrCtrl+Shift+M".to_string();

        config.save(dir.path()).expect("save must succeed");
        let reloaded = Config::load(dir.path()).expect("load must succeed");

        assert_eq!(config, reloaded);
    }
}
