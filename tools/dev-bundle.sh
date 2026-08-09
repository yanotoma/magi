#!/usr/bin/env bash
#
# Builds a runnable Magi.app for testing the permissions that only work from a bundle.
#
# `npm run tauri dev` runs a bare Mach-O with no bundle: no Info.plist bound, and a
# linker-signed identity derived from the binary's hash. macOS TCC identifies applications
# by their code signature, so from its point of view that binary is an anonymous executable
# with a different name after every compile — which is why Magi never appears in System
# Settings › Privacy & Security when run with `tauri dev`, and why granting a permission to
# it would not survive the next `cargo build`.
#
# Exactly the same mechanism as the keychain problem in CLAUDE.md, which is worth noticing:
# a permission keyed to a code signature breaks on every rebuild, whether that permission is
# a keychain ACL or Screen Recording.
#
# `tauri build` produces a bundle but does *not* re-sign it — the copied binary keeps its
# linker-signed identity and the Info.plist stays unbound. The `codesign` pass below is what
# makes the identity `dev.magi.app` and binds the plist, which is what lets macOS list the
# app under its own name.
#
# Ad-hoc, not Developer ID. That is enough for TCC to name the app; it is not enough to
# distribute, and it does not promise that a grant survives a rebuild. Real signing is M7.

set -euo pipefail

cd "$(dirname "$0")/.."

APP="src-tauri/target/debug/bundle/macos/Magi.app"
IDENTIFIER="$(python3 -c '
import json, pathlib
print(json.loads(pathlib.Path("src-tauri/tauri.conf.json").read_text())["identifier"])
')"

echo "==> Building the debug bundle"
# --debug because a release build compiles whisper.cpp from scratch and takes minutes;
# --bundles app because the DMG is not needed to test a permission.
npm run tauri build -- --debug --bundles app

echo "==> Signing as $IDENTIFIER"
# --deep is deprecated for distribution signing but correct here: the bundle has no nested
# code of its own, and it makes the one-liner work whether or not that changes.
codesign --force --deep --sign - --identifier "$IDENTIFIER" "$APP"

echo "==> Signature"
codesign -dv "$APP" 2>&1 | grep -E "^(Identifier|Signature|Info.plist)"

cat <<EOF

==> Built $APP

Run it, and note that the running instance is what matters:

  1. Quit any Magi started by \`npm run tauri dev\` — two instances fight over the hotkey.
  2. open "$APP"
  3. Settings › Screen › "Ask macOS for permission". Magi should now appear in the list.
  4. Turn it on, then QUIT AND REOPEN Magi. macOS does not hand the permission to a
     process that is already running, and skipping this looks exactly like the grant
     having done nothing.
  5. Settings › Screen › "Take a test screenshot".
EOF
