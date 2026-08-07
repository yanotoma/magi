// Typed wrappers over Tauri's invoke.
//
// Components call these rather than `invoke` directly, so command names and
// argument shapes live in one place. A renamed command then breaks the build
// here instead of failing at runtime in whichever component happened to call it.

import { invoke } from "@tauri-apps/api/core";

export type ProviderKind = "openai-compatible" | "anthropic";

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
};

export type ActiveModel = {
  provider: string;
  model: string;
};

export type Theme = "system" | "light" | "dark";

export type AppearanceConfig = {
  theme: Theme;
};

export type ConfigView = {
  providers: ProviderView[];
  active: ActiveModel | null;
  hotkey: string;
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

export const setActiveModel = async (provider: string, model: string): Promise<ConfigView> =>
  invoke<ConfigView>("set_active_model", { provider, model });

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
