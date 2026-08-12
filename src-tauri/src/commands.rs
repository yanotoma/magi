//! The IPC surface: what the frontend is allowed to ask for.
//!
//! Everything here is a thin adapter. The rules live in [`crate::config`]; these
//! functions translate between it and the frontend, and turn errors into strings
//! the UI can show. Keeping logic out of this layer is what lets it be tested
//! without a running app.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri::State;
use tokio::sync::mpsc;

use crate::config::secrets::SecretStore;
use crate::config::{ActiveModel, AppearanceConfig, Config, ProviderConfig, Theme};
use crate::llm::cache::CapabilityCache;
use crate::llm::capability::{assign, Capabilities, Tier};
use crate::llm::provider::{Message, StopReason, StreamEvent, TurnRequest};
use crate::llm::{discovery, preflight, prompt, registry};
use crate::permissions::{self, Permission, PermissionKind};
use crate::stt::model::{self, DownloadError, Model, Progress};

/// Everything a command needs, assembled once at startup.
pub struct AppState {
    /// One client, reused. Building a fresh one per call would discard the
    /// connection pool and the TLS session cache on every request.
    pub http: reqwest::Client,

    /// A second client for the model download, which is blocking.
    ///
    /// Separate rather than shared because the two APIs are different types, not two
    /// views of one. Built once for the same reason as the async one: a fresh client per
    /// call would discard the connection pool and the TLS session cache.
    pub http_blocking: reqwest::blocking::Client,
    pub config: Mutex<Config>,
    pub config_dir: PathBuf,
    /// `Arc` rather than `Box` so it can be cloned into a blocking task. Reading
    /// the keychain must not happen on the main thread — see [`key_hints`].
    pub secrets: Arc<dyn SecretStore>,

    /// Fingerprints of stored keys, by provider id.
    ///
    /// The keychain prompts for access, and in a development build it prompts
    /// every time because the ACL is tied to the binary's signature and `cargo`
    /// produces a new binary on each compile. `get_config` runs after every
    /// mutation, so reading through to the keychain each time turns a settings
    /// screen into a stream of password dialogs.
    ///
    /// Caching the fingerprint rather than the secret keeps the key itself out of
    /// process memory beyond the moment it is read. Invalidated whenever a key is
    /// written or deleted.
    pub key_hints: Mutex<HashMap<String, Option<String>>>,

    /// What each model turned out to be able to do, loaded at startup.
    ///
    /// Held in memory as well as on disk so a turn can read a tier without a file
    /// read on the way to every request.
    pub capabilities: Mutex<CapabilityCache>,

    /// The microphone. Behind the trait, so a test or a future headless mode can
    /// substitute a fake without this module knowing.
    pub microphone: Box<dyn crate::audio::AudioSource>,

    /// The speech model. Loads on first use, so holding one here costs nothing until
    /// somebody speaks.
    ///
    /// Swappable, because the model and the language are settings. Built once at startup,
    /// changing either in Settings would save the config, show the new value, and keep
    /// transcribing with the old model until a restart — which is the kind of silent
    /// wrongness this project spends its effort avoiding.
    ///
    /// An `Arc` inside the `Mutex` so a caller clones the pointer and releases the lock
    /// before running inference. Holding it across a transcription would freeze Settings
    /// for seconds, and a transcription already under way finishes with the model it
    /// started on rather than being cut off.
    pub transcriber: Mutex<Arc<dyn crate::stt::Transcriber>>,

    /// Where speech models live: the app data directory plus `models/`.
    ///
    /// Separate from `config_dir` because these are a different kind of thing — a
    /// 141 MB binary blob has no business next to a file the user is encouraged to open
    /// in an editor and paste into bug reports.
    pub models_dir: PathBuf,

    /// The most recent screenshot, kept for the next question.
    ///
    /// Without it Magi cannot see what it looked at a turn ago: the panel resends history as
    /// role and content only, so a capture lives just inside the turn that took it, and a
    /// follow-up like "and what does that mean?" arrives with nothing attached.
    ///
    /// **One, not all.** Keeping every capture is the quadratic growth the design doc warns
    /// about — an image paid for again on every later turn. Keeping the newest is a fixed
    /// ceiling that covers the case people actually hit, which is asking a second question
    /// about the thing they just showed.
    ///
    /// Held here rather than in the panel because the panel never receives the image. Sending
    /// megabytes out to the webview so it could send them back is the obvious wrong shape.
    pub last_capture: Mutex<Option<RememberedCapture>>,

    /// When the panel was last hidden, or `None` while it is open.
    ///
    /// Exists so the remembered screenshot can expire. A thread that survives being closed —
    /// which is what the maintainer asked for — means a picture of someone's screen could
    /// otherwise sit in memory for a working day.
    ///
    /// A timestamp *and* a timer, because either alone is wrong. A timer that fires five
    /// minutes after closing would also fire five minutes after a close that was followed by
    /// reopening and closing again thirty seconds ago. Checking the timestamp when it fires is
    /// what keeps the expiry honest; the timer is what actually frees the memory rather than
    /// waiting for someone to ask.
    pub panel_hidden_at: Mutex<Option<std::time::Instant>>,

    /// What Magi is doing. The one authority, and the only writer of `magi://state`.
    ///
    /// `Arc` because events arrive from the hotkey handler, from a `spawn_blocking` worker
    /// and from the turn task, and none of them should wait on the others for longer than a
    /// comparison and an assignment.
    pub session: Arc<crate::session::Session>,

    /// The screen. Behind the trait, so the tests that matter — which display was chosen,
    /// how big the result is — need no display attached, and so a Linux build has something
    /// to hold at all.
    ///
    /// `Arc` rather than `Box`, for the same reason as [`secrets`]: a capture is a
    /// synchronous round trip through the window server and `CLAUDE.md` requires it on
    /// `spawn_blocking`, which needs an owned handle to move into the closure.
    ///
    /// [`secrets`]: AppState::secrets
    pub screen: Arc<dyn crate::capture::ScreenCapture>,

    /// Every screenshot this run has taken, and why.
    ///
    /// `Arc` because the capture path records from a `spawn_blocking` worker while Settings
    /// reads from a command, and neither should wait on the other for longer than a push
    /// onto a queue. Deliberately not persisted — see `capture::log`.
    pub capture_log: Arc<crate::capture::CaptureLog>,

    /// Set while a model download is running.
    ///
    /// Guards against a second download of the same file: two writers appending to one
    /// partial file produce a corrupt result of roughly the right size, and the only
    /// thing that would notice is the checksum at the end.
    pub downloading: Mutex<Option<Model>>,

    /// The task running the turn currently in flight, if any.
    ///
    /// Aborting it is what cancels: the task owns both the receiver and the
    /// request future, so dropping it closes the channel the provider is writing
    /// to and the provider stops on its next send.
    ///
    /// Holding the *sender* here would not work. A provider stops when the
    /// receiver goes away, and while both this field and the provider held a
    /// sender, dropping one would leave the channel open.
    pub in_flight: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// A screenshot worth showing the model again next turn.
#[derive(Debug, Clone)]
pub struct RememberedCapture {
    /// What it was of, for the sentence that introduces it.
    pub describes: String,
    pub png: Vec<u8>,
}

/// A command failure, as a message the panel can render.
///
/// Commands return `Result<T, String>` rather than a typed error because the
/// frontend can only act on prose anyway. The typed errors live one layer down,
/// where callers can still match on them.
type CommandResult<T> = Result<T, String>;

fn to_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// A provider as the settings screen needs to see it.
///
/// Deliberately not `ProviderConfig`: the UI must know *whether* a key is stored
/// without ever receiving it. Sending the key so the form could pre-fill it
/// would put the secret in the webview, in devtools, and in any screenshot of
/// the settings window.
#[derive(Debug, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub kind: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub requires_key: bool,

    /// The context window in tokens, or `None` when it has not been set.
    ///
    /// Sent back so the form shows what is stored rather than an empty box that would
    /// silently clear the value on the next save.
    pub context_tokens: Option<u32>,

    pub has_key: bool,
    /// Enough to tell two keys apart, never enough to use one. `None` when no
    /// key is stored.
    pub key_hint: Option<String>,

    /// Probe results for the models of this provider that have been tested.
    ///
    /// Only tested models appear. An absent entry means "not probed yet", which the
    /// UI must show as unknown rather than as incapable — a model nobody has tested
    /// is not a model that failed.
    pub capabilities: Vec<ModelCapabilityView>,
}

/// One model's probe results, as the capability matrix in Settings needs them.
///
/// The individual booleans travel alongside the tier rather than only the tier,
/// because the tier alone does not answer the question a user actually has. "Text
/// only" says screen reading is off; the row of checks says *why*, which is what
/// tells them whether to pick a different model or fix their endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct ModelCapabilityView {
    pub model: String,
    pub tier: Tier,
    pub label: String,
    pub explanation: String,
    pub reachable: bool,
    pub vision: bool,
    pub tools: bool,
    pub structured_output: bool,
}

impl ModelCapabilityView {
    fn new(model: &str, capabilities: Capabilities) -> Self {
        let tier = assign(&capabilities);
        Self {
            model: model.to_string(),
            tier,
            label: tier.label().to_string(),
            explanation: tier.explanation().to_string(),
            reachable: capabilities.reachable,
            vision: capabilities.vision,
            tools: capabilities.tools,
            structured_output: capabilities.structured_output,
        }
    }
}

/// One speech model as the Voice pane renders it.
#[derive(Debug, Clone, Serialize)]
pub struct SpeechModelView {
    pub id: Model,
    pub label: String,
    pub description: String,
    /// Rounded for display. The real length comes from the server at download time —
    /// see `Model::approximate_bytes`.
    pub approximate_mb: u64,
    /// Whether the file is already on disk.
    pub downloaded: bool,
    pub selected: bool,
    /// Whether it understands languages other than English. The single most consequential
    /// fact about a model here, because getting it wrong is silent.
    pub multilingual: bool,
}

/// Everything the Voice pane needs in one shape.
#[derive(Debug, Serialize)]
pub struct VoiceView {
    pub models: Vec<SpeechModelView>,

    /// The languages Magi expects. Empty means detect from all of them.
    pub languages: Vec<String>,

    /// Set when the chosen model cannot honour the chosen language.
    ///
    /// An English-only model given anything else does not fail — it writes English words
    /// that sound similar. So the combination is reported rather than left to produce a
    /// confident wrong transcript.
    pub language_ignored: bool,
    /// Whether the selected model is ready to transcribe with.
    pub ready: bool,
    pub microphone: Permission,
    /// What to tell the user about the microphone, and what they can do.
    pub microphone_explanation: String,
    /// The System Settings deep link for the microphone pane.
    pub microphone_settings_url: String,
    /// The model currently downloading, if any.
    pub downloading: Option<Model>,
}

/// Everything Settings shows about screen reading.
///
/// Same shape as the microphone rows in [`VoiceView`]: the state, what it means, and where
/// to go about it. The permission and the log belong in one view because they answer two
/// halves of the same question — *can* Magi read my screen, and *has* it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureView {
    pub screen_recording: Permission,

    /// What to tell the user, and what they can do about it.
    pub screen_recording_explanation: String,

    /// The System Settings deep link for the Screen Recording pane.
    pub screen_recording_settings_url: String,

    /// Every capture this run of Magi has made, most recent first.
    pub entries: Vec<crate::capture::Entry>,
}

#[derive(Debug, Serialize)]
pub struct ConfigView {
    pub providers: Vec<ProviderView>,
    pub active: Option<ActiveModel>,
    pub hotkey: String,
    pub prompt: crate::config::PromptConfig,
    pub appearance: AppearanceConfig,
    pub config_path: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveProviderRequest {
    pub provider: ProviderConfig,
    /// `None` leaves any stored key untouched; `Some("")` clears it.
    pub api_key: Option<String>,
}

/// Fingerprints for the given providers, reading the keychain off the main thread.
///
/// The thread matters more than anything else here. Reading the keychain is a
/// synchronous Mach round trip to `securityd`, and when the ACL does not already
/// permit this binary, `securityd` blocks until the user answers an access
/// dialog. On the main thread that is an unbreakable deadlock rather than a
/// pause: the main thread is the only one that can present and service UI, so it
/// ends up waiting for an answer to a dialog only it could have drawn. Magi runs
/// as an accessory app with no Dock icon, which removes even the chance of the
/// user finding the prompt behind another window. The symptom is a spinning
/// cursor over a dead tray icon and a hotkey that does nothing.
///
/// So the read goes to a blocking task, and the main thread keeps pumping events
/// while the dialog is up.
///
/// A keychain error is treated as "no key" rather than surfaced: Settings must
/// still render so the user can fix whatever is wrong, and a locked keychain
/// should not blank the whole screen.
async fn key_hints(
    state: &State<'_, AppState>,
    provider_ids: &[String],
) -> CommandResult<HashMap<String, Option<String>>> {
    let mut resolved: HashMap<String, Option<String>> = HashMap::new();
    let mut missing: Vec<String> = Vec::new();

    {
        let cache = state.key_hints.lock().map_err(to_message)?;
        for id in provider_ids {
            match cache.get(id) {
                Some(hint) => {
                    resolved.insert(id.clone(), hint.clone());
                }
                None => missing.push(id.clone()),
            }
        }
    }

    // Every fingerprint was cached, so there is nothing to read and no reason to
    // pay for a task. This is the common path: the cache only misses once per
    // provider per run.
    if missing.is_empty() {
        return Ok(resolved);
    }

    let store = Arc::clone(&state.secrets);
    let fetched = tauri::async_runtime::spawn_blocking(move || {
        missing
            .into_iter()
            .map(|id| {
                let hint = store
                    .get(&id)
                    .ok()
                    .flatten()
                    .filter(|k| !k.is_empty())
                    .as_deref()
                    .map(crate::config::secrets::fingerprint);
                (id, hint)
            })
            .collect::<HashMap<String, Option<String>>>()
    })
    .await
    .map_err(to_message)?;

    if let Ok(mut cache) = state.key_hints.lock() {
        for (id, hint) in &fetched {
            cache.insert(id.clone(), hint.clone());
        }
    }

    resolved.extend(fetched);
    Ok(resolved)
}

/// Runs one keychain operation off the calling thread.
///
/// Every call into the keychain — read, write or delete — is a synchronous Mach
/// round trip that can stop until the user answers an access dialog. None of them
/// belong on the main thread, for the reason spelled out on [`key_hints`], and
/// none belong on an async runtime thread either, where a blocked worker starves
/// whatever else that runtime was going to poll.
///
/// Having a single funnel is deliberate. The bug this was written for came from
/// one keychain call in one command being on the wrong thread; a helper makes the
/// right thing shorter to write than the wrong thing.
async fn with_secrets<T, F>(state: &State<'_, AppState>, operation: F) -> CommandResult<T>
where
    F: FnOnce(&dyn SecretStore) -> Result<T, crate::config::secrets::SecretError> + Send + 'static,
    T: Send + 'static,
{
    let store = Arc::clone(&state.secrets);
    tauri::async_runtime::spawn_blocking(move || operation(store.as_ref()))
        .await
        .map_err(to_message)?
        .map_err(to_message)
}

/// Drops a cached fingerprint after the underlying key changed.
fn forget_key_hint(state: &State<'_, AppState>, provider_id: &str) {
    if let Ok(mut cache) = state.key_hints.lock() {
        cache.remove(provider_id);
    }
}

/// Pushes the active model and its tier into the tray tooltip.
///
/// Called after anything that can change either — selecting a model, probing one,
/// or editing the provider it belongs to. Recomputed rather than tracked, because a
/// cached copy would be one more thing to invalidate and the cost is two map
/// lookups.
fn refresh_tray(app: &tauri::AppHandle, state: &State<'_, AppState>) {
    let active = state
        .config
        .lock()
        .ok()
        .and_then(|config| config.active.clone());

    let tier = active.as_ref().and_then(|active| {
        state
            .capabilities
            .lock()
            .ok()
            .and_then(|cache| cache.tier(&active.provider, &active.model))
    });

    crate::tray::refresh_tooltip(app, active.as_ref().map(|a| a.model.as_str()), tier);
}

/// Discards probe results for a provider, in memory and on disk.
///
/// Called whenever a provider is saved or removed. Capabilities are a property of
/// the endpoint as much as of the model: the same model name behind a different
/// URL, or reached with a different key, is a different deployment. Keeping the old
/// results would report a tier that was measured somewhere else — and it would do
/// so silently, since a stale entry is indistinguishable from a fresh one.
///
/// Discarding on every save is deliberately blunt. It re-probes after an edit that
/// changed nothing relevant, which costs four requests the user asked for by
/// pressing Save; the alternative is comparing old and new fields and being wrong
/// about which ones matter.
fn forget_capabilities(state: &State<'_, AppState>, provider_id: &str) {
    if let Ok(mut cache) = state.capabilities.lock() {
        cache.forget_provider(provider_id);
        if let Err(error) = cache.save(&state.config_dir) {
            tracing::warn!(%error, "could not persist the cleared capability cache");
        }
    }
}

/// The whole configuration, including a fingerprint of each stored key.
///
/// Asynchronous because it reads the keychain, which must not happen on the main
/// thread — see [`key_hints`]. Callers that only need the appearance settings
/// should use [`get_appearance`] instead and touch no secrets at all.
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> CommandResult<ConfigView> {
    // Snapshot under the lock, then release it. A `std::sync::MutexGuard` held
    // across an `await` would be held for as long as the keychain takes, which is
    // unbounded when a dialog is up — every other command would block behind it.
    let (providers, active, hotkey, prompt, appearance) = {
        let config = state.config.lock().map_err(to_message)?;
        (
            config.providers.clone(),
            config.active.clone(),
            config.hotkey.toggle.clone(),
            config.prompt.clone(),
            config.appearance.clone(),
        )
    };

    let ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
    let hints = key_hints(&state, &ids).await?;

    // Snapshotted rather than read per provider inside the loop, so the lock is
    // taken once and never held across the `await` above.
    let probed: HashMap<String, Vec<ModelCapabilityView>> = {
        let cache = state.capabilities.lock().map_err(to_message)?;
        ids.iter()
            .map(|id| {
                let mut rows: Vec<ModelCapabilityView> = cache
                    .tiers_for(id)
                    .keys()
                    .filter_map(|model| {
                        cache
                            .get(id, model)
                            .map(|caps| ModelCapabilityView::new(model, caps))
                    })
                    .collect();
                // A HashMap has no order, and a capability matrix that reshuffles
                // its rows on every render is unreadable.
                rows.sort_by(|a, b| a.model.cmp(&b.model));
                (id.clone(), rows)
            })
            .collect()
    };

    let providers = providers
        .iter()
        .map(|p| {
            let hint = hints.get(&p.id).cloned().flatten();
            ProviderView {
                id: p.id.clone(),
                kind: match p.kind {
                    crate::config::ProviderKind::OpenaiCompatible => "openai-compatible".into(),
                    crate::config::ProviderKind::Anthropic => "anthropic".into(),
                },
                base_url: p.base_url.clone(),
                models: p.models.clone(),
                requires_key: p.requires_key,
                context_tokens: p.context_tokens,
                has_key: hint.is_some(),
                key_hint: hint,
                capabilities: probed.get(&p.id).cloned().unwrap_or_default(),
            }
        })
        .collect();

    Ok(ConfigView {
        providers,
        active,
        hotkey,
        prompt,
        appearance,
        // Shown in Settings so the file is discoverable. A config nobody can
        // find is a config nobody can paste into a bug report.
        config_path: Config::path_in(&state.config_dir).display().to_string(),
    })
}

/// The appearance settings alone, touching no secrets.
///
/// Exists so the panel does not have to call [`get_config`]. The panel is created
/// hidden at launch, so its first request runs before there is any window on
/// screen — and when that request read the keychain, launching Magi meant a
/// keychain dialog with no window to attach to and no way for the user to reach
/// it. The panel needs one boolean; asking for the whole configuration to get it
/// dragged the secret store into the startup path for no reason.
#[tauri::command]
pub fn get_appearance(state: State<'_, AppState>) -> CommandResult<AppearanceConfig> {
    let config = state.config.lock().map_err(to_message)?;
    Ok(config.appearance.clone())
}

/// Adds a provider, or replaces one with the same id.
#[tauri::command]
pub async fn save_provider(
    state: State<'_, AppState>,
    request: SaveProviderRequest,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;

        match config
            .providers
            .iter_mut()
            .find(|p| p.id == request.provider.id)
        {
            Some(existing) => *existing = request.provider.clone(),
            None => config.providers.push(request.provider.clone()),
        }

        // Validate and persist before touching the keychain, so a rejected
        // config does not leave a secret behind for a provider that was never
        // saved.
        config.save(&state.config_dir).map_err(to_message)?;
    }

    // A saved provider may point somewhere new, so anything measured against the
    // old endpoint is no longer about this one.
    forget_capabilities(&state, &request.provider.id);

    if let Some(key) = request.api_key {
        forget_key_hint(&state, &request.provider.id);
        let id = request.provider.id.clone();
        if key.is_empty() {
            with_secrets(&state, move |store| store.delete(&id)).await?;
        } else {
            with_secrets(&state, move |store| store.set(&id, &key)).await?;
        }
    }

    get_config(state).await
}

#[tauri::command]
pub async fn remove_provider(state: State<'_, AppState>, id: String) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        config.providers.retain(|p| p.id != id);

        // Removing the provider a selection points at would leave the config
        // invalid, so the selection goes with it.
        if config.active.as_ref().is_some_and(|a| a.provider == id) {
            config.active = None;
        }

        config.save(&state.config_dir).map_err(to_message)?;
    }

    // Orphaned secrets are worth cleaning up: a key nobody can see is a key
    // nobody remembers to revoke.
    forget_key_hint(&state, &id);
    forget_capabilities(&state, &id);
    let orphaned = id.clone();
    with_secrets(&state, move |store| store.delete(&orphaned)).await?;

    get_config(state).await
}

#[tauri::command]
pub async fn set_active_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    provider: String,
    model: String,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        let previous = config.active.take();

        config.active = Some(ActiveModel {
            provider: provider.clone(),
            model: model.clone(),
        });

        // Save validates. On rejection the previous selection is restored, so a
        // failed change does not leave the app with no model at all.
        if let Err(error) = config.save(&state.config_dir) {
            config.active = previous;
            return Err(to_message(error));
        }
    }

    refresh_tray(&app, &state);

    get_config(state).await
}

/// Asks a provider which models it serves.
///
/// Takes the provider from the form rather than from the saved config, so the
/// user can discover models for an endpoint they have typed but not yet saved —
/// which is the order people actually work in.
#[tauri::command]
pub async fn discover_models(
    state: State<'_, AppState>,
    provider: ProviderConfig,
    api_key: Option<String>,
) -> CommandResult<Vec<String>> {
    // A key typed into the form wins; otherwise fall back to the stored one, so
    // rediscovering an existing provider does not mean retyping its key.
    let key = match api_key.filter(|k| !k.is_empty()) {
        Some(typed) => Some(typed),
        None => {
            let id = provider.id.clone();
            with_secrets(&state, move |store| store.get(&id)).await?
        }
    };

    discovery::discover_models(&state.http, &provider, key.as_deref()).await
}

/// Probes one model and records what it can do.
///
/// Returns the whole configuration so Settings re-renders with the new row, rather
/// than just the result — the alternative leaves the frontend merging state by
/// hand, and every other mutating command here already returns the full view.
///
/// A failure to write the cache is logged rather than returned. The probes have
/// already run and their answer is in memory, so the tier is correct for this
/// session; reporting an error would suggest the pre-flight failed when what
/// actually happened is that it will have to run again next launch.
#[tauri::command]
pub async fn run_preflight(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    model: String,
) -> CommandResult<ConfigView> {
    let provider_config = {
        let config = state.config.lock().map_err(to_message)?;
        config
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .cloned()
            .ok_or_else(|| format!("Provider '{provider_id}' is not configured."))?
    };

    if !provider_config.models.iter().any(|m| m == &model) {
        return Err(format!(
            "Provider '{provider_id}' has no model named '{model}'. \
             Discover its models first."
        ));
    }

    let id_for_key = provider_id.clone();
    let api_key = with_secrets(&state, move |store| store.get(&id_for_key))
        .await?
        .filter(|k| !k.is_empty());

    if provider_config.requires_key && api_key.is_none() {
        return Err(format!(
            "Provider '{provider_id}' needs an API key before it can be tested."
        ));
    }

    let provider = registry::build(state.http.clone(), &provider_config, api_key);
    let capabilities = preflight::run(provider.as_ref(), &model).await;

    tracing::info!(
        provider = %provider_id,
        model = %model,
        tier = ?assign(&capabilities),
        "pre-flight complete"
    );

    {
        let mut cache = state.capabilities.lock().map_err(to_message)?;
        cache.set(&provider_id, &model, capabilities);
        if let Err(error) = cache.save(&state.config_dir) {
            tracing::warn!(%error, "capability results could not be written; they will be re-probed");
        }
    }

    refresh_tray(&app, &state);

    get_config(state).await
}

/// Applies a theme to every window and remembers it.
#[tauri::command]
pub async fn set_theme(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    theme: Theme,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        config.appearance.theme = theme;
        config.save(&state.config_dir).map_err(to_message)?;
    }

    apply_theme(&app, theme);
    get_config(state).await
}

/// Pushes the theme down to the webviews.
///
/// `None` means "follow the system", which is what makes System a real option
/// rather than a synonym for whichever mode happened to be active at startup.
pub fn apply_theme(app: &tauri::AppHandle, theme: Theme) {
    use tauri::Manager;

    let requested = match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    };

    for label in [crate::windows::PANEL, crate::windows::SETTINGS] {
        if let Some(window) = app.get_webview_window(label) {
            if let Err(error) = window.set_theme(requested) {
                tracing::warn!(%error, label, "could not apply the theme to this window");
            }
        }
    }
}

/// Replaces the standing context sent with every turn.
#[tauri::command]
pub async fn set_prompt_context(
    state: State<'_, AppState>,
    context: String,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        // `save` validates, so an over-length context is refused there. Putting
        // the old value back keeps the running app from carrying text that the
        // file rejected — otherwise the limit would hold until the next launch
        // and then quietly stop holding.
        let previous = std::mem::replace(&mut config.prompt.context, context);
        if let Err(error) = config.save(&state.config_dir) {
            config.prompt.context = previous;
            return Err(to_message(error));
        }
    }
    get_config(state).await
}

/// Rebinds the global shortcut, or reports why it could not be.
///
/// The OS is asked before the file is written. A shortcut saved but not
/// registered would show in Settings as the current hotkey while doing nothing —
/// the config would be a record of an intention rather than of the state.
#[tauri::command]
pub async fn set_hotkey(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    shortcut: String,
) -> CommandResult<ConfigView> {
    let previous = {
        let config = state.config.lock().map_err(to_message)?;
        config.hotkey.toggle.clone()
    };

    crate::hotkey::rebind(&app, &previous, &shortcut).map_err(to_message)?;

    {
        let mut config = state.config.lock().map_err(to_message)?;
        config.hotkey.toggle = shortcut.clone();
        if let Err(error) = config.save(&state.config_dir) {
            // The OS accepted the new shortcut but the file would not take it, so
            // put the OS back where the file still says it is. Leaving them out of
            // step would mean a hotkey that works until the next launch and then
            // silently changes back.
            config.hotkey.toggle = previous.clone();
            if let Err(restore) = crate::hotkey::rebind(&app, &shortcut, &previous) {
                tracing::error!(%restore, "could not restore the previous shortcut");
            }
            return Err(to_message(error));
        }
    }

    get_config(state).await
}

/// Turns the reasoning display on or off.
#[tauri::command]
pub async fn set_show_thinking(
    state: State<'_, AppState>,
    show: bool,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        config.appearance.show_thinking = show;
        config.save(&state.config_dir).map_err(to_message)?;
    }
    get_config(state).await
}

/// Sends one text turn and streams the reply back as events.
///
/// Returns as soon as the request is under way. The reply arrives on
/// `magi://token`, and the turn ends with `magi://turn-done` or `magi://error` —
/// a command that awaited the whole answer would block the panel for as long as
/// the model took, which defeats streaming.
#[tauri::command]
pub async fn send_text_turn(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    text: String,
    history: Vec<TurnMessage>,
) -> CommandResult<()> {
    let (provider_config, model, context) = {
        let config = state.config.lock().map_err(to_message)?;
        let context = config.prompt.context.clone();
        let (provider, model) = config.active_provider().ok_or_else(|| {
            "No model selected. Open Settings, add a provider, and pick a model.".to_string()
        })?;
        (provider.clone(), model.to_string(), context)
    };

    // The tier decides which instructions the model gets. An unprobed model falls
    // back to the most conservative tier rather than the most capable: telling a
    // model it can see the screen when nothing has verified that makes it promise
    // to look, which is worse than a model that correctly says it cannot.
    let tier = {
        let cache = state.capabilities.lock().map_err(to_message)?;
        cache
            .tier(&provider_config.id, &model)
            .unwrap_or(Tier::TextOnly)
    };
    let system = prompt::system_prompt(tier, &context);

    let provider_id = provider_config.id.clone();
    let api_key = with_secrets(&state, move |store| store.get(&provider_id))
        .await?
        .filter(|k| !k.is_empty());

    if provider_config.requires_key && api_key.is_none() {
        return Err(format!(
            "Provider '{}' needs an API key. Add one in Settings.",
            provider_config.id
        ));
    }

    let mut messages: Vec<Message> = history
        .into_iter()
        .map(|m| match m.role.as_str() {
            "assistant" => Message::assistant(m.content),
            _ => Message::user(m.content),
        })
        .collect();
    // The Tier 2 path. A model that sees but malforms tool calls is never offered a tool,
    // so Magi decides from the user's own words and attaches the screenshot before asking.
    // By the time the model reads the question the image is already there and there is
    // nothing for it to call — which is why `llm::prompt` tells this tier that an image
    // *may* be attached and never mentions tools at all.
    let attached = if tier == Tier::Heuristic {
        heuristic_capture(&app, &text).await
    } else {
        Vec::new()
    };

    messages.push(if attached.is_empty() {
        Message::user(text)
    } else {
        Message::user_seeing(text, attached)
    });

    // The previous turn's screenshot, introduced rather than smuggled in. A bare image
    // attached to a new question reads as the user having just shown it; a sentence saying
    // when it was taken is what lets the model treat it as context and notice if it looks
    // stale.
    if let Some(remembered) = state.last_capture.lock().ok().and_then(|last| last.clone()) {
        let position = messages.len() - 1;
        messages.insert(
            position,
            Message::user_seeing(
                format!(
                    "For reference, this is what {} looked like when you last checked. \
                     Capture again if the answer depends on it having changed.",
                    remembered.describes
                ),
                vec![crate::llm::provider::Image {
                    media_type: "image/png",
                    bytes: remembered.png,
                }],
            ),
        );
    }

    // Built here rather than inside the request below, because the tool definitions are part
    // of what the context window has to hold — 1.7 KB of JSON schema is 400-odd tokens, and
    // history cannot be budgeted without subtracting them first.
    //
    // Offered to exactly one tier, matching what the system prompt says. A model that
    // malforms tool syntax must not be handed a definition to malform, and one that cannot
    // see has nothing to do with a screenshot.
    let tools = if tier.offers_capture_tool() {
        // Counted per turn rather than cached: a monitor can be unplugged between
        // questions, and a tool description that claims three when there is one sends
        // the model looking for screens that are not there.
        //
        // Falls back to one on failure. Claiming a single display when there are three
        // costs a wrong answer the user can correct by asking again; claiming three when
        // enumeration is broken costs every turn three captures that cannot happen.
        vec![crate::llm::tools::capture_screen(
            state.screen.displays().map(|d| d.len()).unwrap_or(1).max(1),
        )]
    } else {
        Vec::new()
    };

    // How the window divides between the conversation and the reply. With no window
    // configured this is the constant that was here before and the reply cap Magi always
    // asked for — the feature costs a user who configures nothing exactly nothing.
    let plan = crate::llm::budget::plan(
        provider_config.context_tokens,
        crate::llm::budget::request_overhead(Some(&system), &tools),
    );

    // Trimmed after the new question is appended, so the question is part of what is being
    // budgeted rather than an addition to whatever survived. `fit` never drops the newest
    // exchange, so the thing just pushed is safe.
    let before = messages.len();
    let messages = crate::llm::history::fit(messages, plan.history_budget);
    let dropped = before - messages.len();
    if dropped > 0 {
        tracing::info!(
            dropped,
            "trimmed the conversation to fit the history budget"
        );

        // Said out loud, not only logged. Losing the early part of a conversation changes
        // what the model can answer, and a user who is not told will read the difference as
        // the model having become worse. A log line is not telling them: the app does not
        // emit one unless `RUST_LOG` is set, which nobody running a menu bar app has done.
        if let Err(error) = app.emit("magi://trimmed", dropped) {
            tracing::warn!(%error, "could not report the trim");
        }
    }

    let request = TurnRequest {
        model,
        system: Some(system),
        messages,
        // Not a constant any more. On a large window this is the 4096 it always was; on a
        // small one it is a quarter of the window, because reserving 4096 of an 8k model for
        // a reply that is usually a few hundred tokens spends the conversation on space the
        // model will not use.
        max_tokens: plan.max_tokens,
        tools,
    };

    let provider = registry::build(state.http.clone(), &provider_config, api_key);

    let task = tauri::async_runtime::spawn(async move {
        crate::session::report(&app, crate::session::Event::Asked);
        let mut request = request;
        // One budget for the whole turn, not per request: the point is to bound how many
        // times a model can go round, and a fresh budget each iteration would bound
        // nothing.
        let mut budget = crate::llm::tools::CaptureBudget::new();

        loop {
            // A modest buffer, deliberately. If the UI falls behind, the provider should
            // wait rather than let an unbounded queue grow between the model and the panel.
            let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);

            let mut answer = String::new();
            let mut calls = crate::llm::toolstream::ToolCallStream::new();
            let mut stop = StopReason::EndTurn;

            // Forwarding runs alongside the request so tokens reach the panel as they
            // arrive rather than after the answer is complete.
            let forward = async {
                while let Some(event) = rx.recv().await {
                    let emitted = match event {
                        StreamEvent::Token(token) => {
                            // Reported on every token and acted on once: `apply` returns
                            // `None` when nothing moved, so only the first one becomes a
                            // state event. Cheaper than tracking "have I reported yet"
                            // here, and impossible to get wrong.
                            crate::session::report(&app, crate::session::Event::Answering);
                            // Kept as well as forwarded. The assistant's turn has to be
                            // replayed whole when a tool result follows it, and the panel
                            // is not a place to read it back from.
                            answer.push_str(&token);
                            app.emit("magi://token", token)
                        }
                        // Always emitted; the panel decides whether to show it. The
                        // channel is in-process, so the cost of sending it when it is
                        // hidden is not worth a round trip to read a setting here.
                        StreamEvent::Thinking(thought) => app.emit("magi://thinking", thought),
                        // Consumed rather than forwarded: these are wire fragments, and
                        // what the panel hears about is the capture that results.
                        StreamEvent::ToolStart { index, id, name } => {
                            calls.begin(index, &id, &name);
                            Ok(())
                        }
                        StreamEvent::ToolArguments { index, json } => {
                            calls.push_arguments(index, &json);
                            Ok(())
                        }
                        // Held back. A turn that stopped to call a tool is not finished,
                        // and telling the panel it was would end the answer mid-thought.
                        StreamEvent::Done(reason) => {
                            stop = reason;
                            Ok(())
                        }
                    };
                    if let Err(error) = emitted {
                        tracing::warn!(%error, "could not emit a turn event");
                        break;
                    }
                }
            };

            // Both halves run together so tokens reach the panel as they arrive
            // rather than after the answer is complete.
            let (result, ()) = tokio::join!(provider.turn(request.clone(), tx), forward);

            if let Err(error) = result {
                // The log gets the summary, the user gets the whole thing. A provider's
                // rejection body can quote the request back, and the log is now a file.
                tracing::warn!(summary = %error.log_summary(), "turn failed");
                let message = error.to_string();
                crate::session::report(&app, crate::session::Event::Stopped);
                if let Err(error) = app.emit("magi://error", message) {
                    tracing::warn!(%error, "could not emit the turn error");
                }
                return;
            }

            let calls = calls.finish();
            if calls.is_empty() {
                // The ordinary end of a turn. Also where a model that said
                // `stop_reason: tool_use` and then named no call lands, which is a
                // malformed response rather than a loop to continue.
                if matches!(stop, StopReason::ToolUse) {
                    tracing::warn!("the model stopped for a tool call and named none");
                }
                crate::session::report(&app, crate::session::Event::Answered);
                if let Err(error) = app.emit("magi://turn-done", describe(&stop)) {
                    tracing::warn!(%error, "could not emit the turn completion");
                }
                return;
            }

            // The assistant's turn goes back whole, prose and calls together. Dropping
            // the calls and keeping the prose makes the results below reference calls
            // that are no longer in the history, which both APIs reject.
            request.messages.push(Message::Assistant {
                text: std::mem::take(&mut answer),
                calls: calls.clone(),
            });

            // One result per call, always. A call left unanswered is an error rather
            // than a missing detail.
            for call in &calls {
                request
                    .messages
                    .push(answer_call(&app, call, &mut budget).await);
            }

            // Re-fitted before going round again. The budget was computed for the request
            // that went out, and this is no longer that request: a capture result carries a
            // full image, so up to three of them can add more than the conversation they
            // were appended to. Leaving it unchecked meant the one path that grows a request
            // mid-turn was the one path that never measured it.
            //
            // Safe here and nowhere earlier. `fit` keeps the newest exchange whole, so the
            // calls just made and the results answering them stay together — which is the
            // rule both APIs enforce and the reason `fit` drops exchanges rather than
            // messages.
            let before = request.messages.len();
            request.messages = crate::llm::history::fit(
                std::mem::take(&mut request.messages),
                plan.history_budget,
            );
            let dropped = before - request.messages.len();
            if dropped > 0 {
                tracing::info!(dropped, "trimmed the conversation to fit a capture result");
                if let Err(error) = app.emit("magi://trimmed", dropped) {
                    tracing::warn!(%error, "could not report the trim");
                }
            }
        }
    });

    // Replacing the handle aborts the previous turn, so asking a second question
    // cancels the first instead of interleaving two answers into one panel.
    {
        let mut in_flight = state.in_flight.lock().map_err(to_message)?;
        if let Some(previous) = in_flight.replace(task) {
            previous.abort();
        }
    }

    Ok(())
}

/// A history entry as the frontend sends it.
#[derive(Debug, Deserialize)]
pub struct TurnMessage {
    pub role: String,
    pub content: String,
}

/// Cancels the turn in flight, if there is one.
#[tauri::command]
pub fn cancel_turn(app: tauri::AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    crate::session::report(&app, crate::session::Event::Stopped);
    let mut in_flight = state.in_flight.lock().map_err(to_message)?;
    if let Some(task) = in_flight.take() {
        // Aborting drops the receiver, so the provider stops on its next send
        // and the HTTP request is dropped with it.
        task.abort();
    }
    Ok(())
}

/// Turns a stop reason into something worth showing.
///
/// `None` for an ordinary finish: the answer is already on screen and saying
/// "ended normally" underneath it is noise. Anything else is a reason the user
/// would otherwise have to guess at from a short or empty reply.
fn describe(reason: &StopReason) -> Option<String> {
    match reason {
        StopReason::EndTurn => None,
        StopReason::MaxTokens => Some("The reply hit the length limit.".to_string()),
        // Only reachable when the loop ended on a tool stop with no call to run, which
        // is a malformed response. Named rather than hidden: the user is looking at a
        // reply that stops mid-thought and deserves to know why.
        StopReason::ToolUse => {
            Some("The model asked to use a tool and did not say which.".to_string())
        }
        StopReason::Other(other) => Some(format!("The model stopped: {other}")),
    }
}

/// Captures the screen when the user's words point at it, for the tier that cannot ask.
///
/// Returns an empty list when the words did not point at anything, when the capture failed,
/// or when there is no screen — every one of which means "ask without an image" rather than
/// "fail the turn". A Tier 2 model with no screenshot still answers from the conversation,
/// which is what its system prompt tells it to do.
async fn heuristic_capture(app: &tauri::AppHandle, text: &str) -> Vec<crate::llm::provider::Image> {
    use tauri::Manager;

    let Some(deixis) = crate::capture::asks_about_the_screen(text) else {
        return Vec::new();
    };

    crate::session::report(app, crate::session::Event::Looking);

    // The matched phrase chooses the target, which costs nothing because the phrase is
    // already known: someone who said "this screen" meant the display, and someone who
    // said "this error" meant the window they are looking at — which is also the sharper
    // capture, so the common case is the good one.
    let wants_whole_screen = deixis.phrase.contains("screen") || deixis.phrase.contains("pantalla");

    let screen = Arc::clone(&app.state::<AppState>().screen);
    let captured = tauri::async_runtime::spawn_blocking(move || {
        if wants_whole_screen {
            screen.capture_active_display()
        } else {
            screen.capture_focused_window()
        }
    })
    .await;

    crate::session::report(app, crate::session::Event::Looked);

    let capture = match captured {
        Ok(Ok(capture)) => capture,
        Ok(Err(error)) => {
            // Not surfaced to the user. They did not ask for a screenshot — they asked a
            // question, and Magi guessed that a picture would help. A guess that failed
            // should not become an error report about a feature they never invoked.
            // The phrase that triggered this is deliberately not logged. It is a literal
            // fragment of what the user said — "this error", "esta pantalla" — and the log
            // is now a file that outlives the run. What went wrong is diagnosable from the
            // error; which words produced it is the user's business.
            tracing::warn!(%error, "the heuristic capture failed");
            return Vec::new();
        }
        Err(error) => {
            tracing::warn!(%error, "the heuristic capture task failed");
            return Vec::new();
        }
    };

    let state = app.state::<AppState>();
    state.capture_log.record(crate::capture::Entry {
        at: unix_millis(),
        subject: capture.subject.clone(),
        // The phrase that caused it, so the log can say "you said \"this error\"" rather
        // than leaving the user to wonder what triggered a capture they did not request.
        reason: crate::llm::tools::Reason::PhraseMatched {
            phrase: deixis.phrase.clone(),
            language: deixis.language.to_string(),
        },
        width: capture.width,
        height: capture.height,
        visual_tokens: capture.visual_tokens(),
    });

    if let Err(error) = app.emit("magi://captured", capture.subject.describe()) {
        tracing::warn!(%error, "could not announce the capture");
    }

    remember(app, Some(&capture));

    vec![crate::llm::provider::Image {
        media_type: "image/png",
        bytes: capture.png,
    }]
}

/// Keeps `capture` for the next turn, replacing whatever was there.
///
/// The newest wins. An older screenshot is not merely less useful — it is actively misleading,
/// because the screen has moved on and the model has no way to tell.
fn remember(app: &tauri::AppHandle, capture: Option<&crate::capture::Capture>) {
    use tauri::Manager;

    let Some(capture) = capture else {
        return;
    };

    if let Ok(mut last) = app.state::<AppState>().last_capture.lock() {
        *last = Some(RememberedCapture {
            describes: capture.subject.describe(),
            png: capture.png.clone(),
        });
    }
}

/// How long a remembered screenshot outlives the panel being closed.
///
/// Five minutes, which the maintainer chose and which reads as the right shape: long enough
/// that closing the panel to look at something and coming back keeps the context, short enough
/// that a picture of someone's screen is not still in memory after lunch.
///
/// The conversation itself does not expire — only the image. The text is small and is the part
/// worth continuing; the image is megabytes and is the part that is a photograph of what
/// somebody was doing.
pub const CAPTURE_LIFETIME: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Starts the clock on the remembered screenshot.
///
/// Called when the panel is hidden. Spawns a task rather than only recording the time, because
/// the point is to free the memory: a lazy check on next use would leave several megabytes
/// sitting there for as long as nobody reopened the panel, which is exactly the case being
/// guarded against.
pub fn expire_capture_later(app: &tauri::AppHandle) {
    use tauri::Manager;

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut hidden) = state.panel_hidden_at.lock() {
            *hidden = Some(std::time::Instant::now());
        }
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CAPTURE_LIFETIME).await;

        let Some(state) = app.try_state::<AppState>() else {
            return;
        };

        // Re-checked rather than trusted. This task cannot be cancelled by reopening the
        // panel, so it has to establish for itself that the panel is still closed and has
        // been for the whole lifetime — otherwise a close, a reopen and a second close would
        // have the first task expire the second one's screenshot early.
        // Copied out of the guard, not borrowed through it: the lock guard would otherwise
        // live to the end of the expression and outlive the `State` it came from.
        let hidden_at = match state.panel_hidden_at.lock() {
            Ok(hidden) => *hidden,
            Err(_) => return,
        };

        let expired = hidden_at.is_some_and(|hidden| hidden.elapsed() >= CAPTURE_LIFETIME);

        if !expired {
            return;
        }

        // The lock guard is taken and dropped inside the `match` rather than in a trailing
        // `if let`, whose temporaries would outlive the `State` borrow they came from.
        let released = match state.last_capture.lock() {
            Ok(mut last) => last.take().is_some(),
            Err(_) => false,
        };

        if released {
            tracing::debug!("released the remembered screenshot");
        }
    });
}

/// Stops the clock, because the panel is open again.
pub fn cancel_capture_expiry(app: &tauri::AppHandle) {
    use tauri::Manager;

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut hidden) = state.panel_hidden_at.lock() {
            *hidden = None;
        }
    }
}

/// Forgets the thread's backend state.
///
/// Called by **Clear**, and by nothing else. Closing the panel does not reach here.
///
/// That is a reversal of the design doc, which says dismissing the panel discards the thread
/// as a privacy-preserving default. The maintainer's call, and a defensible one: Escape and
/// clicking away are easy to do by accident, and losing a conversation to a mistaken keypress
/// is a cost paid every time it happens, against a privacy benefit that only matters if
/// somebody else is at the machine. Clear is unambiguous — nobody presses it by accident.
///
/// What the reversal costs is that a thread now outlives the panel being closed, so a
/// screenshot Magi took stays in memory until Clear or quit. `docs/TASKS.md` carries the note
/// to correct the design doc.
#[tauri::command]
pub fn clear_session(app: tauri::AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    if let Ok(mut last) = state.last_capture.lock() {
        *last = None;
    }
    crate::session::report(&app, crate::session::Event::Stopped);
    Ok(())
}

/// Runs one tool call and produces the message that answers it.
///
/// Never fails. Every outcome is a `ToolResult` the model can read, because the
/// alternative — aborting the turn — loses an answer the model could still have given
/// from what it already knows. A screenshot it could not get is a fact it can work with;
/// silence is not.
async fn answer_call(
    app: &tauri::AppHandle,
    call: &crate::llm::provider::ToolCall,
    budget: &mut crate::llm::tools::CaptureBudget,
) -> Message {
    use tauri::Manager;

    let refuse = |text: String| Message::ToolResult {
        call_id: call.id.clone(),
        text,
        images: Vec::new(),
    };

    if call.name != crate::llm::tools::CAPTURE_SCREEN {
        // Magi offers exactly one tool, so this is a model inventing a name — which one
        // did, calling `users_screen`. Naming what was asked for beats a generic refusal:
        // it is the difference between the model correcting itself and repeating itself.
        tracing::warn!(name = %call.name, "the model called a tool that does not exist");
        return refuse(format!(
            "There is no tool called `{}`. The only tool available is `{}`.",
            call.name,
            crate::llm::tools::CAPTURE_SCREEN
        ));
    }

    if !budget.has_room() {
        return refuse(budget.exhausted_message());
    }
    budget.spend();

    let reason = crate::llm::tools::Reason::from_tool_arguments(&call.arguments);
    let target = crate::llm::tools::Target::from_tool_arguments(&call.arguments);

    // Bracketed, so the indicator says "looking" for exactly as long as it is true. The
    // `Looked` below runs on every path out of here, including the failures, because a
    // capture that did not work still stopped happening.
    crate::session::report(app, crate::session::Event::Looking);

    // `spawn_blocking`, because a capture is a synchronous round trip through the window
    // server. Holding a runtime worker for it starves everything else that runtime polls.
    let screen = Arc::clone(&app.state::<AppState>().screen);
    let gathered = tauri::async_runtime::spawn_blocking(move || {
        use crate::llm::tools::Target;

        // The window list travels as text alongside the picture, and for some questions it
        // *is* the answer. Asked what applications were open, a model was seen reading
        // blurry pixels to recover names that were available exactly as strings — no
        // resolution beats sending "Activity Monitor" as text. Cheap, precise, and it
        // complements the image rather than competing with it.
        let windows = screen.windows().unwrap_or_default();

        let captures = match target {
            Target::FocusedWindow => screen.capture_focused_window().map(|one| vec![one]),
            Target::ActiveScreen => screen.capture_active_display().map(|one| vec![one]),
            Target::AllScreens => screen.capture_all_displays(),
        };

        captures.map(|captures| (captures, windows))
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result.map_err(|error| error.to_string()));

    crate::session::report(app, crate::session::Event::Looked);

    let (captures, windows) = match gathered {
        Ok(gathered) => gathered,
        Err(message) => {
            tracing::warn!(%message, "the capture the model asked for failed");
            // The model is told, in words it can act on. A permission problem is the
            // likely one and it can say so to the user, which is more useful than Magi
            // failing the turn behind its back.
            return refuse(format!("The screenshot could not be taken: {message}"));
        }
    };

    let state = app.state::<AppState>();
    for capture in &captures {
        state.capture_log.record(crate::capture::Entry {
            at: unix_millis(),
            subject: capture.subject.clone(),
            reason: reason.clone(),
            width: capture.width,
            height: capture.height,
            visual_tokens: capture.visual_tokens(),
        });
    }

    // The panel shows that the screen was read. Emitted after the log entries exist, so
    // opening Settings the moment the indicator appears never shows an empty list.
    let announcement = captures
        .iter()
        .map(|capture| capture.subject.describe())
        .collect::<Vec<_>>()
        .join(", ");
    if let Err(error) = app.emit("magi://captured", announcement.clone()) {
        tracing::warn!(%error, "could not announce the capture");
    }

    // Named windows, frontmost first, so "what have I got open" is answered from strings
    // rather than from pixels. Capped because a busy desktop has dozens and the tail is
    // background noise nobody asked about.
    let listed = windows
        .iter()
        .take(20)
        .map(|window| {
            if window.title.is_empty() {
                window.app.clone()
            } else {
                format!("{} — {}", window.app, window.title)
            }
        })
        .collect::<Vec<_>>();

    let mut text = format!("Screenshot of {announcement}.");
    if !listed.is_empty() {
        text.push_str("\n\nOpen windows, frontmost first:\n");
        text.push_str(&listed.join("\n"));
    }

    remember(app, captures.last());

    Message::ToolResult {
        call_id: call.id.clone(),
        // Short and factual about the image. The model has it; describing it here would
        // compete with what it can see for itself.
        text,
        images: captures
            .into_iter()
            .map(|capture| crate::llm::provider::Image {
                media_type: "image/png",
                bytes: capture.png,
            })
            .collect(),
    }
}

/// Rebuilds the transcriber from the current config.
///
/// Called after anything that changes which model or language is in use. In one place so
/// the two setters cannot drift, and so the reason lives once: the old transcriber holds a
/// loaded model, and dropping it is what frees those hundreds of megabytes.
fn rebuild_transcriber(state: &State<'_, AppState>) -> CommandResult<()> {
    let (model, languages) = {
        let config = state.config.lock().map_err(to_message)?;
        (config.voice.model, config.voice.languages.clone())
    };

    let replacement: Arc<dyn crate::stt::Transcriber> = Arc::new(
        crate::stt::WhisperTranscriber::new(model, &state.models_dir, languages),
    );

    *state.transcriber.lock().map_err(to_message)? = replacement;
    tracing::info!(?model, "transcriber rebuilt");
    Ok(())
}

/// Everything the Voice pane needs.
///
/// One command rather than several, because the pane's three facts — which models
/// exist, which is chosen, and whether the microphone is available — are read together
/// every time and a partial answer would render a half-built screen.
#[tauri::command]
pub fn get_voice(state: State<'_, AppState>) -> CommandResult<VoiceView> {
    let selected = {
        let config = state.config.lock().map_err(to_message)?;
        config.voice.model
    };

    let downloading = state.downloading.lock().map_err(to_message)?.to_owned();

    let models = Model::ALL
        .iter()
        .map(|&id| SpeechModelView {
            id,
            label: id.label().to_string(),
            description: id.description().to_string(),
            approximate_mb: id.approximate_bytes() / 1_000_000,
            downloaded: id.path_in(&state.models_dir).exists(),
            selected: id == selected,
            multilingual: id.is_multilingual(),
        })
        .collect::<Vec<_>>();

    let microphone = permissions::microphone();

    let languages = {
        let config = state.config.lock().map_err(to_message)?;
        config.voice.languages.clone()
    };

    // A non-English selection on an English-only model. Not an error to refuse, because the
    // user may be mid-way through changing both — but it must be visible.
    let language_ignored = !selected.is_multilingual() && languages.iter().any(|code| code != "en");

    Ok(VoiceView {
        languages,
        language_ignored,
        ready: selected.path_in(&state.models_dir).exists(),
        microphone,
        microphone_explanation: microphone.explanation("Voice input"),
        microphone_settings_url: permissions::settings_url(PermissionKind::Microphone).to_string(),
        downloading,
        models,
    })
}

/// Chooses which model transcribes.
///
/// Allowed for a model that has not been downloaded yet. The alternative — refusing
/// until the file exists — would mean picking `small.en` and then separately asking for
/// it, when picking it *is* the request.
#[tauri::command]
pub fn set_speech_model(state: State<'_, AppState>, model: Model) -> CommandResult<VoiceView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        let previous = std::mem::replace(&mut config.voice.model, model);
        if let Err(error) = config.save(&state.config_dir) {
            config.voice.model = previous;
            return Err(to_message(error));
        }
    }

    rebuild_transcriber(&state)?;
    get_voice(state)
}

/// Sets which languages Magi should expect.
///
/// Empty detects from all ninety-nine, one pins it, several restrict detection to those.
/// The third is the useful case: unrestricted detection on a short utterance misreads it in
/// ways a person would not, and naming the two or three you actually speak removes the rest
/// without committing to either.
#[tauri::command]
pub fn set_voice_languages(
    state: State<'_, AppState>,
    languages: Vec<String>,
) -> CommandResult<VoiceView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        let previous = std::mem::replace(&mut config.voice.languages, languages);
        if let Err(error) = config.save(&state.config_dir) {
            config.voice.languages = previous;
            return Err(to_message(error));
        }
    }

    rebuild_transcriber(&state)?;
    get_voice(state)
}

/// Downloads a speech model, reporting progress on `magi://model-download`.
///
/// Runs on a blocking task: it is a long transfer that streams to disk, and the
/// download itself is synchronous. Returning as soon as it is under way rather than
/// awaiting it would leave the UI with nothing to show, so this one does await — the
/// progress events are what keep the panel informed meanwhile.
#[tauri::command]
pub async fn download_speech_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    model: Model,
) -> CommandResult<VoiceView> {
    // One download at a time. Two writers appending to one partial file produce a
    // corrupt result of roughly the right length, and only the checksum would notice.
    {
        let mut slot = state.downloading.lock().map_err(to_message)?;
        if let Some(running) = *slot {
            return Err(format!(
                "{} is already downloading. Wait for it to finish.",
                running.label()
            ));
        }
        *slot = Some(model);
    }

    let dir = state.models_dir.clone();
    let http = state.http_blocking.clone();
    let emitter = app.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        // Emitted only when the whole-number percentage changes. The read loop hands over
        // a progress value every 256 kB, which for a 488 MB model is nearly two thousand
        // events — and a bar cannot show more than a hundred distinct states, so the rest
        // is IPC traffic and reactive updates with no visible effect.
        let mut last_percent = u8::MAX;

        model::download(&http, model, &dir, |progress: Progress| {
            let percent = progress.percent();
            if percent != last_percent {
                last_percent = percent;
                // A dropped listener is not a reason to stop: the user may simply have
                // closed Settings, and abandoning a 488 MB download because nobody is
                // watching the bar would be its own bug.
                let _ = emitter.emit("magi://model-download", progress);
            }
            true
        })
    })
    .await;

    // Cleared before the result is inspected, so a failure cannot leave the guard set
    // and block every later attempt.
    if let Ok(mut slot) = state.downloading.lock() {
        *slot = None;
    }

    let _ = app.emit("magi://model-download-done", ());

    match result.map_err(to_message)? {
        Ok(path) => {
            // File name only — see the note in `lib.rs`. It also happens to be the more
            // useful half: which model is ready, not where the home directory is.
            tracing::info!(
                file = %path.file_name().unwrap_or_default().to_string_lossy(),
                "speech model ready"
            );
            get_voice(state)
        }
        Err(DownloadError::Cancelled) => get_voice(state),
        Err(error) => Err(to_message(error)),
    }
}

/// Deletes a downloaded model.
///
/// Worth having: `medium.en` is 1.4 GB, and an app that can put that on your disk and
/// not take it off again is an app that quietly costs you space forever.
#[tauri::command]
pub fn remove_speech_model(state: State<'_, AppState>, model: Model) -> CommandResult<VoiceView> {
    let path = model.path_in(&state.models_dir);
    if path.exists() {
        std::fs::remove_file(&path).map_err(to_message)?;
        tracing::info!(path = %path.display(), "speech model deleted");
    }
    // The partial goes too, or a resumed download would continue onto a file the user
    // asked to be rid of.
    let _ = std::fs::remove_file(model.partial_path_in(&state.models_dir));

    get_voice(state)
}

/// What Magi can read, and what it has read.
///
/// Synchronous and cheap: the permission query is a CoreGraphics call with no round trip and
/// the log is a clone of an in-memory queue. Nothing here touches the keychain, which is the
/// thing that must never run on the main thread.
#[tauri::command]
pub fn get_capture(state: tauri::State<'_, AppState>) -> CommandResult<CaptureView> {
    let screen_recording = permissions::screen_recording();

    Ok(CaptureView {
        screen_recording,
        // A screen-specific explanation rather than `Permission::explanation`. The generic
        // text says "turn it on in System Settings", which presumes Magi is listed there —
        // and it is not until something has requested the permission. See
        // `permissions::screen_reading_explanation`.
        screen_recording_explanation: permissions::screen_reading_explanation(screen_recording),
        screen_recording_settings_url: permissions::settings_url(PermissionKind::ScreenRecording)
            .to_string(),
        entries: state.capture_log.entries(),
    })
}

/// Asks macOS for screen-recording access.
///
/// `async` and on `spawn_blocking` rather than a plain synchronous command, for the reason
/// written into this project's hard rules: a synchronous `#[tauri::command] fn` runs on the
/// main thread, and anything that may put system UI in front of the user must not. The
/// keychain taught that lesson expensively — a call that reads like a cheap getter turned
/// out to wait on a dialog only the main thread could have drawn, and the app deadlocked
/// with no error anywhere. `CGRequestScreenCaptureAccess` is not documented to block, which
/// is not the same as documented not to.
///
/// Called only from an explicit button. It opens System Settings as a side effect, which is
/// welcome when someone asked for it and startling otherwise.
#[tauri::command]
pub async fn request_screen_recording(app: tauri::AppHandle) -> CommandResult<CaptureView> {
    // The result is deliberately discarded. It reports the state as macOS sees it *now*,
    // which is "denied" even on success — the user has yet to flick the switch. What matters
    // is the side effect: Magi now exists in that list.
    let _ = tauri::async_runtime::spawn_blocking(permissions::request_screen_recording).await;

    use tauri::Manager;
    get_capture(app.state::<AppState>())
}

/// Takes one screenshot, on purpose, so the user can see whether screen reading works.
///
/// The same idea as the pre-flight probes: find out before relying on it. Screen recording
/// fails in ways that produce no error — a permission granted to an app that is already
/// running does not take effect, and the API this replaced returned a picture of an empty
/// desktop rather than complaining — so a button that says "it worked, and here is the size
/// and what it would have cost" is worth more than a status badge.
///
/// The image itself is discarded. What is kept is the log entry, which is the evidence.
/// Showing the screenshot back would mean holding a megabyte of someone's screen in a
/// settings window for as long as it stayed open, to tell them something the dimensions
/// already tell them.
#[tauri::command]
pub async fn test_capture(app: tauri::AppHandle) -> CommandResult<CaptureView> {
    use tauri::Manager;

    // `spawn_blocking`, not merely `async`. A capture is a synchronous round trip through
    // the window server, and an async command runs on a runtime worker — holding one of
    // those for the duration would starve everything else that runtime polls, which is the
    // same reason transcription goes to the blocking pool.
    let screen = Arc::clone(&app.state::<AppState>().screen);
    let capture = tauri::async_runtime::spawn_blocking(move || screen.capture_active_display())
        .await
        .map_err(to_message)?
        .map_err(to_message)?;

    let state = app.state::<AppState>();
    state.capture_log.record(crate::capture::Entry {
        at: unix_millis(),
        subject: capture.subject.clone(),
        reason: crate::llm::tools::Reason::UserAsked,
        width: capture.width,
        height: capture.height,
        visual_tokens: capture.visual_tokens(),
    });

    get_capture(state)
}

/// Now, in milliseconds since the Unix epoch.
///
/// Saturates at zero rather than propagating: a clock set before 1970 is not a reason to
/// refuse a screenshot, and an entry timestamped zero is visibly wrong in a way a missing
/// entry is not.
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

/// Forgets every recorded capture.
///
/// Offered because someone who has just shown Magi something private should be able to
/// remove the record of it without quitting the app.
#[tauri::command]
pub fn clear_capture_log(state: tauri::State<'_, AppState>) -> CommandResult<CaptureView> {
    state.capture_log.clear();
    get_capture(state)
}

/// The one-click questions the active model can actually answer.
///
/// Filtered here rather than in the panel, because the panel does not know the model's tier and
/// should not have to. Offering "summarise my screen" to a model that cannot see is the same
/// mistake as telling one it can look: a promise the code cannot keep, and the user learns to
/// distrust the buttons rather than the model.
#[tauri::command]
pub fn prompt_templates(
    app: tauri::AppHandle,
) -> CommandResult<Vec<crate::llm::templates::Template>> {
    Ok(crate::llm::templates::for_tier(
        crate::session::active_tier_of(&app),
    ))
}

/// Where Magi writes its log, and whether that directory exists yet.
///
/// The path is shown rather than only opened, because the two are for different moments:
/// a button helps someone sitting in front of Settings, and a path helps someone reading
/// an instruction in a GitHub issue with the app already closed.
#[derive(Debug, Serialize)]
pub struct LogView {
    /// The directory, for display. `None` only if there is no home directory to put it in.
    pub directory: Option<String>,
    /// Whether anything has been written there yet.
    pub exists: bool,
}

#[tauri::command]
pub fn get_logs() -> LogView {
    let directory = crate::logging::directory();

    LogView {
        exists: directory.as_ref().is_some_and(|d| d.is_dir()),
        directory: directory.map(|d| d.display().to_string()),
    }
}

/// Reveals the log folder in Finder.
///
/// Through the OS, the same way [`open_permission_settings`] reaches System Settings, and
/// for the same reason: telling someone a path and expecting them to navigate Library —
/// which Finder hides by default — is most of the difficulty of finding a log.
#[tauri::command]
pub fn open_log_folder() -> CommandResult<()> {
    let directory =
        crate::logging::directory().ok_or_else(|| "no home directory to log into".to_string())?;

    // Created on demand rather than reported as missing. Magi makes this at startup, so an
    // absence means logging failed to start — and opening an empty folder is a clearer
    // answer than an error about a folder, since it shows there is nothing to send.
    if let Err(error) = std::fs::create_dir_all(&directory) {
        return Err(format!("could not open the log folder: {error}"));
    }

    std::process::Command::new("open")
        .arg(&directory)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the log folder: {e}"))
}

/// Opens the System Settings pane for a permission.
///
/// Through the OS rather than by instructing the user to navigate there. Sending
/// someone to the top of Privacy & Security and expecting them to find Microphone is
/// most of the difficulty of granting a permission.
#[tauri::command]
pub fn open_permission_settings(kind: PermissionKind) -> CommandResult<()> {
    let url = permissions::settings_url(kind);
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open System Settings: {e}"))
}
