#!/usr/bin/env bash
set -uo pipefail

HEALTH_URLS=("http://127.0.0.1:18082/health" "http://127.0.0.1:18092/health")
NODE_NAMES=("node1" "node2")
LOG="/var/log/baals-monitor.log"

STATE_DIR="${BAALS_MONITOR_STATE_DIR:-/var/lib/baals-monitor}"
ALERT_COOLDOWN_SECONDS="${BAALS_MONITOR_ALERT_COOLDOWN_SECONDS:-900}"
WEBHOOK_URL="${BAALS_MONITOR_WEBHOOK_URL:-}"
EMAIL_TO="${BAALS_MONITOR_EMAIL_TO:-}"
EMAIL_SUBJECT_PREFIX="${BAALS_MONITOR_EMAIL_SUBJECT_PREFIX:-[BaaLS Monitor]}"
HOST_NAME="$(hostname -f 2>/dev/null || hostname)"

mkdir -p "$STATE_DIR"

log() {
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $*" >> "$LOG"
    logger -t baals-monitor "$*"
}

json_escape() {
    local input="$1"
    printf '%s' "$input" | jq -Rsa .
}

send_webhook() {
    local level="$1"
    local message="$2"

    [ -z "$WEBHOOK_URL" ] && return 0

    local ts payload
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    payload=$(cat <<EOF
{
  "source": "baals-monitor",
  "host": "$HOST_NAME",
  "timestamp": "$ts",
  "level": "$level",
  "message": $(json_escape "$message")
}
EOF
)

    curl -fsS --max-time 8 \
        -H "Content-Type: application/json" \
        -d "$payload" \
        "$WEBHOOK_URL" >/dev/null 2>&1 || log "WARN webhook delivery failed level=$level message=$message"
}

send_email() {
    local level="$1"
    local message="$2"

    [ -z "$EMAIL_TO" ] && return 0

    local mail_bin=""
    if command -v mail >/dev/null 2>&1; then
        mail_bin="mail"
    elif command -v mailx >/dev/null 2>&1; then
        mail_bin="mailx"
    fi

    if [ -z "$mail_bin" ]; then
        log "WARN email requested but mail/mailx not found"
        return 0
    fi

    local subject body
    subject="$EMAIL_SUBJECT_PREFIX [$level] $HOST_NAME"
    body="$(date -u +%Y-%m-%dT%H:%M:%SZ)\n$message"
    printf '%b\n' "$body" | "$mail_bin" -s "$subject" "$EMAIL_TO" || log "WARN email delivery failed level=$level message=$message"
}

notify() {
    local level="$1"
    local message="$2"
    send_webhook "$level" "$message"
    send_email "$level" "$message"
}

safe_key() {
    printf '%s' "$1" | tr -cs 'A-Za-z0-9._-' '_'
}

read_state() {
    local state_file="$1"
    local default_last=0
    local default_active=0

    if [ -f "$state_file" ]; then
        # shellcheck disable=SC1090
        source "$state_file"
        echo "${last_ts:-$default_last}" "${active:-$default_active}"
    else
        echo "$default_last" "$default_active"
    fi
}

write_state() {
    local state_file="$1"
    local last_ts="$2"
    local active="$3"
    cat > "$state_file" <<EOF
last_ts=$last_ts
active=$active
EOF
}

raise_alert() {
    local key="$1"
    local message="$2"

    local file now state last_ts active
    file="$STATE_DIR/$(safe_key "$key").state"
    now=$(date +%s)
    state=( $(read_state "$file") )
    last_ts="${state[0]}"
    active="${state[1]}"

    local since=$(( now - last_ts ))

    if [ "$active" -eq 0 ] || [ "$since" -ge "$ALERT_COOLDOWN_SECONDS" ]; then
        log "ALERT $message"
        notify "ALERT" "$message"
        write_state "$file" "$now" 1
    else
        log "ALERT_SUPPRESSED (cooldown=${ALERT_COOLDOWN_SECONDS}s) $message"
    fi
}

clear_alert() {
    local key="$1"
    local message="$2"

    local file state active
    file="$STATE_DIR/$(safe_key "$key").state"
    state=( $(read_state "$file") )
    active="${state[1]}"

    if [ "$active" -eq 1 ]; then
        log "RECOVERY $message"
        notify "RECOVERY" "$message"
    fi

    write_state "$file" 0 0
}

heights=()
for i in "${!HEALTH_URLS[@]}"; do
    name="${NODE_NAMES[$i]}"
    url="${HEALTH_URLS[$i]}"

    resp=$(curl -sf --max-time 5 "$url" 2>/dev/null) || {
        raise_alert "${name}_unreachable" "[$name] unreachable at $url"
        continue
    }

    clear_alert "${name}_unreachable" "[$name] reachable again"

    status=$(echo "$resp" | jq -r .status)
    height=$(echo "$resp" | jq -r .latest_block_index)
    peers=$(echo "$resp" | jq -r .connected_peers)
    uptime=$(echo "$resp" | jq -r .uptime_seconds)
    storage=$(echo "$resp" | jq -r .storage_healthy)

    if [ "$status" != "healthy" ]; then
        raise_alert "${name}_status" "[$name] status=$status"
    else
        clear_alert "${name}_status" "[$name] status back to healthy"
    fi

    if [ "$storage" != "true" ]; then
        raise_alert "${name}_storage" "[$name] storage unhealthy"
    else
        clear_alert "${name}_storage" "[$name] storage healthy"
    fi

    log "OK [$name] height=$height peers=$peers uptime=${uptime}s"
    heights+=("$height")
done

if [ "${#heights[@]}" -eq 2 ]; then
    diff=$(( ${heights[0]} - ${heights[1]} ))
    abs=${diff#-}
    if [ "$abs" -gt 5 ]; then
        raise_alert "height_divergence" "height divergence: ${heights[0]} vs ${heights[1]}"
    else
        clear_alert "height_divergence" "height divergence recovered: ${heights[0]} vs ${heights[1]}"
    fi
fi

exit 0
