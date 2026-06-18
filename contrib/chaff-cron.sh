#!/usr/bin/env bash
# chaff-cron.sh — scheduled git hygiene pass for the fleet
# Survey = always. Repair = dry-run by default; real repair if ~/.config/chaff/auto-repair exists.
# Logs when something changed or errored; silent on a clean pass.
# Emits agorabus chaff.report event when bus is reachable (fail-open).
# Always exits 0.

set -uo pipefail

JOURNAL_DIR="${HOME}/brain/journal/chaff-cron"
TODAY="$(date +%Y-%m-%d)"
LOG_FILE="${JOURNAL_DIR}/${TODAY}.log"
AUTO_REPAIR_SENTINEL="${HOME}/.config/chaff/auto-repair"

log_journal() {
    mkdir -p "$JOURNAL_DIR"
    printf '%s  chaff-cron: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$LOG_FILE"
}

# Bail gracefully if chaff isn't on PATH
if ! command -v chaff &>/dev/null; then
    exit 0
fi

# --- Survey (NDJSON: one JSON object per line) ---
SURVEY_FILE="$(mktemp /tmp/chaff-survey-XXXXXX.ndjson)"
REPAIR_FILE="$(mktemp /tmp/chaff-repair-XXXXXX.json)"
trap 'rm -f "$SURVEY_FILE" "$REPAIR_FILE"' EXIT

SURVEY_EXIT=0
chaff survey --format json >"$SURVEY_FILE" 2>&1 || SURVEY_EXIT=$?

REPOS=0
FILES=0
BYTES=0
if command -v jq &>/dev/null && [[ -s "$SURVEY_FILE" ]]; then
    # NDJSON — use -s (slurp) to read all lines as an array
    REPOS=$(jq -s 'length' "$SURVEY_FILE" 2>/dev/null || echo 0)
    FILES=$(jq -s '[.[].tracked_junk] | add // 0' "$SURVEY_FILE" 2>/dev/null || echo 0)
    BYTES=$(jq -s '[.[].bytes_in_index_est] | add // 0' "$SURVEY_FILE" 2>/dev/null || echo 0)
fi

if [[ "$SURVEY_EXIT" -ne 0 ]]; then
    log_journal "survey failed (exit $SURVEY_EXIT)"
    if command -v agorabus &>/dev/null; then
        agorabus publish chaff.report '{"repos":0,"files":0,"error":"survey-failed"}' 2>/dev/null || true
    fi
    exit 0
fi

# --- Repair ---
REPAIR_EXIT=0
REPAIRED=0
ERRORS=0

if [[ -f "$AUTO_REPAIR_SENTINEL" ]]; then
    chaff repair --no-dry-run --format json >"$REPAIR_FILE" 2>&1 || REPAIR_EXIT=$?
else
    chaff repair --format json >"$REPAIR_FILE" 2>&1 || REPAIR_EXIT=$?
fi

if command -v jq &>/dev/null && [[ -s "$REPAIR_FILE" ]]; then
    if jq -e . "$REPAIR_FILE" &>/dev/null; then
        REPAIRED=$(jq '[.[] | select(.verdict == "repaired")] | length' "$REPAIR_FILE" 2>/dev/null || echo 0)
        ERRORS=$(jq '[.[] | select(.verdict == "error")] | length' "$REPAIR_FILE" 2>/dev/null || echo 0)
    fi
fi

# --- Emit agorabus event (fail-open) ---
if command -v agorabus &>/dev/null; then
    agorabus publish chaff.report \
      "{\"repos\":${REPOS},\"files\":${FILES},\"bytes\":${BYTES},\"repaired\":${REPAIRED},\"errors\":${ERRORS}}" \
      2>/dev/null || true
fi

# --- Journal only when something changed or errored ---
if [[ "$REPAIRED" -gt 0 || "$ERRORS" -gt 0 || "$REPAIR_EXIT" -ne 0 ]]; then
    DRY=""
    [[ ! -f "$AUTO_REPAIR_SENTINEL" ]] && DRY=" (dry-run)"
    MIB=$(( BYTES / 1048576 ))
    log_journal "survey: ${REPOS} repos, ${FILES} files, ${MIB} MiB; repair${DRY}: ${REPAIRED} fixed, ${ERRORS} errors"
fi

exit 0
