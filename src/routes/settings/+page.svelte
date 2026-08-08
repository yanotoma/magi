<script lang="ts">
  import {
    getConfig,
    saveProvider,
    removeProvider,
    setActiveModel,
    setTheme,
    setShowThinking,
    setPromptContext,
    setHotkey,
    discoverModels,
    runPreflight,
    getVoice,
    setSpeechModel,
    setVoiceLanguage,
    LANGUAGES,
    downloadSpeechModel,
    removeSpeechModel,
    openPermissionSettings,
    onDownloadProgress,
    type ModelCapability,
    type VoiceView,
    type SpeechModel,
    type DownloadProgress,
    MAX_PROMPT_CONTEXT,
    PRESETS,
    type ConfigView,
    type ProviderKind,
    type ProviderView,
    type Theme,
  } from "$lib/ipc";
  import { acceleratorFrom, describeShortcut } from "$lib/shortcut";
  import Icon from "$lib/Icon.svelte";

  type Pane = "general" | "models" | "voice" | "hotkeys";

  const PANES: ReadonlyArray<{ id: Pane; label: string }> = [
    { id: "general", label: "General" },
    { id: "models", label: "Models" },
    { id: "voice", label: "Voice" },
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
  let capturing = $state(false);

  let voice = $state<VoiceView | null>(null);
  let progress = $state<DownloadProgress | null>(null);
  let voiceError = $state<string | null>(null);

  /** Which model this window is downloading, tracked locally.
   *
   *  Not read from `voice.downloading`, and that was a real bug rather than a
   *  preference: `downloadSpeechModel` awaits the entire transfer before returning a
   *  fresh view, so `voice` stays the object from before the click for the whole
   *  download — with `downloading` still null. The bar was gated on backend state the
   *  frontend could not see until it no longer mattered, and appeared for an instant at
   *  the end.
   *
   *  The side that started the download already knows it started. The same mistake as
   *  gating the panel's cancelled state on an event from a task that had been aborted. */
  let downloadingHere = $state<SpeechModel | null>(null);

  /** Which model is being probed, or null.
   *
   *  Two fields rather than a joined `"provider model"` key. Both halves are
   *  arbitrary text — provider ids are typed by the user, model names come from the
   *  endpoint — so any separator could appear inside one of them. The Rust cache
   *  avoids the same trap with a nested map. */
  let testing = $state<{ provider: string; model: string } | null>(null);

  // An editable copy rather than binding straight to `config.prompt.context`. The
  // backend is the source of truth and every mutation replaces `config` wholesale,
  // so a direct binding would have the box rewritten under the cursor by an
  // unrelated save.
  let promptContext = $state("");
  let contextSaved = $state(false);
  let contextTimer: ReturnType<typeof setTimeout> | undefined;

  /** Whether the add/edit form is on screen.
   *
   *  The form used to be permanently visible below the provider list, which made an
   *  empty "Add a provider" section the largest thing on the screen even when the
   *  user only came to switch models. */
  let formOpen = $state(false);

  // `apiKey` starts undefined and stays that way unless the user types, so
  // editing an endpoint never silently drops a stored credential.
  let form = $state({
    id: "",
    kind: "openai-compatible" as ProviderKind,
    base_url: "",
    requires_key: false,
    apiKey: undefined as string | undefined,
  });

  /** Every model name known for the provider being edited.
   *
   *  Starts as whatever is already saved and grows when the endpoint is asked. Not
   *  persisted: the catalogue is a working list for the form, and only the chosen
   *  subset is written to the config. */
  let catalog = $state<string[]>([]);

  /** The subset the user wants Magi to offer.
   *
   *  Separate from the catalogue because those are different questions — "what does
   *  this endpoint serve" and "what do I want to see". OpenRouter serves hundreds;
   *  listing all of them in the provider card and the capability matrix would bury
   *  the two or three anyone actually uses. */
  let chosen = $state<Set<string>>(new Set());

  let modelSearch = $state("");
  let selectedOnly = $state(false);

  /** Providers whose model table is folded away.
   *
   *  Deliberately not persisted. It is view state, and `config.toml` is a documented
   *  contract surface — putting "which cards were folded" in there would make a UI
   *  preference part of the schema Magi promises not to break.
   *
   *  Collapsed is opt-in rather than the default: a provider whose models are hidden
   *  on first sight looks like a provider with no models, and the capability matrix
   *  is the reason to open this screen. */
  let collapsed = $state<Set<string>>(new Set());

  const toggleCollapsed = (id: string) => {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  };

  /** The catalogue as the picker shows it: filtered, and in stable order.
   *
   *  Alphabetical rather than selected-first, deliberately. Sorting by selection
   *  would make rows jump under the cursor as they are ticked, which is the one thing
   *  a list of three hundred checkboxes must not do. The count and the "Selected
   *  only" filter are how you review a selection instead. */
  const visibleModels = $derived.by(() => {
    const needle = modelSearch.trim().toLowerCase();
    return catalog.filter((model) => {
      if (selectedOnly && !chosen.has(model)) return false;
      return needle === "" || model.toLowerCase().includes(needle);
    });
  });

  const run = async (action: () => Promise<ConfigView>) => {
    error = null;
    try {
      config = await action();
    } catch (e) {
      error = String(e);
    }
  };

  // The first load seeds the context box; later refreshes deliberately do not.
  // `run` replaces `config` after every mutation — saving a provider, switching
  // theme — and copying the context across on each of those would discard
  // whatever the user was in the middle of typing.
  $effect(() => {
    getConfig()
      .then((loaded) => {
        config = loaded;
        promptContext = loaded.prompt.context;
      })
      .catch((e) => (error = String(e)));
  });

  /** Writes the context, if it differs from what is stored. */
  const flushContext = async () => {
    clearTimeout(contextTimer);
    if (!config || promptContext === config.prompt.context) return;

    await run(() => setPromptContext(promptContext));
    if (error) return;

    // A written setting with no visible effect needs to say so. Everything else
    // in this screen changes something the user can see; this changes what the
    // next answer is told, which is invisible until they ask something.
    contextSaved = true;
    setTimeout(() => (contextSaved = false), 1800);
  };

  /** Saves shortly after typing stops.
   *
   *  This used to save on blur alone, and lost the text every time: the Settings
   *  window *hides* rather than closes, and a field that still has focus when its
   *  window disappears is not reliably blurred. So the text was typed, the window
   *  was closed, and nothing was ever written — with a box that still showed the
   *  text on reopening, because the box is seeded from a config that never got it.
   *
   *  Saving as you type removes the need for any particular closing gesture to
   *  fire. Blur and window-blur below still flush immediately, so the delay only
   *  ever costs a moment while typing continues. */
  const queueContextSave = () => {
    clearTimeout(contextTimer);
    contextTimer = setTimeout(flushContext, 600);
  };

  const loadVoice = async () => {
    try {
      voice = await getVoice();
    } catch (e) {
      voiceError = String(e);
    }
  };

  // Read once on mount, and again whenever the pane is opened: the microphone
  // permission can change in System Settings while this window sits in the
  // background, and a stale "blocked" row would have the user granting it twice.
  $effect(() => {
    if (pane === "voice") loadVoice();
  });

  // Progress arrives whether or not this pane is open, so the subscription is not tied
  // to it — a download started and then navigated away from still finishes, and coming
  // back should find the bar where it actually is.
  $effect(() => {
    let stop: (() => void) | undefined;
    onDownloadProgress({
      progress: (p) => (progress = p),
      done: () => {
        progress = null;
        loadVoice();
      },
    }).then((off) => (stop = off));
    return () => stop?.();
  });

  const runVoice = async (action: () => Promise<VoiceView>) => {
    voiceError = null;
    try {
      voice = await action();
    } catch (e) {
      voiceError = String(e);
    }
  };

  /** Starts a download and shows it immediately. */
  const download = async (model: SpeechModel) => {
    downloadingHere = model;
    progress = null;
    voiceError = null;
    try {
      voice = await downloadSpeechModel(model);
    } catch (e) {
      voiceError = String(e);
    } finally {
      // Cleared whatever happened, so a failure cannot leave the row stuck showing a
      // download that is no longer running.
      downloadingHere = null;
      progress = null;
    }
  };

  const percent = (p: DownloadProgress): number =>
    p.total === 0 ? 0 : Math.min(100, Math.round((p.downloaded / p.total) * 100));

  const megabytes = (bytes: number): string => `${Math.round(bytes / 1_000_000)} MB`;

  /** Probe results for one model, or undefined when it has not been tested. */
  const capabilityFor = (
    provider: ProviderView,
    model: string,
  ): ModelCapability | undefined => provider.capabilities.find((c) => c.model === model);

  const isTesting = (providerId: string, model: string): boolean =>
    testing?.provider === providerId && testing?.model === model;

  /** What each probe actually sent, and what the result means.
   *
   *  Written per outcome rather than per column, because the interesting half is the
   *  failure. "JSON ✕" on its own reads as "cannot produce JSON", when what was
   *  measured is narrower and more useful: a schema was sent and the reply did not
   *  match it. A model can return perfectly good JSON and still fail that. */
  const explain = (probe: string, value: boolean | undefined): string => {
    if (value === undefined) return "Not tested yet. Press Test to find out.";

    const passed: Record<string, string> = {
      reachable: "The endpoint answered with this model and this key.",
      vision:
        "Magi sent a generated image of a digit and the model named it correctly, " +
        "so it genuinely reads images rather than accepting and ignoring them.",
      tools:
        "Magi offered one tool and the model made a structurally valid call with " +
        "the required argument filled in.",
      json:
        "Magi sent a JSON Schema and the reply matched it — the exact field names " +
        "and types that were asked for.",
    };

    const failed: Record<string, string> = {
      reachable:
        "The endpoint did not answer. Check the URL and the key, and — for a local " +
        "server — that it is running and the model is downloaded.",
      vision:
        "The model did not read the test image. Either it has no vision, or it " +
        "accepted the image and ignored it.",
      tools:
        "No usable tool call came back. The model may have described the call in " +
        "prose instead of making it, or left the required argument empty.",
      json:
        "The reply did not match the schema that was sent. Returning valid JSON is " +
        "not enough — the field names and types have to be the ones requested, or " +
        "code reading the answer has to guess.",
    };

    return (value ? passed : failed)[probe] ?? "";
  };

  /** A capability cell: yes, no, or not yet asked.
   *
   *  Three states rather than a boolean. "Untested" and "failed" are different
   *  claims, and only one of them is Magi's to make — rendering an untested model
   *  as a cross would assert something nobody has checked. */
  const glyph = (value: boolean | undefined): string => {
    if (value === undefined) return "–";
    return value ? "✓" : "✕";
  };

  /** Runs pre-flight for one model.
   *
   *  One at a time. Each probe is four requests, and against a local server sharing
   *  one GPU they would queue anyway; against a metered API, concurrent probes can
   *  trip a rate limit, which would come back as a failure and be recorded as a
   *  capability the model does not have. */
  const test = async (providerId: string, model: string) => {
    if (testing !== null) return;
    testing = { provider: providerId, model };
    try {
      await run(() => runPreflight(providerId, model));
    } finally {
      testing = null;
    }
  };

  const toggleCapture = () => {
    capturing = !capturing;
    error = null;
  };

  /** Handles keys during capture, and is attached to the window rather than to
   *  the button.
   *
   *  On macOS, clicking a `<button>` does not focus it — WebKit deliberately
   *  leaves focus where it was. A `keydown` handler on the button therefore never
   *  fires after a click, which is why the first version of this appeared to
   *  ignore every combination *and* Escape: the handler was never reached at all,
   *  so no key mapping was involved. The window always receives the event. */
  const onWindowKeydown = (event: KeyboardEvent) => {
    if (!capturing) return;

    // Once capturing, every key belongs to the capture. Without this, Space
    // scrolls the pane, Tab moves focus, and on macOS Cmd+W would close the
    // window instead of being recorded.
    event.preventDefault();
    event.stopPropagation();

    // Bare Escape backs out. With a modifier it is a legitimate shortcut, so only
    // the unmodified press cancels.
    if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey) {
      capturing = false;
      return;
    }

    const accelerator = acceleratorFrom(event);
    // Null means the combination is not finished — modifiers are usually pressed
    // before the key, so this is the normal path, not an error.
    if (!accelerator) return;

    capturing = false;
    run(() => setHotkey(accelerator));
  };

  /** The window losing focus ends a capture and commits pending text.
   *
   *  Both matter for the same reason: this window is hidden rather than closed, and
   *  whatever state it was left in is the state it comes back in. A capture left
   *  armed would swallow the first keystroke of the next visit. */
  const onWindowBlur = () => {
    capturing = false;
    flushContext();
  };

  const resetForm = () => {
    editing = null;
    discovered = null;
    catalog = [];
    chosen = new Set();
    modelSearch = "";
    selectedOnly = false;
    form = {
      id: "",
      kind: "openai-compatible",
      base_url: "",
      requires_key: false,
      apiKey: undefined,
    };
  };

  /** Opens a blank form for a new provider. */
  const addProvider = () => {
    resetForm();
    formOpen = true;
  };

  const closeForm = () => {
    formOpen = false;
    resetForm();
  };

  const toggleModel = (model: string) => {
    const next = new Set(chosen);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    chosen = next;
  };

  /** Selects everything currently shown, which is what makes search useful.
   *
   *  Scoped to the visible rows rather than the whole catalogue on purpose: with a
   *  search active, "select all shown" is the point — filter to `gpt-5`, take the
   *  matches. Without one it selects everything, and the count says so. */
  const selectShown = () => {
    const next = new Set(chosen);
    for (const model of visibleModels) next.add(model);
    chosen = next;
  };

  const clearShown = () => {
    const next = new Set(chosen);
    for (const model of visibleModels) next.delete(model);
    chosen = next;
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
    // Both entry points have to open the form. `addProvider` did and this did not,
    // so Edit silently did nothing: it set every piece of form state and left the
    // form hidden. The visibility flag was added later and only one caller was
    // updated.
    formOpen = true;
    editing = provider.id;
    discovered = null;
    modelSearch = "";
    selectedOnly = false;
    // The saved models seed both lists: they are what is known and what is chosen.
    // Editing a provider without asking the endpoint again still shows the current
    // selection, so a URL can be corrected without losing it.
    catalog = [...provider.models].sort();
    chosen = new Set(provider.models);
    form = {
      id: provider.id,
      kind: provider.kind,
      base_url: provider.base_url,
      requires_key: provider.requires_key,
      apiKey: undefined,
    };
  };

  const formAsProvider = () => ({
    id: form.id.trim(),
    kind: form.kind,
    base_url: form.base_url.trim(),
    // Sorted so the saved order does not depend on the order things were clicked,
    // which would otherwise show up as noise in a config.toml diff.
    models: [...chosen].sort(),
    requires_key: form.requires_key,
  });

  const submit = async (event: Event) => {
    event.preventDefault();
    await run(() => saveProvider(formAsProvider(), form.apiKey));
    if (!error) closeForm();
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
        return;
      }

      // Merged into the catalogue, and nothing is selected automatically. That is
      // the whole change: on OpenRouter this call returns hundreds, and selecting
      // them all would put hundreds of rows in the provider card and offer hundreds
      // of models nobody asked for. The user picks.
      const merged = new Set([...catalog, ...models]);
      catalog = [...merged].sort();

      const fresh = models.filter((model) => !chosen.has(model)).length;
      discovered =
        `The endpoint serves ${models.length} model${models.length === 1 ? "" : "s"}` +
        (fresh > 0 ? `. Choose the ones you want below.` : `, all already chosen.`);
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

<!--
  Keydown is bound here rather than on the capture button because on macOS a
  clicked button does not take focus, so a handler on it would never run.
  Window blur ends a capture and commits any pending context text: this window
  hides rather than closes, so whatever state it is left in is what comes back.
-->
<svelte:window onkeydown={onWindowKeydown} onblur={onWindowBlur} />

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

      <h2 class="spaced">Panel</h2>
      <label class="checkbox">
        <input
          type="checkbox"
          checked={config?.appearance.show_thinking ?? false}
          onchange={(e) => run(() => setShowThinking(e.currentTarget.checked))}
        />
        Show the model's reasoning
      </label>
      <p class="hint">
        Only some models produce any. Where they do it is usually longer than the
        answer, so it appears behind a disclosure rather than inline.
      </p>

      <h2 class="spaced">Context</h2>
      <p class="hint">
        Standing facts you want every answer to account for — where you are, what
        you work on, which units you think in. Sent with every question, so keep it
        short.
      </p>
      <textarea
        class="context"
        rows="5"
        maxlength={MAX_PROMPT_CONTEXT}
        placeholder="I work in Kitchener, Ontario, mostly in Rust and TypeScript."
        bind:value={promptContext}
        oninput={queueContextSave}
        onblur={flushContext}
      ></textarea>
      <p class="hint counter" class:near={promptContext.length > MAX_PROMPT_CONTEXT * 0.9}>
        {promptContext.length} / {MAX_PROMPT_CONTEXT}
        {#if contextSaved}<span class="saved">saved</span>{/if}
      </p>
      <p class="hint">
        This adds to Magi's instructions and cannot replace them, so it is not a
        place to change how Magi works — only what it knows about you.
      </p>

      {#if config}
        <h2 class="spaced">Configuration file</h2>
        <code class="path">{config.config_path}</code>
        <p class="hint">
          Safe to share: API keys are in the macOS keychain, never in this file.
        </p>
      {/if}
    {/if}

    {#if pane === "voice"}
      <h2>Microphone</h2>
      {#if voice}
        <!--
          A status row rather than a message that appears when something fails. Every
          macOS permission fails silently, so the only way a user learns the state is
          by being shown it.
        -->
        <div class="perm-row">
          <span class="badge {voice.microphone}">{voice.microphone.replace("-", " ")}</span>
          <span class="hint">{voice.microphone_explanation}</span>
        </div>

        <!--
          Offered only where it helps. `restricted` means a configuration profile on a
          managed Mac, and the pane it would open holds a toggle the user cannot move.
        -->
        {#if voice.microphone === "denied"}
          <button type="button" onclick={() => openPermissionSettings("microphone")}>
            Open Privacy &amp; Security
          </button>
        {/if}

        <h2 class="spaced">Language</h2>
        <label>
          Spoken language
          <select
            value={voice.language}
            onchange={(e) => runVoice(() => setVoiceLanguage(e.currentTarget.value))}
          >
            {#each LANGUAGES as language (language.code)}
              <option value={language.code}>{language.label}</option>
            {/each}
          </select>
        </label>
        <p class="hint">
          Detection is usually right, and handles switching languages between questions.
          Choosing one is slightly faster and removes the chance of a short phrase being
          detected wrongly.
        </p>

        <!--
          The trap this exists to close. An English-only model given Spanish does not
          fail — it writes English words that sound similar — so the combination has to
          be said out loud rather than left to produce a confident wrong transcript.
        -->
        {#if voice.language_ignored}
          <p class="error" role="alert">
            The selected model understands English only, so this language is ignored.
            Pick a multilingual model below.
          </p>
        {/if}

        <h2 class="spaced">Speech model</h2>
        <p class="hint">
          Transcription happens on this Mac. Nothing you say is sent anywhere.
        </p>

        {#if voiceError}
          <p class="error" role="alert">{voiceError}</p>
        {/if}

        <ul class="speech-models">
          {#each voice.models as model (model.id)}
            <li>
              <div class="row">
                <button
                  type="button"
                  class="speech-choice"
                  class:selected={model.selected}
                  onclick={() => runVoice(() => setSpeechModel(model.id))}
                >
                  <span class="bullet" aria-hidden="true">{model.selected ? "●" : ""}</span>
                  <span class="ident">
                    <strong>{model.label}</strong>
                    {#if !model.multilingual}
                      <!-- Marked in the row as well as the label, because the label is
                           what someone reads and the badge is what they notice. -->
                      <span class="badge">EN</span>
                    {/if}
                    <span class="hint">{model.description}</span>
                  </span>
                </button>

                <div class="actions">
                  {#if downloadingHere === model.id}
                    <span class="hint">
                      {progress ? `${percent(progress)}%` : "Connecting…"}
                    </span>
                  {:else if model.downloaded}
                    <span class="badge granted">On disk</span>
                    <button
                      type="button"
                      class="icon danger"
                      title="Delete {model.label}"
                      aria-label="Delete {model.label}"
                      onclick={() => runVoice(() => removeSpeechModel(model.id))}
                    >
                      <Icon name="trash" />
                    </button>
                  {:else}
                    <button
                      type="button"
                      disabled={downloadingHere !== null}
                      onclick={() => download(model.id)}
                    >
                      Download {megabytes(model.approximate_mb * 1_000_000)}
                    </button>
                  {/if}
                </div>
              </div>

              {#if downloadingHere === model.id}
                <!--
                  A real bar, not a spinner. 488 MB deserves to be told how far along it
                  is, and the byte counts are what make a stalled download visible where a
                  spinner would look identical either way.

                  Shown from the click, not from the first byte: the gap while the request
                  is opening is exactly when someone wonders whether their click worked.
                  An indeterminate stripe covers it.
                -->
                <div
                  class="bar"
                  class:indeterminate={!progress}
                  role="progressbar"
                  aria-valuenow={progress ? percent(progress) : undefined}
                >
                  <div
                    class="bar-fill"
                    style={progress ? `width: ${percent(progress)}%` : ""}
                  ></div>
                </div>
                <p class="hint">
                  {#if progress}
                    {megabytes(progress.downloaded)} of {megabytes(progress.total)} —
                    leaving this screen will not stop it
                  {:else}
                    Opening the connection…
                  {/if}
                </p>
              {/if}
            </li>
          {/each}
        </ul>

        {#if !voice.ready}
          <p class="hint">
            The selected model is not downloaded yet, so voice input is unavailable
            until it is.
          </p>
        {/if}
      {:else if voiceError}
        <p class="error" role="alert">{voiceError}</p>
      {/if}
    {/if}

    {#if pane === "hotkeys"}
      <h2>Global shortcut</h2>
      <p class="hint">Toggles the panel from anywhere, whatever has focus.</p>

      <button type="button" class="capture" class:listening={capturing} onclick={toggleCapture}>
        {#if capturing}
          <span class="prompt">Press a combination…</span>
        {:else}
          <kbd>{describeShortcut(config?.hotkey ?? "")}</kbd>
        {/if}
      </button>

      {#if capturing}
        <p class="hint">
          Hold at least one modifier. <kbd>Esc</kbd> or clicking again cancels, and
          nothing changes until a combination is accepted.
        </p>
      {:else}
        <p class="hint">
          Click to change it. A combination another application already owns will
          be refused, and the current one keeps working.
        </p>
      {/if}
    {/if}

    {#if pane === "models"}
      <!--
        The form replaces the list rather than joining it. They are separate tasks —
        configuring an endpoint, and reading what its models can do — and showing both
        at once leaves the form as a narrow column beside a table it has nothing to do
        with. One screen, one job.
      -->
      {#if formOpen}
      {@render providerForm()}
      {:else}
      <div class="section-head">
        <h2>Providers</h2>
        <button type="button" class="primary with-icon" onclick={addProvider}>
          <Icon name="plus" />
          Add a provider
        </button>
      </div>

      {#if !config || config.providers.length === 0}
        <p class="empty">No providers yet. Ollama needs no key and no account.</p>
      {:else}
        <ul class="providers">
          {#each config.providers as provider (provider.id)}
            <li>
              <div class="row">
                <!--
                  The whole heading is the disclosure control, not a separate
                  triangle. A 10px target next to a card the user already reads as
                  one unit is a worse click than the card itself.
                -->
                <button
                  type="button"
                  class="disclosure"
                  aria-expanded={!collapsed.has(provider.id)}
                  onclick={() => toggleCollapsed(provider.id)}
                >
                  <strong class="ident">{provider.id}</strong>
                  <span class="tags">
                    <!--
                      The endpoint and the key fingerprint used to sit here and have
                      moved to the edit form. Neither is actionable from a list, and
                      a row of URLs and masked secrets is a lot of ink spent on
                      things nobody reads twice.

                      A missing key is the exception, because it is a problem rather
                      than a detail: without it the only symptom is every model
                      reporting Unreachable, which sends the user to check the URL.
                    -->
                    {#if provider.requires_key && !provider.has_key}
                      <span class="badge warn">No API key</span>
                    {/if}
                    <!-- Shown when folded, so collapsing does not hide the one fact
                         that decides what Magi can do. -->
                    {#if collapsed.has(provider.id)}
                      <span class="badge">
                        {provider.models.length} model{provider.models.length === 1 ? "" : "s"}
                      </span>
                    {/if}
                  </span>
                </button>

                <div class="actions">
                  <!--
                    The heading is still the fold target — it is the big, obvious
                    thing to click. This is the indicator, sized and drawn like the
                    other icons so it reads as a chevron rather than as a dot.
                  -->
                  <button
                    type="button"
                    class="icon"
                    title={collapsed.has(provider.id) ? "Show models" : "Hide models"}
                    aria-label={collapsed.has(provider.id) ? "Show models" : "Hide models"}
                    onclick={() => toggleCollapsed(provider.id)}
                  >
                    <Icon name={collapsed.has(provider.id) ? "chevron-right" : "chevron-down"} />
                  </button>
                  <button
                    type="button"
                    class="icon"
                    title="Edit {provider.id}"
                    aria-label="Edit {provider.id}"
                    onclick={() => edit(provider)}
                  >
                    <Icon name="pencil" />
                  </button>
                  <button
                    type="button"
                    class="icon danger"
                    title="Remove {provider.id}"
                    aria-label="Remove {provider.id}"
                    onclick={() => run(() => removeProvider(provider.id))}
                  >
                    <Icon name="trash" />
                  </button>
                </div>
              </div>

              {#if collapsed.has(provider.id)}
                <!-- folded -->
              {:else if provider.models.length === 0}
                <p class="hint">
                  No models chosen yet — press Edit, fetch them, and pick the ones you
                  want.
                </p>
              {:else}
                <div class="matrix-scroll">
                <table class="matrix">
                  <thead>
                    <tr>
                      <th>Model</th>
                      <th title="The endpoint answered with this model and key">Reach</th>
                      <th title="Read a generated test image correctly">Sees</th>
                      <th title="Made a structurally valid tool call">Tools</th>
                      <th title="Returned JSON matching a schema">JSON</th>
                      <th>Capability</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each provider.models as model (model)}
                      {@const probed = capabilityFor(provider, model)}
                      <tr class:selected={isActive(provider.id, model)}>
                        <td class="model-cell">
                          <!--
                            Plain text, not a pill. As a filled button the long ids
                            these endpoints use wrapped inside their own background,
                            which turned a table of names into a column of two-line
                            blobs. The active model is marked with a bullet and the
                            row tint instead, which needs no box.
                          -->
                          <button
                            type="button"
                            class="model"
                            class:selected={isActive(provider.id, model)}
                            title={isActive(provider.id, model)
                              ? "The model Magi is using"
                              : `Use ${model}`}
                            onclick={() => run(() => setActiveModel(provider.id, model))}
                          >
                            <span class="bullet" aria-hidden="true">
                              {isActive(provider.id, model) ? "●" : ""}
                            </span>
                            {model}
                          </button>
                        </td>

                        <!--
                          Three states per cell, not two. An untested model shows a
                          dash: it has not failed, nobody has asked it yet, and
                          rendering that as a cross would be a claim Magi has not
                          earned.

                          Each cell states what was actually tried, not just what it
                          measured. A column header saying "JSON" leaves a cross
                          looking arbitrary — the useful information is that a schema
                          was sent and the reply did not match it, which is a
                          different claim from "does not do JSON".
                        -->
                        <td class="mark" title={explain("reachable", probed?.reachable)}>
                          {glyph(probed?.reachable)}
                        </td>
                        <td class="mark" title={explain("vision", probed?.vision)}>
                          {glyph(probed?.vision)}
                        </td>
                        <td class="mark" title={explain("tools", probed?.tools)}>
                          {glyph(probed?.tools)}
                        </td>
                        <td class="mark" title={explain("json", probed?.structured_output)}>
                          {glyph(probed?.structured_output)}
                        </td>

                        <td>
                          {#if probed}
                            <span class="tier {probed.tier}" title={probed.explanation}>
                              {probed.label}
                            </span>
                          {:else}
                            <span class="untested">Not tested</span>
                          {/if}
                        </td>

                        <td>
                          <button
                            type="button"
                            class="test"
                            disabled={testing !== null}
                            onclick={() => test(provider.id, model)}
                          >
                            {isTesting(provider.id, model)
                              ? "Testing…"
                              : probed
                                ? "Re-test"
                                : "Test"}
                          </button>
                        </td>
                      </tr>

                      {#if probed && isActive(provider.id, model)}
                        <!-- The explanation is shown outright for the model actually
                             in use, rather than hidden in a tooltip. Someone
                             wondering why screen reading is off is asking about the
                             model they selected. -->
                        <tr class="why">
                          <td colspan="7">{probed.explanation}</td>
                        </tr>
                      {/if}
                    {/each}
                  </tbody>
                </table>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      {/if}
    {/if}
  </main>
</div>

{#snippet providerForm()}
  <form class="provider-form" onsubmit={submit}>
    <div class="form-head">
      <h2>{editing ? `Edit ${editing}` : "New provider"}</h2>
      <button type="button" onclick={closeForm}>Back to providers</button>
    </div>

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

    <!--
      The model picker. The key change from the first version: asking the endpoint
      no longer selects everything it returns. OpenRouter answers with hundreds, and
      taking all of them would fill the provider card and the capability matrix with
      models nobody asked for — and offer them all as choices in the panel.
    -->
    <div class="picker">
      <div class="picker-head">
        <span>
          Models
          <span class="hint-inline">
            {chosen.size} chosen{catalog.length > 0 ? ` of ${catalog.length} known` : ""}
          </span>
        </span>
        <button
          type="button"
          onclick={discover}
          disabled={discovering || !form.base_url.trim()}
        >
          {discovering ? "Asking…" : "Fetch from endpoint"}
        </button>
      </div>

      {#if discovered}
        <p class="hint">{discovered}</p>
      {/if}

      {#if catalog.length === 0}
        <p class="hint">
          No models known yet. Fetch them from the endpoint, and then choose the ones
          you want Magi to offer.
        </p>
      {:else}
        <!-- The search exists for OpenRouter and AI Studio, where the list runs to
             hundreds and scrolling is not a way to find anything. -->
        <div class="picker-tools">
          <input
            class="search"
            type="search"
            bind:value={modelSearch}
            placeholder="Search {catalog.length} models…"
          />
          <label class="checkbox tight">
            <input type="checkbox" bind:checked={selectedOnly} />
            Selected only
          </label>
        </div>

        <div class="picker-bulk">
          <button type="button" class="link" onclick={selectShown} disabled={visibleModels.length === 0}>
            Select {visibleModels.length === catalog.length ? "all" : `these ${visibleModels.length}`}
          </button>
          <button type="button" class="link" onclick={clearShown} disabled={visibleModels.length === 0}>
            Clear
          </button>
        </div>

        {#if visibleModels.length === 0}
          <p class="hint">Nothing matches.</p>
        {:else}
          <ul class="model-list">
            {#each visibleModels as model (model)}
              <li>
                <label class="checkbox">
                  <input
                    type="checkbox"
                    checked={chosen.has(model)}
                    onchange={() => toggleModel(model)}
                  />
                  <span class="model-name">{model}</span>
                </label>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>

    <div class="form-actions">
      <button type="submit" class="primary">
        {editing ? "Save changes" : "Add provider"}
      </button>
      <button type="button" onclick={closeForm}>Cancel</button>
    </div>
  </form>
{/snippet}

<style>
  .settings {
    display: grid;
    /* Sidebar plus content, the shape every macOS settings window uses. */
    grid-template-columns: 152px 1fr;
    min-height: 100vh;
  }

  nav {
    background: var(--surface-hover);
    border-right: 1px solid var(--line-edge);
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 16px 10px;
  }

  nav h1 {
    font-size: 10px;
    letter-spacing: 0.18em;
    margin: 0 0 14px 8px;
    opacity: var(--muted);
    text-transform: uppercase;
  }

  .nav-item {
    background: none;
    border: none;
    border-radius: var(--radius-control);
    color: CanvasText;
    cursor: pointer;
    font: inherit;
    padding: 5px 9px;
    text-align: left;
  }

  .nav-item:hover {
    background: var(--line);
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
    opacity: var(--muted);
    text-transform: uppercase;
  }

  /* A section break is a rule, everywhere. General and Hotkeys used whitespace
     alone while Models used bordered cards — two different ideas of what a group
     is, which is what made the screens look unrelated. */
  h2.spaced {
    border-top: 1px solid var(--line);
    margin-top: var(--gap-lg);
    padding-top: var(--gap-lg);
  }

  .error {
    background: var(--tone-bad);
    border-left: 3px solid crimson;
    border-radius: var(--radius-control);
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
    border-radius: var(--radius-control) 0 0 var(--radius-control);
  }

  .segmented button:last-child {
    border-radius: 0 var(--radius-control) var(--radius-control) 0;
  }

  .segmented button.selected {
    background: AccentColor;
    border-color: AccentColor;
    color: AccentColorText;
  }

  /* Separated by rules rather than boxed.
     A bordered card draws a frame around content that already reads as a group, and
     with several of them the frames become the loudest thing on the screen. A single
     hairline says the same thing with one pixel. The first row needs no rule — there
     is nothing above it to separate from. */
  .providers {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .providers li {
    border-top: 1px solid var(--line);
    padding: var(--gap-md) 0;
  }

  .providers li:first-child {
    border-top: none;
    padding-top: var(--gap-xs);
  }

  .row {
    align-items: flex-start;
    display: flex;
    gap: 12px;
    justify-content: space-between;
  }

  /* The whole provider heading is the fold control. It has to look like a heading
     rather than a button, so everything a button brings is removed and only the
     cursor and the twisty say it is clickable. */
  button.disclosure {
    align-items: baseline;
    background: none;
    border: none;
    color: inherit;
    display: flex;
    flex: 1;
    font: inherit;
    gap: 7px;
    min-width: 0;
    padding: 0;
    text-align: left;
  }

  button.disclosure:hover {
    background: none;
  }

  .ident {
    min-width: 0;
    /* Wraps rather than stretching the row: a provider id is user-typed and can be
       any length. */
    overflow-wrap: anywhere;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .badge {
    background: var(--tone-warn);
    border-radius: var(--radius-control);
    display: inline-block;
    font-family: ui-monospace, monospace;
    font-size: 10px;
    margin-top: 5px;
    padding: 1px 6px;
  }

  /* Square, borderless, and quiet until pointed at. These are secondary actions on
     a row whose subject is the provider name; giving them button chrome would make
     two frames per row compete with the name for attention.

     26px square, holding a 14px glyph. The button is the target, not the glyph —
     sizing the control to the drawing would leave a 14px hit area. */
  button.icon {
    align-items: center;
    background: none;
    border: none;
    border-radius: var(--radius-control);
    display: flex;
    height: 26px;
    justify-content: center;
    opacity: var(--muted-strong);
    padding: 0;
    width: 26px;
  }

  button.icon:hover:not(:disabled) {
    background: var(--surface-hover);
    opacity: 1;
  }

  /* Crimson glyph on the same hover surface as any other icon button. The other
     danger control on this screen signals with foreground colour, and inventing a
     second idiom for the same meaning is what made these screens look unrelated in
     the first place. */
  button.icon.danger:hover:not(:disabled) {
    color: crimson;
  }

  /* An icon beside a word, aligned on the word's centre rather than its baseline. */
  button.with-icon {
    align-items: center;
    display: inline-flex;
    gap: 6px;
  }

  .badge.warn {
    background: var(--tone-warn);
  }

  .actions {
    display: flex;
    flex-shrink: 0;
    gap: 6px;
  }

  /* The name as text, with no box of its own. As a filled pill the long ids these
     endpoints hand out wrapped inside their own background, so a column of names
     became a column of two-line blobs. */
  .model {
    background: none;
    border: none;
    color: inherit;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    padding: 2px 0;
    text-align: left;
    /* One line. The cell is as wide as the longest name, and the table scrolls
       inside its own container when that exceeds the window. */
    white-space: nowrap;
  }

  .model:hover {
    background: none;
    text-decoration: underline;
  }

  /* The selected model is where every turn goes, so it is the most important piece
     of state on this screen. Marked by weight, colour and a bullet rather than a
     filled shape — three signals, none of which needs a box or depends on colour
     alone. */
  .model.selected {
    color: AccentColor;
    font-weight: 600;
  }

  .bullet {
    color: AccentColor;
    display: inline-block;
    font-size: 9px;
    /* Reserved whether or not it is filled, so names line up down the column
       instead of shifting by a bullet's width. */
    width: 1em;
  }

  .model-cell {
    /* The names column takes what it needs; the capability columns are fixed. */
    width: 100%;
  }

  /* A provider serving ids like meta-llama/llama-3.3-70b-instruct:free can exceed
     the window. The table scrolls rather than the page. */
  .matrix-scroll {
    overflow-x: auto;
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

  input,
  select,
  textarea {
    background: Field;
    border: 1px solid var(--line-edge);
    border-radius: var(--radius-control);
    color: FieldText;
    font: inherit;
    padding: 5px 8px;
  }

  input[readonly] {
    opacity: var(--muted);
  }

  textarea {
    font-family: ui-monospace, monospace;
    resize: vertical;
  }

  button {
    background: ButtonFace;
    border: 1px solid var(--line-edge);
    border-radius: var(--radius-control);
    color: ButtonText;
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    padding: 4px 11px;
  }

  button:hover:not(:disabled) {
    border-color: var(--line-strong);
  }

  button:disabled {
    cursor: default;
    opacity: var(--muted);
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
    opacity: var(--muted);
  }

  .hint {
    margin: 7px 0 0;
  }

  .hint-inline code {
    font-family: ui-monospace, monospace;
  }

  .empty {
    margin: 0;
    opacity: var(--muted-strong);
  }

  kbd {
    background: var(--line);
    border-radius: var(--radius-control);
    font-family: ui-monospace, monospace;
    padding: 2px 6px;
  }

  /* Prose, so it overrides the monospace the model-list textarea wants. */
  .context {
    box-sizing: border-box;
    font-family: inherit;
    margin-top: 8px;
    width: 100%;
  }

  .counter {
    display: flex;
    gap: 8px;
    /* Digits that do not shift the "saved" label sideways as the count changes. */
    font-variant-numeric: tabular-nums;
  }

  .counter.near {
    opacity: 0.9;
  }

  .saved {
    color: AccentColor;
  }

  .section-head {
    align-items: center;
    display: flex;
    justify-content: space-between;
    /* The button sat directly on top of the first provider row. */
    margin-bottom: var(--gap-md);
  }

  .section-head h2 {
    margin: 0;
  }

  /* The form reads as a card so it is visibly a separate task from the list of
     providers below it, rather than more of the same page. */
  /* No card. It owns the whole pane now, so a frame would only inset it from the
     window edge for no reason. */
  .provider-form {
    margin: 0;
    width: 100%;
  }

  /* The picker is the one part that keeps an outline, because it scrolls: a
     boundary is what tells you the list continues past what is shown. */
  .provider-form .picker {
    border-top: 1px solid var(--line);
    margin-top: var(--gap-lg);
    padding-top: var(--gap-md);
  }

  .form-head h2 {
    margin: 0;
  }

  .form-head {
    align-items: baseline;
    display: flex;
    gap: var(--gap-md);
    justify-content: space-between;
    margin-bottom: var(--gap-md);
  }

  .picker {
    margin-top: 10px;
  }

  .picker-head {
    align-items: center;
    display: flex;
    gap: 8px;
    justify-content: space-between;
  }

  .picker-tools {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
  }

  .search {
    flex: 1;
    min-width: 0;
  }

  .checkbox.tight {
    font-size: 11px;
    white-space: nowrap;
  }

  .picker-bulk {
    display: flex;
    gap: 12px;
    margin-top: 6px;
  }

  /* Text buttons: these are shortcuts, not the actions the form is for, and giving
     them the same weight as Save would misrepresent what matters here. */
  .link {
    background: none;
    border: none;
    color: AccentColor;
    font-size: 11px;
    padding: 0;
  }

  .link:hover:not(:disabled) {
    background: none;
    text-decoration: underline;
  }

  /* Capped and scrolling. A provider that serves three hundred models must not make
     the settings window three hundred rows tall. */
  .model-list {
    border: 1px solid var(--line);
    border-radius: var(--radius-control);
    list-style: none;
    margin: 8px 0 0;
    max-height: 15em;
    overflow-y: auto;
    padding: 4px 0;
  }

  .model-list li {
    padding: 1px 8px;
  }

  .model-list li:hover {
    background: var(--surface-hover);
  }

  .model-name {
    font-family: ui-monospace, monospace;
    font-size: 11px;
    /* A long model id wraps rather than stretching the card sideways — OpenRouter
       ids like "meta-llama/llama-3.3-70b-instruct:free" are routinely this long. */
    overflow-wrap: anywhere;
  }

  .perm-row {
    align-items: baseline;
    display: flex;
    gap: var(--gap-sm);
    margin-bottom: var(--gap-sm);
  }

  /* The permission states, using the same tone scale as the capability tiers so a
     colour means one thing across the whole app. Always beside a written label. */
  .badge.granted {
    background: var(--tone-good);
  }

  .badge.denied,
  .badge.restricted {
    background: var(--tone-bad);
  }

  .badge.not-asked {
    background: var(--tone-neutral);
  }

  .speech-models {
    list-style: none;
    margin: var(--gap-sm) 0 0;
    padding: 0;
  }

  .speech-models li {
    border-top: 1px solid var(--line);
    padding: var(--gap-md) 0;
  }

  .speech-models li:first-child {
    border-top: none;
  }

  /* The name and its trade-off are one target: choosing a model is the action, and a
     separate radio would be a smaller thing to hit for the same result. */
  .speech-choice {
    align-items: baseline;
    background: none;
    border: none;
    color: inherit;
    display: flex;
    flex: 1;
    font: inherit;
    gap: 6px;
    min-width: 0;
    padding: 0;
    text-align: left;
  }

  .speech-choice:hover {
    background: none;
  }

  .speech-choice.selected strong {
    color: AccentColor;
  }

  .bar {
    background: var(--line);
    border-radius: var(--radius-control);
    height: 5px;
    margin-top: var(--gap-sm);
    overflow: hidden;
  }

  /* Before the first byte, the bar has nothing to measure. A sliding stripe says
     "working" without claiming a position it does not know. */
  .bar.indeterminate .bar-fill {
    animation: sliding 1.1s ease-in-out infinite;
    width: 40%;
  }

  @keyframes sliding {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .bar.indeterminate .bar-fill {
      animation: none;
      width: 100%;
      opacity: 0.4;
    }
  }

  .bar-fill {
    background: AccentColor;
    height: 100%;
    /* Eased, because progress arriving in 256 kB chunks would otherwise step visibly
       rather than move. */
    transition: width 200ms linear;
  }

  /* The capability matrix. A table because it is one: models down, capabilities
     across, and a cell answers "can this model do this". */
  .matrix {
    border-collapse: collapse;
    font-size: 11px;
    margin-top: 8px;
    width: 100%;
  }

  .matrix th {
    font-weight: 500;
    opacity: var(--muted);
    padding: var(--gap-xs) var(--gap-sm);
    text-align: left;
    white-space: nowrap;
  }

  .matrix td {
    border-top: 1px solid var(--line);
    padding: var(--gap-xs) var(--gap-sm);
    vertical-align: middle;
  }

  .matrix tr.selected td {
    background: var(--surface-active);
  }

  .mark {
    text-align: center;
    /* Fixed width so the columns do not shift when a dash becomes a tick. */
    width: 2.4em;
  }

  .tier {
    border-radius: var(--radius-control);
    padding: 1px 5px;
    white-space: nowrap;
  }

  /* Colour carries the same ranking as the tier itself, but never alone: the
     label is always spelled out beside it, since a colour-only signal excludes
     anyone who cannot distinguish them and says nothing on a screenshot. */
  .tier.agentic {
    background: var(--tone-good);
  }

  .tier.heuristic {
    background: var(--tone-warn);
  }

  .tier.text-only {
    background: var(--line-edge);
  }

  .tier.unreachable {
    background: var(--tone-bad);
  }

  .untested {
    opacity: var(--muted);
  }

  .why td {
    border-top: none;
    opacity: var(--muted);
    padding-top: 0;
  }

  .test {
    font-size: 11px;
    padding: 2px 8px;
    white-space: nowrap;
  }

  /* Wide and tall enough that the label does not move when it swaps between the
     shortcut and the prompt to press one — a control that changes size on click
     reads as a glitch. */
  .capture {
    margin-top: 4px;
    min-width: 15em;
    padding: 7px 11px;
    text-align: center;
  }

  .capture.listening {
    border-color: AccentColor;
    box-shadow: 0 0 0 3px var(--focus-ring);
  }

  .capture kbd {
    background: none;
    font-size: 14px;
    padding: 0;
  }

  .capture .prompt {
    opacity: var(--muted-strong);
  }

  .path {
    display: block;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    opacity: var(--muted-strong);
    overflow-wrap: anywhere;
  }
</style>
