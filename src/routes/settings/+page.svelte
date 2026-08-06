<script lang="ts">
  const shortcut = "Alt+Space";
</script>

<main>
  <h1>Magi Settings</h1>

  <section>
    <h2>Hotkey</h2>
    <p>Toggle panel: <kbd>{shortcut}</kbd></p>
    <p class="note">Editing the hotkey arrives in M2, alongside the config file.</p>
  </section>
</main>

<style>
  main {
    /* Opaque: only the panel window declares transparency. Canvas/CanvasText are
       the system colours, so this follows the OS light/dark setting for free. */
    background: Canvas;
    box-sizing: border-box;
    color: CanvasText;
    font: 14px/1.6 -apple-system, BlinkMacSystemFont, sans-serif;
    min-height: 100vh;
    padding: 28px 32px;
  }

  h1 {
    font-size: 20px;
    margin: 0 0 24px;
  }

  h2 {
    font-size: 12px;
    letter-spacing: 0.08em;
    margin: 0 0 8px;
    opacity: 0.55;
    text-transform: uppercase;
  }

  kbd {
    background: rgba(128, 128, 128, 0.18);
    border-radius: 4px;
    font-family: ui-monospace, monospace;
    padding: 2px 6px;
  }

  .note {
    font-size: 13px;
    opacity: 0.6;
  }
</style>
