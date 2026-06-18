#!/usr/bin/env bash
# chaff-cron.sh — scheduled git hygiene pass for the fleet
# Runs chaff survey + repair (dry-run by default, real repair if ~/.config/chaff/auto-repair exists)
# Appends a one-line summary to the daily journal ONLY when repair changed something or an error occurred.
# Silent on a clean pass. Emits agorabus chaff.report event when bus is reachable (fail-open).
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

# --- Survey ---
SURVEY_FILE="$(mktemp /tmp/chaff-survey-$(date +%s).XXXXXX.json)"
trap 'rm -f "$SURVEY_FILE" "$REPAIR_FILE"' EXIT

SURVEY_EXIT=0
chaff survey --format json >"$SURVEY_FILE" 2>&1 || SURVEY_EXIT=$?

REPOS=0
FILES=0
if command -v jq &>/dev/null && [[ -s "$SURVEY_FILE" ]]; then
    if jq -e . "$SURVEY_FILE" &>/dev/null; then
        REPOS=$(jq '[.[] | select(.tracked_build_files | length > 0)] | length' "$SURVEY_FILE" 2>/dev/null || echo 0)
        FILES=$(jq '[.[].tracked_build_files | length] | add // 0' "$SURVEY_FILE" 2>/dev/null || echo 0)
    fi
fi

if [[ "$SURVEY_EXIT" -ne 0 ]]; then
    log_journal "survey failed (exit $SURVEY_EXIT)"
    # Emit agorabus event (fail-open)
    if command -v agorabus &>/dev/null; then
        agorabus emit chaff.report "{\"repos\":0,\"files\":0,\"error\":\"survey-failed\"}" 2>/dev/null || true
    fi
    exit 0
fi

# --- Repair ---
REPAIR_FILE="$(mktemp /tmp/chaff-repair-$(date +%s).XXXXXX.json)"
REPAIR_EXIT=0
REPAIRED=0
ERRORS=0

if [[ -f "$AUTO_REPAIR_SENTINEL" ]]; then
    chaff repair --no-dry-run --format json >"$REPAIR_FILE" 2>&1 || REPAIR_EXIT=$?
else
    # Dry-run: no mutations
    chaff repair --format json >"$REPAIR_FILE" 2>&1 || REPAIR_EXIT=$?
fi

if command -v jq &>/dev/null && [[ -s "$REPAIR_FILE" ]]; then
    if jq -e . "$REPAIR_FILE" &>/dev/null; then
        REPAIRED=$(jq '[.[] | select(.result == "repaired")] | length' "$REPAIR_FILE" 2>/dev/null || echo 0)
        ERRORS=$(jq '[.[] | select(.result == "error")] | length' "$REPAIR_FILE" 2>/dev/null || echo 0)
    fi
fi

# --- Emit agorabus event (fail-open) ---
if command -v agorabus &>/dev/null; then
    agorabus emit chaff.report "{\"repos\":${REPOS},\"files\":${FILES},\"repaired\":${REPAIRED},\"errors\":${ERRORS}}" 2>/dev/null || true
fi

# --- Journal only when something changed or errored ---
if [[ "$REPAIRED" -gt 0 || "$ERRORS" -gt 0 || "$REPAIR_EXIT" -ne 0 ]]; then
    DRY=""
    [[ ! -f "$AUTO_REPAIR_SENTINEL" ]] && DRY=" (dry-run)"
    log_journal "survey: ${REPOS} repos, ${FILES} files; repair${DRY}: ${REPAIRED} fixed, ${ERRORS} errors"
fi

exit 0
