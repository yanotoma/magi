<script lang="ts">
  import {
    getConfig,
    saveProvider,
    removeProvider,
    setActiveModel,
    setTheme,
    discoverModels,
    PRESETS,
    type ConfigView,
    type ProviderKind,
    type ProviderView,
    type Theme,
  } from "$lib/ipc";

  type Pane = "general" | "models" | "hotkeys";

  const PANES: ReadonlyArray<{ id: Pane; label: string }> = [
    { id: "general", label: "General" },
    { id: "models", label: "Models" },
    { id: "hotkeys", label: "Hotkeys" },
  ];

  const THEMES: ReadonlyArray<{ id: Theme; label: string }> = [
    { id: "system", label: "System" },
    { id: "light", label: "Light" },
    { id: "dark", label: "Dark" },
  ];

  let pane = $state<Pane>("models");
  let config = $state<ConfigView | null>(null);
  let error = $state<string | null>(null);
  let editing = $state<string | null>(null);
  let discovering = $state(false);
  let discovered = $state<string | null>(null);

  // `apiKey` starts undefined and stays that way unless the user types, so
  // editing an endpoint never silently drops a stored credential.
  let form = $state({
    id: "",
    kind: "openai-compatible" as ProviderKind,
    base_url: "",
    models: "",
    requires_key: false,
    apiKey: undefined as string | undefined,
  });

  const run = async (action: () => Promise<ConfigView>) => {
    error = null;
    try {
      config = await action();
    } catch (e) {
      error = String(e);
    }
  };

  $effect(() => {
    run(getConfig);
  });

  const resetForm = () => {
    editing = null;
    discovered = null;
    form = {
      id: "",
      kind: "openai-compatible",
      base_url: "",
      models: "",
      requires_key: false,
      apiKey: undefined,
    };
  };

  const applyPreset = (label: string) => {
    const preset = PRESETS.find((p) => p.label === label);
    if (!preset) return;
    form.id = preset.id;
    form.kind = preset.kind;
    form.base_url = preset.base_url;
    form.requires_key = preset.requires_key;
  };

  const edit = (provider: ProviderView) => {
    editing = provider.id;
    discovered = null;
    form = {
      id: provider.id,
      kind: provider.kind,
      base_url: provider.base_url,
      models: provider.models.join("\n"),
      requires_key: provider.requires_key,
      apiKey: undefined,
    };
  };

  const formAsProvider = () => ({
    id: form.id.trim(),
    kind: form.kind,
    base_url: form.base_url.trim(),
    models: form.models
      .split("\n")
      .map((m) => m.trim())
      .filter(Boolean),
    requires_key: form.requires_key,
  });

  const submit = async (event: Event) => {
    event.preventDefault();
    await run(() => saveProvider(formAsProvider(), form.apiKey));
    if (!error) resetForm();
  };

  const discover = async () => {
    error = null;
    discovered = null;
    discovering = true;
    try {
      const models = await discoverModels({ ...formAsProvider(), models: [] }, form.apiKey);
      if (models.length === 0) {
        // An empty list is a valid answer, not a failure — a fresh Ollama with
        // nothing pulled replies exactly this. Saying so beats an empty box.
        discovered = "The endpoint answered, but serves no models yet.";
      } else {
        form.models = models.join("\n");
        discovered = `Found ${models.length} model${models.length === 1 ? "" : "s"}.`;
      }
    } catch (e) {
      error = String(e);
    } finally {
      discovering = false;
    }
  };

  const editingProvider = $derived(
    editing ? (config?.providers.find((p) => p.id === editing) ?? null) : null,
  );

  const isActive = (providerId: string, model: string) =>
    config?.active?.provider === providerId && config?.active?.model === model;
</script>

<div class="settings">
  <nav>
    <h1>Magi</h1>
    {#each PANES as item (item.id)}
      <button
        type="button"
        class="nav-item"
        class:current={pane === item.id}
        onclick={() => (pane = item.id)}
      >
        {item.label}
      </button>
    {/each}
  </nav>

  <main>
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    {#if pane === "general"}
      <h2>Appearance</h2>
      <div class="segmented" role="group" aria-label="Theme">
        {#each THEMES as option (option.id)}
          <button
            type="button"
            class:selected={config?.appearance.theme === option.id}
            onclick={() => run(() => setTheme(option.id))}
          >
            {option.label}
          </button>
        {/each}
      </div>
      <p class="hint">System follows macOS and changes with it.</p>

      {#if config}
        <h2 class="spaced">Configuration file</h2>
        <code class="path">{config.config_path}</code>
        <p class="hint">
          Safe to share: API keys are in the macOS keychain, never in this file.
        </p>
      {/if}
    {/if}

    {#if pane === "hotkeys"}
      <h2>Global shortcut</h2>
      <p>Toggle the panel: <kbd>{config?.hotkey ?? "Alt+Space"}</kbd></p>
      <p class="hint">Editing the shortcut is not wired up yet.</p>
    {/if}

    {#if pane === "models"}
      <h2>Providers</h2>
      {#if !config || config.providers.length === 0}
        <p class="empty">No providers yet. Ollama needs no key and no account.</p>
      {:else}
        <ul class="providers">
          {#each config.providers as provider (provider.id)}
            <li>
              <div class="row">
                <div>
                  <strong>{provider.id}</strong>
                  <span class="meta">{provider.base_url}</span>
                  {#if provider.requires_key}
                    <span class="badge" class:ok={provider.has_key}>
                      {provider.key_hint ?? "no key"}
                    </span>
                  {/if}
                </div>
                <div class="actions">
                  <button type="button" onclick={() => edit(provider)}>Edit</button>
                  <button
                    type="button"
                    class="danger"
                    onclick={() => run(() => removeProvider(provider.id))}
                  >
                    Remove
                  </button>
                </div>
              </div>

              {#if provider.models.length === 0}
                <p class="hint">No models yet — edit and fetch them.</p>
              {:else}
                <div class="models">
                  {#each provider.models as model (model)}
                    <button
                      type="button"
                      class="model"
                      class:selected={isActive(provider.id, model)}
                      onclick={() => run(() => setActiveModel(provider.id, model))}
                    >
                      {model}
                    </button>
                  {/each}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      <h2 class="spaced">{editing ? `Edit ${editing}` : "Add a provider"}</h2>

      <form onsubmit={submit}>
        {#if !editing}
          <label>
            Start from
            <select onchange={(e) => applyPreset(e.currentTarget.value)}>
              <option value="">Custom endpoint…</option>
              {#each PRESETS as preset (preset.id)}
                <option value={preset.label}>{preset.label}</option>
              {/each}
            </select>
          </label>
        {/if}

        <label>
          Name
          <input bind:value={form.id} placeholder="ollama" required readonly={!!editing} />
        </label>

        <label>
          Protocol
          <select bind:value={form.kind}>
            <option value="openai-compatible">OpenAI-compatible</option>
            <option value="anthropic">Anthropic</option>
          </select>
        </label>

        <label>
          Endpoint
          <input bind:value={form.base_url} placeholder="http://localhost:11434/v1" required />
        </label>

        <label>
          <span class="label-row">
            <span>Models <span class="hint-inline">one per line</span></span>
            <button
              type="button"
              onclick={discover}
              disabled={discovering || !form.base_url.trim()}
            >
              {discovering ? "Asking…" : "Fetch from endpoint"}
            </button>
          </span>
          <textarea bind:value={form.models} rows="4" placeholder="qwen2.5-vl:7b"></textarea>
          {#if discovered}
            <span class="hint-inline">{discovered}</span>
          {/if}
        </label>

        <label class="checkbox">
          <input type="checkbox" bind:checked={form.requires_key} />
          This endpoint needs an API key
        </label>

        {#if form.requires_key}
          <label>
            API key
            <input
              type="password"
              value={form.apiKey ?? ""}
              oninput={(e) => (form.apiKey = e.currentTarget.value)}
              placeholder={editingProvider?.key_hint ?? "sk-…"}
              autocomplete="off"
            />
            <span class="hint-inline">
              {#if editingProvider?.key_hint}
                Stored key: <code>{editingProvider.key_hint}</code>. Leave blank to keep it.
              {:else}
                Stored in the macOS keychain, never in config.toml.
              {/if}
            </span>
          </label>
        {/if}

        <div class="form-actions">
          <button type="submit" class="primary">
            {editing ? "Save changes" : "Add provider"}
          </button>
          {#if editing}
            <button type="button" onclick={resetForm}>Cancel</button>
          {/if}
        </div>
      </form>
    {/if}
  </main>
</div>

<style>
  .settings {
    display: grid;
    /* Sidebar plus content, the shape every macOS settings window uses. */
    grid-template-columns: 152px 1fr;
    min-height: 100vh;
  }

  nav {
    background: color-mix(in srgb, Canvas 92%, CanvasText 8%);
    border-right: 1px solid color-mix(in srgb, Canvas 78%, CanvasText 22%);
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 16px 10px;
  }

  nav h1 {
    font-size: 10px;
    letter-spacing: 0.18em;
    margin: 0 0 14px 8px;
    opacity: 0.45;
    text-transform: uppercase;
  }

  .nav-item {
    background: none;
    border: none;
    border-radius: 5px;
    color: CanvasText;
    cursor: pointer;
    font: inherit;
    padding: 5px 9px;
    text-align: left;
  }

  .nav-item:hover {
    background: color-mix(in srgb, Canvas 84%, CanvasText 16%);
  }

  .nav-item.current {
    background: AccentColor;
    color: AccentColorText;
  }

  main {
    background: Canvas;
    box-sizing: border-box;
    color: CanvasText;
    padding: 22px 26px 36px;
  }

  :global(body) {
    font: 13px/1.55 -apple-system, BlinkMacSystemFont, sans-serif;
  }

  h2 {
    font-size: 11px;
    letter-spacing: 0.08em;
    margin: 0 0 10px;
    opacity: 0.55;
    text-transform: uppercase;
  }

  h2.spaced {
    margin-top: 28px;
  }

  .error {
    background: color-mix(in srgb, Canvas 82%, crimson 18%);
    border-left: 3px solid crimson;
    border-radius: 3px;
    margin: 0 0 18px;
    padding: 9px 12px;
  }

  .segmented {
    display: inline-flex;
    gap: 1px;
  }

  .segmented button {
    border-radius: 0;
  }

  .segmented button:first-child {
    border-radius: 5px 0 0 5px;
  }

  .segmented button:last-child {
    border-radius: 0 5px 5px 0;
  }

  .segmented button.selected {
    background: AccentColor;
    border-color: AccentColor;
    color: AccentColorText;
  }

  .providers {
    display: flex;
    flex-direction: column;
    gap: 9px;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .providers li {
    border: 1px solid color-mix(in srgb, Canvas 76%, CanvasText 24%);
    border-radius: 7px;
    padding: 11px 13px;
  }

  .row {
    align-items: flex-start;
    display: flex;
    gap: 12px;
    justify-content: space-between;
  }

  .meta {
    display: block;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    opacity: 0.6;
  }

  .badge {
    background: color-mix(in srgb, Canvas 80%, orange 20%);
    border-radius: 3px;
    display: inline-block;
    font-family: ui-monospace, monospace;
    font-size: 10px;
    margin-top: 5px;
    padding: 1px 6px;
  }

  .badge.ok {
    background: color-mix(in srgb, Canvas 82%, seagreen 18%);
  }

  .actions {
    display: flex;
    flex-shrink: 0;
    gap: 6px;
  }

  .models {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 10px;
  }

  .model {
    border-radius: 20px;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    padding: 3px 11px;
  }

  /* The selected model is where every turn goes, so it is the most important
     piece of state on this screen. */
  .model.selected {
    background: AccentColor;
    border-color: AccentColor;
    color: AccentColorText;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 13px;
    max-width: 440px;
  }

  label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    gap: 5px;
    opacity: 0.9;
  }

  label.checkbox {
    align-items: center;
    flex-direction: row;
    gap: 7px;
  }

  .label-row {
    align-items: center;
    display: flex;
    gap: 10px;
    justify-content: space-between;
  }

  input,
  select,
  textarea {
    background: Field;
    border: 1px solid color-mix(in srgb, Canvas 70%, CanvasText 30%);
    border-radius: 5px;
    color: FieldText;
    font: inherit;
    padding: 5px 8px;
  }

  input[readonly] {
    opacity: 0.6;
  }

  textarea {
    font-family: ui-monospace, monospace;
    resize: vertical;
  }

  button {
    background: ButtonFace;
    border: 1px solid color-mix(in srgb, Canvas 70%, CanvasText 30%);
    border-radius: 5px;
    color: ButtonText;
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    padding: 4px 11px;
  }

  button:hover:not(:disabled) {
    border-color: color-mix(in srgb, Canvas 50%, CanvasText 50%);
  }

  button:disabled {
    cursor: default;
    opacity: 0.45;
  }

  button.primary {
    background: AccentColor;
    border-color: AccentColor;
    color: AccentColorText;
  }

  button.danger:hover {
    border-color: crimson;
    color: crimson;
  }

  .form-actions {
    display: flex;
    gap: 8px;
  }

  .hint,
  .hint-inline {
    font-size: 11px;
    opacity: 0.55;
  }

  .hint {
    margin: 7px 0 0;
  }

  .hint-inline code {
    font-family: ui-monospace, monospace;
  }

  .empty {
    margin: 0;
    opacity: 0.7;
  }

  kbd {
    background: color-mix(in srgb, Canvas 82%, CanvasText 18%);
    border-radius: 4px;
    font-family: ui-monospace, monospace;
    padding: 2px 6px;
  }

  .path {
    display: block;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    opacity: 0.75;
    overflow-wrap: anywhere;
  }
</style>
