# Ecosystem Explorer Architecture Spec

**Appendix Title:** Explorer & Registry Layer for BaaLS, ChronoNode, Resurgence, and CanvasContracts  
**Version:** v0.1  
**Status:** Scaffolding spec aligned to current implementation (2026-05-25)  
**Target Projects:** BaaLS, ChronoNode, Resurgence Protocol, CanvasContracts  

---

## 1. Purpose

This appendix defines the explorer and registry surfaces needed across the four-project ecosystem.

This is a scaffolding spec. When current implementation differs from ideal architecture, this document should prefer what is live now and mark future capabilities explicitly as planned.

The ecosystem should not use one generic “block explorer” for everything. Only BaaLS is a chain/runtime that needs a true block explorer. ChronoNode, Resurgence, and CanvasContracts each need role-specific visibility tools:

```text
BaaLS             -> Block Explorer
ChronoNode        -> Proof / Archive Explorer
Resurgence        -> Protocol / Staking / Governance Explorer
CanvasContracts   -> Contract Artifact / Manifest Registry
```

The goal is to give users, developers, auditors, and operators a clear way to inspect activity, verify proofs, trace attestations, and understand contract deployments without forcing every project into the same explorer model.

---

## 2. High-Level Explorer Split

| Project | Explorer Type | Primary Purpose |
|---|---|---|
| BaaLS | Block Explorer | Inspect blocks, transactions, accounts, contracts, validators, fees, mempool, and oracle attestations. |
| ChronoNode | Proof / Archive Explorer | Inspect archived chains, Merkle checkpoints, content-addressed storage, watched addresses, dormancy proofs, and proof verification results. |
| Resurgence Protocol | Protocol Explorer | Inspect staking pools, rewards, governance, oracle mints, CCIP messages, treasury fees, and non-EVM dormancy claims. |
| CanvasContracts | Artifact Registry | Inspect visual graphs, compiled WASM, WIT interfaces, manifests, validation reports, deployments, and graph-to-WASM provenance. |

---

## 3. BaaLS Block Explorer

### 3.1 Role

BaaLS is the runtime and local-first chain layer, so it needs the main block explorer.

The BaaLS explorer should answer:

```text
What block was produced?
Who signed it?
What transactions were included?
Which contract was deployed or called?
What changed in state?
What fees were charged and where did they go?
Which validators/signers are active?
What oracle attestations are recorded?
```

### 3.2 Required Pages

```text
/explorer
/explorer/blocks
/explorer/blocks/:height
/explorer/blocks/hash/:hash
/explorer/tx/:hash
/explorer/accounts/:address
/explorer/contracts
/explorer/contracts/:contract_id
/explorer/contracts/:contract_id/storage
/explorer/proofs/account/:address
/explorer/proofs/contract/:contract_id/:key
/explorer/validators
/explorer/peers
/explorer/mempool
/explorer/fees
/explorer/oracle/attestations
/explorer/health
```

### 3.3 Core Features

#### Blocks

Display:

```text
height
hash
previous hash
timestamp
producer/proposer
quorum signatures
validator signatures
transaction count
gas used
fees collected
state root
contract root
finality status
```

#### Transactions

Display:

```text
hash
sender
recipient or contract id
payload type
gas limit
gas used
gas price
fee mode
operator/treasury/burn split
status
block height
timestamp
signature verification status
events
```

#### Accounts

Display:

```text
public key/address
balance
nonce
transaction history
contract interactions
proof availability
last activity
```

#### Contracts

Display:

```text
contract id
code hash
deployer
deployment block
ABI/manifest link if available
storage keys
events
calls
gas history
ChronoNode archive pointer
Canvas manifest pointer if applicable
```

#### Validators / Signers

Display:

```text
active signer set
current proposer
quorum threshold
round-robin status
validator public keys
recent signed blocks
missed proposals
peer status
```

#### Oracle Attestations

Display:

```text
chain id
source wallet/address
evm wallet
proof hash
chrononode signer pubkey/signature presence
baals signer pubkey/signature presence
attested_at_block
baals_block_hash
EVM submitter status (pending/submitted/failed)
Resurgence tx hash (if submitted)
submission timestamp
```

### 3.4 API Dependencies

The BaaLS explorer should use:

```text
GET /api/v1/health
GET /api/v1/blocks/latest
GET /api/v1/blocks/:height
GET /api/v1/blocks/hash/:hash
GET /api/v1/transactions/:hash
GET /api/v1/transactions/:hash/finality
GET /api/v1/transactions/address/:address
GET /api/v1/contracts/:contract_id/state
GET /api/v1/contracts/:contract_id/abi
GET /api/v1/proofs/account/:address
GET /api/v1/proofs/contract/:contract_id/storage/:key
GET /api/v1/oracle/attestations/:chain
GET /api/v1/oracle/attestations/:chain/:address
WebSocket channels: blocks, transactions, mempool
```

### 3.5 Priority

**Priority:** Highest  
**Reason:** This is the base explorer for the whole ecosystem.

---

## 4. ChronoNode Proof / Archive Explorer

### 4.1 Role

ChronoNode is not a chain. It is a verifiable archival and proof layer.

Its explorer should answer:

```text
What data was archived?
Where is the content stored?
What checkpoint proves it?
Can the proof be verified?
When was this wallet last active?
Is this wallet dormant?
Was a dormancy proof submitted to BaaLS?
```

### 4.2 Required Pages

```text
/proofs
/proofs/chains
/proofs/chains/:chain_id
/proofs/chains/:chain_id/blocks/:height
/proofs/chains/:chain_id/checkpoints
/proofs/checkpoints/:checkpoint_id
/proofs/verify
/proofs/addresses/:chain_id/:address
/proofs/addresses/:chain_id/:address/last-seen
/proofs/addresses/:chain_id/:address/dormancy
/proofs/attestations
/proofs/attestations/:id
/proofs/storage/:pointer
/proofs/health
```

### 4.3 Core Features

#### Chain Archive View

Display:

```text
chain id
adapter type
latest indexed height
latest archived height
storage backend
checkpoint count
proof count
last ingest time
index health
```

#### Block Archive Detail

Display:

```text
chain id
height
block hash
storage pointer
content hash
timestamp
indexed transaction count
event count
checkpoint membership
Merkle proof availability
```

#### Checkpoint Detail

Display:

```text
checkpoint id
chain id
height range
root hash
signature
signer public key
created timestamp
storage pointer
verification status
```

#### Proof Verification

Allow users to paste/upload:

```text
proof.json
checkpoint.json
storage pointer
block hash
dormancy proof
```

Return:

```text
valid: true/false
reason
verified root
verified block hash
verified storage pointer
verified signer
```

#### Dormancy Explorer

Display:

```text
watched address
chain id
last seen block
last seen timestamp
dormancy threshold
dormancy status
proof hash
signed proof
BaaLS attestation status
Resurgence submission status
```

### 4.4 API Dependencies

ChronoNode explorer (read-only mode) should use:

```text
GET /health
GET /v1/chains
GET /v1/chains/{chain_id}/blocks/{height}
GET /v1/chains/{chain_id}/blocks?from=0&to=100
GET /v1/chains/{chain_id}/blocks/hash/{hash}
GET /v1/checkpoints/{checkpoint_id}
GET /v1/chains/{chain_id}/proofs/block/{height}
POST /v1/proofs/verify
GET /v1/chains/{chain_id}/addresses/{address}/last-seen
GET /v1/chains/{chain_id}/addresses/{address}/dormancy
GET /v1/chains/{chain_id}/addresses/{address}/dormancy/proof
GET /v1/artifacts/{content_hash}
GET /v1/artifacts/{content_hash}/bytes
GET /metrics
```

Operator/submitter actions (not default public explorer reads):

```text
POST /v1/chains/{chain_id}/checkpoints
POST /v1/attestations/submit
POST /v1/artifacts
```

### 4.5 Priority

**Priority:** High  
**Reason:** ChronoNode is the trust/proof layer. Its explorer makes the ecosystem auditable.

---

## 5. Resurgence Protocol Explorer

### 5.1 Role

Resurgence runs on EVM chains, so it does not need a raw block explorer. Arbiscan, Polygonscan, Basescan, etc. already handle raw EVM blocks and transactions.

Resurgence needs a protocol-specific explorer.

It should answer:

```text
Which pools exist?
Which dead coins are staked?
Who claimed rewards?
How much RESURGE was minted?
Which governance proposals passed?
Which CCIP messages are pending or delivered?
Which non-EVM dormancy proofs minted rewards?
How much treasury fee was collected?
```

### 5.2 Required Pages

```text
/resurgence
/resurgence/pools
/resurgence/pools/:pool_address
/resurgence/users/:address
/resurgence/rewards
/resurgence/claims
/resurgence/governance
/resurgence/governance/:proposal_id
/resurgence/oracle
/resurgence/oracle/non-evm
/resurgence/oracle/proofs/:proof_hash
/resurgence/ccip
/resurgence/ccip/:message_id
/resurgence/treasury
/resurgence/subgraph-status
/resurgence/contracts
```

### 5.3 Core Features

#### Pool Explorer

Display:

```text
pool address
dead coin token
staking pool status
reward rate
TVL
user count
total staked
total rewards minted
protocol fee bps
treasury address
last claim
pause status
```

#### User Explorer

Display:

```text
wallet address
staked dead coins
staked RESURGE
pending rewards
claimed rewards
governance voting power
non-EVM registered wallets
attestation history
```

#### Governance Explorer

Display:

```text
proposal id
title
description
status
snapshot block
voting start/end
for/against/abstain votes
quorum progress
queue status
execute status
timelock transaction
linked contract calls
```

#### Oracle / Non-EVM Explorer

Display:

```text
source chain: bitcoin-light / dogecoin / other
source wallet
registered EVM wallet
dormancy proof hash
ChronoNode proof link
BaaLS attestation reference (chain_id + source wallet)
BaaLS attested_at_block / baals_block_hash
EVM submission tx
RewardDistributor processed status
mint amount
replay protection status
```

#### CCIP Explorer

Display:

```text
source chain
destination chain
sender
receiver
message id
amount
fee
gas limit
delivery status
processed status
linked pool
linked user
```

#### Treasury Explorer

Display:

```text
fees collected
fee source pool
fee token / RESURGE amount
claim tx
treasury balance
distribution history
```

### 5.4 Data Sources

Resurgence explorer should use:

```text
Self-hosted subgraph
EVM RPC read calls
Arbiscan/Polygonscan links
BaaLS oracle endpoints (read-only attestations)
ChronoNode proof endpoints (dormancy + proof verify)
CCIP explorer links
Frontend contract config
Deployment manifest
```

### 5.5 Priority

**Priority:** High  
**Reason:** Resurgence is the user-facing protocol and needs a clean trust dashboard.

---

## 6. CanvasContracts Artifact Registry

### 6.1 Role

CanvasContracts is not a chain and should not have a block explorer.

It needs an artifact registry that proves:

```text
This visual graph produced this WASM.
This WASM passed validation.
This manifest describes the artifact.
This artifact was deployed to BaaLS.
This deployment was archived by ChronoNode.
```

### 6.2 Required Pages

```text
/canvas
/canvas/projects
/canvas/projects/:project_id
/canvas/graphs/:graph_hash
/canvas/artifacts/:wasm_hash
/canvas/manifests/:manifest_hash
/canvas/interfaces/:wit_package
/canvas/deployments/:contract_id
/canvas/security-reports/:report_id
/canvas/templates
/canvas/node-packs
```

### 6.3 Core Features

#### Project View

Display:

```text
project name
owner/local identity
graph versions
compiled artifacts
deployments
validation history
last modified
```

#### Graph View

Display:

```text
graph hash
graph JSON
node count
edge count
node types
validation status
warnings
simulation output
compiled wasm hash
```

#### WASM Artifact View

Display:

```text
wasm hash
wasm size
compiler version
target runtime profile
exports
imports
forbidden import check
wasm-tools validation result
gas estimate
BaaLS compatibility status
ChronoNode archive pointer
```

#### Manifest View

Display:

```text
manifest hash
graph hash
wasm hash
compiler version
WIT interfaces
runtime profile
host imports
security report hash
deployment records
signature
```

#### Deployment View

Display:

```text
BaaLS contract id
BaaLS tx hash
deployer
deployment block
manifest hash
wasm hash
init args hash
current state link
ChronoNode proof link
```

### 6.4 Manifest Schema

Every Canvas compile should produce a manifest:

```json
{
  "schema_version": "canvas.manifest.v1",
  "project_name": "DormancyOracle",
  "graph_hash": "sha256:<hash>",
  "wasm_hash": "sha256:<hash>",
  "manifest_hash": "sha256:<hash>",
  "compiler": {
    "name": "canvas-contracts",
    "version": "0.1.0",
    "commit": "<git_commit>"
  },
  "target": {
    "runtime": "baals-wasm",
    "runtime_profile": "baals-wasm-v1"
  },
  "interfaces": {
    "wit_packages": [
      "baals:storage@1.0.0",
      "baals:crypto@1.0.0",
      "baals:oracle@1.0.0"
    ]
  },
  "wasm": {
    "size_bytes": 0,
    "exports": ["main"],
    "imports": [],
    "validation": {
      "wasm_tools": "pass",
      "wasmtime": "pass",
      "forbidden_imports": []
    }
  },
  "gas": {
    "estimated": 0,
    "max_allowed": 1000000
  },
  "security": {
    "report_hash": "sha256:<hash>",
    "risk_level": "low"
  },
  "deployments": [
    {
      "chain": "baals",
      "contract_id": "<contract_id>",
      "tx_hash": "<tx_hash>",
      "block_height": 0
    }
  ]
}
```

### 6.5 Priority

**Priority:** Medium-High  
**Reason:** This turns CanvasContracts into a verifiable build/deployment platform instead of just a visual editor.

---

## 7. Unified Ecosystem UI

### 7.1 Recommended Route Layout

A unified ecosystem UI can live under `baals.network` or another umbrella domain.

```text
baals.network
  /explorer       -> BaaLS Block Explorer
  /proofs         -> ChronoNode Proof Explorer
  /resurgence     -> Resurgence Protocol Explorer
  /canvas         -> Canvas Artifact Registry
```

Alternative subdomain layout:

```text
explorer.baals.network     -> BaaLS Block Explorer
chrono.baals.network       -> ChronoNode API + Proof Explorer
resurge.baals.network      -> Resurgence Protocol Explorer
canvas.baals.network       -> Canvas Artifact Registry
```

### 7.2 Shared Components

Build shared UI components:

```text
AddressCard
HashCard
BlockLink
TxLink
ProofStatusBadge
FinalityBadge
ContractManifestCard
DeploymentManifestCard
AttestationTimeline
EventTimeline
CopyableHash
ExternalExplorerLink
```

### 7.3 Shared Data Model

Use common identifiers across explorers:

```text
baals_tx_hash
baals_block_height
chrono_proof_hash
chrono_checkpoint_id
evm_tx_hash
ccip_message_id
canvas_graph_hash
canvas_wasm_hash
canvas_manifest_hash
resurgence_proof_hash
```

### 7.4 Cross-Linking Rules

```text
BaaLS oracle attestation -> ChronoNode dormancy proof
BaaLS oracle attestation -> Resurgence EVM mint tx
Canvas deployment -> BaaLS contract page
Canvas manifest -> ChronoNode archived artifact
Resurgence non-EVM proof -> ChronoNode proof page
Resurgence oracle mint -> BaaLS attestation page
ChronoNode proof -> BaaLS attestation if submitted
```

---

## 8. Explorer Data Flow

### 8.1 Dormancy Proof Flow

```text
ChronoNode detects dormant BTC/DOGE wallet
  -> ChronoNode signs DormancyProof
  -> ChronoNode submits proof to BaaLS
  -> BaaLS records attestation
  -> BaaLS EVMSubmitter submits to Resurgence RewardDistributor
  -> Resurgence mints RESURGE
  -> Subgraph indexes event
  -> Explorers cross-link all steps
```

### 8.2 Canvas Deployment Flow

```text
Canvas graph created
  -> graph validates
  -> graph compiles to WASM
  -> manifest generated
  -> artifact archived by ChronoNode
  -> WASM deployed to BaaLS
  -> BaaLS explorer shows contract
  -> Canvas registry shows graph -> WASM -> deployment provenance
```

---

## 9. Implementation Priorities

### Phase 1 — BaaLS Explorer Hardening

```text
1. Block list/detail
2. Transaction detail
3. Account detail
4. Contract detail
5. Validator/signers panel
6. Oracle attestation panel
7. WS live updates
```

### Phase 2 — ChronoNode Proof Explorer

```text
1. Chain archive list
2. Block archive detail
3. Checkpoint detail
4. Proof verification UI
5. Watched address / dormancy status
6. Attestation timeline
```

### Phase 3 — Resurgence Protocol Explorer

```text
1. Pool explorer
2. User explorer
3. Governance explorer
4. Oracle proof explorer
5. CCIP message explorer
6. Treasury explorer
7. Subgraph health page
```

### Phase 4 — Canvas Artifact Registry

```text
1. Manifest schema
2. Graph hash page
3. WASM artifact page
4. Deployment page
5. Validation/security report page
6. ChronoNode archival links
```

### Phase 5 — Unified Ecosystem UI

```text
1. Shared route shell
2. Shared cards/components
3. Unified search
4. Cross-project timeline
5. Public proof dashboard
```

---

## 10. Unified Search

A global search bar should detect and route:

```text
Block height             -> BaaLS block page or ChronoNode block archive
BaaLS tx hash            -> BaaLS tx page
EVM tx hash              -> Resurgence tx/external explorer link
Contract id              -> BaaLS contract or Resurgence contract page
Address                  -> BaaLS account / Resurgence user / ChronoNode watched address
Proof hash               -> ChronoNode proof / Resurgence oracle proof
CCIP message id          -> Resurgence CCIP page
WASM hash                -> Canvas artifact page
Manifest hash            -> Canvas manifest page
```

---

## 11. Security & Trust Requirements

### 11.1 Do Not Expose Secrets

Explorers must never show:

```text
private validator keys
JWT signing secrets
EVM private keys
webhook URLs
RPC private keys
server-only config
```

### 11.2 Publicly Safe Data

Explorers may show:

```text
public keys
contract addresses
transaction hashes
proof hashes
checkpoint roots
deployment manifests
public service health
public validator identities
```

### 11.3 Proof Verification

Any proof page should clearly show:

```text
Verified
Unverified
Failed
Pending
Experimental
Trusted oracle
Cryptographic proof
```

Do not imply full trustless verification when a proof is currently role-gated or oracle-submitted.

---

## 12. Recommended Names

```text
BaaLS Explorer
ChronoNode Proof Explorer
Resurgence Protocol Explorer
Canvas Artifact Registry
Ecosystem Timeline
```

---

## 13. Definition of Done

This appendix is complete when:

```text
1. BaaLS has a usable block/tx/account/contract explorer.
2. ChronoNode has a proof verification and dormancy status explorer.
3. Resurgence has staking/governance/oracle/CCIP protocol visibility.
4. CanvasContracts has graph/WASM/manifest/deployment provenance pages.
5. All four surfaces cross-link shared identifiers.
6. Public users can trace:
   ChronoNode proof -> BaaLS attestation -> Resurgence mint.
7. Developers can trace:
   Canvas graph -> WASM artifact -> BaaLS contract deployment.
```

---

## 14. Final Recommendation

Only BaaLS needs a true block explorer.

The rest should use role-specific explorers:

```text
BaaLS             = Block Explorer
ChronoNode        = Proof Explorer
Resurgence        = Protocol Explorer
CanvasContracts   = Artifact Registry
```

This gives the ecosystem more legitimacy than forcing all projects into one generic explorer. It also makes the full stack easier to explain, audit, and demonstrate.
