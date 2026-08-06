---
name: svelte-5
description: Use when writing or reviewing any Svelte component in Magi's frontend. Svelte 5 runes replace Svelte 4 stores and `$:` reactivity; most training data and search results still show Svelte 4 patterns that are wrong here.
---

# Svelte 5 in Magi

Magi's frontend is **Svelte 5 + TypeScript + Vite**. Svelte 5 uses runes. Do not write Svelte 4.

## Runes replace the old reactivity

| Svelte 4 (WRONG here) | Svelte 5 (CORRECT) |
|---|---|
| `let count = 0` (implicitly reactive) | `let count = $state(0)` |
| `$: double = count * 2` | `const double = $derived(count * 2)` |
| `$: { sideEffect(count) }` | `$effect(() => { sideEffect(count) })` |
| `export let title` | `let { title } = $props()` |
| `writable(0)` + `$store` | `$state` in a `.svelte.ts` module |
| `<slot />` | `{@render children()}` with `$props()` snippets |

```svelte
<script lang="ts">
  let { title, onDismiss }: { title: string; onDismiss: () => void } = $props();

  let count = $state(0);
  const double = $derived(count * 2);

  $effect(() => {
    if (count > 5) onDismiss();
  });
</script>
```

## Why this matters beyond syntax

`$state` is explicit, so reactivity **survives refactoring out of component top-level scope**. In Svelte 4, moving a `let` into a helper function silently killed its reactivity. In Svelte 5 you can put `$state` in a plain `.svelte.ts` module and it stays reactive — this is how Magi shares conversation state across the panel and settings windows without a store library.

```ts
// src/lib/conversation.svelte.ts   <- the .svelte.ts extension is REQUIRED for runes
export const conversation = $state({
  turns: [] as Turn[],
  status: 'idle' as 'idle' | 'listening' | 'thinking',
});
```

Forgetting the `.svelte.ts` extension is the most common Svelte 5 mistake: runes in a plain `.ts` file are a compile error.

## `$derived` vs `$effect`

Reach for `$derived` first. `$effect` is for escaping to the outside world (DOM measurement, Tauri IPC, timers) — **never** for computing state from other state. Writing `$state` inside an `$effect` that reads it creates an infinite loop, and Svelte 5 will warn but the design is already wrong.

Use `$derived.by(() => {...})` when the computation needs a function body rather than an expression.

## Project conventions

- **Arrow functions everywhere** (user preference, global). `const handleClick = () => {...}`, not `function handleClick()`.
- TypeScript on every component: `<script lang="ts">`.
- Props destructured with explicit types, never `any`.
- Keep components small. The overlay panel is the only stateful component; everything else is presentational.

## Tauri IPC from Svelte

```ts
import { invoke } from '@tauri-apps/api/core'; // v2 path — NOT '@tauri-apps/api/tauri' (that's v1)
import { listen } from '@tauri-apps/api/event';

const reply = await invoke<string>('send_turn', { text });

const unlisten = await listen<string>('magi://token', (e) => {
  // streaming tokens from the Rust orchestrator
});
```

Import path trap: v1 was `@tauri-apps/api/tauri`, v2 is `@tauri-apps/api/core`.

Wrap `listen` cleanup in `$effect`:

```svelte
<script lang="ts">
  $effect(() => {
    let unlisten: (() => void) | undefined;
    listen('magi://token', handleToken).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  });
</script>
```
