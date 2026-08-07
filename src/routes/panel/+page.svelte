<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import MagiSpinner from "$lib/MagiSpinner.svelte";
  import { cancelTurn, getConfig, onTurnEvents, sendTextTurn } from "$lib/ipc";
  import { renderMarkdown } from "$lib/markdown";
  import {
    appendThinking,
    appendToken,
    cancelStream,
    conversation,
    failTurn,
    finishTurn,
    historyForRequest,
    reset,
    startTurn,
  } from "$lib/conversation.svelte";

  let input = $state("");
  let thread = $state<HTMLElement | null>(null);
  let showThinking = $state(false);
  let expanded = $state<Set<number>>(new Set());

  const busy = $derived(conversation.streaming !== null);
  /** The request is away but nothing has come back yet. */
  const waiting = $derived(conversation.streaming === "" && !conversation.thinking);

  $effect(() => {
    let stop: (() => void) | undefined;
    onTurnEvents({
      token: appendToken,
      thinking: appendThinking,
      done: finishTurn,
      error: failTurn,
    }).then((off) => (stop = off));
    return () => stop?.();
  });

  // The panel reads its own copy of the setting: Settings is a separate window,
  // and this one is not reloaded when that one changes.
  $effect(() => {
    getConfig()
      .then((config) => (showThinking = config.appearance.show_thinking))
      .catch(() => {});
  });

  // Follow the answer as it grows. Reading `streaming` is what subscribes this
  // effect to every token.
  $effect(() => {
    if (conversation.streaming !== null && thread) {
      thread.scrollTop = thread.scrollHeight;
    }
  });

  const send = async () => {
    const text = input.trim();
    if (!text || busy) return;

    // Read history before pushing this turn, or the question arrives twice: once
    // as history and once as itself.
    const history = historyForRequest();
    input = "";
    startTurn(text);

    try {
      await sendTextTurn(text, history);
    } catch (e) {
      failTurn(String(e));
    }
  };

  /** Stops the turn and resolves the local state.
   *
   *  Both halves are needed. The backend abort cannot emit a completion — an
   *  aborted task is gone — so nothing would ever clear `streaming` and the panel
   *  would sit showing Stop with no way to type again. */
  const stop = async () => {
    await cancelTurn().catch(() => {});
    cancelStream();
  };

  const dismiss = async () => {
    // Stop before hiding: a request left running would keep streaming into a
    // panel nobody is looking at, and still cost tokens.
    if (busy) await stop();
    await getCurrentWindow().hide();
  };

  const toggleThinking = (index: number) => {
    const next = new Set(expanded);
    if (next.has(index)) next.delete(index);
    else next.add(index);
    expanded = next;
  };

  const onKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape") dismiss();
  };

  const onInputKeydown = (event: KeyboardEvent) => {
    // Enter sends, Shift+Enter breaks the line. This is for questions, not for
    // composing paragraphs.
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
    }
  };
</script>

<svelte:window onkeydown={onKeydown} />

<!--
  The blur behind this panel is drawn by macOS, not CSS. `backdrop-filter` blurs
  what is behind the element *within the page*, and the webview cannot see the
  desktop — the OS composites that outside the renderer. So the panel declares
  `windowEffects` in tauri.conf.json and stays translucent to let it through.
-->
<div class="panel">
  <header data-tauri-drag-region>
    <span class="mark" data-tauri-drag-region>magi</span>
    {#if conversation.turns.length > 0}
      <button type="button" class="ghost" onclick={reset}>Clear</button>
    {/if}
  </header>

  <div class="thread" bind:this={thread}>
    {#each conversation.turns as turn, i (i)}
      <div class="bubble-row {turn.role}">
        <div class="bubble {turn.role}">
          {#if turn.role === "assistant" && turn.thinking && showThinking}
            <button type="button" class="disclosure" onclick={() => toggleThinking(i)}>
              {expanded.has(i) ? "▾" : "▸"} reasoning
            </button>
            {#if expanded.has(i)}
              <p class="reasoning">{turn.thinking}</p>
            {/if}
          {/if}

          {#if turn.role === "assistant"}
            <!-- Safe because the renderer cannot emit HTML. See lib/markdown.ts. -->
            <div class="md">{@html renderMarkdown(turn.content)}</div>
          {:else}
            <!-- Own words stay literal. Nobody typing a question into a one-line
                 box means it as markup, and formatting it would silently eat the
                 asterisks and underscores out of what was actually asked. -->
            <p class="content">{turn.content}</p>
          {/if}
        </div>
      </div>
    {/each}

    {#if conversation.streaming !== null}
      <div class="bubble-row assistant">
        <div class="bubble assistant">
          {#if showThinking && conversation.thinking}
            <p class="reasoning live">{conversation.thinking}</p>
          {/if}

          {#if waiting}
            <MagiSpinner />
          {:else}
            <div class="md streaming">{@html renderMarkdown(conversation.streaming)}</div>
          {/if}
        </div>
      </div>
    {/if}

    {#if conversation.notice}
      <p class="notice">{conversation.notice}</p>
    {/if}

    {#if conversation.error}
      <p class="failure" role="alert">{conversation.error}</p>
    {/if}

    {#if conversation.turns.length === 0 && conversation.streaming === null && !conversation.error}
      <p class="empty">Ask something. Enter sends, Shift+Enter for a new line.</p>
    {/if}
  </div>

  <div class="composer">
    <textarea
      bind:value={input}
      onkeydown={onInputKeydown}
      placeholder="Ask Magi…"
      rows="1"
    ></textarea>
    {#if busy}
      <button type="button" onclick={stop}>Stop</button>
    {/if}
  </div>
</div>

<style>
  .panel {
    /* A light scrim over the OS material: enough to hold text contrast against a
       bright desktop, not so much that it defeats the blur underneath.

       The radius must match `windowEffects.radius` in tauri.conf.json. The OS
       rounds the blurred material; this element paints the scrim on top of it.
       If only one of the two is rounded, the corners disagree — a square scrim
       over rounded material leaves four tabs of scrim with no blur behind them,
       which is visible as a lighter patch in each corner. */
    background: rgba(12, 12, 16, 0.3);
    border-radius: 14px;
    box-sizing: border-box;
    color: #f4f4f5;
    display: flex;
    flex-direction: column;
    font: 13px/1.5 -apple-system, BlinkMacSystemFont, sans-serif;
    height: 100vh;
    /* Keeps children from painting over the rounded corners. */
    overflow: hidden;
    padding: 12px 14px;
  }

  header {
    align-items: center;
    cursor: default;
    display: flex;
    justify-content: space-between;
    /* Dragging a window by its title should not select the title. */
    user-select: none;
  }

  .mark {
    font-size: 10px;
    letter-spacing: 0.16em;
    opacity: 0.4;
    text-transform: uppercase;
  }

  .thread {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 8px;
    margin: 10px 0;
    min-height: 0;
    overflow-y: auto;
    /* Room for the scrollbar to sit in. Without it the overlay scrollbar lands
       on top of the bubbles' right edge, and on a right-aligned bubble it lands
       on the text. */
    padding-right: 8px;
    /* Copying an answer is the most likely thing wanted from this area, so it
       stays selectable while the draggable header does not. */
    user-select: text;
  }

  .bubble-row {
    display: flex;
  }

  .bubble-row.user {
    justify-content: flex-end;
  }

  .bubble {
    border-radius: 12px;
    max-width: 82%;
    /* A table or a long unbroken token must not stretch the bubble past the
       panel; it scrolls or wraps inside instead. */
    min-width: 0;
    padding: 7px 10px;
  }

  /* Own words on the right, the agent's on the left — the arrangement every chat
     uses, so it needs no explaining. */
  .bubble.user {
    background: rgba(255, 255, 255, 0.14);
    border-bottom-right-radius: 4px;
  }

  .bubble.assistant {
    background: rgba(255, 255, 255, 0.06);
    border-bottom-left-radius: 4px;
  }

  .content {
    margin: 0;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .disclosure {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font: inherit;
    font-size: 10px;
    letter-spacing: 0.06em;
    opacity: 0.45;
    padding: 0 0 4px;
    text-transform: uppercase;
  }

  .disclosure:hover {
    opacity: 0.8;
  }

  .reasoning {
    border-left: 2px solid rgba(255, 255, 255, 0.16);
    font-size: 12px;
    margin: 0 0 7px;
    opacity: 0.55;
    padding-left: 8px;
    white-space: pre-wrap;
  }

  /* Reasoning arriving live is capped: it is often longer than the answer, and
     letting it push the reply off screen would defeat the point of showing it. */
  .reasoning.live {
    max-height: 7em;
    overflow-y: auto;
  }

  /* ---- Rendered markdown ------------------------------------------------
     Selectors here are `:global` because this markup comes from
     `renderMarkdown` at runtime, so Svelte's compiler never sees the elements
     and would scope the classes off them. The `.md` wrapper is the boundary:
     everything is written as a descendant of it, so none of it leaks. */

  .md {
    overflow-wrap: anywhere;
  }

  .md :global(> :first-child) {
    margin-top: 0;
  }

  .md :global(> :last-child) {
    margin-bottom: 0;
  }

  .md :global(p),
  .md :global(ul),
  .md :global(ol),
  .md :global(blockquote),
  .md :global(pre) {
    margin: 0 0 0.55em;
  }

  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4) {
    font-size: 1em;
    font-weight: 600;
    margin: 0.7em 0 0.3em;
  }

  .md :global(ul),
  .md :global(ol) {
    padding-left: 1.25em;
  }

  .md :global(li) {
    margin: 0.15em 0;
  }

  .md :global(strong) {
    font-weight: 600;
  }

  .md :global(blockquote) {
    border-left: 2px solid rgba(255, 255, 255, 0.2);
    opacity: 0.75;
    padding-left: 8px;
  }

  .md :global(code) {
    background: rgba(255, 255, 255, 0.1);
    border-radius: 3px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.92em;
    padding: 1px 4px;
  }

  .md :global(pre) {
    background: rgba(0, 0, 0, 0.28);
    border-radius: 6px;
    overflow-x: auto;
    padding: 7px 9px;
  }

  /* A code block keeps its own whitespace, so it must not inherit the wrapping
     that the surrounding prose wants. */
  .md :global(pre code) {
    background: none;
    overflow-wrap: normal;
    padding: 0;
    white-space: pre;
  }

  .md :global(hr) {
    border: none;
    border-top: 1px solid rgba(255, 255, 255, 0.14);
    margin: 0.7em 0;
  }

  .md :global(.md-table) {
    margin: 0 0 0.55em;
    overflow-x: auto;
  }

  .md :global(table) {
    border-collapse: collapse;
    font-size: 0.95em;
  }

  .md :global(th),
  .md :global(td) {
    border: 1px solid rgba(255, 255, 255, 0.16);
    padding: 3px 7px;
    text-align: left;
    /* Cells wrap only where the text allows; the wrapper scrolls instead of the
       table collapsing into unreadable columns. */
    white-space: nowrap;
  }

  .md :global(th) {
    background: rgba(255, 255, 255, 0.07);
    font-weight: 600;
  }

  /* Links are text, not anchors — see lib/markdown.ts. Underlined so the model's
     intent still reads, dimmed alongside the destination so a mismatch between
     the two is visible. */
  .md :global(.md-link) {
    text-decoration: underline;
    text-decoration-color: rgba(255, 255, 255, 0.4);
  }

  .md :global(.md-url) {
    font-size: 0.85em;
    margin-left: 3px;
    opacity: 0.5;
  }

  /* The caret belongs at the end of the last line of text, but the markdown is
     injected as a block of elements, so there is no place in the template to put
     a sibling — one would land on its own line below the paragraph. A pseudo
     element on the last child sits inline at the end of that child's text
     instead, which is where a cursor goes. */
  .md.streaming :global(> :last-child)::after {
    animation: blink 1.1s steps(2, start) infinite;
    background: currentColor;
    content: "";
    display: inline-block;
    height: 1em;
    margin-left: 2px;
    vertical-align: text-bottom;
    width: 2px;
  }

  @keyframes blink {
    50% {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .md.streaming :global(> :last-child)::after {
      animation: none;
    }
  }

  .notice,
  .empty {
    font-size: 12px;
    margin: 0;
    opacity: 0.45;
  }

  .failure {
    background: rgba(220, 70, 70, 0.16);
    border-left: 2px solid rgb(230, 90, 90);
    border-radius: 3px;
    font-size: 12px;
    margin: 0;
    overflow-wrap: anywhere;
    padding: 7px 9px;
  }

  .composer {
    align-items: flex-end;
    display: flex;
    gap: 7px;
  }

  textarea {
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 7px;
    color: inherit;
    field-sizing: content;
    flex: 1;
    font: inherit;
    max-height: 30vh;
    padding: 7px 9px;
    resize: none;
  }

  textarea::placeholder {
    color: rgba(244, 244, 245, 0.35);
  }

  textarea:focus {
    border-color: rgba(255, 255, 255, 0.28);
    outline: none;
  }

  button {
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 6px;
    color: inherit;
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    padding: 6px 10px;
  }

  button:hover {
    background: rgba(255, 255, 255, 0.16);
  }

  button.ghost {
    background: none;
    border: none;
    font-size: 11px;
    opacity: 0.5;
    padding: 2px 4px;
  }

  button.ghost:hover {
    background: none;
    opacity: 0.9;
  }
</style>
