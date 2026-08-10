#!/usr/bin/env bash
#
# Builds a runnable Magi.app for testing the permissions that only work from a bundle.
#
# macOS TCC attributes a permission request to the *responsible process* — the nearest
# ancestor in the process tree with a bundle identifier — and not to the process that asked.
# So `magi` started by a dev server registers the dev server, and `magi` started from a
# terminal registers the terminal. Magi itself never gets a row, and if the parent already
# holds the permission then capture works while belonging to something else entirely.
#
# A bundle launched standalone has no such ancestor, so it becomes its own responsible
# process and TCC uses its CFBundleIdentifier. That is the only arrangement in which Magi
# appears under its own name — which is why this script exists and why the steps below insist
# on `open`ing the app rather than running the binary.
#
# `tauri build` produces a bundle but does not re-sign it: the copied binary keeps its
# linker-signed, hash-shaped identifier and the Info.plist stays unbound, leaving macOS with
# no name to display. The `codesign` pass below fixes that.
#
# Ad-hoc, not Developer ID. Enough for macOS to name the app, not enough to distribute.
# Real signing is M7.

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
