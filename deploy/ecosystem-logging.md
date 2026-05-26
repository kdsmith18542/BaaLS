# Ecosystem Pipeline Logging

Lightweight structured logging for the ChronoNode → BaaLS → Resurgence dormancy pipeline.

## Architecture

```
systemd services (baalsd, chrononode, etc.)
        ↓  journald
eco-collect (cron every 2 min)  →  /var/log/ecosystem/pipeline.jsonl
        ↓
eco-trace  →  grep search by chain, address, correlation ID, tx hash
        ↓
logrotate (weekly, 90-day retention)
```

## Files on VPS

| Path | Purpose |
|------|---------|
| `/usr/local/bin/eco-log` | Append a structured JSON event to pipeline.jsonl |
| `/usr/local/bin/eco-trace` | Search the pipeline log |
| `/usr/local/bin/eco-collect` | Extract events from journald (cron-driven) |
| `/var/log/ecosystem/pipeline.jsonl` | The unified pipeline event log |
| `/etc/logrotate.d/ecosystem` | Weekly rotation, 90-day retention, compression |
| `/etc/cron.d/ecosystem-collect` | Runs eco-collect every 2 minutes |

## Event Schema

```json
{
  "ts":             "ISO 8601 UTC",
  "level":          "info|warn|error",
  "service":        "baals|chrononode|resurgence",
  "component":      "evm_submitter|dormancy_scanner",
  "event_type":     "evm_submitter_tx_sent|evm_submitter_tx_failed|...",
  "status":         "success|failed",
  "chain":          "bitcoin-light|dogecoin|...",
  "source_address": "1A1z...",
  "evm_wallet":     "0x4206...",
  "tx_hash":        "0x9e21...",
  "correlation_id": "dormancy:<chain>:<address>",
  "message":        "optional detail"
}
```

## Quick Reference

```bash
# Trace a specific address through the full pipeline
eco-trace bitcoin-light 12c6DSiU4Rq3P4ZxziKxzrL5LmMBrzjrJX

# Filter by chain
eco-trace --chain dogecoin --tail 5

# Filter by status
eco-trace --status failed --tail 10

# Filter by event type
eco-trace --event-type evm_submitter_tx_sent --tail 5

# Search by EVM tx hash
eco-trace --tx-hash 0x9e216bba85151813660522adc314378c49059a17c3

# Search by service
eco-trace --service baals --since "2026-05-25" --tail 20

# Show all events since a date
eco-trace --since "2026-05-25T12:00:00Z" --tail 50

# Tail the log live
tail -f /var/log/ecosystem/pipeline.jsonl | grep dogecoin

# Count events by chain
grep -oP '"chain":"[^"]+"' /var/log/ecosystem/pipeline.jsonl | sort | uniq -c
```

## Manual Logging

Services and scripts can call eco-log directly for events not covered by the journald collector:

```bash
eco-log chrononode dormancy_proof_signed success \
  --chain bitcoin-light \
  --source-address 1A1z... \
  --proof-hash 0x... \
  --correlation-id "dormancy:bitcoin-light:1A1z..."
```
