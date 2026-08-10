// Typed wrappers over Tauri's invoke.
//
// Components call these rather than `invoke` directly, so command names and
// argument shapes live in one place. A renamed command then breaks the build
// here instead of failing at runtime in whichever component happened to call it.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ProviderKind = "openai-compatible" | "anthropic";

/**
 * How much of Magi works with a model.
 *
 * Assigned by pre-flight, never chosen. `unreachable` is separate from
 * `text-only` on purpose: a text-only model works, an unreachable one does not,
 * and the fixes are unrelated.
 */
export type Tier = "agentic" | "heuristic" | "text-only" | "unreachable";

/** One model's probe results, as the capability matrix renders them. */
export type ModelCapability = {
  model: string;
  tier: Tier;
  /** Short name for the tier, from the backend so the wording lives in one place. */
  label: string;
  /** Why the tier is what it is, in terms the user can act on. */
  explanation: string;
  reachable: boolean;
  vision: boolean;
  tools: boolean;
  structured_output: boolean;
};

export type ProviderView = {
  id: string;
  kind: ProviderKind;
  base_url: string;
  models: string[];
  requires_key: boolean;
  /** Whether a key is stored. The key itself never reaches the frontend. */
  has_key: boolean;
  /**
   * Enough of the stored key to tell two apart, never enough to use one.
   * Computed in the backend — masking in the UI would mean the real key had
   * already crossed into the webview.
   */
  key_hint: string | null;

  /**
   * Results for the models that have been tested. Models absent from this list
   * have not been probed, which must render as unknown rather than as incapable
   * — a model nobody tested is not a model that failed.
   */
  capabilities: ModelCapability[];
};

export type ActiveModel = {
  provider: string;
  model: string;
};

export type Theme = "system" | "light" | "dark";

export type AppearanceConfig = {
  theme: Theme;
  show_thinking: boolean;
};

export type PromptConfig = {
  /** Appended to Magi's own system prompt. It never replaces it. */
  context: string;
};

/** Mirrors `MAX_PROMPT_CONTEXT` in `config/mod.rs`, which is the real limit. */
export const MAX_PROMPT_CONTEXT = 4000;

export type ConfigView = {
  providers: ProviderView[];
  active: ActiveModel | null;
  hotkey: string;
  prompt: PromptConfig;
  appearance: AppearanceConfig;
  config_path: string;
};

export type ProviderInput = {
  id: string;
  kind: ProviderKind;
  base_url: string;
  models: string[];
  requires_key: boolean;
};

export const getConfig = async (): Promise<ConfigView> => invoke<ConfigView>("get_config");

/**
 * The appearance settings alone, reading no secrets.
 *
 * The panel uses this instead of `getConfig`. `getConfig` reports a fingerprint
 * for every stored key, so it reads the keychain — and the panel is created
 * hidden at launch, which made starting Magi trigger a keychain prompt before any
 * window existed to attach it to. Asking for the whole configuration to read one
 * boolean pulled the secret store into the startup path for nothing.
 */
export const getAppearance = async (): Promise<AppearanceConfig> =>
  invoke<AppearanceConfig>("get_appearance");

/**
 * Adds a provider, or replaces the one with the same id.
 *
 * `apiKey` of `undefined` leaves any stored key untouched, so editing a
 * provider's URL does not silently drop its credential. An empty string clears
 * it deliberately.
 */
export const saveProvider = async (
  provider: ProviderInput,
  apiKey?: string,
): Promise<ConfigView> =>
  invoke<ConfigView>("save_provider", { request: { provider, api_key: apiKey ?? null } });

/**
 * Asks the endpoint which models it serves.
 *
 * Takes the provider from the form rather than a saved id, so models can be
 * discovered for an endpoint that has been typed but not yet saved — which is
 * the order people actually work in.
 */
export const discoverModels = async (
  provider: ProviderInput,
  apiKey?: string,
): Promise<string[]> =>
  invoke<string[]>("discover_models", { provider, apiKey: apiKey ?? null });

/**
 * Probes one model and records what it can do.
 *
 * Four requests against the endpoint, so it is slow and — on a metered provider —
 * costs a little. Results are cached, and cleared automatically whenever the
 * provider is saved, since capabilities belong to the endpoint as much as to the
 * model.
 */
export const runPreflight = async (providerId: string, model: string): Promise<ConfigView> =>
  invoke<ConfigView>("run_preflight", { providerId, model });

export const removeProvider = async (id: string): Promise<ConfigView> =>
  invoke<ConfigView>("remove_provider", { id });

/**
 * Sets the theme and remembers it.
 *
 * Applied by Rust to the webviews rather than by CSS alone: the native controls
 * the webview draws — inputs, scrollbars, select popups — follow the window's
 * theme, so a stylesheet-only override would be half applied.
 */
export const setTheme = async (theme: Theme): Promise<ConfigView> =>
  invoke<ConfigView>("set_theme", { theme });

/** Whether the panel shows the model's reasoning. */
export const setShowThinking = async (show: boolean): Promise<ConfigView> =>
  invoke<ConfigView>("set_show_thinking", { show });

export const setActiveModel = async (provider: string, model: string): Promise<ConfigView> =>
  invoke<ConfigView>("set_active_model", { provider, model });

/** Replaces the standing context sent with every turn. */
export const setPromptContext = async (context: string): Promise<ConfigView> =>
  invoke<ConfigView>("set_prompt_context", { context });

/**
 * Rebinds the global shortcut.
 *
 * Rejects rather than saves when the OS refuses the combination — most often
 * because another application already owns it. The previous shortcut keeps
 * working in that case, so a failed attempt costs nothing.
 */
export const setHotkey = async (shortcut: string): Promise<ConfigView> =>
  invoke<ConfigView>("set_hotkey", { shortcut });

/** Which speech model transcribes. Mirrors `stt::Model`. */
export type SpeechModel = "base-en" | "small-en" | "medium-en";

/**
 * Whether macOS has granted a permission.
 *
 * Four states, not two. `not-asked` is the intended first-run path rather than a
 * failure, and `restricted` cannot be fixed in System Settings at all — so the UI
 * must not offer the same advice for both.
 */
export type Permission = "not-asked" | "granted" | "denied" | "restricted" | "not-applicable";

export type SpeechModelView = {
  id: SpeechModel;
  label: string;
  description: string;
  /** Rounded for display; the real length comes from the server at download time. */
  approximate_mb: number;
  downloaded: boolean;
  selected: boolean;
  /** Whether it understands languages other than English. */
  multilingual: boolean;
};

export type VoiceView = {
  models: SpeechModelView[];
  /**
   * The languages Magi expects, in no particular order.
   *
   * Its length is the setting: empty detects from all ninety-nine, one pins it, several
   * restrict detection to those. The third is the useful case — unrestricted detection on
   * a short utterance misreads it in ways a person would not.
   */
  languages: string[];
  /**
   * Set when the chosen model cannot honour the chosen language.
   *
   * An English-only model given anything else does not fail — it writes English words
   * that sound similar — so the combination has to be visible rather than left to
   * produce a confident wrong transcript.
   */
  language_ignored: boolean;
  /** Whether the selected model is on disk and usable. */
  ready: boolean;
  microphone: Permission;
  microphone_explanation: string;
  microphone_settings_url: string;
  /** The model currently downloading, if any. */
  downloading: SpeechModel | null;
};

/** How far along a model download is. */
export type DownloadProgress = { downloaded: number; total: number };

export const getVoice = async (): Promise<VoiceView> => invoke<VoiceView>("get_voice");

/**
 * Chooses which model transcribes.
 *
 * Allowed for a model that has not been downloaded. Refusing until the file exists
 * would mean picking Small and then separately asking for it, when picking it *is* the
 * request.
 */
export const setSpeechModel = async (model: SpeechModel): Promise<VoiceView> =>
  invoke<VoiceView>("set_speech_model", { model });

/**
 * Downloads a model. Resolves when it is on disk and verified.
 *
 * Progress arrives on `magi://model-download` meanwhile — see `onDownloadProgress`.
 */
export const downloadSpeechModel = async (model: SpeechModel): Promise<VoiceView> =>
  invoke<VoiceView>("download_speech_model", { model });

/**
 * Sets which languages Magi should expect.
 *
 * Empty detects from all of them, one pins it, several restrict detection to those.
 */
export const setVoiceLanguages = async (languages: string[]): Promise<VoiceView> =>
  invoke<VoiceView>("set_voice_languages", { languages });

/**
 * The languages offered in Settings.
 *
 * A shortlist, not all ninety-nine whisper.cpp supports: a dropdown of ninety-nine is
 * worse than one of twelve plus a config file for the rest. Hand-editing `[voice] language`
 * accepts any code.
 */
export const LANGUAGES: ReadonlyArray<{ code: string; label: string }> = [
  { code: "en", label: "English" },
  { code: "es", label: "Español" },
  { code: "pt", label: "Português" },
  { code: "fr", label: "Français" },
  { code: "de", label: "Deutsch" },
  { code: "it", label: "Italiano" },
  { code: "nl", label: "Nederlands" },
  { code: "ja", label: "日本語" },
  { code: "zh", label: "中文" },
  { code: "ko", label: "한국어" },
  { code: "ru", label: "Русский" },
];

export const removeSpeechModel = async (model: SpeechModel): Promise<VoiceView> =>
  invoke<VoiceView>("remove_speech_model", { model });

/** Opens the System Settings pane for a permission, rather than describing where it is. */
export const openPermissionSettings = async (kind: "microphone" | "accessibility" | "screen-recording"): Promise<void> =>
  invoke("open_permission_settings", { kind });

/**
 * The captured subject — either a full display or a specific application window.
 *
 * macOS withholds other apps' window titles when screen-recording permission is
 * absent, so `title` can be empty even when the window exists.
 */
export type CaptureSubject =
  | { kind: "display"; id: number; label: string }
  | { kind: "window"; id: number; title: string; app: string };

/** Why a capture happened, and who or what triggered it. */
export type CaptureReason = {
  /** The full rendered sentence — render verbatim, do not rebuild it. */
  text: string;
  asked_by: "model" | "you";
};

/** One screen-capture event, as the backend recorded it. */
export type CaptureEntry = {
  /** Milliseconds since the Unix epoch. */
  at: number;
  subject: CaptureSubject;
  reason: CaptureReason;
  width: number;
  height: number;
  visual_tokens: number;
};

export type CaptureView = {
  screen_recording: Permission;
  screen_recording_explanation: string;
  screen_recording_settings_url: string;
  /** Most recent first, ordered by the backend. */
  entries: CaptureEntry[];
};

export const getCapture = async (): Promise<CaptureView> =>
  invoke<CaptureView>("get_capture");

/**
 * Clears the in-memory capture log and returns the updated view.
 *
 * The log is never written to disk — this clears only what has accumulated
 * since launch.
 */
/**
 * Asks macOS for screen-recording permission.
 *
 * The only way Magi appears in System Settings › Privacy & Security › Screen Recording at
 * all: that list is built from apps that have *requested* the permission, and reading the
 * state registers nothing. Opens System Settings as a side effect, so only ever call it
 * from an explicit action.
 */
export const requestScreenRecording = async (): Promise<CaptureView> =>
  invoke<CaptureView>("request_screen_recording");

/**
 * Takes one screenshot deliberately, so the user can confirm screen reading works.
 *
 * The image is not returned. What comes back is the refreshed view, whose newest log entry
 * is the evidence — dimensions and token cost included. Screen recording fails in ways that
 * produce no error, so trying it is the only way to know.
 */
/**
 * Fires when a model has read the screen during a turn.
 *
 * Carries what was captured, so the panel can say *what* was looked at rather than only
 * that something was. The details — size, cost, why — go to Settings › Screen.
 */
export const onCaptured = async (handler: (subject: string) => void) =>
  listen<string>("magi://captured", (event) => handler(event.payload));

export const testCapture = async (): Promise<CaptureView> =>
  invoke<CaptureView>("test_capture");

export const clearCaptureLog = async (): Promise<CaptureView> =>
  invoke<CaptureView>("clear_capture_log");

/**
 * Subscribes to model-download progress.
 *
 * Bundled with the completion event so a caller cannot unsubscribe from one and leak
 * the other.
 */
export const onDownloadProgress = async (handlers: {
  progress: (progress: DownloadProgress) => void;
  done: () => void;
}): Promise<UnlistenFn> => {
  const unlisten = await Promise.all([
    listen<DownloadProgress>("magi://model-download", (e) => handlers.progress(e.payload)),
    listen("magi://model-download-done", () => handlers.done()),
  ]);
  return () => unlisten.forEach((off) => off());
};

/** Endpoints common enough to be worth not typing out. */
export const PRESETS: ReadonlyArray<{
  label: string;
  id: string;
  kind: ProviderKind;
  base_url: string;
  requires_key: boolean;
  models: string[];
}> = [
  {
    label: "Ollama (local)",
    id: "ollama",
    kind: "openai-compatible",
    base_url: "http://localhost:11434/v1",
    requires_key: false,
    models: [],
  },
  {
    label: "LM Studio (local)",
    id: "lmstudio",
    kind: "openai-compatible",
    base_url: "http://localhost:1234/v1",
    requires_key: false,
    models: [],
  },
  {
    label: "OpenAI",
    id: "openai",
    kind: "openai-compatible",
    base_url: "https://api.openai.com/v1",
    requires_key: true,
    models: [],
  },
  {
    label: "OpenRouter",
    id: "openrouter",
    kind: "openai-compatible",
    base_url: "https://openrouter.ai/api/v1",
    requires_key: true,
    models: [],
  },
  {
    // Xiaomi serves both protocols on the same host at different paths, which
    // is the clearest evidence that "OpenAI-compatible" names a protocol rather
    // than a vendor. The Anthropic-shaped one lives at /anthropic; register it
    // as a second provider with the Anthropic protocol if you want it.
    label: "Xiaomi MiMo",
    id: "mimo",
    kind: "openai-compatible",
    base_url: "https://api.xiaomimimo.com/v1",
    requires_key: true,
    models: [],
  },
  {
    label: "Anthropic",
    id: "anthropic",
    kind: "anthropic",
    base_url: "https://api.anthropic.com/v1",
    requires_key: true,
    models: [],
  },
];

/**
 * What the panel is told while a voice turn happens.
 *
 * Distinct states rather than a boolean: recording ends when you let go, and
 * transcription ends when it ends, and those feel different to wait through.
 */
export type VoiceState = "idle" | "recording" | "transcribing";

/**
 * Subscribes to push-to-talk events.
 *
 * Bundled so a caller cannot unsubscribe from the state and leak the transcript.
 *
 * `transcript` delivers the words to put in the input — not to send. Voice fills the
 * box; asking is still the user's move, and a mis-transcription sent straight to a model
 * is a wrong question asked confidently.
 */
export const onVoiceEvents = async (handlers: {
  state: (state: VoiceState) => void;
  transcript: (text: string) => void;
  notice: (message: string) => void;
  error: (message: string) => void;
}): Promise<UnlistenFn> => {
  const unlisten = await Promise.all([
    listen<VoiceState>("magi://voice", (e) => handlers.state(e.payload)),
    listen<string>("magi://transcript", (e) => handlers.transcript(e.payload)),
    listen<string>("magi://voice-notice", (e) => handlers.notice(e.payload)),
    listen<string>("magi://voice-error", (e) => handlers.error(e.payload)),
  ]);
  return () => unlisten.forEach((off) => off());
};

/** One history entry, as the backend expects it. */
export type TurnMessage = { role: string; content: string };

/**
 * Starts a turn. Returns as soon as the request is under way — the reply arrives
 * through the listeners below, so the panel is never blocked waiting for a model.
 */
export const sendTextTurn = async (text: string, history: TurnMessage[]): Promise<void> =>
  invoke("send_text_turn", { text, history });

export const cancelTurn = async (): Promise<void> => invoke("cancel_turn");

/**
 * Subscribes to a turn's events and returns a single teardown function.
 *
 * Bundled rather than exposed one at a time so a component cannot unsubscribe
 * from two of the three and leak the other.
 */
export const onTurnEvents = async (handlers: {
  token: (token: string) => void;
  thinking: (thought: string) => void;
  done: (notice: string | null) => void;
  error: (message: string) => void;
}): Promise<UnlistenFn> => {
  const unlisten = await Promise.all([
    listen<string>("magi://token", (e) => handlers.token(e.payload)),
    listen<string>("magi://thinking", (e) => handlers.thinking(e.payload)),
    listen<string | null>("magi://turn-done", (e) => handlers.done(e.payload)),
    listen<string>("magi://error", (e) => handlers.error(e.payload)),
  ]);
  return () => unlisten.forEach((off) => off());
};
