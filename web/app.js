/* ── Config ── */
const isLocal = ["localhost", "127.0.0.1"].includes(window.location.hostname);
const defaults = {
  api: isLocal ? "http://127.0.0.1:18080" : window.location.origin,
  ws: isLocal ? "ws://127.0.0.1:18081" : `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`,
  chrono: "https://chrono.baals.network",
  resurge: "https://resurge.baals.network",
  canvas: "https://canvas.baals.network",
};
const STORAGE_KEY = "baals-cfg-v2";
const ORACLE_CHAIN_CANDIDATES = ["bitcoin", "bitcoin-light", "dogecoin", "litecoin", "ethereum"];

function loadCfg() {
  try {
    const c = JSON.parse(localStorage.getItem(STORAGE_KEY));
    return {
      api: c.api || defaults.api,
      ws: c.ws || defaults.ws,
      chrono: c.chrono || defaults.chrono,
      resurge: c.resurge || defaults.resurge,
      canvas: c.canvas || defaults.canvas,
    };
  }
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
  unifiedForm: $("unifiedForm"), unifiedType: $("unifiedType"), unifiedInput: $("unifiedInput"), unifiedResult: $("unifiedResult"),
  blocksBody: $("blocksBody"),
  oracleChain: $("oracleChain"), oracleReload: $("oracleReload"), oracleBody: $("oracleBody"),
  mempoolList: $("mempoolList"), mempoolReload: $("mempoolReload"),
  netValidatorCount: $("netValidatorCount"), netValidatorList: $("netValidatorList"),
  netCurrentProposer: $("netCurrentProposer"), netQuorumThreshold: $("netQuorumThreshold"), netRoundRobin: $("netRoundRobin"),
  netPeerCount: $("netPeerCount"), netPeerList: $("netPeerList"),
  ciStorage: $("ciStorage"), ciMemory: $("ciMemory"),
  ciFeePolicy: $("ciFeePolicy"), ciWsPort: $("ciWsPort"), ciBackend: $("ciBackend"), ciChainId: $("ciChainId"),
  cfgApi: $("cfgApi"), cfgWs: $("cfgWs"), cfgSave: $("cfgSave"),
  cfgChrono: $("cfgChrono"), cfgResurge: $("cfgResurge"), cfgCanvas: $("cfgCanvas"),
  wsConnect: $("wsConnect"), wsDisconnect: $("wsDisconnect"), wsClear: $("wsClear"), eventLog: $("eventLog"),
  ecoChronoUrl: $("ecoChronoUrl"), ecoResurgeUrl: $("ecoResurgeUrl"), ecoCanvasUrl: $("ecoCanvasUrl"),
  ecoBaalsStatus: $("ecoBaalsStatus"), ecoChronoStatus: $("ecoChronoStatus"),
  ecoResurgeStatus: $("ecoResurgeStatus"), ecoCanvasStatus: $("ecoCanvasStatus"),
  ecoOracleTrace: $("ecoOracleTrace"), ecoChronoLinks: $("ecoChronoLinks"),
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

/* ── Copyable Hash Utility ── */
function makeCopyable(text) {
  const span = document.createElement("span");
  span.className = "copyable";
  span.textContent = short(text, 10);
  span.title = `Click to copy: ${text}`;
  span.onclick = () => {
    navigator.clipboard.writeText(text).then(() => {
      showCopyToast("Copied to clipboard");
    }).catch(() => {
      // fallback
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      showCopyToast("Copied to clipboard");
    });
  };
  return span;
}

function showCopyToast(msg) {
  const existing = document.querySelector(".copy-toast");
  if (existing) existing.remove();
  const toast = document.createElement("div");
  toast.className = "copy-toast";
  toast.textContent = msg;
  document.body.appendChild(toast);
  setTimeout(() => toast.remove(), 1600);
}

function bindEcosystemLinks() {
  if (el.ecoChronoUrl) el.ecoChronoUrl.href = cfg.chrono;
  if (el.ecoResurgeUrl) el.ecoResurgeUrl.href = cfg.resurge;
  if (el.ecoCanvasUrl) el.ecoCanvasUrl.href = cfg.canvas;
  if (el.ecoChronoLinks) {
    const base = cfg.chrono.replace(/\/+$/, "");
    el.ecoChronoLinks.innerHTML = `
      <a href="${base}/health" target="_blank" rel="noopener">health</a>
      <a href="${base}/v1/chains" target="_blank" rel="noopener">chains</a>
      <a href="${base}/metrics" target="_blank" rel="noopener">metrics</a>
    `;
  }
}

function setEcoStatus(target, text, level = "neutral") {
  if (!target) return;
  target.textContent = text;
  const color = {
    ok: "var(--green)",
    warn: "var(--amber)",
    error: "var(--red)",
    neutral: "var(--text-2)",
  }[level] || "var(--text-2)";
  target.style.color = color;
}

function setUnifiedResult(html = "") {
  if (!el.unifiedResult) return;
  el.unifiedResult.innerHTML = html;
}

async function probeUrl(url) {
  try {
    const r = await fetch(url, { method: "GET" });
    return r.ok;
  } catch {
    return false;
  }
}

async function fetchBaalsOracleSummary() {
  const results = [];
  for (const chainId of ORACLE_CHAIN_CANDIDATES) {
    try {
      const data = await api(`/api/v1/oracle/attestations/${encodeURIComponent(chainId)}`);
      const attestations = Array.isArray(data?.attestations) ? data.attestations : [];
      const samples = attestations
        .slice(0, 2)
        .map((a) => ({
          chainId,
          address: a?.proof?.address || "unknown",
          evmWallet: a?.proof?.evm_wallet || "unknown",
          block: a?.attested_at_block ?? "—",
          hash: a?.baals_block_hash || "",
        }));
      results.push({ chainId, ok: true, count: Number(data?.count || 0), samples });
    } catch {
      results.push({ chainId, ok: false, count: 0, samples: [] });
    }
  }
  const live = results.filter((r) => r.ok);
  const total = live.reduce((acc, r) => acc + r.count, 0);
  const recent = live.flatMap((r) => r.samples).slice(0, 6);
  return {
    chainsQueried: results.length,
    chainsLive: live.length,
    totalAttestations: total,
    recent,
  };
}

function renderOracleTrace(items) {
  if (!el.ecoOracleTrace) return;
  if (!items || items.length === 0) {
    el.ecoOracleTrace.textContent = "No sample attestations available.";
    return;
  }
  el.ecoOracleTrace.innerHTML = "";
  items.forEach((i) => {
    const row = document.createElement("div");
    row.textContent = `${i.chainId}:${short(i.address, 8)} -> ${short(i.evmWallet, 8)} @ ${i.block} (${short(i.hash, 8)})`;
    el.ecoOracleTrace.appendChild(row);
  });
}

async function refreshEcosystemStatus() {
  // ChronoNode health
  try {
    const chronoHealth = await fetch(`${cfg.chrono}/health`, { headers: { Accept: "application/json" } })
      .then((r) => r.json());
    const status = chronoHealth?.status || "ok";
    const uptime = chronoHealth?.uptime_seconds;
    const extra = uptime != null ? `, uptime ${fmtUptime(Number(uptime))}` : "";
    setEcoStatus(el.ecoChronoStatus, `Health: ${status}${extra}`, status === "ok" ? "ok" : "warn");
  } catch {
    setEcoStatus(el.ecoChronoStatus, "Health: unavailable (endpoint/CORS/offline)", "warn");
  }

  // BaaLS oracle attestation summary
  try {
    const summary = await fetchBaalsOracleSummary();
    if (summary.chainsLive === 0) {
      setEcoStatus(el.ecoBaalsStatus, "Oracle attestations: no readable chains yet", "warn");
    } else {
      setEcoStatus(
        el.ecoBaalsStatus,
        `Oracle attestations: ${fmtNum(summary.totalAttestations)} across ${summary.chainsLive}/${summary.chainsQueried} chains`,
        "ok"
      );
    }
    renderOracleTrace(summary.recent);
  } catch {
    setEcoStatus(el.ecoBaalsStatus, "Oracle attestations: unavailable", "warn");
    renderOracleTrace([]);
  }

  // Resurge + Canvas reachability (basic availability signal)
  const [resurgeUp, canvasUp] = await Promise.all([probeUrl(cfg.resurge), probeUrl(cfg.canvas)]);
  setEcoStatus(el.ecoResurgeStatus, `Reachability: ${resurgeUp ? "online" : "unknown/offline"}`, resurgeUp ? "ok" : "warn");
  setEcoStatus(el.ecoCanvasStatus, `Reachability: ${canvasUp ? "online" : "unknown/offline"}`, canvasUp ? "ok" : "warn");
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

    // Extended health fields (Task 5)
    if (el.ciFeePolicy) el.ciFeePolicy.textContent = health.fee_policy || health.fee_mode || "—";
    if (el.ciWsPort) el.ciWsPort.textContent = health.ws_port ?? "—";
    if (el.ciBackend) el.ciBackend.textContent = health.storage_backend || "—";
    if (el.ciChainId) el.ciChainId.textContent = health.chain_id ?? "—";

    // Validators (Task 4 — enriched)
    el.netValidatorCount.textContent = String(sigTotal);
    el.netValidatorList.innerHTML = "";
    if (signers) {
      if (signers.primary) {
        const d = document.createElement("div");
        d.className = "net-list-item primary";
        d.appendChild(makeCopyable(signers.primary));
        el.netValidatorList.appendChild(d);
      }
      (signers.additional || []).forEach(pk => {
        const d = document.createElement("div");
        d.className = "net-list-item";
        d.appendChild(makeCopyable(pk));
        el.netValidatorList.appendChild(d);
      });
    }

    // Validator meta from latest block (Task 4)
    if (latestBlock) {
      const proposer = latestBlock.signer || latestBlock.metadata?.signer || "";
      if (el.netCurrentProposer) {
        el.netCurrentProposer.textContent = proposer ? short(proposer, 8) : "—";
        el.netCurrentProposer.title = proposer;
      }
    }
    if (health.quorum_threshold != null && el.netQuorumThreshold) {
      el.netQuorumThreshold.textContent = String(health.quorum_threshold);
    }
    if (health.round_robin != null && el.netRoundRobin) {
      el.netRoundRobin.textContent = health.round_robin ? "Enabled" : "Disabled";
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
    await loadOracleAttestations();
    await loadMempool();
    await refreshEcosystemStatus();
  } catch (e) {
    setStatus(false);
    console.error("refresh failed", e);
    await refreshEcosystemStatus();
  }
}

async function loadBlocks(latestHeight) {
  const start = Math.max(0, latestHeight - 9);
  const heights = [];
  for (let h = latestHeight; h >= start; h--) heights.push(h);

  const blocks = [];
  for (const h of heights) {
    const b = await api(`/api/v1/blocks/${h}`).catch(() => null);
    blocks.push(b);
  }
  el.blocksBody.innerHTML = "";

  blocks.filter(Boolean).forEach(b => {
    const tr = document.createElement("tr");
    const hash = b.hash || "—";
    const txCount = Array.isArray(b.transactions) ? b.transactions.length : (b.tx_count ?? 0);
    const signer = b.signer || b.metadata?.signer || "";

    const tdH = document.createElement("td");
    tdH.appendChild(makeLink(String(b.height ?? b.index ?? "—"), "block-height", String(b.height ?? b.index)));

    const tdHash = document.createElement("td");
    tdHash.appendChild(makeLink(short(hash, 10), "block-hash", hash));

    const tdProducer = document.createElement("td");
    if (signer) {
      tdProducer.appendChild(makeLink(short(signer, 6), "account", signer));
    } else {
      tdProducer.textContent = "—";
    }
    tdProducer.style.fontFamily = "var(--mono)";
    tdProducer.style.fontSize = "0.78rem";

    const tdTime = document.createElement("td");
    tdTime.textContent = fmtTime(b.timestamp);

    const tdTx = document.createElement("td");
    tdTx.textContent = String(txCount);

    tr.append(tdH, tdHash, tdProducer, tdTime, tdTx);
    el.blocksBody.appendChild(tr);
  });
}

async function loadOracleAttestations() {
  if (!el.oracleBody || !el.oracleChain) return;
  const chainId = el.oracleChain.value;
  el.oracleBody.innerHTML = "";
  const chronoBase = cfg.chrono.replace(/\/+$/, "");
  const resurgeBase = cfg.resurge.replace(/\/+$/, "");

  try {
    const data = await api(`/api/v1/oracle/attestations/${encodeURIComponent(chainId)}`);
    const list = Array.isArray(data?.attestations) ? data.attestations : [];
    if (list.length === 0) {
      const tr = document.createElement("tr");
      tr.innerHTML = `<td colspan="6">No attestations for ${chainId}</td>`;
      el.oracleBody.appendChild(tr);
      return;
    }

    list.slice(0, 50).forEach((a) => {
      const tr = document.createElement("tr");
      const addr = a?.proof?.address || "—";
      const evm = a?.proof?.evm_wallet || "—";
      const block = a?.attested_at_block ?? "—";
      const hash = a?.baals_block_hash || "—";
      const proofHash = a?.proof?.proof_hash || a?.proof_hash || "";
      const evmTx = a?.evm_tx_hash || a?.resurgence_tx_hash || "";

      const tdAddr = document.createElement("td");
      tdAddr.appendChild(makeLink(short(addr, 10), "account", addr));

      const tdEvm = document.createElement("td");
      tdEvm.textContent = evm;

      const tdBlock = document.createElement("td");
      tdBlock.textContent = String(block);

      const tdHash = document.createElement("td");
      tdHash.textContent = short(hash, 10);
      tdHash.title = hash;

      // Cross-link: ChronoNode dormancy proof
      const tdProof = document.createElement("td");
      if (addr && addr !== "—") {
        const proofLink = document.createElement("a");
        proofLink.className = "cross-link";
        proofLink.href = `${chronoBase}/v1/chains/${encodeURIComponent(chainId)}/addresses/${encodeURIComponent(addr)}/dormancy/proof`;
        proofLink.target = "_blank";
        proofLink.rel = "noopener";
        proofLink.textContent = "Proof";
        tdProof.appendChild(proofLink);
      } else {
        tdProof.textContent = "—";
      }

      // Cross-link: Resurgence EVM mint tx
      const tdMint = document.createElement("td");
      if (evmTx) {
        const mintLink = document.createElement("a");
        mintLink.className = "cross-link";
        mintLink.href = `${resurgeBase}/oracle/proofs/${encodeURIComponent(evmTx)}`;
        mintLink.target = "_blank";
        mintLink.rel = "noopener";
        mintLink.textContent = short(evmTx, 6);
        tdMint.appendChild(mintLink);
      } else if (proofHash) {
        const mintLink = document.createElement("a");
        mintLink.className = "cross-link";
        mintLink.href = `${resurgeBase}/oracle/proofs/${encodeURIComponent(proofHash)}`;
        mintLink.target = "_blank";
        mintLink.rel = "noopener";
        mintLink.textContent = "Pending";
        tdMint.appendChild(mintLink);
      } else {
        tdMint.textContent = "—";
      }

      tr.append(tdAddr, tdEvm, tdBlock, tdHash, tdProof, tdMint);
      el.oracleBody.appendChild(tr);
    });
  } catch (err) {
    const tr = document.createElement("tr");
    tr.innerHTML = `<td colspan="6">Unable to load attestations: ${err.message}</td>`;
    el.oracleBody.appendChild(tr);
  }
}

/* ── Search ── */
function detectType(val) {
  if (/^\d+$/.test(val)) return "block-height";
  if (val.length === 64 && /^[0-9a-fA-F]+$/.test(val)) return "tx-hash"; // could be block or tx hash
  if (/^contract_/.test(val) || /^[0-9a-fA-F]{8}-/.test(val)) return "contract-id";
  return "account";
}

function detectUnifiedType(value) {
  const raw = value.trim();
  if (!raw) return "auto";
  if (/^\d+$/.test(raw)) return "block-height";
  if (/^0x[0-9a-fA-F]{64}$/.test(raw)) return "evm-tx";
  if (/^[0-9a-fA-F]{64}$/.test(raw)) return "baals-tx";
  if (/^sha256:[0-9a-fA-F]{64}$/.test(raw)) return "proof-hash";
  if (/^wasm:[0-9a-fA-F]{64}$/.test(raw) || /^wasm_[0-9a-fA-F]+$/.test(raw)) return "wasm-hash";
  if (/^manifest:[0-9a-fA-F]{64}$/.test(raw) || /^manifest_[0-9a-fA-F]+$/.test(raw) || /^manifest:[a-zA-Z0-9_-]+$/.test(raw)) return "manifest-hash";
  if (/^0x[0-9a-fA-F]{40}$/.test(raw)) return "address";
  if (/^contract_/.test(raw) || /^[0-9a-fA-F]{8}-/.test(raw)) return "contract-id";
  return "address";
}

async function runUnifiedSearch(e) {
  e.preventDefault();
  const raw = (el.unifiedInput?.value || "").trim();
  if (!raw) {
    setUnifiedResult("");
    return;
  }
  let type = el.unifiedType?.value || "auto";
  if (type === "auto") type = detectUnifiedType(raw);

  const chronoBase = cfg.chrono.replace(/\/+$/, "");
  const resurgeBase = cfg.resurge.replace(/\/+$/, "");
  const canvasBase = cfg.canvas.replace(/\/+$/, "");

  if (type === "block-height") {
    doSearch("block-height", raw);
    setUnifiedResult(
      `Opened BaaLS block search for height <strong>${raw}</strong>. ` +
      `View in <a target="_blank" rel="noopener" href="${chronoBase}/proofs/chains/bitcoin-light/blocks/${encodeURIComponent(raw)}">ChronoNode Block Archive</a>`
    );
    return;
  }

  if (type === "baals-tx") {
    doSearch("tx-hash", raw);
    setUnifiedResult(`Opened BaaLS tx search for hash <strong>${short(raw, 12)}</strong>.`);
    return;
  }

  if (type === "address") {
    if (/^0x[0-9a-fA-F]{40}$/.test(raw)) {
      const enc = encodeURIComponent(raw);
      setUnifiedResult(
        `EVM address detected. ` +
        `<a target="_blank" rel="noopener" href="${resurgeBase}/users/${enc}">Resurgence User</a> · ` +
        `<a target="_blank" rel="noopener" href="${resurgeBase}/pools/${enc}">Resurgence Pool</a> · ` +
        `<a target="_blank" rel="noopener" href="${chronoBase}/v1/chains/bitcoin-light/addresses/${enc}/dormancy">ChronoNode Dormancy</a>`
      );
    } else {
      doSearch("account", raw);
      setUnifiedResult(`Opened BaaLS account search for <strong>${short(raw, 10)}</strong>.`);
    }
    return;
  }

  if (type === "evm-tx") {
    const enc = encodeURIComponent(raw);
    setUnifiedResult(
      `EVM tx hash detected. ` +
      `<a target="_blank" rel="noopener" href="${resurgeBase}/claims?tx=${enc}">Resurgence Claims</a> · ` +
      `<a target="_blank" rel="noopener" href="https://arbiscan.io/tx/${enc}">Arbiscan</a> · ` +
      `<a target="_blank" rel="noopener" href="https://amoy.polygonscan.com/tx/${enc}">Polygonscan (Amoy)</a> · ` +
      `<a target="_blank" rel="noopener" href="https://sepolia.basescan.org/tx/${enc}">Basescan (Sepolia)</a>`
    );
    return;
  }

  if (type === "contract-id") {
    doSearch("contract-id", raw);
    setUnifiedResult(
      `Contract ID detected. ` +
      `<a target="_blank" rel="noopener" href="${canvasBase}/canvas/deployments/${encodeURIComponent(raw)}">Canvas Deployment</a> · ` +
      `<a target="_blank" rel="noopener" href="${chronoBase}/v1/artifacts/${encodeURIComponent(raw)}">ChronoNode Archive</a>`
    );
    return;
  }

  if (type === "wasm-hash") {
    const hashPart = raw.replace(/^wasm:/, "");
    setUnifiedResult(
      `WASM artifact hash detected. ` +
      `<a target="_blank" rel="noopener" href="${canvasBase}/canvas/artifacts/${encodeURIComponent(hashPart)}">Canvas Artifact</a> · ` +
      `<a target="_blank" rel="noopener" href="${chronoBase}/v1/artifacts/${encodeURIComponent(hashPart)}">ChronoNode Archive</a>`
    );
    return;
  }

  if (type === "manifest-hash") {
    const hashPart = raw.replace(/^manifest:/, "");
    setUnifiedResult(
      `Manifest hash detected. ` +
      `<a target="_blank" rel="noopener" href="${canvasBase}/canvas/manifests/${encodeURIComponent(hashPart)}">Canvas Manifest</a> · ` +
      `<a target="_blank" rel="noopener" href="${chronoBase}/v1/artifacts/${encodeURIComponent(hashPart)}">ChronoNode Archive</a>`
    );
    return;
  }

  if (type === "proof-hash") {
    const hashPart = raw.replace(/^sha256:/, "");
    setUnifiedResult(
      `Proof hash detected. ` +
      `<a target="_blank" rel="noopener" href="${chronoBase}/v1/proofs/verify?hash=${encodeURIComponent(hashPart)}">ChronoNode Verify</a> · ` +
      `<a target="_blank" rel="noopener" href="${resurgeBase}/oracle/proofs/${encodeURIComponent(hashPart)}">Resurgence Oracle</a>`
    );
    return;
  }

  if (type === "ccip-id") {
    setUnifiedResult(
      `CCIP message ID detected. ` +
      `<a target="_blank" rel="noopener" href="${resurgeBase}/ccip/${encodeURIComponent(raw)}">Resurgence CCIP</a> · ` +
      `<a target="_blank" rel="noopener" href="https://ccip.chain.link/msg/${encodeURIComponent(raw)}" >CCIP Explorer</a>`
    );
    return;
  }

  setUnifiedResult("No routing rule matched this identifier yet.");
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
    "contract-id": `/api/v1/contracts/${encodeURIComponent(raw)}/state`,
  };

  el.searchResults.innerHTML = "";

  try {
    const data = await api(endpoints[type]);

    // For tx-hash, attempt receipt fetch in parallel for fee data
    if (type === "tx-hash") {
      const receipt = await api(`/api/v1/transactions/${encodeURIComponent(raw)}/receipt`).catch(() => null);
      renderResult(type, data, { receipt });
    } else if (type === "contract-id") {
      const abi = await api(`/api/v1/contracts/${encodeURIComponent(raw)}/abi`).catch(() => null);
      renderResult(type, data, { abi, contractId: raw });
    } else {
      renderResult(type, data);
    }
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

function renderResult(type, data, extra = {}) {
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
    el.searchResults.appendChild(buildTxCard(data, extra.receipt));
    return;
  }
  if (type === "account") {
    el.searchResults.appendChild(buildAccountCard(data));
    return;
  }
  if (type === "contract-id") {
    el.searchResults.appendChild(buildContractCard(data, extra.abi, extra.contractId));
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
  const signer = b.signer || b.metadata?.signer || "";
  const qSigs = b.quorum_signatures || [];
  const gasUsed = b.total_gas_used ?? b.gas_used ?? null;
  const stateRoot = b.state_root || b.metadata?.state_root || "";

  const rows = [
    ["Height", b.height ?? b.index ?? "—"],
    ["Hash", makeCopyable(hash)],
    ["Parent", b.parentHash || b.previous_hash ? makeLink(short(b.parentHash || b.previous_hash, 10), "block-hash", b.parentHash || b.previous_hash) : "—"],
    ["Time", fmtTime(b.timestamp)],
    ["Producer", signer ? makeLink(short(signer, 10), "account", signer) : "—"],
    ["Quorum Sigs", qSigs.length > 0 ? `${qSigs.length} signature${qSigs.length > 1 ? "s" : ""}` : "—"],
    ["Gas Used", gasUsed != null ? fmtNum(gasUsed) : "—"],
    ["Transactions", txs.length],
  ];

  if (stateRoot) {
    const stateSpan = document.createElement("span");
    stateSpan.textContent = short(stateRoot, 12);
    stateSpan.title = stateRoot;
    stateSpan.className = "result-value";
    stateSpan.style.fontFamily = "var(--mono)";
    rows.push(["State Root", stateSpan]);
  }

  // Finality badge
  const finalityStatus = b.finality_status || b.finality || (b.height != null ? "finalized" : "unknown");
  const badge = document.createElement("span");
  badge.className = `finality-badge ${finalityStatus === "finalized" ? "finalized" : finalityStatus === "pending" ? "pending" : "unknown"}`;
  badge.textContent = finalityStatus;
  rows.push(["Finality", badge]);

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

function buildTxCard(tx, receipt) {
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
    ["Hash", makeCopyable(hash)],
    ["From", makeLink(short(sender, 10), "account", sender)],
    ["To", makeLink(short(recipientStr, 10), "account", recipientStr)],
    ["Payload", payloadStr],
    ["Nonce", tx.nonce ?? "—"],
    ["Time", fmtTime(tx.timestamp)],
    ["Gas", `${fmtNum(tx.gas_limit || tx.gas || 0)} limit / ${tx.gas_price ?? 0} price`],
  ];

  // Fee breakdown from receipt (Task 4)
  if (receipt) {
    const statusBadge = document.createElement("span");
    statusBadge.className = `finality-badge ${receipt.status === "success" ? "finalized" : receipt.status === "failure" ? "pending" : "unknown"}`;
    statusBadge.textContent = receipt.status || "unknown";
    rows.push(["Status", statusBadge]);
    rows.push(["Gas Used", fmtNum(receipt.gas_used)]);

    if (receipt.contract_address) {
      rows.push(["Contract", makeLink(short(receipt.contract_address, 10), "contract-id", receipt.contract_address)]);
    }

    // Fee split visualization
    if (receipt.fee_operator != null || receipt.fee_treasury != null || receipt.fee_burn != null) {
      const splitEl = document.createElement("span");
      splitEl.className = "fee-split";
      splitEl.innerHTML = `
        <span class="fee-split-item"><span class="fee-split-dot operator"></span>${fmtNum(receipt.fee_operator ?? 0)} op</span>
        <span class="fee-split-item"><span class="fee-split-dot treasury"></span>${fmtNum(receipt.fee_treasury ?? 0)} treas</span>
        <span class="fee-split-item"><span class="fee-split-dot burn"></span>${fmtNum(receipt.fee_burn ?? 0)} burn</span>
      `;
      rows.push(["Fee Split", splitEl]);
    }
  } else {
    // Show fee mode from tx itself
    const feeMode = tx.fee_mode || tx.metadata?.fee_mode || "—";
    if (feeMode !== "—") rows.push(["Fee Mode", feeMode]);
  }

  return card("Transaction", rows);
}

function buildAccountCard(a) {
  const pk = a.public_key || a.pubkey || "—";
  const rows = [
    ["Public Key", makeCopyable(pk)],
    ["Balance", fmtNum(a.balance ?? 0)],
    ["Nonce", a.nonce ?? 0],
  ];

  const c = card("Account", rows);

  // Fetch recent transactions for this account (Task 3)
  const address = pk;
  if (address && address !== "—") {
    const txSection = document.createElement("div");
    txSection.className = "account-tx-list";
    txSection.innerHTML = '<h5>Recent Transactions</h5><div style="color:var(--text-3);font-size:0.8rem;">Loading…</div>';
    c.appendChild(txSection);

    api(`/api/v1/transactions/address/${encodeURIComponent(address)}?limit=10`)
      .then(txs => {
        txSection.innerHTML = '';
        const h5 = document.createElement("h5");
        const list = Array.isArray(txs) ? txs : [];
        h5.textContent = `Recent Transactions (${list.length})`;
        txSection.appendChild(h5);

        if (list.length === 0) {
          const empty = document.createElement("div");
          empty.style.cssText = "color:var(--text-3);font-size:0.82rem;";
          empty.textContent = "No transactions found for this address.";
          txSection.appendChild(empty);
          return;
        }

        list.slice(0, 10).forEach(tx => {
          const txHash = tx.hash_hex || tx.hash || bytesToHex(tx.hash) || "—";
          const time = fmtTime(tx.timestamp);
          let payloadStr = "—";
          const p = tx.payload;
          if (p) {
            if (p.Transfer) payloadStr = `Transfer ${fmtNum(p.Transfer.amount)}`;
            else if (p.ContractDeploy) payloadStr = "Deploy";
            else if (p.ContractCall) payloadStr = `Call: ${p.ContractCall.method || "?"}`;
            else if (p.Data) payloadStr = "Data";
          }

          const row = document.createElement("div");
          row.className = "account-tx-row";
          row.appendChild(makeLink(short(txHash, 8), "tx-hash", txHash));
          const payEl = document.createElement("span");
          payEl.className = "mempool-payload";
          payEl.textContent = payloadStr;
          row.appendChild(payEl);
          const timeEl = document.createElement("span");
          timeEl.className = "tx-time";
          timeEl.textContent = time;
          row.appendChild(timeEl);
          txSection.appendChild(row);
        });
      })
      .catch(() => {
        txSection.innerHTML = '<h5>Recent Transactions</h5><div style="color:var(--text-3);font-size:0.82rem;">Unable to load transaction history.</div>';
      });
  }

  return c;
}

function buildContractCard(data, abi, contractId) {
  const id = contractId || data?.contract_id || "—";
  const codeHash = data?.code_hash || data?.wasm_hash || "—";
  const deployer = data?.deployer || data?.owner || "";
  const deployBlock = data?.deployment_block ?? data?.deployed_at ?? null;
  const storageKeys = data?.storage_keys || data?.keys || [];
  const canvasManifest = data?.metadata?.canvas_manifest_hash || "";
  const chronoPointer = data?.metadata?.chrono_archive_pointer || "";
  const canvasBase = cfg.canvas.replace(/\/+$/, "");
  const chronoBase = cfg.chrono.replace(/\/+$/, "");

  const rows = [
    ["Contract ID", id],
    ["Code Hash", codeHash],
    ["Deployer", deployer ? makeLink(short(deployer, 10), "account", deployer) : "—"],
    ["Deployed At", deployBlock != null ? makeLink(String(deployBlock), "block-height", String(deployBlock)) : "—"],
    ["Storage Keys", Array.isArray(storageKeys) ? storageKeys.length : (typeof storageKeys === "number" ? storageKeys : "—")],
  ];

  // ABI link
  if (abi) {
    const abiLink = document.createElement("a");
    abiLink.className = "abi-link";
    abiLink.href = `${cfg.api}/api/v1/contracts/${encodeURIComponent(id)}/abi`;
    abiLink.target = "_blank";
    abiLink.rel = "noopener";
    abiLink.textContent = "View ABI";
    rows.push(["ABI", abiLink]);
  } else {
    rows.push(["ABI", "Not available"]);
  }

  // Canvas manifest cross-link
  if (canvasManifest) {
    const link = document.createElement("a");
    link.className = "cross-link";
    link.href = `${canvasBase}/manifests/${encodeURIComponent(canvasManifest)}`;
    link.target = "_blank";
    link.rel = "noopener";
    link.textContent = short(canvasManifest, 8);
    rows.push(["Canvas Manifest", link]);
  }

  // ChronoNode archive cross-link
  if (chronoPointer) {
    const link = document.createElement("a");
    link.className = "cross-link";
    link.href = `${chronoBase}/v1/artifacts/${encodeURIComponent(chronoPointer)}`;
    link.target = "_blank";
    link.rel = "noopener";
    link.textContent = short(chronoPointer, 8);
    rows.push(["ChronoNode Archive", link]);
  }

  // Storage key listing (if present)
  let storageSection = null;
  if (Array.isArray(storageKeys) && storageKeys.length > 0) {
    storageSection = document.createElement("div");
    storageSection.className = "result-txlist";
    const h5 = document.createElement("h5");
    h5.textContent = `Storage Keys (${storageKeys.length})`;
    storageSection.appendChild(h5);
    storageKeys.slice(0, 20).forEach(key => {
      const row = document.createElement("div");
      row.style.cssText = "font-family:var(--mono); font-size:0.76rem; padding:0.2rem 0; color:var(--text-2); word-break:break-all;";
      row.textContent = typeof key === "string" ? key : JSON.stringify(key);
      storageSection.appendChild(row);
    });
    if (storageKeys.length > 20) {
      const more = document.createElement("div");
      more.style.cssText = "font-size:0.72rem; color:var(--text-3); padding-top:0.3rem;";
      more.textContent = `+ ${storageKeys.length - 20} more keys`;
      storageSection.appendChild(more);
    }
  }

  return card("Contract", rows, storageSection);
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

/* ── Mempool ── */
async function loadMempool() {
  if (!el.mempoolList) return;

  try {
    const data = await api("/api/v1/mempool").catch(() => null);
    const pending = data?.transactions || data?.pending || [];
    el.mempoolList.innerHTML = "";

    if (!Array.isArray(pending) || pending.length === 0) {
      el.mempoolList.innerHTML = '<div class="mempool-empty">Mempool is empty — no pending transactions.</div>';
      return;
    }

    pending.slice(0, 25).forEach(tx => {
      const txHash = tx.hash_hex || tx.hash || bytesToHex(tx.hash) || "—";
      const sender = tx.sender_hex || tx.from || bytesToHex(tx.sender) || "?";

      let payloadStr = "—";
      const p = tx.payload;
      if (p) {
        if (p.Transfer) payloadStr = `Transfer ${fmtNum(p.Transfer.amount)}`;
        else if (p.ContractDeploy) payloadStr = "Contract Deploy";
        else if (p.ContractCall) payloadStr = `Call: ${p.ContractCall.method || "?"}`;
        else if (p.Data) payloadStr = `Data (${(p.Data.data || []).length}B)`;
      }

      const gasStr = `${fmtNum(tx.gas_limit || 0)} gas`;
      const priority = tx.priority ?? 0;

      const item = document.createElement("div");
      item.className = "mempool-item";

      const hashEl = makeLink(short(txHash, 8), "tx-hash", txHash);
      const payEl = document.createElement("span");
      payEl.className = "mempool-payload";
      payEl.textContent = `${short(sender, 6)} → ${payloadStr}`;
      const gasEl = document.createElement("span");
      gasEl.className = "mempool-gas";
      gasEl.textContent = gasStr;
      const prioEl = document.createElement("span");
      prioEl.className = `mempool-priority ${priority > 0 ? "high" : "normal"}`;
      prioEl.textContent = priority > 0 ? `P${priority}` : "STD";

      item.append(hashEl, payEl, gasEl, prioEl);
      el.mempoolList.appendChild(item);
    });

    if (pending.length > 25) {
      const more = document.createElement("div");
      more.style.cssText = "font-size:0.78rem; color:var(--text-3); padding-top:0.3rem;";
      more.textContent = `+ ${pending.length - 25} more pending`;
      el.mempoolList.appendChild(more);
    }
  } catch {
    el.mempoolList.innerHTML = '<div class="mempool-empty">Unable to load mempool.</div>';
  }
}

/* ── Hash Routing (Ecosystem Explorer Spec alignment) ── */
function handleHashRoute() {
  const hash = window.location.hash;
  if (!hash) return;

  if (hash.startsWith("#explorer")) {
    const route = hash.replace(/^#explorer/, "");
    if (!route || route === "/") {
      document.getElementById("explorer")?.scrollIntoView({ behavior: "smooth" });
      return;
    }

    if (route.startsWith("/blocks/hash/")) {
      const bHash = route.replace(/^\/blocks\/hash\//, "");
      if (bHash) doSearch("block-hash", bHash);
    } else if (route.startsWith("/blocks/")) {
      const height = route.replace(/^\/blocks\//, "");
      if (height) doSearch("block-height", height);
    } else if (route.startsWith("/tx/")) {
      const txHash = route.replace(/^\/tx\//, "");
      if (txHash) doSearch("tx-hash", txHash);
    } else if (route.startsWith("/accounts/")) {
      const address = route.replace(/^\/accounts\//, "");
      if (address) doSearch("account", address);
    } else if (route.startsWith("/contracts/")) {
      const sub = route.replace(/^\/contracts\//, "");
      const parts = sub.split("/");
      const contractId = parts[0];
      if (contractId) {
        doSearch("contract-id", contractId);
        if (parts[1] === "storage") {
          setTimeout(() => {
            const elCard = document.querySelector(".result-card");
            if (elCard) elCard.scrollIntoView({ behavior: "smooth" });
          }, 300);
        }
      }
    } else if (route === "/blocks") {
      document.querySelector(".blocks-section")?.scrollIntoView({ behavior: "smooth" });
    } else if (route === "/contracts") {
      el.searchType.value = "contract-id";
      el.searchInput.value = "";
      document.getElementById("explorer")?.scrollIntoView({ behavior: "smooth" });
    } else if (route.startsWith("/proofs/account/")) {
      const address = route.replace(/^\/proofs\/account\//, "");
      if (address) doSearch("account", address);
    } else if (route.startsWith("/proofs/contract/")) {
      const sub = route.replace(/^\/proofs\/contract\//, "");
      const parts = sub.split("/");
      const contractId = parts[0];
      if (contractId) doSearch("contract-id", contractId);
    } else if (route === "/validators") {
      document.getElementById("network")?.scrollIntoView({ behavior: "smooth" });
    } else if (route === "/peers") {
      document.getElementById("network")?.scrollIntoView({ behavior: "smooth" });
    } else if (route === "/mempool") {
      document.getElementById("mempoolReload")?.scrollIntoView({ behavior: "smooth" });
    } else if (route === "/fees") {
      document.getElementById("ciFeePolicy")?.scrollIntoView({ behavior: "smooth" });
    } else if (route === "/oracle/attestations") {
      document.getElementById("oracleReload")?.scrollIntoView({ behavior: "smooth" });
    } else if (route === "/health") {
      document.getElementById("network")?.scrollIntoView({ behavior: "smooth" });
    }
  }
}

/* ── Init ── */
function init() {
  el.cfgApi.value = cfg.api;
  el.cfgWs.value = cfg.ws;
  if (el.cfgChrono) el.cfgChrono.value = cfg.chrono;
  if (el.cfgResurge) el.cfgResurge.value = cfg.resurge;
  if (el.cfgCanvas) el.cfgCanvas.value = cfg.canvas;
  bindEcosystemLinks();

  el.cfgSave.onclick = () => {
    cfg = {
      api: el.cfgApi.value.replace(/\/+$/, "") || defaults.api,
      ws: el.cfgWs.value.replace(/\/+$/, "") || defaults.ws,
      chrono: (el.cfgChrono?.value || "").replace(/\/+$/, "") || defaults.chrono,
      resurge: (el.cfgResurge?.value || "").replace(/\/+$/, "") || defaults.resurge,
      canvas: (el.cfgCanvas?.value || "").replace(/\/+$/, "") || defaults.canvas,
    };
    saveCfg(cfg);
    bindEcosystemLinks();
    refresh();
  };

  el.searchForm.addEventListener("submit", runSearch);
  if (el.unifiedForm) el.unifiedForm.addEventListener("submit", runUnifiedSearch);
  if (el.oracleReload) el.oracleReload.onclick = () => loadOracleAttestations();
  if (el.oracleChain) el.oracleChain.onchange = () => loadOracleAttestations();
  if (el.mempoolReload) el.mempoolReload.onclick = () => loadMempool();
  el.wsConnect.onclick = wsConnect;
  el.wsDisconnect.onclick = () => { if (ws) { ws.close(); ws = null; } };
  el.wsClear.onclick = () => { el.eventLog.innerHTML = ""; };

  refresh();
  handleHashRoute();
  window.addEventListener("hashchange", handleHashRoute);
  setInterval(refresh, 15000);
}

init();
