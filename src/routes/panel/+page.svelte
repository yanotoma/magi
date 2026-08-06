<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";

  const dismiss = async () => {
    await getCurrentWindow().hide();
  };

  const onKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape") dismiss();
  };
</script>

<svelte:window onkeydown={onKeydown} />

<div class="panel">
  <header>magi</header>
  <p>Shell only. There is no intelligence behind this yet.</p>
</div>

<style>
  .panel {
    /* The window itself is transparent — this is what makes the panel visible.
       Rounded corners and the border are ours to draw because decorations are
       off; nothing else is painting a frame. */
    background: rgba(18, 18, 22, 0.82);
    backdrop-filter: blur(24px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 14px;
    box-sizing: border-box;
    color: #f4f4f5;
    font: 14px/1.5 -apple-system, BlinkMacSystemFont, sans-serif;
    height: 100vh;
    padding: 16px 18px;
  }

  header {
    font-size: 11px;
    letter-spacing: 0.14em;
    opacity: 0.45;
    text-transform: uppercase;
  }

  p {
    margin: 10px 0 0;
    opacity: 0.7;
  }
</style>
