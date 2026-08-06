---
name: tauri-v2
description: Use when writing or reviewing any Tauri code in Magi — tray icon, global shortcuts, overlay windows, sidecars, IPC commands, capabilities/permissions, or bundling. Tauri v2 APIs differ incompatibly from v1 and search results mix the two; this skill pins the correct ones.
---

# Tauri v2 in Magi

Magi targets **Tauri v2**. Every snippet below is verified against the v2 docs.

## The v1/v2 trap

Search results and model memory mix v1 and v2 APIs freely. They are not compatible.

| Concern | v1 (WRONG) | v2 (CORRECT) |
|---|---|---|
| Tray | `SystemTray`, `SystemTrayMenu`, `CustomMenuItem`, `SystemTrayEvent` | `TrayIconBuilder`, `MenuBuilder`, `MenuItemBuilder`, `TrayIconEvent` |
| Tray handle | `app.tray_handle_by_id("main")` | `TrayIconBuilder::with_id(...)`, `app.tray_by_id(...)` |
| Shell / sidecar | `tauri::api::process::Command` | `tauri_plugin_shell::ShellExt` |
| Global shortcut | `app.global_shortcut_manager()` | `tauri_plugin_global_shortcut::GlobalShortcutExt` |
| Permissions | `tauri.conf.json > allowlist` | `capabilities/*.json > permissions` |

If you find yourself writing `SystemTray::new()` or `allowlist`, stop — you are writing v1.

## Capabilities are mandatory

v2 replaced the v1 allowlist with capability files. **A plugin that is registered in Rust but missing from a capability file will fail at runtime, not at compile time.** This is the single most common source of "the plugin does nothing" bugs.

`src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-capability",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "global-shortcut:allow-register",
    "global-shortcut:allow-unregister",
    "global-shortcut:allow-is-registered"
  ]
}
```

Rule: every time you add a plugin, add its permissions in the same commit.

## Tray icon

```rust
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

tauri::Builder::default()
    .setup(|app| {
        let toggle = MenuItemBuilder::with_id("toggle", "Toggle").build(app)?;
        let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
        let menu = MenuBuilder::new(app).items(&[&toggle, &quit]).build()?;

        TrayIconBuilder::new()
            .menu(&menu)
            .show_menu_on_left_click(false) // left click = open panel, right click = menu
            .on_menu_event(|app, event| match event.id().as_ref() {
                "quit" => app.exit(0),
                "toggle" => { /* toggle panel */ }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let app = tray.app_handle();
                    if let Some(w) = app.get_webview_window("panel") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            })
            .build(app)?;
        Ok(())
    })
```

**Keep running when the window closes.** A tray app must not exit on window close. Set `"windows": [{ "visible": false }]` in config and intercept close to hide instead of destroy. On macOS also set the activation policy to `Accessory` so no Dock icon appears.

## Global shortcut

```rust
use tauri_plugin_global_shortcut::GlobalShortcutExt;

tauri::Builder::default()
    .plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, shortcut, event| {
                // IMPORTANT: fires on BOTH press and release.
                // Filter on event.state() or you will toggle twice per keypress.
            })
            .build(),
    )
    .setup(|app| {
        app.global_shortcut().register("CmdOrCtrl+Y")?;
        Ok(())
    })
```

Gotchas:
- The handler fires for press **and** release. Always filter by state.
- Registration fails silently if another app owns the combo. Surface the error to the user in Settings; never assume success.
- macOS needs Accessibility permission before global shortcuts work reliably.

## Sidecars (Piper TTS, whisper.cpp server)

Declare in `tauri.conf.json`:

```json
{ "bundle": { "externalBin": ["binaries/piper"] } }
```

Binaries must be suffixed with the target triple on disk (`piper-aarch64-apple-darwin`) but referenced by **bare name**:

```rust
use tauri_plugin_shell::{ShellExt, process::CommandEvent};

let (mut rx, mut child) = app.shell().sidecar("piper")?.spawn()?;

tauri::async_runtime::spawn(async move {
    while let Some(event) = rx.recv().await {
        if let CommandEvent::Stdout(bytes) = event {
            // handle line
        }
    }
});
child.write(b"text to speak\n")?;
```

Long-running sidecars with bidirectional stdin/stdout are supported and are the intended pattern. Always kill children on app exit — Tauri does not reap them for you.

## Overlay window for the Magi panel

The panel window needs: `transparent: true`, `decorations: false`, `alwaysOnTop: true`, `skipTaskbar: true`, `visible: false` (shown on hotkey).

On macOS, `transparent: true` requires `macOSPrivateApi: true` in config. This is acceptable for a self-distributed open-source app but **blocks Mac App Store distribution** — a deliberate tradeoff for Magi.

## Checklist before committing Tauri code

- [ ] Every registered plugin has matching entries in a capability file
- [ ] Global shortcut handler filters press vs release
- [ ] Window close hides instead of exits
- [ ] Spawned sidecars are killed on app exit
- [ ] No v1 API names (`SystemTray`, `allowlist`, `tauri::api::*`)
