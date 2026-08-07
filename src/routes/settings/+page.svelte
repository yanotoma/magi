<script lang="ts">
  import {
    getConfig,
    saveProvider,
    removeProvider,
    setActiveModel,
    discoverModels,
    PRESETS,
    type ConfigView,
    type ProviderKind,
    type ProviderView,
  } from "$lib/ipc";

  let config = $state<ConfigView | null>(null);
  let error = $state<string | null>(null);
  let editing = $state<string | null>(null);
  let discovering = $state(false);
  let discovered = $state<string | null>(null);

  // The form. `apiKey` starts undefined and stays that way unless the user types
  // something, so editing a URL never silently drops a stored credential.
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
      // Commands return prose, which is all the UI can act on anyway.
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

  const submit = async (event: Event) => {
    event.preventDefault();
    await run(() =>
      saveProvider(
        {
          id: form.id.trim(),
          kind: form.kind,
          base_url: form.base_url.trim(),
          models: form.models
            .split("\n")
            .map((m) => m.trim())
            .filter(Boolean),
          requires_key: form.requires_key,
        },
        form.apiKey,
      ),
    );
    if (!error) resetForm();
  };

  const discover = async () => {
    error = null;
    discovered = null;
    discovering = true;
    try {
      const models = await discoverModels(
        {
          id: form.id.trim(),
          kind: form.kind,
          base_url: form.base_url.trim(),
          models: [],
          requires_key: form.requires_key,
        },
        form.apiKey,
      );
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

  const isActive = (providerId: string, model: string) =>
    config?.active?.provider === providerId && config?.active?.model === model;
</script>

<main>
  <h1>Magi Settings</h1>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <section>
    <h2>Models</h2>
    {#if !config || config.providers.length === 0}
      <p class="empty">
        No providers yet. Add one below — Ollama needs no key and no account.
      </p>
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
                    {provider.has_key ? "key stored" : "key missing"}
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
              <p class="hint">No models listed yet. Edit to add them.</p>
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
  </section>

  <section>
    <h2>{editing ? `Edit ${editing}` : "Add a provider"}</h2>

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
        <input
          bind:value={form.base_url}
          placeholder="http://localhost:11434/v1"
          required
        />
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
            placeholder={editing ? "leave blank to keep the stored key" : "sk-…"}
            autocomplete="off"
          />
          <span class="hint-inline">
            Stored in the macOS keychain, never in config.toml.
          </span>
        </label>
      {/if}

      <div class="form-actions">
        <button type="submit">{editing ? "Save changes" : "Add provider"}</button>
        {#if editing}
          <button type="button" onclick={resetForm}>Cancel</button>
        {/if}
      </div>
    </form>
  </section>

  <section>
    <h2>Hotkey</h2>
    <p>Toggle panel: <kbd>{config?.hotkey ?? "Alt+Space"}</kbd></p>
    <p class="hint">Editing the hotkey is not wired up yet.</p>
  </section>

  {#if config}
    <footer>
      <span class="hint">Configuration file</span>
      <code>{config.config_path}</code>
    </footer>
  {/if}
</main>

<style>
  main {
    /* Canvas/CanvasText are the system colours, so this follows the OS
       light/dark setting without a theme of its own. */
    background: Canvas;
    box-sizing: border-box;
    color: CanvasText;
    font: 13px/1.55 -apple-system, BlinkMacSystemFont, sans-serif;
    min-height: 100vh;
    padding: 24px 28px 40px;
  }

  h1 {
    font-size: 19px;
    margin: 0 0 22px;
  }

  h2 {
    font-size: 11px;
    letter-spacing: 0.08em;
    margin: 0 0 10px;
    opacity: 0.55;
    text-transform: uppercase;
  }

  section {
    margin-bottom: 30px;
  }

  .error {
    background: rgba(220, 60, 60, 0.12);
    border-left: 3px solid rgb(200, 60, 60);
    border-radius: 3px;
    margin: 0 0 20px;
    padding: 9px 12px;
  }

  .providers {
    display: flex;
    flex-direction: column;
    gap: 10px;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .providers li {
    border: 1px solid rgba(128, 128, 128, 0.28);
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
    background: rgba(200, 120, 40, 0.18);
    border-radius: 3px;
    display: inline-block;
    font-size: 10px;
    margin-top: 5px;
    padding: 1px 6px;
  }

  .badge.ok {
    background: rgba(60, 160, 90, 0.18);
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

  /* The selected model is the one a turn goes to, so it is the single most
     important piece of state on this screen. */
  .model.selected {
    background: AccentColor;
    border-color: AccentColor;
    color: AccentColorText;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 13px;
    max-width: 460px;
  }

  label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    gap: 5px;
    opacity: 0.9;
  }

  .label-row {
    align-items: center;
    display: flex;
    gap: 10px;
    justify-content: space-between;
  }

  button:disabled {
    cursor: default;
    opacity: 0.45;
  }

  label.checkbox {
    align-items: center;
    flex-direction: row;
    gap: 7px;
  }

  input,
  select,
  textarea {
    background: Field;
    border: 1px solid rgba(128, 128, 128, 0.4);
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
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 5px;
    color: ButtonText;
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    padding: 4px 11px;
  }

  button:hover {
    border-color: rgba(128, 128, 128, 0.7);
  }

  button.danger:hover {
    border-color: rgb(200, 60, 60);
    color: rgb(200, 60, 60);
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

  .empty {
    margin: 0;
    opacity: 0.7;
  }

  kbd {
    background: rgba(128, 128, 128, 0.18);
    border-radius: 4px;
    font-family: ui-monospace, monospace;
    padding: 2px 6px;
  }

  footer {
    border-top: 1px solid rgba(128, 128, 128, 0.22);
    padding-top: 14px;
  }

  footer code {
    display: block;
    font-size: 11px;
    margin-top: 3px;
    opacity: 0.75;
    overflow-wrap: anywhere;
  }
</style>
