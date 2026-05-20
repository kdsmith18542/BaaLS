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

function renderJson(target, data) {
  target.textContent = JSON.stringify(data, null, 2);
}

function shortHash(hash, width = 12) {
  if (!hash || typeof hash !== "string") return "-";
  if (hash.length <= width * 2) return hash;
  return `${hash.slice(0, width)}...${hash.slice(-width)}`;
}

async function fetchJson(url) {
  const response = await fetch(url, {
    method: "GET",
    headers: {
      Accept: "application/json",
    },
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return response.json();
}

function addLogLine(text, level = "info") {
  const row = document.createElement("div");
  row.className = `log-line ${level === "info" ? "" : level}`.trim();
  row.textContent = `[${new Date().toLocaleTimeString()}] ${text}`;
  el.eventLog.prepend(row);

  while (el.eventLog.children.length > 120) {
    el.eventLog.removeChild(el.eventLog.lastChild);
  }
}

function applyConfigInputs() {
  const config = getConfig();
  el.apiBase.value = config.apiBase;
  el.wsBase.value = config.wsBase;
}

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
  rows
    .filter(Boolean)
    .forEach((block) => {
      const tr = document.createElement("tr");
      const timestamp = block.timestamp ? new Date(block.timestamp * 1000).toISOString() : "-";
      const txCount = Array.isArray(block.transactions)
        ? block.transactions.length
        : Number(block.tx_count ?? 0);

      const tdHeight = document.createElement("td");
      tdHeight.textContent = String(block.index ?? "-");

      const tdHash = document.createElement("td");
      const hash = String(block.hash || "-");
      tdHash.title = hash;
      tdHash.textContent = shortHash(hash, 10);

      const tdTime = document.createElement("td");
      tdTime.textContent = timestamp;

      const tdTxCount = document.createElement("td");
      tdTxCount.textContent = String(txCount);

      tr.appendChild(tdHeight);
      tr.appendChild(tdHash);
      tr.appendChild(tdTime);
      tr.appendChild(tdTxCount);
      el.blocksTableBody.appendChild(tr);
    });
}

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
    el.txCountValue.textContent = String(latest.tx_count ?? 0);
    setStatus("Connected", "status-ok");
    await loadLatestBlocks();
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

  const endpoint = endpoints[type];
  try {
    const data = await fetchJson(endpoint);
    renderJson(el.queryOutput, data);
  } catch (error) {
    renderJson(el.queryOutput, {
      error: error.message,
      endpoint,
      hint: "Verify endpoint config and query value format.",
    });
  }
}

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
