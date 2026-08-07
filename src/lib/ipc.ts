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
