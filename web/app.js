/* ── Config ── */
const isLocal = ["localhost", "127.0.0.1"].includes(window.location.hostname);
const defaults = {
  api: isLocal ? "http://127.0.0.1:8080" : window.location.origin,
  ws: isLocal ? "ws://127.0.0.1:8081" : `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`,
};
const STORAGE_KEY = "baals-cfg-v2";

function loadCfg() {
  try { const c = JSON.parse(localStorage.getItem(STORAGE_KEY)); return { api: c.api || defaults.api, ws: c.ws || defaults.ws }; }
  catch { return { ...defaults }; }
}
function saveCfg(c) { localStorage.setItem(STORAGE_KEY, JSON.stringify(c)); }

let cfg = loadCfg();

/* ── DOM refs ── */
const $ = (id) => document.getElementById(id);
const el = {
  statusDot: $("statusDot"), statusText: $("statusText"),
  heroHeight: $("heroHeight"), heroSupply: $("heroSupply"), heroPeers: $("heroPeers"), heroValidators: $("heroValidators"),
  tickerHash: $("tickerHash"), tickerUptime: $("tickerUptime"), tickerMempool: $("tickerMempool"), tickerVersion: $("tickerVersion"),
  searchForm: $("searchForm"), searchType: $("searchType"), searchInput: $("searchInput"), searchResults: $("searchResults"),
  blocksBody: $("blocksBody"),
  netValidatorCount: $("netValidatorCount"), netValidatorList: $("netValidatorList"),
  netPeerCount: $("netPeerCount"), netPeerList: $("netPeerList"),
  ciStorage: $("ciStorage"), ciMemory: $("ciMemory"),
  cfgApi: $("cfgApi"), cfgWs: $("cfgWs"), cfgSave: $("cfgSave"),
  wsConnect: $("wsConnect"), wsDisconnect: $("wsDisconnect"), wsClear: $("wsClear"), eventLog: $("eventLog"),
};

/* ── Helpers ── */
async function api(path) {
  const r = await fetch(`${cfg.api}${path}`, { headers: { Accept: "application/json" } });
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
  return r.json();
}

function short(h, w = 8) {
  if (!h || typeof h !== "string") return "—";
  return h.length <= w * 2 + 3 ? h : `${h.slice(0, w)}…${h.slice(-w)}`;
}

function fmtNum(n) { return n == null ? "—" : Number(n).toLocaleString(); }

function fmtTime(ts) {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const now = Date.now();
  const diff = Math.floor((now - d.getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return d.toISOString().slice(0, 16).replace("T", " ");
}

function fmtUptime(s) {
  if (!s) return "—";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function bytesToHex(arr) {
  if (!Array.isArray(arr)) return String(arr || "");
  return arr.map(b => b.toString(16).padStart(2, "0")).join("");
}

/* ── Navigation ── */
function doSearch(type, value) {
  el.searchType.value = type;
  el.searchInput.value = value;
  el.searchForm.dispatchEvent(new Event("submit", { cancelable: true }));
  document.getElementById("explorer").scrollIntoView({ behavior: "smooth" });
}

function makeLink(text, type, value) {
  const a = document.createElement("a");
  a.className = "hash-link";
  a.textContent = text;
  a.title = value;
  a.href = "#";
  a.onclick = (e) => { e.preventDefault(); doSearch(type, value); };
  return a;
}

/* ── Status ── */
function setStatus(ok) {
  el.statusDot.className = ok ? "pulse ok" : "pulse error";
  el.statusText.textContent = ok ? "Mainnet" : "Offline";
}

/* ── Data loading ── */
async function refresh() {
  try {
    const [health, supply, latestBlock, signers, peers] = await Promise.all([
      api("/api/v1/health"),
      api("/api/v1/supply").catch(() => null),
      api("/api/v1/blocks/latest").catch(() => null),
      api("/api/v1/admin/signers").catch(() => null),
      api("/api/v1/peers").catch(() => null),
    ]);

    setStatus(true);

    const height = health.latest_block_index ?? 0;
    el.heroHeight.textContent = fmtNum(height);
    el.heroSupply.textContent = supply ? fmtNum(supply.total_supply) : "—";
    el.heroPeers.textContent = String(health.connected_peers ?? 0);

    const sigTotal = signers?.total ?? 0;
    el.heroValidators.textContent = String(sigTotal);

    el.tickerHash.textContent = short(health.latest_block_hash, 12);
    el.tickerUptime.textContent = fmtUptime(health.uptime_seconds);
    el.tickerMempool.textContent = String(health.mempool_size ?? 0);
    el.tickerVersion.textContent = health.version || "—";

    el.ciStorage.textContent = health.storage_healthy ? "Healthy" : "Degraded";
    el.ciMemory.textContent = health.memory_usage_mb ? `${health.memory_usage_mb.toFixed(0)} MB` : "—";

    // Validators
    el.netValidatorCount.textContent = String(sigTotal);
    el.netValidatorList.innerHTML = "";
    if (signers) {
      if (signers.primary) {
        const d = document.createElement("div");
        d.className = "net-list-item primary";
        d.textContent = signers.primary;
        el.netValidatorList.appendChild(d);
      }
      (signers.additional || []).forEach(pk => {
        const d = document.createElement("div");
        d.className = "net-list-item";
        d.textContent = pk;
        el.netValidatorList.appendChild(d);
      });
    }

    // Peers (filter out localhost/loopback — internal topology)
    const peerArr = (peers?.peers || []).filter(p => !/^(127\.|localhost|::1)/.test(p));
    el.netPeerCount.textContent = String(peerArr.length);
    el.netPeerList.innerHTML = "";
    peerArr.forEach(p => {
      const d = document.createElement("div");
      d.className = "net-list-item";
      d.textContent = p;
      el.netPeerList.appendChild(d);
    });

    await loadBlocks(height);
  } catch (e) {
    setStatus(false);
    console.error("refresh failed", e);
  }
}

async function loadBlocks(latestHeight) {
  const start = Math.max(0, latestHeight - 9);
  const heights = [];
  for (let h = latestHeight; h >= start; h--) heights.push(h);

  const blocks = await Promise.all(heights.map(h => api(`/api/v1/blocks/${h}`).catch(() => null)));
  el.blocksBody.innerHTML = "";

  blocks.filter(Boolean).forEach(b => {
    const tr = document.createElement("tr");
    const hash = b.hash || "—";
    const txCount = Array.isArray(b.transactions) ? b.transactions.length : (b.tx_count ?? 0);

    const tdH = document.createElement("td");
    tdH.appendChild(makeLink(String(b.height ?? b.index ?? "—"), "block-height", String(b.height ?? b.index)));

    const tdHash = document.createElement("td");
    tdHash.appendChild(makeLink(short(hash, 10), "block-hash", hash));

    const tdTime = document.createElement("td");
    tdTime.textContent = fmtTime(b.timestamp);

    const tdTx = document.createElement("td");
    tdTx.textContent = String(txCount);

    tr.append(tdH, tdHash, tdTime, tdTx);
    el.blocksBody.appendChild(tr);
  });
}

/* ── Search ── */
function detectType(val) {
  if (/^\d+$/.test(val)) return "block-height";
  if (val.length === 64 && /^[0-9a-fA-F]+$/.test(val)) return "tx-hash"; // could be block or tx hash
  return "account";
}

async function runSearch(e) {
  e.preventDefault();
  const raw = el.searchInput.value.trim();
  if (!raw) return;

  let type = el.searchType.value;
  if (type === "auto") type = detectType(raw);

  const endpoints = {
    "block-height": `/api/v1/blocks/${encodeURIComponent(raw)}`,
    "block-hash": `/api/v1/blocks/hash/${encodeURIComponent(raw)}`,
    "tx-hash": `/api/v1/transactions/${encodeURIComponent(raw)}`,
    "tx-address": `/api/v1/transactions/address/${encodeURIComponent(raw)}?limit=25`,
    account: `/api/v1/accounts/${encodeURIComponent(raw)}`,
  };

  el.searchResults.innerHTML = "";

  try {
    const data = await api(endpoints[type]);
    renderResult(type, data);
  } catch (err) {
    // For auto-detect hash, try block hash if tx hash failed
    if (type === "tx-hash") {
      try {
        const data = await api(`/api/v1/blocks/hash/${encodeURIComponent(raw)}`);
        renderResult("block-hash", data);
        return;
      } catch {}
    }
    const div = document.createElement("div");
    div.className = "result-error";
    div.textContent = `Not found: ${err.message}`;
    el.searchResults.appendChild(div);
  }
}

function renderResult(type, data) {
  if (type === "tx-address" && Array.isArray(data)) {
    if (data.length === 0) {
      el.searchResults.innerHTML = '<div class="result-error">No transactions found for this address.</div>';
      return;
    }
    data.forEach(tx => el.searchResults.appendChild(buildTxCard(tx)));
    return;
  }
  if (type === "block-height" || type === "block-hash") {
    el.searchResults.appendChild(buildBlockCard(data));
    return;
  }
  if (type === "tx-hash") {
    el.searchResults.appendChild(buildTxCard(data));
    return;
  }
  if (type === "account") {
    el.searchResults.appendChild(buildAccountCard(data));
    return;
  }
}

function card(typeLabel, rows, extra) {
  const div = document.createElement("div");
  div.className = "result-card";
  const lbl = document.createElement("div");
  lbl.className = "result-type";
  lbl.textContent = typeLabel;
  div.appendChild(lbl);

  const grid = document.createElement("div");
  grid.className = "result-grid";
  rows.forEach(([k, v]) => {
    const l = document.createElement("span");
    l.className = "result-label";
    l.textContent = k;
    grid.appendChild(l);

    const r = document.createElement("span");
    r.className = "result-value";
    if (v instanceof HTMLElement) { r.appendChild(v); }
    else if (typeof v === "number") { r.className = "result-value num"; r.textContent = fmtNum(v); }
    else { r.textContent = String(v ?? "—"); }
    grid.appendChild(r);
  });
  div.appendChild(grid);
  if (extra) div.appendChild(extra);
  return div;
}

function buildBlockCard(b) {
  const hash = b.hash || "—";
  const txs = Array.isArray(b.transactions) ? b.transactions : [];

  const rows = [
    ["Height", b.height ?? b.index ?? "—"],
    ["Hash", hash],
    ["Parent", b.parentHash ? makeLink(short(b.parentHash, 10), "block-hash", b.parentHash) : "—"],
    ["Time", fmtTime(b.timestamp)],
    ["Transactions", txs.length],
  ];

  let txSection = null;
  if (txs.length > 0) {
    txSection = document.createElement("div");
    txSection.className = "result-txlist";
    const h5 = document.createElement("h5");
    h5.textContent = "Transactions";
    txSection.appendChild(h5);
    txs.forEach(tx => {
      const txHash = tx.hash || tx.hash_hex || bytesToHex(tx.hash) || "—";
      const from = tx.from || tx.sender_hex || bytesToHex(tx.sender) || "?";
      const to = tx.to || "?";
      const val = tx.value != null ? tx.value : "—";

      const row = document.createElement("div");
      row.style.cssText = "display:flex; gap:1rem; align-items:center; padding:0.3rem 0; font-size:0.82rem;";
      row.appendChild(makeLink(short(txHash, 8), "tx-hash", txHash));

      const info = document.createElement("span");
      info.className = "result-value";
      info.style.fontSize = "0.78rem";
      info.textContent = `${short(from, 6)} → ${short(to, 6)}  ·  ${val}`;
      row.appendChild(info);
      txSection.appendChild(row);
    });
  }

  return card("Block", rows, txSection);
}

function buildTxCard(tx) {
  const hash = tx.hash_hex || bytesToHex(tx.hash) || "—";
  const sender = tx.sender_hex || tx.from || bytesToHex(tx.sender) || "—";

  let recipientStr = "—";
  const r = tx.recipient || {};
  if (tx.to) recipientStr = tx.to;
  else if (r.Wallet) recipientStr = bytesToHex(r.Wallet);
  else if (r.Contract) recipientStr = bytesToHex(r.Contract?.id || r.Contract);

  let payloadStr = "—";
  const p = tx.payload;
  if (p) {
    if (p.Transfer) payloadStr = `Transfer ${fmtNum(p.Transfer.amount)}`;
    else if (p.ContractDeploy) payloadStr = "Contract Deploy";
    else if (p.ContractCall) payloadStr = `Call: ${p.ContractCall.method || "?"}`;
    else if (p.Data) payloadStr = `Data (${(p.Data.data || []).length} bytes)`;
    else payloadStr = JSON.stringify(p);
  }
  if (tx.value != null && payloadStr === "—") payloadStr = `Transfer ${tx.value}`;

  const rows = [
    ["Hash", hash],
    ["From", makeLink(short(sender, 10), "account", sender)],
    ["To", makeLink(short(recipientStr, 10), "account", recipientStr)],
    ["Payload", payloadStr],
    ["Nonce", tx.nonce ?? "—"],
    ["Time", fmtTime(tx.timestamp)],
    ["Gas", `${fmtNum(tx.gas_limit || tx.gas || 0)} limit / ${tx.gas_price ?? 0} price`],
  ];
  return card("Transaction", rows);
}

function buildAccountCard(a) {
  const pk = a.public_key || a.pubkey || "—";
  const rows = [
    ["Public Key", pk],
    ["Balance", fmtNum(a.balance ?? 0)],
    ["Nonce", a.nonce ?? 0],
  ];
  return card("Account", rows);
}

/* ── WebSocket ── */
let ws = null;

function wsConnect() {
  if (ws && ws.readyState <= 1) return;
  ws = new WebSocket(cfg.ws);
  ws.onopen = () => {
    logLine("Connected", "info");
    ["blocks", "transactions", "mempool"].forEach(ch =>
      ws.send(JSON.stringify({ type: "subscribe", channel: ch }))
    );
  };
  ws.onmessage = (e) => {
    try {
      const p = JSON.parse(e.data);
      const ch = p.channel || p.type || "event";
      logLine(`${ch}: ${JSON.stringify(p.event ?? p)}`);
    } catch {
      logLine(e.data, "warn");
    }
  };
  ws.onerror = () => logLine("Connection error", "error");
  ws.onclose = () => logLine("Disconnected", "warn");
}

function logLine(text, level = "info") {
  const d = document.createElement("div");
  d.className = `log-line${level !== "info" ? " " + level : ""}`;
  d.textContent = `${new Date().toLocaleTimeString()} · ${text}`;
  el.eventLog.prepend(d);
  while (el.eventLog.children.length > 150) el.eventLog.removeChild(el.eventLog.lastChild);
}

/* ── Init ── */
function init() {
  el.cfgApi.value = cfg.api;
  el.cfgWs.value = cfg.ws;

  el.cfgSave.onclick = () => {
    cfg = { api: el.cfgApi.value.replace(/\/+$/, "") || defaults.api, ws: el.cfgWs.value.replace(/\/+$/, "") || defaults.ws };
    saveCfg(cfg);
    refresh();
  };

  el.searchForm.addEventListener("submit", runSearch);
  el.wsConnect.onclick = wsConnect;
  el.wsDisconnect.onclick = () => { if (ws) { ws.close(); ws = null; } };
  el.wsClear.onclick = () => { el.eventLog.innerHTML = ""; };

  refresh();
  setInterval(refresh, 15000);
}

init();
