---
name: macos-permissions
description: Use when working on anything that touches macOS TCC permissions in Magi — microphone, screen recording, accessibility — or on Info.plist entitlements, code signing, notarization, and the DMG. These fail in ways that produce no error and no log entry, so the handling has to be designed, not discovered.
---

# macOS permissions and distribution for Magi

Magi needs three separate TCC permissions. Each has a different request mechanism, a different failure mode, and a different recovery path. Getting this wrong produces an app that silently does nothing.

## The three permissions

| Permission | Needed for | Requested at | If denied |
|---|---|---|---|
| **Microphone** | Voice input | First hotkey activation | Clean API error; recoverable without restart |
| **Screen Recording** | `capture_screen` | First tool-initiated capture | **Requires an app restart after granting** |
| **Accessibility** | Reliable global hotkeys; `enigo` in v3 | First launch | No prompt for some APIs — must be checked and surfaced |

## The failure mode that defines the design

**Screen Recording cannot be granted at runtime.** macOS shows the prompt, the user grants it in System Settings, and the running process still does not have it — the permission takes effect for the *next* launch. Worse, `xcap` does not fail loudly when it lacks permission: on some macOS versions it returns a capture of the desktop wallpaper with all windows missing, which looks like a successful capture of an empty screen.

Two consequences, both non-negotiable:

1. **Check permission state explicitly before capturing**, rather than inferring it from whether the capture call returned bytes.
2. **The UI must offer a restart**, not just a link to System Settings. A user who grants permission and sees nothing change concludes the app is broken.

Accessibility has a similar shape: some APIs prompt, others just fail silently. Always check state rather than trusting a call to error.

## Design rules

**Request lazily, at first genuine use.** Prompting for three permissions on first launch reads as invasive for an app that is supposed to be privacy-first, and the user has no context for why any of them are needed. Ask when the feature is first used, with the reason visible.

**Read the state, do not infer it.** `AVCaptureDevice.authorizationStatusForMediaType` answers without prompting, so Settings can show what is true instead of finding out when a feature quietly fails. In Rust that is `objc2-av-foundation` — the objc2 family is already in the tree via Tauri and cpal, so it costs one aligned dependency, against hand-rolled `msg_send!` where a wrong selector or return type is undefined behaviour in `unsafe` code.

The state has **four** values, not two, and collapsing them misleads:

| State | Why it is distinct |
|---|---|
| `NotDetermined` | Never asked. The **intended** first-run path, not a failure — wording it as one has every new user believing something is broken before they have done anything |
| `Authorized` | Working |
| `Denied` | Only the user can change it, in System Settings. Say so, **and** that Magi must be reopened: macOS does not re-check a permission for a running process, so granting it and carrying on looks like the grant did nothing |
| `Restricted` | A configuration profile on a managed Mac. Pointing at System Settings here sends someone to a toggle they cannot move |

**Every permission gets a live status row in Settings**, with current state and a button that opens the correct System Settings pane:

```
x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone
x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture
x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility
```

**Degrade with a visible explanation, never silently.** A missing permission produces a panel banner naming the permission and what stopped working. Silent degradation in a tray app is indistinguishable from a crash.

## The keychain is a fourth permission, and it behaves differently

TCC is not the only thing that prompts. The keychain has its own access-control model, and it caught this project out once already.

**Never call the keychain from the main thread.** `keyring`'s `get_password` looks like a cheap getter and is a synchronous Mach round trip to `securityd`. When the ACL does not already permit this exact binary, `securityd` blocks until the user answers an access dialog — and the main thread is the only thread that can present and service that dialog. So the main thread ends up waiting for an answer to a prompt only it could have drawn. That is a permanent deadlock, not a pause.

It is worse in Magi than in a normal app, for a reason specific to this design: `ActivationPolicy::Accessory` means there is no Dock icon, so the user cannot even bring the stuck prompt to the front. The whole symptom is a spinning cursor over a dead tray icon and a hotkey that does nothing. No panic, no log line, no error.

Two rules follow:

- A **synchronous `#[tauri::command] fn` runs on the main thread.** Any command that can reach the keychain must be `async` and route the call through `commands::with_secrets`, which puts it on a blocking task.
- **Never read the keychain in a startup path.** Windows created hidden at launch — Magi's panel — issue their first IPC before anything is on screen. A keychain read there means asking for access with no window to attach the prompt to. Give such callers a command that touches no secrets (`get_appearance`), rather than the whole config.

**"Always Allow" does not stick across a rebuild.** The ACL is keyed to the binary's code signature, and `cargo build` produces a new binary every time, so a development build is a different application to macOS on each compile. Repeated prompts in `tauri dev` are expected and are not a bug to chase. A signed, notarised release build has one stable signature, so the user is asked once per install.

Because of that, **keep the number of reads low rather than relying on the ACL**: cache the derived value (Magi caches the key *fingerprint*, never the secret) and read lazily, only when a screen actually needs it.

## Info.plist usage descriptions

Required, and they are user-visible in the prompt. Write them for the user, not for the linter:

```xml
<key>NSMicrophoneUsageDescription</key>
<string>Magi listens only while you hold the hotkey, so you can ask questions out loud.</string>
```

Screen Recording has no `NS*UsageDescription` key — macOS uses a system-supplied string. This is exactly why the in-app explanation matters: it is the only place you get to say why.

## Transparency and the App Store tradeoff

The panel is a transparent window, which on macOS requires **both** `macOSPrivateApi: true` in `tauri.conf.json` and the `macos-private-api` Cargo feature.

**This permanently blocks Mac App Store distribution.** It is a deliberate, documented decision for Magi — the project distributes a signed DMG directly. Do not "fix" it by removing transparency; the overlay design depends on it.

## Signing and notarization (M7)

The order matters, and each step fails differently:

1. **Code sign** with a Developer ID Application certificate, hardened runtime enabled.
2. **Notarize** — upload to Apple, wait for the ticket. This is where the useless error messages live: a failure returns a log URL, and the actual reason is inside that JSON, not in the CLI output. Always fetch and read the log.
3. **Staple** the ticket to the `.app`, then build the DMG.
4. **Verify** on a machine that has never seen the app: `spctl -a -vvv Magi.app` and `xcrun stapler validate`.

Common notarization rejections: an unsigned nested binary (Piper's sidecar in v2 is the likely culprit), a missing hardened runtime flag, or a `.dylib` that was not signed with the same identity.

**Verifying on the build machine proves nothing** — Gatekeeper caches provenance for locally-built apps. Test the actual downloaded DMG on a clean machine or a fresh VM.

## Sidecar signing (v2, Piper)

Every nested executable must be signed individually before the outer bundle is signed, with the same Developer ID and hardened runtime. Sign inside-out: sidecars first, then frameworks, then the app. Signing the app first and adding a binary afterwards invalidates the signature.

## Checklist before touching permission code

- [ ] Permission state is checked explicitly, never inferred from a call succeeding
- [ ] Screen Recording path offers a restart, not just a Settings link
- [ ] Every failure produces a user-visible message naming the permission
- [ ] Settings shows live status per permission
- [ ] Tested the denial path, not only the grant path
- [ ] Tested revoking permission while the app is running
