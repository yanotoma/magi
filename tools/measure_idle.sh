#!/usr/bin/env bash
#
# What Magi costs while doing nothing.
#
# The README claims Magi sits out of the way. That claim needs a number, and a number needs
# to be reproducible or the next person cannot tell whether a regression happened or the
# first measurement was taken on a busy laptop.
#
# So this is a script rather than a figure someone wrote down once.
#
# ## Two things it refuses to do
#
# **It will not measure a debug build.** A debug binary carries no optimisation and its own
# allocator behaviour; its RSS is not the shipped app's RSS, and publishing it would
# understate or overstate by an amount nobody can predict. Point this at a release build.
#
# **It will not measure a busy machine.** Idle means idle. If the load average is high the
# sample is contending with whatever else is running, and the resulting number is noise
# presented as a measurement. The check is crude on purpose — it only has to catch the case
# where someone runs this in the middle of a build, which is exactly when it is tempting to.
#
# Usage:
#   tools/measure_idle.sh              # measure whatever Magi is already running
#   SAMPLES=60 tools/measure_idle.sh   # longer window
#
# Magi must already be running and settled. Launch it, leave it alone for a minute so
# first-run work is over, then run this.

set -euo pipefail

PROCESS_NAME="Magi"
SAMPLES="${SAMPLES:-30}"
INTERVAL="${INTERVAL:-2}"

# Above this one-minute load average, refuse. Not a tuned threshold — a machine at 2.0 is
# doing something, and something is what we are trying not to measure.
MAX_LOAD="${MAX_LOAD:-2.0}"

die() {
  printf 'measure_idle: %s\n' "$1" >&2
  exit 1
}

command -v ps >/dev/null || die "no ps on PATH"

# The bundle's executable, not a cargo target directory — `pgrep -f magi` would also match
# this script, an editor with the file open, and a cargo build.
PID="$(pgrep -x "$PROCESS_NAME" 2>/dev/null | head -1 || true)"
[ -n "$PID" ] || die "Magi is not running. Launch it, let it settle for a minute, then retry."

EXE="$(ps -o comm= -p "$PID" | tr -d ' ')"
printf 'process:  %s (pid %s)\n' "$EXE" "$PID"

# Refuse a debug build. The path is the only signal available without inspecting the binary,
# and it is a reliable one: cargo and Tauri both put debug output under a `debug` directory.
FULL_PATH="$(ps -o args= -p "$PID" | awk '{print $1}')"
case "$FULL_PATH" in
  *"/target/debug/"* | *"/debug/"*)
    die "that is a debug build ($FULL_PATH).
       A debug binary's memory is not the shipped app's. Build a release bundle first:
         npm run tauri build
       then launch the app from the bundle and retry."
    ;;
esac
printf 'binary:   %s\n' "$FULL_PATH"

LOAD="$(sysctl -n vm.loadavg 2>/dev/null | awk '{print $2}')"
if [ -n "$LOAD" ]; then
  if awk -v l="$LOAD" -v m="$MAX_LOAD" 'BEGIN { exit !(l > m) }'; then
    die "one-minute load average is $LOAD, over the $MAX_LOAD ceiling.
       Idle means idle — this sample would be contending with whatever else is running.
       Wait for the machine to settle, or set MAX_LOAD if you know what you are doing."
  fi
  printf 'load avg: %s\n' "$LOAD"
fi

printf 'sampling: %s samples, %ss apart (%ss total)\n\n' "$SAMPLES" "$INTERVAL" "$((SAMPLES * INTERVAL))"

RSS_FILE="$(mktemp)"
CPU_FILE="$(mktemp)"
trap 'rm -f "$RSS_FILE" "$CPU_FILE"' EXIT

for _ in $(seq "$SAMPLES"); do
  # rss is in kilobytes; pcpu is a percentage of one core.
  if ! LINE="$(ps -o rss=,pcpu= -p "$PID" 2>/dev/null)"; then
    die "Magi exited during sampling"
  fi
  [ -n "$LINE" ] || die "Magi exited during sampling"
  printf '%s\n' "$LINE" | awk '{print $1}' >> "$RSS_FILE"
  printf '%s\n' "$LINE" | awk '{print $2}' >> "$CPU_FILE"
  sleep "$INTERVAL"
done

# Median rather than mean for both. A single scheduler blip or a tray redraw skews a mean
# over thirty samples, and the question is what Magi costs while sitting there, not what its
# worst two seconds looked like. Max is printed alongside so a spike is still visible.
summarise() {
  local file="$1" label="$2" unit="$3" scale="$4"
  sort -n "$file" | awk -v label="$label" -v unit="$unit" -v scale="$scale" '
    { v[NR] = $1 }
    END {
      median = (NR % 2) ? v[(NR + 1) / 2] : (v[NR / 2] + v[NR / 2 + 1]) / 2
      printf "%-12s median %7.1f %s   min %7.1f   max %7.1f\n",
        label, median / scale, unit, v[1] / scale, v[NR] / scale
    }
  '
}

summarise "$RSS_FILE" "memory" "MB" 1024
summarise "$CPU_FILE" "cpu" "%" 1

printf '\nPublish the median in README.md. Say which Mac and which macOS it was measured on —\n'
printf 'a number with no machine attached is not reproducible, and reproducible is the point.\n'
