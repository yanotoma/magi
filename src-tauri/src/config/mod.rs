//! User configuration: a human-editable TOML file plus its migration path.
//!
//! Lives in the OS config directory as `config.toml`. It is meant to be opened
//! in an editor and pasted into bug reports, which is why API keys are not in it
//! — see [`secrets`](crate::config::secrets).

pub mod secrets;

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

    #[error("the selected provider '{0}' is not configured")]
    UnknownActiveProvider(String),

    #[error("provider '{provider}' has no model named '{model}'")]
    UnknownActiveModel { provider: String, model: String },

    #[error(
        "[prompt] context is {found} characters, over the {limit} limit. It is sent with \
         every single turn, so a long one costs tokens on each question and crowds out \
         the conversation. Keep it to standing facts — who you are, what you work on."
    )]
    PromptContextTooLong { found: usize, limit: usize },

    #[error("[hotkey] toggle is not a usable shortcut: {reason}")]
    InvalidHotkey { reason: String },
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

    /// Models reachable at this endpoint.
    ///
    /// A list rather than a single name because one endpoint routinely serves
    /// many: OpenRouter exposes hundreds, Ollama whatever has been pulled. It
    /// may be empty when a provider has just been added and its models have not
    /// been discovered yet — such a provider simply cannot be the active one.
    #[serde(default)]
    pub models: Vec<String>,

    /// Whether this provider needs a key at all. Ollama and LM Studio do not.
    #[serde(default)]
    pub requires_key: bool,
}

/// Which model is in use.
///
/// A pair rather than a single id because capability tiers are a property of
/// the model, not the endpoint: the same OpenRouter key reaches models that can
/// see images and models that cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveModel {
    pub provider: String,
    pub model: String,
}

/// Which appearance the windows use.
///
/// Application state rather than a CSS class. Forcing dark from stylesheets
/// alone would leave the native controls the webview draws — inputs, scrollbars,
/// select popups — following the system, so a forced theme would be half
/// applied. Rust tells the webview, and the CSS follows from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AppearanceConfig {
    pub theme: Theme,

    /// Whether the panel shows the model's reasoning.
    ///
    /// Off by default: most models emit none, and for those that do the working
    /// is longer than the answer and would bury it.
    #[serde(default)]
    pub show_thinking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyConfig {
    /// Defaulted, like every other field here, so each one is independently optional.
    ///
    /// Without this, `[hotkey]` carrying only `push_to_talk` fails to parse with
    /// "missing field `toggle`" — which in a file whose whole point is being edited by
    /// hand means changing one shortcut obliges you to restate the other.
    #[serde(default = "default_toggle")]
    pub toggle: String,

    /// Held to record, released to transcribe.
    ///
    /// A second shortcut rather than a gesture on `toggle`. Telling a tap from a hold on
    /// one key means the panel can only toggle on *release*, behind a timer — which
    /// would put latency and a heuristic into the one interaction that already works
    /// well. Two keys, two jobs, nothing to get wrong.
    #[serde(default = "default_push_to_talk")]
    pub push_to_talk: String,
}

fn default_toggle() -> String {
    crate::hotkey::DEFAULT_SHORTCUT.to_string()
}

fn default_push_to_talk() -> String {
    crate::hotkey::DEFAULT_PUSH_TO_TALK.to_string()
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle: crate::hotkey::DEFAULT_SHORTCUT.to_string(),
            push_to_talk: default_push_to_talk(),
        }
    }
}

/// The ceiling on `[prompt] context`.
///
/// Generous for its purpose and still small next to any model's window. The limit
/// exists because this text is prepended to *every* turn: an essay here is a
/// permanent tax on every question, and it competes with the conversation for the
/// model's attention rather than adding to it.
pub const MAX_PROMPT_CONTEXT: usize = 4000;

/// Standing context the user wants every answer to account for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PromptConfig {
    /// Free text **appended** to Magi's own system prompt, never replacing it.
    ///
    /// The distinction is structural, not stylistic. Magi's prompt carries the
    /// contract that makes the rest of the app work — from M5 on it is what tells
    /// the model that a screen-capture tool exists and when to reach for it. A
    /// user who could overwrite it would silently disable agentic capture and see
    /// only a model that stopped looking at their screen, with nothing anywhere
    /// to connect that to a text box in Settings.
    ///
    /// So this field can add to the instructions and cannot remove them. The
    /// worst a hostile value can do is argue with the prompt above it, and
    /// whoever typed it is the person it would mislead.
    pub context: String,
}

/// Voice input settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VoiceConfig {
    /// Which speech model transcribes.
    ///
    /// Stored rather than derived from what is on disk, because the two are different
    /// questions: a user who downloaded `small.en` and then switched back to `base.en`
    /// still has both files, and the choice is theirs rather than whichever happens to
    /// be present.
    pub model: crate::stt::Model,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    #[serde(default)]
    pub hotkey: HotkeyConfig,

    #[serde(default)]
    pub voice: VoiceConfig,

    #[serde(default)]
    pub prompt: PromptConfig,

    #[serde(default)]
    pub appearance: AppearanceConfig,

    /// The provider and model a turn goes to. `None` until one is chosen.
    #[serde(default)]
    pub active: Option<ActiveModel>,

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
            voice: VoiceConfig::default(),
            prompt: PromptConfig::default(),
            appearance: AppearanceConfig::default(),
            active: None,
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
        // Counted in characters, not bytes: the limit is about how much text the
        // user wrote, and `len()` would give a Spanish or Japanese context a
        // smaller allowance than an English one for the same amount of writing.
        let context_length = self.prompt.context.chars().count();
        if context_length > MAX_PROMPT_CONTEXT {
            return Err(ConfigError::PromptContextTooLong {
                found: context_length,
                limit: MAX_PROMPT_CONTEXT,
            });
        }

        // Checked on load, not only when Settings writes it. This file is meant to
        // be hand-edited, and a hand-written `toggle = "Space"` would otherwise be
        // registered as typed — swallowing the spacebar in every application on
        // the machine, with Magi's own text box among the casualties.
        crate::hotkey::validate_shortcut(&self.hotkey.toggle).map_err(|e| {
            ConfigError::InvalidHotkey {
                reason: e.to_string(),
            }
        })?;

        crate::hotkey::validate_shortcut(&self.hotkey.push_to_talk).map_err(|e| {
            ConfigError::InvalidHotkey {
                reason: format!("push_to_talk: {e}"),
            }
        })?;

        // Two shortcuts that are the same string means the OS gives one of them to
        // whichever registered first and the other silently never fires. Caught here so
        // it reads as a configuration mistake rather than as a broken hotkey.
        if self
            .hotkey
            .toggle
            .eq_ignore_ascii_case(&self.hotkey.push_to_talk)
        {
            return Err(ConfigError::InvalidHotkey {
                reason: format!(
                    "toggle and push_to_talk are both '{}'. They must differ, or only one \
                     of them will ever fire.",
                    self.hotkey.toggle
                ),
            });
        }

        let mut seen = HashSet::new();
        for provider in &self.providers {
            if !seen.insert(provider.id.as_str()) {
                return Err(ConfigError::DuplicateProviderId(provider.id.clone()));
            }
        }

        // A dangling selection is worth catching here rather than at send time.
        // Removing a model from a provider and forgetting the selection is easy
        // to do by hand, and the failure it causes otherwise — a request for a
        // model the endpoint has never heard of — reads as a server problem.
        if let Some(active) = &self.active {
            let provider = self
                .providers
                .iter()
                .find(|p| p.id == active.provider)
                .ok_or_else(|| ConfigError::UnknownActiveProvider(active.provider.clone()))?;

            if !provider.models.contains(&active.model) {
                return Err(ConfigError::UnknownActiveModel {
                    provider: active.provider.clone(),
                    model: active.model.clone(),
                });
            }
        }

        Ok(())
    }

    /// The active provider and model, if one is selected and still valid.
    pub fn active_provider(&self) -> Option<(&ProviderConfig, &str)> {
        let active = self.active.as_ref()?;
        let provider = self.providers.iter().find(|p| p.id == active.provider)?;
        Some((provider, active.model.as_str()))
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

    /// Validates, then writes.
    ///
    /// Validation happens on the way out as well as on the way in. Loading is
    /// not the only path that produces a `Config` — the settings screen mutates
    /// one in memory — and writing an invalid file would mean the app refuses to
    /// start next launch, with the damage already on disk.
    pub fn save(&self, dir: &Path) -> Result<(), ConfigError> {
        self.validate()?;
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

    #[test]
    fn push_to_talk_has_its_own_default() {
        let config = Config::from_toml("").expect("an empty file is valid");
        assert_eq!(
            config.hotkey.push_to_talk,
            crate::hotkey::DEFAULT_PUSH_TO_TALK
        );
        assert_ne!(config.hotkey.push_to_talk, config.hotkey.toggle);
    }

    #[test]
    fn a_config_written_before_push_to_talk_existed_still_loads() {
        // `[hotkey]` with only `toggle` is what every existing installation has. Refusing
        // it would break the app for everyone who upgrades.
        let config = Config::from_toml(
            r#"
            [hotkey]
            toggle = "Alt+Space"
            "#,
        )
        .expect("an older config must still load");
        assert_eq!(
            config.hotkey.push_to_talk,
            crate::hotkey::DEFAULT_PUSH_TO_TALK
        );
    }

    #[test]
    fn two_identical_shortcuts_are_refused() {
        // The OS gives the combination to whichever registered first; the other silently
        // never fires. Caught here so it reads as a configuration mistake.
        let error = Config::from_toml(
            r#"
            [hotkey]
            toggle = "Alt+Space"
            push_to_talk = "alt+space"
            "#,
        )
        .expect_err("a collision must be refused");
        assert!(matches!(error, ConfigError::InvalidHotkey { .. }));
        assert!(error.to_string().contains("must differ"), "got: {error}");
    }

    #[test]
    fn each_hotkey_field_is_independently_optional() {
        // A hand-edited file should be able to set one shortcut without restating the
        // other. Before `toggle` had a default, this failed with "missing field".
        let only_ptt = Config::from_toml(
            r#"
            [hotkey]
            push_to_talk = "Control+Shift+M"
            "#,
        )
        .expect("setting only push_to_talk must work");
        assert_eq!(only_ptt.hotkey.toggle, crate::hotkey::DEFAULT_SHORTCUT);
        assert_eq!(only_ptt.hotkey.push_to_talk, "Control+Shift+M");

        let only_toggle = Config::from_toml(
            r#"
            [hotkey]
            toggle = "Control+Shift+K"
            "#,
        )
        .expect("setting only toggle must work");
        assert_eq!(only_toggle.hotkey.toggle, "Control+Shift+K");
        assert_eq!(
            only_toggle.hotkey.push_to_talk,
            crate::hotkey::DEFAULT_PUSH_TO_TALK
        );
    }

    #[test]
    fn an_invalid_push_to_talk_names_which_field_is_wrong() {
        let error = Config::from_toml(
            r#"
            [hotkey]
            push_to_talk = "Space"
            "#,
        )
        .expect_err("a bare key must be refused");
        assert!(error.to_string().contains("push_to_talk"), "got: {error}");
    }

    #[test]
    fn the_voice_model_defaults_to_the_smallest_download() {
        // First run. 1.4 GB is not a first-run experience.
        let config = Config::from_toml("").expect("an empty file is valid");
        assert_eq!(config.voice.model, crate::stt::Model::BaseEn);
    }

    #[test]
    fn the_voice_model_round_trips_through_toml() {
        let config = Config::from_toml(
            r#"
            [voice]
            model = "small-en"
            "#,
        )
        .expect("valid");
        assert_eq!(config.voice.model, crate::stt::Model::SmallEn);

        // And survives a write, which is what `set_speech_model` depends on.
        let written = toml::to_string_pretty(&config).expect("serialisable");
        assert!(written.contains("small-en"), "got:\n{written}");
    }

    #[test]
    fn an_unknown_model_name_is_refused_rather_than_defaulted() {
        // Silently falling back would transcribe with a model the user did not pick and
        // give no clue why their choice was ignored.
        assert!(Config::from_toml(
            r#"
            [voice]
            model = "large-v3"
            "#,
        )
        .is_err());
    }

    #[test]
    fn prompt_context_defaults_to_empty() {
        let config = Config::from_toml("").expect("an empty file is valid");
        assert_eq!(config.prompt.context, "");
    }

    #[test]
    fn prompt_context_round_trips() {
        let config = Config::from_toml(
            r#"
            [prompt]
            context = "I work in Kitchener and prefer metric units."
            "#,
        )
        .expect("valid");
        assert_eq!(
            config.prompt.context,
            "I work in Kitchener and prefer metric units."
        );
    }

    #[test]
    fn rejects_an_over_length_prompt_context() {
        let source = format!(
            "[prompt]\ncontext = \"{}\"",
            "a".repeat(MAX_PROMPT_CONTEXT + 1)
        );
        assert!(matches!(
            Config::from_toml(&source),
            Err(ConfigError::PromptContextTooLong { .. })
        ));
    }

    #[test]
    fn the_prompt_context_limit_counts_characters_not_bytes() {
        // Every one of these is three bytes in UTF-8, so a byte-based limit would
        // give a Japanese context a third of the allowance of an English one for
        // the same amount of writing.
        let source = format!(
            "[prompt]\ncontext = \"{}\"",
            "あ".repeat(MAX_PROMPT_CONTEXT)
        );
        assert!(
            Config::from_toml(&source).is_ok(),
            "a context at exactly the character limit must be accepted"
        );
    }

    #[test]
    fn rejects_a_hand_written_hotkey_with_no_modifier() {
        // The destructive case, and the reason validation runs on load rather than
        // only when Settings writes the field: registering a bare key globally
        // swallows it in every application on the machine.
        let error = Config::from_toml(
            r#"
            [hotkey]
            toggle = "Space"
            "#,
        )
        .expect_err("a bare key must be refused");
        assert!(matches!(error, ConfigError::InvalidHotkey { .. }));
    }

    #[test]
    fn accepts_a_hand_written_valid_hotkey() {
        let config = Config::from_toml(
            r#"
            [hotkey]
            toggle = "CmdOrCtrl+Shift+M"
            "#,
        )
        .expect("valid");
        assert_eq!(config.hotkey.toggle, "CmdOrCtrl+Shift+M");
    }

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
    fn theme_defaults_to_following_the_system() {
        assert_eq!(Config::default().appearance.theme, Theme::System);
    }

    #[test]
    fn reasoning_is_hidden_by_default() {
        // Most models emit none, and where they do the working is longer than the
        // answer and would bury it.
        assert!(!Config::default().appearance.show_thinking);
    }

    #[test]
    fn theme_is_written_and_read_in_lower_case() {
        let config = Config::from_toml("[appearance]\ntheme = \"dark\"\n")
            .expect("a lower-case theme must parse");
        assert_eq!(config.appearance.theme, Theme::Dark);
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
            models = ["qwen2.5"]
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
            models = ["gpt-4o"]
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
            models = ["qwen2.5"]
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
            models = ["a"]

            [[provider]]
            id = "local"
            kind = "openai-compatible"
            base_url = "http://localhost:1234/v1"
            models = ["b"]
            "#,
        );
        assert!(matches!(result, Err(ConfigError::DuplicateProviderId(_))));
    }

    #[test]
    fn one_provider_can_host_many_models() {
        // The common case, not an edge case: OpenRouter serves hundreds behind
        // one endpoint and one key.
        let config = Config::from_toml(
            r#"
            [[provider]]
            id = "openrouter"
            kind = "openai-compatible"
            base_url = "https://openrouter.ai/api/v1"
            models = ["qwen/qwen3-vl", "meta/llama-4"]
            requires_key = true

            [active]
            provider = "openrouter"
            model = "meta/llama-4"
            "#,
        )
        .expect("a provider with several models must load");

        let (provider, model) = config.active_provider().expect("a selection was made");
        assert_eq!(provider.id, "openrouter");
        assert_eq!(model, "meta/llama-4");
    }

    #[test]
    fn selecting_a_model_the_provider_does_not_have_is_rejected() {
        // Easy to cause by hand: remove a model from the list and forget the
        // selection. The failure otherwise is a request for a model the endpoint
        // has never heard of, which reads as a server problem.
        let result = Config::from_toml(
            r#"
            [[provider]]
            id = "local"
            kind = "openai-compatible"
            base_url = "http://localhost:11434/v1"
            models = ["qwen2.5"]

            [active]
            provider = "local"
            model = "a-model-that-was-removed"
            "#,
        );
        assert!(matches!(
            result,
            Err(ConfigError::UnknownActiveModel { .. })
        ));
    }

    #[test]
    fn selecting_a_provider_that_is_not_configured_is_rejected() {
        let result = Config::from_toml(
            r#"
            [active]
            provider = "typo"
            model = "whatever"
            "#,
        );
        assert!(matches!(result, Err(ConfigError::UnknownActiveProvider(_))));
    }

    #[test]
    fn a_provider_with_no_models_yet_is_allowed_but_cannot_be_active() {
        // Adding an endpoint before discovering what it serves is a real step in
        // the flow, so an empty list must load.
        let config = Config::from_toml(
            r#"
            [[provider]]
            id = "new"
            kind = "openai-compatible"
            base_url = "https://example.com/v1"
            "#,
        )
        .expect("a provider awaiting discovery must load");
        assert!(config.providers[0].models.is_empty());
        assert!(config.active_provider().is_none());
    }

    #[test]
    fn saving_an_invalid_config_is_refused() {
        // The settings screen mutates a Config in memory, so loading is not the
        // only way to produce one. Writing an invalid file would mean the app
        // refuses to start next launch with the damage already on disk.
        let dir = tempdir();
        let config = Config {
            active: Some(ActiveModel {
                provider: "not-configured".into(),
                model: "whatever".into(),
            }),
            ..Config::default()
        };

        assert!(matches!(
            config.save(dir.path()),
            Err(ConfigError::UnknownActiveProvider(_))
        ));
        assert!(
            !Config::path_in(dir.path()).exists(),
            "nothing may be written when validation fails"
        );
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

    /// Written after a live `config.toml` turned out to have no `[prompt]` section
    /// at all, to establish which half of the path was at fault. It was the
    /// frontend — the save was never requested — but the check belongs here, since
    /// a context that survives a round trip is the whole point of storing it.
    #[test]
    fn a_saved_prompt_context_survives_a_reload() {
        let dir = tempdir();
        let mut config = Config::default();
        config.prompt.context = "I work in Kitchener and prefer metric units.".to_string();

        config.save(dir.path()).expect("save must succeed");

        let written = fs::read_to_string(Config::path_in(dir.path())).expect("readable");
        assert!(
            written.contains("[prompt]"),
            "the section must be written, not omitted as a default:\n{written}"
        );

        let reloaded = Config::load(dir.path()).expect("load must succeed");
        assert_eq!(
            reloaded.prompt.context,
            "I work in Kitchener and prefer metric units."
        );
    }

    #[test]
    fn a_multiline_prompt_context_survives_a_reload() {
        // The Settings box is a textarea, so newlines and quotes are ordinary
        // input. TOML has to escape both, and a naive writer would produce a file
        // that no longer parses — turning a saved setting into a config the app
        // falls back to defaults from on next launch.
        let dir = tempdir();
        let mut config = Config::default();
        config.prompt.context = "Line one.\nLine \"two\".\n\tIndented.".to_string();

        config.save(dir.path()).expect("save must succeed");
        let reloaded = Config::load(dir.path()).expect("load must succeed");

        assert_eq!(
            reloaded.prompt.context,
            "Line one.\nLine \"two\".\n\tIndented."
        );
    }
}
