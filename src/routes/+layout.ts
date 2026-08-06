// Tauri has no Node server, so there is nothing to server-render.
export const ssr = false;

// Prerendering emits a static file per route at build time rather than relying
// on an SPA fallback. A fallback is only ever exercised in the production
// bundle, which is the worst place to discover a window's URL does not resolve.
export const prerender = true;

// Directory-style URLs: /panel/ resolves to build/panel/index.html in production
// and to the same path under `vite dev`. Keeping dev and prod in agreement is
// the point — a mismatch here surfaces only at bundle time.
//
// This is a page option, not a `kit` config option. `kit.trailingSlash` was
// removed from svelte.config.js and setting it there is a hard build error.
export const trailingSlash = "always";
