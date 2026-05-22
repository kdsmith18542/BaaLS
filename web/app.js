const isLocalHost =
  window.location.hostname === "localhost" ||
  window.location.hostname === "127.0.0.1";

const defaultApiBase = isLocalHost
  ? "http://127.0.0.1:8080"
  : window.location.origin;

const defaultWsBase = isLocalHost
  ? "ws://127.0.0.1:8081"
  : `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/ws`;
const configKey = "baals-web-config-v1";

const el = {
  apiBase: document.getElementById("apiBase"),
  wsBase: document.getElementById("wsBase"),
  saveConfigBtn: document.getElementById("saveConfigBtn"),
  refreshBtn: document.getElementById("refreshBtn"),
  clearLogBtn: document.getElementById("clearLogBtn"),
  statusBadge: document.getElementById("statusBadge"),
  healthValue: document.getElementById("healthValue"),
  heightValue: document.getElementById("heightValue"),
  hashValue: document.getElementById("hashValue"),
  txCountValue: document.getElementById("txCountValue"),
  supplyValue: document.getElementById("supplyValue"),
  accountsValue: document.getElementById("accountsValue"),
  peersValue: document.getElementById("peersValue"),
  signersValue: document.getElementById("signersValue"),
  peerList: document.getElementById("peerList"),
  signerList: document.getElementById("signerList"),
  searchForm: document.getElementById("searchForm"),
  queryType: document.getElementById("queryType"),
  queryValue: document.getElementById("queryValue"),
  queryOutput: document.getElementById("queryOutput"),
  blocksTableBody: document.getElementById("blocksTableBody"),
  wsConnectBtn: document.getElementById("wsConnectBtn"),
  wsDisconnectBtn: document.getElementById("wsDisconnectBtn"),
  eventLog: document.getElementById("eventLog"),
};

let ws = null;

function normalizeApiBase(raw) {
  const trimmed = (raw || "").trim().replace(/\/+$/, "");
  return trimmed || defaultApiBase;
}

function normalizeWsBase(raw) {
  const trimmed = (raw || "").trim().replace(/\/+$/, "");
  return trimmed || defaultWsBase;
}

function getConfig() {
  const fromStorage = localStorage.getItem(configKey);
  if (!fromStorage) {
    return { apiBase: defaultApiBase, wsBase: defaultWsBase };
  }
  try {
    const parsed = JSON.parse(fromStorage);
    return {
      apiBase: normalizeApiBase(parsed.apiBase),
      wsBase: normalizeWsBase(parsed.wsBase),
    };
  } catch {
    return { apiBase: defaultApiBase, wsBase: defaultWsBase };
  }
}

function setConfig(config) {
  localStorage.setItem(configKey, JSON.stringify(config));
}

function setStatus(text, mode) {
  el.statusBadge.textContent = text;
  el.statusBadge.classList.remove("status-idle", "status-ok", "status-error");
  el.statusBadge.classList.add(mode);
}

function shortHash(hash, width = 12) {
  if (!hash || typeof hash !== "string") return "-";
  if (hash.length <= width * 2) return hash;
  return `${hash.slice(0, width)}...${hash.slice(-width)}`;
}

function formatAmount(raw) {
  return Number(raw).toLocaleString();
}

function formatTime(ts) {
  if (!ts) return "-";
  return new Date(ts * 1000).toISOString().replace("T", " ").replace(/\.000Z$/, " UTC");
}

async function fetchJson(url) {
  const response = await fetch(url, {
    method: "GET",
    headers: { Accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return response.json();
}

// ---- Clickable navigation helpers ----

function doSearch(type, value) {
  el.queryType.value = type;
  el.queryValue.value = value;
  el.searchForm.dispatchEvent(new Event("submit", { cancelable: true }));
  el.queryOutput.scrollIntoView({ behavior: "smooth", block: "start" });
}

function hashLink(hash, type) {
  const a = document.createElement("a");
  a.className = "hash-link";
  a.textContent = shortHash(hash, 10);
  a.title = hash;
  a.href = "#";
  a.addEventListener("click", (e) => {
    e.preventDefault();
    doSearch(type, hash);
  });
  return a;
}

function addressLink(addr) {
  const a = document.createElement("a");
  a.className = "hash-link";
  a.textContent = shortHash(addr, 10);
  a.title = addr;
  a.href = "#";
  a.addEventListener("click", (e) => {
    e.preventDefault();
    doSearch("account", addr);
  });
  return a;
}

// ---- Structured result rendering ----

function renderResultCard(type, rows, extra) {
  const card = document.createElement("div");
  card.className = "result-card";
  const label = document.createElement("p");
  label.className = "result-type";
  label.textContent = type;
  card.appendChild(label);

  const grid = document.createElement("div");
  grid.className = "result-row";
  rows.forEach(([k, v, isLink]) => {
    const lbl = document.createElement("span");
    lbl.className = "result-label";
    lbl.textContent = k;
    grid.appendChild(lbl);

    const val = document.createElement("span");
    if (isLink && v instanceof HTMLElement) {
      val.className = "result-value";
      val.appendChild(v);
    } else {
      val.className = typeof v === "number" ? "result-value number" : "result-value";
      val.textContent = String(v);
    }
    grid.appendChild(val);
  });
  card.appendChild(grid);
  if (extra) card.appendChild(extra);
  return card;
}

function renderBlock(data) {
  const height = data.height ?? data.index ?? "-";
  const hash = data.hash || "-";
  const txs = Array.isArray(data.transactions) ? data.transactions : [];
  const rows = [
    ["Height", height],
    ["Hash", hashLink(hash, "block-hash"), true],
    ["Timestamp", formatTime(data.timestamp)],
    ["Signer", data.signer ? addressLink(data.signer) : "-", !!data.signer],
    ["Tx Count", txs.length],
    ["Prev Hash", data.previous_hash ? hashLink(data.previous_hash, "block-hash") : "-", !!data.previous_hash],
  ];

  let txSection = null;
  if (txs.length > 0) {
    txSection = document.createElement("div");
    txSection.className = "result-tx-list";
    const h4 = document.createElement("h4");
    h4.textContent = "Transactions";
    txSection.appendChild(h4);
    txs.forEach((tx) => {
      const txHash = typeof tx === "string" ? tx : (tx.hash_hex || tx.hash || "-");
      const link = hashLink(String(txHash), "tx-hash");
      const row = document.createElement("div");
      row.style.marginBottom = "0.2rem";
      row.appendChild(link);
      txSection.appendChild(row);
    });
  }
  return renderResultCard("Block", rows, txSection);
}

function renderTransaction(data) {
  const hash = data.hash_hex || (Array.isArray(data.hash) ? data.hash.map((b) => b.toString(16).padStart(2, "0")).join("") : data.hash || "-");
  const sender = data.sender_hex || (Array.isArray(data.sender) ? data.sender.map((b) => b.toString(16).padStart(2, "0")).join("") : data.sender || "-");

  let recipientStr = "-";
  const r = data.recipient;
  if (r) {
    if (r.Wallet) {
      recipientStr = Array.isArray(r.Wallet) ? r.Wallet.map((b) => b.toString(16).padStart(2, "0")).join("") : String(r.Wallet);
    } else if (r.Contract) {
      const cid = r.Contract.id || r.Contract;
      recipientStr = Array.isArray(cid) ? cid.map((b) => b.toString(16).padStart(2, "0")).join("") : String(cid);
    }
  }

  let payloadStr = "-";
  const p = data.payload;
  if (p) {
    if (p.Transfer) payloadStr = `Transfer ${formatAmount(p.Transfer.amount)}`;
    else if (p.ContractDeploy) payloadStr = "Contract Deploy";
    else if (p.ContractCall) payloadStr = `Call ${p.ContractCall.method || "?"}`;
    else if (p.Data) payloadStr = `Data (${(p.Data.data || []).length} bytes)`;
    else payloadStr = JSON.stringify(p);
  }

  const rows = [
    ["Hash", hash],
    ["Sender", addressLink(sender), true],
    ["Recipient", addressLink(recipientStr), true],
    ["Payload", payloadStr],
    ["Nonce", data.nonce ?? "-"],
    ["Timestamp", formatTime(data.timestamp)],
    ["Gas", `${formatAmount(data.gas_limit || 0)} limit / ${data.gas_price || 0} price`],
    ["Chain ID", data.chain_id ?? 1],
  ];
  return renderResultCard("Transaction", rows);
}

function renderAccount(data) {
  const pk = data.public_key || data.pubkey || "-";
  const rows = [
    ["Public Key", pk],
    ["Balance", formatAmount(data.balance ?? 0)],
    ["Nonce", data.nonce ?? 0],
  ];
  return renderResultCard("Account", rows);
}

function renderSearchResult(type, data) {
  el.queryOutput.innerHTML = "";

  if (type === "tx-address" && Array.isArray(data)) {
    if (data.length === 0) {
      el.queryOutput.innerHTML = '<p class="muted-hint">No transactions found for this address.</p>';
      return;
    }
    data.forEach((tx) => el.queryOutput.appendChild(renderTransaction(tx)));
    return;
  }

  if (type === "block-height" || type === "block-hash") {
    el.queryOutput.appendChild(renderBlock(data));
    return;
  }
  if (type === "tx-hash") {
    el.queryOutput.appendChild(renderTransaction(data));
    return;
  }
  if (type === "account") {
    el.queryOutput.appendChild(renderAccount(data));
    return;
  }

  const pre = document.createElement("pre");
  pre.className = "result-fallback";
  pre.textContent = JSON.stringify(data, null, 2);
  el.queryOutput.appendChild(pre);
}

// ---- Event log ----

function addLogLine(text, level = "info") {
  const row = document.createElement("div");
  row.className = `log-line ${level === "info" ? "" : level}`.trim();
  row.textContent = `[${new Date().toLocaleTimeString()}] ${text}`;
  el.eventLog.prepend(row);
  while (el.eventLog.children.length > 120) {
    el.eventLog.removeChild(el.eventLog.lastChild);
  }
}

// ---- Config ----

function applyConfigInputs() {
  const config = getConfig();
  el.apiBase.value = config.apiBase;
  el.wsBase.value = config.wsBase;
}

// ---- Blocks table ----

async function loadLatestBlocks() {
  const apiBase = normalizeApiBase(el.apiBase.value);
  const latest = await fetchJson(`${apiBase}/api/v1/blocks/latest`);
  const latestHeight = Number(latest.height ?? latest.index ?? 0);
  const start = Math.max(0, latestHeight - 9);
  const heights = [];
  for (let h = latestHeight; h >= start; h -= 1) {
    heights.push(h);
  }

  const rows = await Promise.all(
    heights.map(async (height) => {
      try {
        return await fetchJson(`${apiBase}/api/v1/blocks/${height}`);
      } catch {
        return null;
      }
    }),
  );

  el.blocksTableBody.innerHTML = "";
  rows.filter(Boolean).forEach((block) => {
    const tr = document.createElement("tr");
    const hash = String(block.hash || "-");
    const txCount = Array.isArray(block.transactions)
      ? block.transactions.length
      : Number(block.tx_count ?? 0);

    const tdHeight = document.createElement("td");
    const heightLink = document.createElement("a");
    heightLink.className = "hash-link";
    heightLink.textContent = String(block.height ?? block.index ?? "-");
    heightLink.href = "#";
    heightLink.addEventListener("click", (e) => {
      e.preventDefault();
      doSearch("block-height", String(block.height ?? block.index));
    });
    tdHeight.appendChild(heightLink);

    const tdHash = document.createElement("td");
    tdHash.appendChild(hashLink(hash, "block-hash"));

    const tdTime = document.createElement("td");
    tdTime.textContent = formatTime(block.timestamp);

    const tdTxCount = document.createElement("td");
    tdTxCount.textContent = String(txCount);

    tr.appendChild(tdHeight);
    tr.appendChild(tdHash);
    tr.appendChild(tdTime);
    tr.appendChild(tdTxCount);
    el.blocksTableBody.appendChild(tr);
  });
}

// ---- Network panel ----

async function refreshNetwork() {
  const apiBase = normalizeApiBase(el.apiBase.value);
  try {
    const [supply, peers, signers] = await Promise.all([
      fetchJson(`${apiBase}/api/v1/supply`).catch(() => null),
      fetchJson(`${apiBase}/api/v1/peers`).catch(() => null),
      fetchJson(`${apiBase}/api/v1/admin/signers`).catch(() => null),
    ]);

    if (supply) {
      el.supplyValue.textContent = formatAmount(supply.total_supply ?? 0);
      el.accountsValue.textContent = supply.account_count != null ? formatAmount(supply.account_count) : "-";
    }

    if (peers) {
      const peerArr = Array.isArray(peers) ? peers : [];
      el.peersValue.textContent = String(peerArr.length);
      el.peerList.textContent = peerArr.length ? peerArr.map((p) => (typeof p === "string" ? p : JSON.stringify(p))).join("\n") : "No peers connected";
    }

    if (signers) {
      const sigArr = Array.isArray(signers) ? signers : [];
      el.signersValue.textContent = String(sigArr.length);
      el.signerList.textContent = sigArr.length ? sigArr.map((s) => (typeof s === "string" ? s : JSON.stringify(s))).join("\n") : "No signers";
    }
  } catch {
    // network panel is best-effort
  }
}

// ---- Snapshot ----

async function refreshSnapshot() {
  const apiBase = normalizeApiBase(el.apiBase.value);
  try {
    const [health, latest] = await Promise.all([
      fetchJson(`${apiBase}/api/v1/health`),
      fetchJson(`${apiBase}/api/v1/blocks/latest`),
    ]);

    el.healthValue.textContent = health.status || "ok";
    el.heightValue.textContent = String(latest.height ?? latest.index ?? "-");
    el.hashValue.textContent = latest.hash || "-";
    const txCount = Array.isArray(latest.transactions)
      ? latest.transactions.length
      : Number(latest.tx_count ?? 0);
    el.txCountValue.textContent = String(txCount);
    setStatus("Connected", "status-ok");
    await Promise.all([loadLatestBlocks(), refreshNetwork()]);
  } catch (error) {
    setStatus("Unreachable", "status-error");
    el.healthValue.textContent = "error";
    el.heightValue.textContent = "-";
    el.hashValue.textContent = "-";
    el.txCountValue.textContent = "-";
    el.blocksTableBody.innerHTML = "";
    addLogLine(`Snapshot failed: ${error.message}`, "error");
  }
}

// ---- Search ----

async function runSearch(event) {
  event.preventDefault();
  const apiBase = normalizeApiBase(el.apiBase.value);
  const type = el.queryType.value;
  const value = el.queryValue.value.trim();
  if (!value) return;

  const endpoints = {
    "block-height": `${apiBase}/api/v1/blocks/${encodeURIComponent(value)}`,
    "block-hash": `${apiBase}/api/v1/blocks/hash/${encodeURIComponent(value)}`,
    "tx-hash": `${apiBase}/api/v1/transactions/${encodeURIComponent(value)}`,
    "tx-address": `${apiBase}/api/v1/transactions/address/${encodeURIComponent(value)}?limit=25`,
    account: `${apiBase}/api/v1/accounts/${encodeURIComponent(value)}`,
  };

  try {
    const data = await fetchJson(endpoints[type]);
    renderSearchResult(type, data);
  } catch (error) {
    el.queryOutput.innerHTML = "";
    const pre = document.createElement("pre");
    pre.className = "result-fallback";
    pre.textContent = JSON.stringify(
      { error: error.message, hint: "Verify endpoint config and query value format." },
      null,
      2,
    );
    el.queryOutput.appendChild(pre);
  }
}

// ---- WebSocket ----

function connectWs() {
  if (ws && ws.readyState <= 1) return;
  const wsBase = normalizeWsBase(el.wsBase.value);

  ws = new WebSocket(wsBase);
  ws.onopen = () => {
    addLogLine(`WS connected to ${wsBase}`);
    setStatus("Connected", "status-ok");
    ["blocks", "transactions", "mempool"].forEach((channel) => {
      ws.send(JSON.stringify({ type: "subscribe", channel }));
    });
  };

  ws.onmessage = (event) => {
    let payload = event.data;
    try {
      payload = JSON.parse(event.data);
    } catch {
      addLogLine(`WS raw: ${event.data}`, "warn");
      return;
    }
    const channel = payload.channel || payload.type || "unknown";
    addLogLine(`${channel}: ${JSON.stringify(payload.event ?? payload)}`);
  };

  ws.onerror = () => {
    addLogLine("WS connection error", "error");
    setStatus("WS Error", "status-error");
  };

  ws.onclose = () => {
    addLogLine("WS disconnected", "warn");
  };
}

function disconnectWs() {
  if (!ws) return;
  ws.close();
  ws = null;
}

// ---- Init ----

function saveConfigFromInputs() {
  const config = {
    apiBase: normalizeApiBase(el.apiBase.value),
    wsBase: normalizeWsBase(el.wsBase.value),
  };
  setConfig(config);
  addLogLine("Endpoint config saved");
}

function wireUi() {
  el.saveConfigBtn.addEventListener("click", () => {
    saveConfigFromInputs();
    refreshSnapshot();
  });
  el.refreshBtn.addEventListener("click", () => refreshSnapshot());
  el.clearLogBtn.addEventListener("click", () => {
    el.eventLog.innerHTML = "";
  });
  el.searchForm.addEventListener("submit", runSearch);
  el.wsConnectBtn.addEventListener("click", connectWs);
  el.wsDisconnectBtn.addEventListener("click", disconnectWs);
}

function init() {
  applyConfigInputs();
  wireUi();
  refreshSnapshot();
}

init();
