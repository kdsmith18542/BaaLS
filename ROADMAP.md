# BaaLS Ecosystem Roadmap

## Platform Vision

BaaLS is the core runtime in a multi-project blockchain ecosystem. The full stack:

```
┌─────────────────────────────────────────────────────┐
│              Canvas Contracts (UX layer)             │
│        Visual contract builder → WASM output         │
│        Deploys to BaaLS, queries ChronoNode          │
└──────────────┬─────────────────────┬────────────────┘
               │ deploy              │ history/proofs
               ▼                    ▼
┌──────────────────────┐  ┌──────────────────────────┐
│   BaaLS (runtime)    │  │  ChronoNode (indexer)     │
│   Ed25519 PoA        │──│  Content-addressed        │
│   WASM execution     │  │  archival + Merkle proofs │
│   P2P consensus      │  │  over IPFS/Pinata/local   │
└──────────────────────┘  └──────────────────────────┘
        blocks ──────────────►  ingest

┌─────────────────────────────────────────────────────┐
│           Resurgence (application layer)             │
│     Proof-of-Dormancy staking + governance           │
│     First real transacting app on BaaLS              │
└─────────────────────────────────────────────────────┘
```

### Project Locations

| Project | Path | Stack | Status |
|---------|------|-------|--------|
| BaaLS | `G:\BACKUP\baals` | Rust | Live — 2 validators on VPS |
| Canvas Contracts | `G:\BACKUP\canvascontract` | Rust/React/Tauri | ~65% — WASM codegen needs implementation |
| ChronoNode | `G:\BACKUP\chrononode\chrononode` | Rust (6 crates, ~43k LOC) | Mid-beta — working binary, Docker/K8s, BaaLS adapter functional |
| Resurgence Protocol | `G:\BACKUP\resurgence-protocol` | Solidity/TypeScript | 107 tests pass — porting to BaaLS WASM |
| Trellis | `C:\...\GrindSquad.Online\sub-projects\trellis` | F# / .NET 10 | Working — future BaaLS developer tooling |

---

## Phase 1: Resurgence Port (Active Priority)

Port Resurgence Protocol's staking and governance model from Solidity/EVM to WASM contracts on BaaLS. This is the first application that will generate real, recurring transaction traffic on the BaaLS chain.

### What Resurgence Does (Currently)

- Users stake abandoned/dead ERC-20 tokens into specialized pools
- Earns RESURGE governance token based on staking duration
- On-chain governance (OZ Governor + TimelockController) for adding new pools, adjusting rates
- Boost multipliers for long-term stakers, early-unstake penalties
- Chainlink oracle integration for reward distribution
- UUPS upgradeable proxy contracts

### Port Scope

**Contracts to rewrite as BaaLS WASM:**

1. **ResurgeToken** — ERC-20-equivalent with voting, permit, cap, burn, access control
2. **DeadCoinStakingPool** — Per-pool staking with per-second reward accrual
3. **ResurgeStakingPool** — Native RESURGE staking with boost multipliers and penalties
4. **StakingPoolManager** — Registry for deploying/pausing/removing pools, rate adjustment
5. **RewardDistributor** — Controlled minting gated by authorized pools
6. **ResurgenceGovernance** — Proposal creation, voting, execution with timelock

**Transaction types this generates on BaaLS:**

- `stake` / `unstake` — every user deposit/withdrawal
- `claim_rewards` — reward accrual claims
- `create_proposal` / `vote` / `execute_proposal` — governance activity
- `add_pool` / `pause_pool` / `adjust_rate` — admin operations
- `boost` — multiplier activations

**Integration points:**

- Canvas Contracts: build the staking contracts visually, compile to WASM
- BaaLS: deploy and execute the contracts, process transactions
- ChronoNode: archive staking history, generate Merkle proofs for audit

### Steps

1. Define WASM contract interface for token + staking operations on BaaLS
2. Implement ResurgeToken as WASM contract (mint, transfer, burn, balance, voting power)
3. Implement staking pool contracts (deposit, withdraw, reward calculation)
4. Implement governance contract (propose, vote, execute)
5. Deploy to live BaaLS chain
6. Build frontend (can adapt existing Resurgence Next.js frontend, point at BaaLS API)
7. Wire ChronoNode to index staking events

---

## Phase 2: ChronoNode Live Integration

ChronoNode already has a functional BaaLS adapter (`chrononode-adapter-baals`). Connect it to the live VPS nodes.

### Steps

1. Point BaaLS adapter at VPS node API (via Caddy proxy at baals.network)
2. Configure IPFS/Pinata or local-fs storage backend for block archival
3. Start ingesting live blocks — verify protobuf conversion and indexing
4. Enable Merkle proof generation over archived chain history
5. Expose query API for block/tx/event lookups with proofs
6. Wire into Resurgence frontend for staking history audit

### Current State

- 43k lines of Rust across 6 crates
- Working CLI binary (32MB)
- Docker and docker-compose configs ready
- Kubernetes manifests for horizontal scaling
- BaaLS adapter: 255 LOC with retry logic and integration tests
- SQLite indexing, HTTP API with 10+ endpoints

---

## Phase 3: Canvas Contracts Completion

Finish the visual contract builder so developers can build WASM contracts without writing Rust directly.

### Critical Gap — ✅ RESOLVED

The `AST → WASM` codegen is **fully implemented** (1015 lines in `canvascontract/src/compiler/wasm_gen.rs`). The compilation pipeline (Graph IR → AST → validator → WASM codegen → wasmtime validation) produces real WASM modules. 9 tests pass including wasmtime execution tests for arithmetic, conditionals, storage, and imports for all 4 host function families (baals, crypto, chrononode, resurgence).

### Steps

1. ~~Implement real WASM codegen from Canvas AST (replace hardcoded stub)~~ ✅ DONE
2. Wire BaaLS deployment to live endpoints (replace mock BaalsClient)
3. ~~Complete remaining frontend node types in React palette~~ ✅ DONE — now 39 node types across 8 categories
4. ~~Connect debugger/timeline view to ChronoNode for contract execution history~~ ✅ DONE — simulation trace panel with per-step input/output/error
5. Test end-to-end: build contract visually → compile → deploy to BaaLS → verify execution

### Current State

- ~85% feature complete
- Backend compiler pipeline works end-to-end (Graph IR → AST → validator → WASM codegen → wasmtime validation)
- CLI functional (compile, simulate, validate, deploy, editor commands)
- React frontend with drag-and-drop canvas, 39 node types, 8 categories, compilation + simulation workflows
- 39 node types implemented (logic, arithmetic, control flow, state, crypto, BaaLS runtime, ChronoNode, Resurgence)

---

## Phase 4: Trellis Integration (Future)

Adapt Trellis's "bring your own chain" developer experience for BaaLS node bootstrapping.

### Concept

Trellis currently provides `trellis init` → scaffold a new DPoS chain, `trellis start` → run it, `trellis dev-net` → local test network. The idea is to bring that same one-command developer experience to BaaLS:

```
baals-cli init my-chain    # generate keys, config, data dir
baals-cli start            # run a node with sane defaults
baals-cli dev-net          # spin up a local multi-validator test network
```

### Considerations

- Trellis is F#/.NET, BaaLS is Rust — code reuse is minimal, this is a concept port
- Focus on the developer onboarding experience, not the chain internals
- Each scaffolded BaaLS instance could connect to the main network as a peer
- Dev-net mode useful for contract development and testing before mainnet deploy

### Current Trellis State

- Working F#/.NET blockchain dev kit
- DPoS consensus, RocksDB storage, Avalonia desktop wallet
- CLI scaffolding, gRPC, WebSocket streaming, block explorer
- Benchmarks at ~2,400 tx/s and ~850 blocks/s sync

---

## BaaLS Core — Remaining Work

Independent of ecosystem integrations, BaaLS itself still has:

### Medium Priority

- RocksDB storage backend (third option alongside sled and redb)

### Lower Priority

- Mobile SDKs (iOS/Android)
- PoS / PoW / CRDT consensus plugins
- Remove stale 3rd signer from validator list

---

## Related Projects (Independent)

These are separate blockchain projects in the portfolio, not integrated with BaaLS but part of the broader body of work:

| Project | Path | What | Status |
|---------|------|------|--------|
| Rusty-Coin ($RUST) | `G:\BACKUP\Rusty-Coin` | Hybrid PoW+PoS, masternodes, sidechains, FerrisScript, PQ crypto | Mid-stage, 15 Rust crates |
| Vigil (VGL) | `C:\...\vigil-protocol\Vigil-Project` | Decred fork with KawPoW GPU mining + PoS ticket voting | Phase 1 ~60%, node compiles, KawPoW integrated |
