# BaaLS Web

This folder is the project web app for `baals.online`. It includes:

- A public website shell for BaaLS.
- A built-in block explorer.
- Live stream support over the BaaLS WebSocket API.

## Features

- Configurable API endpoint (`/api/v1/*`).
- Configurable WebSocket endpoint (`ws://...`).
- Chain snapshot (health, height, latest hash, tx count).
- Latest blocks table.
- Search tools for:
  - block by height
  - block by hash
  - transaction by hash
  - account by public key
  - transactions by address
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

## Deploy to baals.online

Any static host works (Cloudflare Pages, Netlify, Vercel static, S3+CloudFront, etc.).

Deploy the contents of this `web/` folder as a static site root and point `baals.online` DNS to the selected host.

If the BaaLS node API is on a different origin than the website domain, configure CORS/reverse proxy rules so browser requests to `/api/v1/*` and WS traffic are allowed.
