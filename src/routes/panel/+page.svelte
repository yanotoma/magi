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

<!--
  The blur behind this panel is drawn by macOS, not by CSS. `backdrop-filter`
  cannot help here: it blurs what is behind the element *within the page*, and
  the webview has no access to what is behind the window — the desktop is
  composited by the OS, outside the renderer. So the panel declares
  `windowEffects` in tauri.conf.json and stays transparent to let it show.

  The corner radius is set there too, for the same reason: rounding this element
  in CSS would leave the OS material square behind rounded content.
-->
<div class="panel">
  <!--
    No title bar to grab, so the header is the drag handle. The attribute
    applies only to the element it is on — child elements need their own.
  -->
  <header data-tauri-drag-region>magi</header>
  <p>Shell only. There is no intelligence behind this yet.</p>
</div>

<style>
  .panel {
    /* A light scrim over the OS material: enough to hold text contrast against
       a bright desktop, not so much that it defeats the blur underneath. */
    background: rgba(12, 12, 16, 0.28);
    box-sizing: border-box;
    color: #f4f4f5;
    font: 14px/1.5 -apple-system, BlinkMacSystemFont, sans-serif;
    height: 100vh;
    padding: 16px 18px;
  }

  header {
    cursor: default;
    font-size: 11px;
    letter-spacing: 0.14em;
    opacity: 0.45;
    text-transform: uppercase;
    /* Dragging a window by text selects the text on the way. */
    user-select: none;
  }

  p {
    margin: 10px 0 0;
    opacity: 0.7;
  }
</style>
