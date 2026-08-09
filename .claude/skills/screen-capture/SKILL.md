---
name: screen-capture
description: Use when writing or reviewing Magi's screen capture — display and window enumeration, the capture API choice, Screen Recording permission, Retina scaling, or downscaling before a vision request. The obvious capture API is obsoleted as of macOS 15, it degrades silently rather than failing when permission is absent, and "1568 pixels" is not the image budget it looks like.
---

# Screen capture on macOS

Everything here was verified on macOS 15.5 against SDK headers or by running code. Where
something is inferred rather than measured it says so.

## The capture API is obsoleted, but not dead

`CGWindowListCreateImage` is what almost every example and every current Rust crate uses.
This machine's SDK marks it, and `CGWindowListCreateImageFromArray`, as:

```c
#define SCREEN_CAPTURE_OBSOLETE(x,y,z) \
    __attribute__((availability(macos,introduced=x,deprecated=y,obsoleted=z, \
                   message="Please use ScreenCaptureKit instead.")));

CG_EXTERN CGImageRef __nullable CGWindowListCreateImage(...)
    SCREEN_CAPTURE_OBSOLETE(10.5,14.0,15.0);
```

`obsoleted=15.0` means the symbol is **unavailable to compile against when the deployment
target is 15.0 or above**. It does not mean the implementation was removed. Magi builds
with `MACOSX_DEPLOYMENT_TARGET=11.0`, so it compiles, and measured on macOS 15.5 it still
runs:

```
Calling CGWindowListCreateImage(CGRectInfinite, kCGWindowListOptionOnScreenOnly, ...)
  Result: NON-NULL CGImageRef @ 0x116e058f0
  Dimensions: 9904x5040
```

`CGDisplayCreateImage` carries the same marking and also still runs.

**Do not read "it works today" as "it is fine".** Obsoleted is the last stop before
removal, and a capture implementation built on it is one that gets rewritten. That is a
scheduling question, not a correctness one — decide it deliberately.

`CGPreflightScreenCaptureAccess` and `CGRequestScreenCaptureAccess` are in the same header
and are **not** deprecated: plain `API_AVAILABLE(macos(10.15))`. Using them is safe.

## Without permission it returns an image, not an error

This is the most important fact in this file.

Measured on macOS 15.5 with `CGPreflightScreenCaptureAccess()` returning `0`:
`CGWindowListCreateImage` still returned a **valid, non-null 9904×5040 image**. Not NULL,
not an error — a real composite with other applications' window contents missing.

So the failure mode is a screenshot of a desktop with nothing on it, handed to a model that
then answers confidently about a screen it effectively never saw. This is the same shape as
the vision-probe bug recorded in `llm-providers`: the thing that looked like it worked was
answering a question nobody asked.

**The only reliable guard is checking permission before capturing.** There is nothing in
the returned image to test for — a genuinely empty desktop and a permission-denied capture
look alike.

ScreenCaptureKit behaves differently and better here: it fails with a real error,
`SCStreamErrorUserDeclined = -3801`, verified in `SCError.h`.

Related trap, documented by Apple in `SCShareableContent.h` (macOS 14.4): content belonging
to **the current process is capturable without TCC consent**. A developer who tests capture
against Magi's own panel window will see it work perfectly and never discover the
permission problem.

## `tauri dev` cannot hold this permission at all

Measured, after Magi failed to appear in System Settings at all.

`npm run tauri dev` runs a bare Mach-O with no bundle:

```
Executable=.../target/debug/magi
Identifier=magi-c8c1c0812cc6eba8
Signature=adhoc, linker-signed
Info.plist=not bound
```

TCC identifies applications by their **code signature**, and that identifier is derived from
the binary's contents. Every `cargo build` produces a different one, so from TCC's point of
view each compile is a different anonymous executable. There is also no bound `Info.plist`,
so macOS has no name to list it under.

This is the same mechanism as the keychain problem in `CLAUDE.md` — *"the ACL is tied to the
binary's signature and `cargo` produces a new binary on each compile"* — and it was written
down there for the keychain without anyone noticing it applies to every TCC permission
equally.

**`tauri build` alone does not fix it.** The bundle it produces keeps the copied binary's
linker-signed identity and leaves the plist unbound:

```
Identifier=magi-6610ce2c77f94cc5   ← not dev.magi.app
Info.plist=not bound
```

An explicit `codesign` pass is what makes the identity stable and binds the plist:

```sh
codesign --force --deep --sign - --identifier dev.magi.app Magi.app
# Identifier=dev.magi.app
# Info.plist entries=15
```

`tools/dev-bundle.sh` does the build and the signing together. Ad-hoc signing is enough for
macOS to name the app; it is not enough to distribute, and it does not promise a grant
survives a rebuild — real signing is M7.

**Practical consequence: never debug a permission problem from `tauri dev`.** The symptom
there is indistinguishable from a bug in the permission code.

## Screen Recording permission has only two states

Unlike the microphone. `CGPreflightScreenCaptureAccess` returns a bare `bool`, and there is
no equivalent of `AVAuthorizationStatus::NotDetermined` — "never asked" and "explicitly
denied" are the same value.

That is not a gap in the binding. macOS never shows an in-app prompt for screen recording
the way it does for the microphone; it sends the user to System Settings. Both states lead
to the same instruction, so both map to `Permission::Denied`. `Permission::Restricted` is
unreachable too: Apple documents no managed state for it, and a bool could not carry one.

**There is no `NS*UsageDescription` key for Screen Recording.** Do not add one by analogy
with `NSMicrophoneUsageDescription`. Grepping the SDK headers for `NSScreenCapture`,
`NSScreenRecording` and `kTCCScreen` returns nothing; macOS supplies the prompt text
itself. A missing microphone key kills the process silently — there is no equivalent
failure here because there is no key.

`CGRequestScreenCaptureAccess` opens System Settings as a side effect. Call it when a user
asks for it, never while drawing a settings pane.

Deep link, unchanged across the Ventura System Settings rewrite:

```
x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture
```

## Granting requires a restart

A grant does not apply to the already-running process. Granting and carrying on looks
exactly like the grant having done nothing, which sends the user back to System Settings to
do it again.

Say so explicitly. For a tray app with no window, the surface for saying it is the tray
menu or a notification — there is nothing else on screen.

*(The mechanism — that TCC is resolved once into the process's context with no re-check
path — is inference, not something Apple documents. The behaviour itself is consistent from
10.15 through 15.x.)*

## Logical points are not pixels

The single easiest mistake in this area.

`xcap`'s `Monitor::width()`/`height()` come from `CGDisplayBounds` and are **logical
points**. `capture_image()` returns **physical pixels**. On a MacBook Pro at default
scaling that is 1512 versus 3024 — code that sizes a buffer from the former gets a quarter
of what the platform hands back.

`scale_factor()` is `CGDisplayMode::pixel_width / CGDisplayBounds::width`, so 2.0 on a
Retina display.

**Round, do not truncate**, when deriving pixels from points. Scale arrives as a float and
`1512 × 1.9999998` truncates to 3023 — one row short, which shears every row progressively
and produces an image that looks like a slightly skewed screenshot rather than like a bug.

## Vision cost: 1568 is not a pixel budget

Anthropic's documented arithmetic, which is what `capture/downscale.rs` reproduces:

```
visual tokens = ceil(width / 28) * ceil(height / 28)      one token per 28×28 patch
standard tier: maxEdge = 1568 px, maxTokens = 1568
high tier:     maxEdge = 2576 px, maxTokens = 4784
```

**The token cap binds long before the edge cap.** At 16:10 the budget is reached around
1372×882 — comfortably under 1568 on both axes, and still resized. 1400×900 is 1650 tokens.
A square at the edge limit is 3136, over twice the budget. Anthropic states it directly: the
token limit is the primary constraint and the edge limit only bites on elongated images,
which in practice means ultrawide monitors.

**Downscaling saves bytes, not tokens.** The server resizes anything over the cap *before*
charging, so a 3024×1964 Retina capture and a 1568-token thumbnail cost the same tokens —
the extra pixels are uploaded and then discarded. What they cost is bandwidth, and the
design's own argument that history is resent every turn applies to those bytes too. The
second reason to resize client-side is that resizing twice blurs.

**Round the short edge half to even.** The live API uses ties-to-even; Rust's `f64::round`
rounds ties away from zero. They disagree only on exact `.5` ties, and an image sized by the
wrong rule is resized again on arrival — it arrives blurrier because of a rounding rule.
Use `f64::round_ties_even`. Anthropic publishes `1075×1520 → 924×1307` as a reference
vector; assert it, and drift from their implementation fails loudly.

The design doc's "roughly 1,100 vision tokens" for a 1512×982 screenshot is 1944 under this
formula. The conclusion it draws survives; the number does not.

## Choosing the capture backend

| | `xcap` 0.9.8 | `objc2-screen-capture-kit` 0.3.2 |
|---|---|---|
| Underlying API | `CGWindowListCreateImage` — obsoleted 15.0 | ScreenCaptureKit — Apple's replacement |
| Minimum macOS | 10.10, covers Magi's 11.0 | 12.3; `SCScreenshotManager` one-shot is 14.0+ |
| No permission | **returns a degraded image, no error** | real error, `SCStreamErrorUserDeclined` |
| Shape | synchronous, blocking | completion handlers, async |
| Family | `objc2` internally since 0.9 | the `objc2` family already in the tree |
| Longevity | rewrite pending | current |

`xcap` returns `image::RgbaImage` from `image ^0.25` — it swaps the BGRA that CoreGraphics
returns into RGBA itself, and trims macOS's row padding, both confirmed in its source. It
enables only the `png` feature of `image`, so `.save("x.jpg")` panics at runtime. `Monitor`
is `Send + Sync`; `Window` is `Send` but **not** `Sync`. `friendly_name()` calls
`MainThreadMarker::new_unchecked()` and is unsound off the main thread even though it
appears to work.

Either backend belongs behind `[target.'cfg(target_os = "macos")'.dependencies]`. The trait
and its fake stay cross-platform, which is what lets the whole crate build and test on a
Linux CI runner with no display server.

## Test what cannot be tested with a display

No test may require a screen. What is testable without one is most of what matters:

- which display was chosen, and whether the primary one wins
- logical-to-physical conversion, including a scale factor that is not exactly 2.0
- that a capture is downscaled before it is encoded
- that the encoded bytes decode back to the stated dimensions
- that the words in a transcript did or did not point at the screen

Encode with the `png` crate already in the tree rather than adding `image`, for the reason
`probe_image.rs` gives: one encoder, and a subtly malformed PNG is the most expensive bug
this area produces — accepted by the HTTP layer, misread by every model, and reported to
the user as "this model cannot see". Round-trip it in a test; asserting the bytes are
non-empty would pass for a truncated file.

Check the pixel buffer length in `usize` with `checked_mul`. In `u32`, `width * height * 4`
wraps for large dimensions, and a wrapped expected length can match a short buffer — so the
check passes and the encoder reads past the end.

## Checklist

- [ ] Permission is checked with `CGPreflightScreenCaptureAccess` **before** every capture,
      not only at startup
- [ ] A capture taken without permission is never sent to a model — there is nothing in the
      image to detect it by
- [ ] The user is told a restart is required after granting
- [ ] No `NS*UsageDescription` key was added for Screen Recording
- [ ] Physical pixels are derived from logical points by rounding, never truncating
- [ ] Downscaling happens before encoding, and the short edge rounds half to even
- [ ] The capture call runs in `spawn_blocking`
- [ ] Enumeration and capture sit behind a trait with a fake, and no test needs a display
- [ ] Capture was tested against another application's window, not only Magi's own — the
      app's own windows need no permission and will pass regardless
- [ ] Any permission problem was reproduced from a signed `.app` (`tools/dev-bundle.sh`),
      never from `tauri dev`, whose unbundled binary cannot hold a TCC grant at all
