<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { cancelTurn, onTurnEvents, sendTextTurn } from "$lib/ipc";
  import {
    appendToken,
    conversation,
    failTurn,
    finishTurn,
    historyForRequest,
    reset,
    startTurn,
  } from "$lib/conversation.svelte";

  let input = $state("");
  let thread = $state<HTMLElement | null>(null);

  const busy = $derived(conversation.streaming !== null);

  $effect(() => {
    let stop: (() => void) | undefined;
    onTurnEvents({
      token: appendToken,
      done: finishTurn,
      error: failTurn,
    }).then((off) => (stop = off));
    return () => stop?.();
  });

  // Follow the answer as it grows. Reading `conversation.streaming` is what
  // subscribes this effect to every token.
  $effect(() => {
    if (conversation.streaming !== null && thread) {
      thread.scrollTop = thread.scrollHeight;
    }
  });

  const send = async () => {
    const text = input.trim();
    if (!text || busy) return;

    // Read the history before pushing this turn, or the question would arrive
    // twice: once as history and once as itself.
    const history = historyForRequest();
    input = "";
    startTurn(text);

    try {
      await sendTextTurn(text, history);
    } catch (e) {
      failTurn(String(e));
    }
  };

  const dismiss = async () => {
    // Cancel before hiding. A request left running would keep streaming into a
    // panel nobody is looking at, and still cost tokens.
    await cancelTurn().catch(() => {});
    await getCurrentWindow().hide();
  };

  const onKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape") dismiss();
  };

  const onInputKeydown = (event: KeyboardEvent) => {
    // Enter sends, Shift+Enter breaks the line. The panel is for questions, not
    // for composing paragraphs.
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
  The corner radius lives there too, or the OS material would stay square behind
  rounded content.
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
      <p class="turn {turn.role}">{turn.content}</p>
    {/each}

    {#if conversation.streaming !== null}
      <p class="turn assistant streaming">
        {conversation.streaming}<span class="caret"></span>
      </p>
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
      <button type="button" onclick={cancelTurn}>Stop</button>
    {/if}
  </div>
</div>

<style>
  .panel {
    /* A light scrim over the OS material: enough to hold text contrast against a
       bright desktop, not so much that it defeats the blur underneath. */
    background: rgba(12, 12, 16, 0.3);
    box-sizing: border-box;
    color: #f4f4f5;
    display: flex;
    flex-direction: column;
    font: 13px/1.5 -apple-system, BlinkMacSystemFont, sans-serif;
    height: 100vh;
    padding: 12px 14px 12px;
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
    flex: 1;
    margin: 10px 0;
    min-height: 0;
    overflow-y: auto;
    /* Selecting an answer to copy it is the most likely thing a user wants from
       this area, so it stays selectable while the header does not. */
    user-select: text;
  }

  .turn {
    margin: 0 0 10px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .turn.user {
    opacity: 0.55;
  }

  /* A caret while tokens arrive: without it, a model that pauses mid-answer looks
     like one that has finished. */
  .caret {
    animation: blink 1.1s steps(2, start) infinite;
    background: currentColor;
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
    .caret {
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
