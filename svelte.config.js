import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    // Tauri has no Node server, so there is nothing to server-render. Routes are
    // prerendered to real files rather than served through an SPA fallback: a
    // fallback is only ever exercised in the production bundle, which is the
    // worst place to discover that a window's URL does not resolve.
    adapter: adapter(),
  },
};

export default config;
