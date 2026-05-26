# BaaLS Web

This folder is the project web app for `baals.network`. It includes:

- A public website shell for BaaLS.
- A built-in block explorer.
- An ecosystem surfaces scaffold (BaaLS + ChronoNode + Resurgence + Canvas lanes).
- Live stream support over the BaaLS WebSocket API.

## Features

- Configurable API endpoint (`/api/v1/*`).
- Configurable WebSocket endpoint (`ws://...`).
- Chain snapshot (health, height, latest hash, tx count).
- Latest blocks table with producer attribution.
- Ecosystem surface cards with configurable external links for:
  - ChronoNode proof/archive surface
  - Resurgence protocol surface
  - Canvas artifact registry surface
- Live ecosystem status signals:
  - BaaLS oracle attestation rollup + recent trace samples
  - ChronoNode health check
  - Resurgence/Canvas reachability checks
- Search tools for:
  - block by height
  - block by hash
  - transaction by hash (with receipt/fee breakdown when available)
  - account by public key
  - transactions by address
  - contract by ID (code hash, deployer, ABI link, storage keys)
- Unified jump bar for cross-surface identifiers:
  - BaaLS tx → local search
  - EVM tx → Resurgence claims + Arbiscan/Polygonscan/Basescan deep-links
  - EVM address → Resurgence user/pool pages + ChronoNode dormancy
  - Contract ID → local search + Canvas deployment + ChronoNode archive
  - WASM hash → Canvas artifact + ChronoNode archive
  - Proof hash → ChronoNode verify + Resurgence oracle
  - CCIP message ID → Resurgence CCIP + Chainlink CCIP explorer
- Oracle Attestation table with chain filter and cross-links:
  - ChronoNode dormancy proof deep-links
  - Resurgence EVM mint tx deep-links
- Mempool panel with pending transaction cards (hash, sender, payload, gas, priority).
- Block detail cards with producer, quorum signatures, gas used, state root, and finality badge.
- Transaction detail cards with fee mode, receipt status, gas used, and operator/treasury/burn fee split.
- Account detail cards with recent transaction history (async fetch).
- Click-to-copy on all hash displays (block hashes, tx hashes, public keys) with toast notification.
- Validator panel with current proposer, quorum threshold, and round-robin status.
- Extended Chain Info panel: fee policy, WS port, storage backend, chain ID.
- Live event feed for `blocks`, `transactions`, `mempool`.

## Run Locally

From repo root:

```bash
cd web
python -m http.server 4173
```

Then open:

```text
http://127.0.0.1:4173
```

Default node endpoints expected by the explorer:

- API: `http://127.0.0.1:8080`
- WS: `ws://127.0.0.1:8081`

You can override both in the UI and save to browser local storage.

## Deploy to baals.network

Any static host works (Cloudflare Pages, Netlify, Vercel static, S3+CloudFront, etc.).

Deploy the contents of this `web/` folder as a static site root and point `baals.network` DNS to the selected host.

If the BaaLS node API is on a different origin than the website domain, configure CORS/reverse proxy rules so browser requests to `/api/v1/*` and WS traffic are allowed.

### Included Deployment Assets

- `CNAME`: custom domain target (`baals.network`) for GitHub Pages-style hosting.
- `.github/workflows/pages.yml`: GitHub Pages deploy workflow from `web/`.
- `deploy/nginx.conf`: reverse proxy + TLS + security headers example.
- `deploy/Caddyfile`: reverse proxy + security headers example.

### Production Endpoint Behavior

The app defaults are automatic:

- Localhost: API `http://127.0.0.1:8080`, WS `ws://127.0.0.1:8081`
- Non-localhost: API same-origin (`https://baals.network`), WS same-origin `/ws` (`wss://baals.network/ws`)

That means if you use the provided reverse-proxy configs, the explorer works without manual endpoint edits.
