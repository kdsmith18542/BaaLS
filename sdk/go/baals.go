// Package baals provides a pure Go SDK for interacting with a BaaLS blockchain node.
//
// The SDK communicates with a baalsd daemon process via HTTP endpoints and
// the baalsd CLI tool. It replaces the previous CGo-based implementation that
// linked directly against the Rust FFI library.
//
// Usage:
//
//	client, err := baals.New("./data")
//	if err != nil {
//	    log.Fatal(err)
//	}
//	defer client.Close()
//	client.Start()
//	state := client.ChainStateJSON()
//	println(state)
package baals

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"time"
)

// ---------------------------------------------------------------------------
// Core types — these mirror the Rust types and use JSON tags compatible with
// serde_json's default external-tag enum representation so that the SDK can
// both produce and consume the same wire format as the baalsd daemon.
// ---------------------------------------------------------------------------

// PublicKey is an Ed25519 verifying key (32 bytes). In JSON it is serialized
// as a byte array (the same format serde_json produces for [u8; 32]).
type PublicKey [32]byte

// NewPublicKeyFromHex decodes a hex-encoded Ed25519 public key.
func NewPublicKeyFromHex(s string) (PublicKey, error) {
	var pk PublicKey
	b, err := hex.DecodeString(s)
	if err != nil {
		return pk, fmt.Errorf("invalid hex: %w", err)
	}
	if len(b) != 32 {
		return pk, errors.New("public key must be 32 bytes")
	}
	copy(pk[:], b)
	return pk, nil
}

// Hex returns the hex-encoded representation of the public key.
func (pk PublicKey) Hex() string { return hex.EncodeToString(pk[:]) }

// MarshalJSON serialises the public key as a JSON array of 32 numbers,
// matching serde_json's default representation of [u8; 32].
func (pk PublicKey) MarshalJSON() ([]byte, error) {
	return marshalByteArray32(pk[:])
}

// UnmarshalJSON deserialises a JSON array of numbers into a PublicKey.
func (pk *PublicKey) UnmarshalJSON(data []byte) error {
	return unmarshalByteArray32(data, pk[:])
}

// Signature is an Ed25519 signature (64 bytes).
type Signature [64]byte

// MarshalJSON serialises the signature as a JSON array of 64 numbers.
func (s Signature) MarshalJSON() ([]byte, error) {
	return marshalByteArray64(s[:])
}

// UnmarshalJSON deserialises a JSON array of numbers into a Signature.
func (s *Signature) UnmarshalJSON(data []byte) error {
	return unmarshalByteArray64(data, s[:])
}

// ContractID is a 32-byte contract identifier.
type ContractID [32]byte

// Hex returns the hex-encoded contract identifier.
func (c ContractID) Hex() string { return hex.EncodeToString(c[:]) }

// NewContractIDFromHex decodes a hex contract ID.
func NewContractIDFromHex(s string) (ContractID, error) {
	var cid ContractID
	b, err := hex.DecodeString(s)
	if err != nil {
		return cid, fmt.Errorf("invalid hex: %w", err)
	}
	if len(b) != 32 {
		return cid, errors.New("contract ID must be 32 bytes")
	}
	copy(cid[:], b)
	return cid, nil
}

// MarshalJSON serialises the contract ID as a JSON array of 32 numbers.
func (c ContractID) MarshalJSON() ([]byte, error) {
	return marshalByteArray32(c[:])
}

// UnmarshalJSON deserialises a JSON array of numbers into a ContractID.
func (c *ContractID) UnmarshalJSON(data []byte) error {
	return unmarshalByteArray32(data, c[:])
}

// Address is either a wallet (public key) or a contract.
type Address struct {
	Wallet   *PublicKey  `json:"Wallet,omitempty"`
	Contract *ContractID `json:"Contract,omitempty"`
}

// TransactionPayload mirrors the Rust TransactionPayload enum.
type TransactionPayload struct {
	Transfer       *TransferPayload       `json:"Transfer,omitempty"`
	ContractDeploy *ContractDeployPayload `json:"ContractDeploy,omitempty"`
	ContractCall   *ContractCallPayload   `json:"ContractCall,omitempty"`
	Data           *DataPayload           `json:"Data,omitempty"`
}

// TransferPayload is a simple value transfer.
type TransferPayload struct {
	Amount uint64 `json:"amount"`
}

// ContractDeployPayload carries WASM bytecode and optional init arguments.
type ContractDeployPayload struct {
	WasmBytes   []byte `json:"wasm_bytes"`
	InitPayload []byte `json:"init_payload"`
}

// ContractCallPayload describes a call to a smart-contract method.
type ContractCallPayload struct {
	Method string   `json:"method"`
	Args   [][]byte `json:"args"`
	Value  *uint64  `json:"value"`
}

// DataPayload is an opaque data blob stored on-chain.
type DataPayload struct {
	Data []byte `json:"data"`
}

// Transaction is a signed transaction on the BaaLS network.
type Transaction struct {
	Hash      [32]byte            `json:"hash"`
	Sender    PublicKey           `json:"sender"`
	Nonce     uint64              `json:"nonce"`
	Timestamp uint64              `json:"timestamp"`
	Recipient Address             `json:"recipient"`
	Payload   TransactionPayload  `json:"payload"`
	Signature Signature           `json:"signature"`
	GasLimit  uint64              `json:"gas_limit"`
	Priority  uint8               `json:"priority"`
	Metadata  map[string]string   `json:"metadata"`
}

// Verify checks the transaction signature against the sender's public key.
func (tx *Transaction) Verify() bool {
	hash := tx.CalculateHash()
	if hash != tx.Hash {
		return false
	}
	return ed25519.Verify(tx.Sender[:], hash[:], tx.Signature[:])
}

// CalculateHash computes the SHA-256 hash of the transaction fields in the
// same order as the Rust implementation. This is used for signature
// verification and the transaction's on-chain identifier.
func (tx *Transaction) CalculateHash() [32]byte {
	h := sha256.New()
	h.Write(tx.Sender[:])
	var buf [8]byte
	putUint64LE(buf[:], tx.Nonce)
	h.Write(buf[:])
	putUint64LE(buf[:], tx.Timestamp)
	h.Write(buf[:])
	putUint64LE(buf[:], tx.GasLimit)
	h.Write(buf[:])
	h.Write([]byte{tx.Priority})

	// recipient (use canonical JSON representation)
	recipJSON, _ := json.Marshal(tx.Recipient)
	h.Write(recipJSON)

	// payload (canonical JSON)
	payloadJSON, _ := json.Marshal(tx.Payload)
	h.Write(payloadJSON)

	// metadata (sorted keys, canonical JSON)
	if len(tx.Metadata) > 0 {
		metaJSON, _ := json.Marshal(tx.Metadata)
		h.Write(metaJSON)
	}

	var out [32]byte
	h.Sum(out[:0])
	return out
}

func putUint64LE(b []byte, v uint64) {
	b[0] = byte(v)
	b[1] = byte(v >> 8)
	b[2] = byte(v >> 16)
	b[3] = byte(v >> 24)
	b[4] = byte(v >> 32)
	b[5] = byte(v >> 40)
	b[6] = byte(v >> 48)
	b[7] = byte(v >> 56)
}

// Account mirrors the Rust Account enum.
type Account struct {
	Wallet   *WalletAccount   `json:"Wallet,omitempty"`
	Contract *ContractAccount `json:"Contract,omitempty"`
}

// WalletAccount holds the balance and nonce of a wallet.
type WalletAccount struct {
	Balance uint64 `json:"balance"`
	Nonce   uint64 `json:"nonce"`
}

// ContractAccount holds the code hash, storage root, and nonce of a contract.
type ContractAccount struct {
	CodeHash        [32]byte `json:"code_hash"`
	StorageRootHash [32]byte `json:"storage_root_hash"`
	Nonce           uint64   `json:"nonce"`
}

// Block represents a block in the BaaLS chain.
type Block struct {
	Index        uint64            `json:"index"`
	Timestamp    uint64            `json:"timestamp"`
	PrevHash     [32]byte          `json:"prev_hash"`
	Hash         [32]byte          `json:"hash"`
	Nonce        uint64            `json:"nonce"`
	Transactions []Transaction     `json:"transactions"`
	Metadata     map[string]string `json:"metadata"`
}

// ChainState summarises the current state of the chain.
type ChainState struct {
	LatestBlockHash  [32]byte `json:"latest_block_hash"`
	LatestBlockIndex uint64   `json:"latest_block_index"`
	AccountsRootHash [32]byte `json:"accounts_root_hash"`
	TotalSupply      uint64   `json:"total_supply"`
}

// ---------------------------------------------------------------------------
// BaalsClient
// ---------------------------------------------------------------------------

const (
	defaultBinName   = "baalsd"
	defaultHTTPPort  = 8080
	defaultStartWait = 5 * time.Second
)

// BaalsClient manages a connection to a BaaLS node by controlling a baalsd
// daemon process and communicating with it over HTTP.
type BaalsClient struct {
	mu sync.Mutex

	dataDir  string
	httpPort int
	binPath  string

	cmd    *exec.Cmd
	cancel context.CancelFunc

	httpClient *http.Client
}

// New creates a new BaalsClient that will store chain data in dataDir.
// The data directory is created if it does not exist.  The baalsd binary is
// located by searching PATH, ../../target/release, and ../../target/debug
// (relative to this source file).
func New(dataDir string) (*BaalsClient, error) {
	abs, err := filepath.Abs(dataDir)
	if err != nil {
		return nil, fmt.Errorf("resolve data dir: %w", err)
	}
	if err := os.MkdirAll(abs, 0755); err != nil {
		return nil, fmt.Errorf("create data dir: %w", err)
	}

	bin, err := findBinary()
	if err != nil {
		return nil, err
	}

	return &BaalsClient{
		dataDir:    abs,
		httpPort:   defaultHTTPPort,
		binPath:    bin,
		httpClient: &http.Client{Timeout: 10 * time.Second},
	}, nil
}

// SetHTTPPort overrides the default daemon health-port (8080).  Must be
// called before Start.
func (b *BaalsClient) SetHTTPPort(port int) {
	b.httpPort = port
}

// SetBinaryPath overrides the automatically-discovered baalsd executable
// path.  Must be called before Start.
func (b *BaalsClient) SetBinaryPath(path string) {
	b.binPath = path
}

// Start launches the baalsd node daemon in the background.  Subsequent
// queries and transactions will be routed through the running daemon.
func (b *BaalsClient) Start() error {
	b.mu.Lock()
	defer b.mu.Unlock()

	if b.cmd != nil {
		return errors.New("baals: already started")
	}
	validPath := regexp.MustCompile(`^[a-zA-Z0-9_\-\./\\:]+$`)
	if !validPath.MatchString(b.binPath) {
		return fmt.Errorf("invalid input")
	}
	if !validPath.MatchString(b.dataDir) {
		return fmt.Errorf("invalid input")
	}

	ctx, cancel := context.WithCancel(context.Background())

	// `baalsd node start --daemon` spawns the actual daemon as a child
	// process and returns immediately.  We just use it as a launcher.
	cmd := exec.CommandContext(ctx, b.binPath,
		"node", "start",
		"--data-dir", b.dataDir,
		"--port", fmt.Sprintf("%d", b.httpPort),
		"--daemon",
	)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = nil

	if err := cmd.Run(); err != nil {
		cancel()
		return fmt.Errorf("baals: start daemon: %w", err)
	}

	b.cmd = cmd // keeps a reference for tracking; cmd has already exited
	b.cancel = cancel

	// Poll the health endpoint to confirm the daemon is running.
	deadline := time.Now().Add(defaultStartWait)
	for time.Now().Before(deadline) {
		if b.healthOK() {
			return nil
		}
		time.Sleep(200 * time.Millisecond)
	}

	// Daemon did not come up in time.
	cancel()
	b.cmd = nil
	b.cancel = nil
	return errors.New("baals: daemon failed to start within timeout")
}

// Stop signals the baalsd daemon to shut down gracefully by writing a stop
// signal file to the data directory and then waiting for the health
// endpoint to become unreachable.
func (b *BaalsClient) Stop() error {
	b.mu.Lock()
	defer b.mu.Unlock()

	if b.cmd == nil {
		return nil
	}
	validPath := regexp.MustCompile(`^[a-zA-Z0-9_\-\./\\:]+$`)
	if !validPath.MatchString(b.binPath) {
		return fmt.Errorf("invalid input")
	}
	if !validPath.MatchString(b.dataDir) {
		return fmt.Errorf("invalid input")
	}

	stop := exec.Command(b.binPath,
		"node", "stop",
		"--data-dir", b.dataDir,
	)
	stop.Stdout = os.Stdout
	stop.Stderr = os.Stderr
	if err := stop.Run(); err != nil {
		return fmt.Errorf("baals: stop command failed: %w", err)
	}

	// Wait for the daemon to shut down (health check will fail).
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		if !b.healthOK() {
			break
		}
		time.Sleep(200 * time.Millisecond)
	}

	if b.cancel != nil {
		b.cancel()
	}
	b.cmd = nil
	b.cancel = nil
	return nil
}

// Close releases all resources held by the client.  If the daemon is still
// running it will be stopped gracefully.
func (b *BaalsClient) Close() error {
	_ = b.Stop()
	return nil
}

// healthURL returns the base URL of the daemon's HTTP health server.
func (b *BaalsClient) healthURL() string {
	return fmt.Sprintf("http://127.0.0.1:%d", b.httpPort)
}

// healthOK returns true when the daemon responds to /health.
func (b *BaalsClient) healthOK() bool {
	resp, err := b.httpClient.Get(b.healthURL() + "/health")
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

// ---------------------------------------------------------------------------
// Query methods — use the baalsd CLI "query" subcommands with --json.
// ---------------------------------------------------------------------------

// ChainStateJSON returns the current chain head as a JSON object.
func (b *BaalsClient) ChainStateJSON() string {
	out, err := runQueryCmd(b.binPath, b.dataDir, "head")
	if err != nil {
		return "{}"
	}
	return string(out)
}

// GetBlockByHeight returns the block at the given height as a JSON object,
// or an empty string if not found.
func (b *BaalsClient) GetBlockByHeight(height uint64) string {
	out, err := runQueryCmd(b.binPath, b.dataDir, "block", fmt.Sprintf("%d", height))
	if err != nil {
		return ""
	}
	return string(out)
}

// GetTransaction returns the transaction with the given 32-byte hex hash as
// a JSON object, or an empty string if not found.
func (b *BaalsClient) GetTransaction(hashHex string) string {
	if !isHex32(hashHex) {
		return ""
	}
	out, err := runQueryCmd(b.binPath, b.dataDir, "tx", hashHex)
	if err != nil {
		return ""
	}
	return string(out)
}

// GetAccount returns the account for the given 32-byte hex public key as a
// JSON object, or an empty string if not found.
func (b *BaalsClient) GetAccount(pubkeyHex string) string {
	if !isHex32(pubkeyHex) {
		return ""
	}
	out, err := runQueryCmd(b.binPath, b.dataDir, "account", pubkeyHex)
	if err != nil {
		return ""
	}
	return string(out)
}

// ---------------------------------------------------------------------------
// Write methods — use HTTP POST endpoints on the daemon's health server.
// ---------------------------------------------------------------------------

// SubmitTx submits a pre-signed transaction (JSON-serialised in serde_json
// format) to the BaaLS node's mempool.
func (b *BaalsClient) SubmitTx(txJSON string) error {
	url := b.healthURL() + "/tx/submit"
	resp, err := b.httpClient.Post(url, "application/json", strings.NewReader(txJSON))
	if err != nil {
		return fmt.Errorf("baals: submit tx: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("baals: submit tx rejected (status %d): %s", resp.StatusCode, string(body))
	}
	return nil
}

// CreateAccount creates a wallet account on-chain with the given initial
// balance.  The public key must be a 32-byte hex string.
func (b *BaalsClient) CreateAccount(pubkeyHex string, balance uint64) error {
	if !isHex32(pubkeyHex) {
		return errors.New("baals: public key must be 32 hex bytes")
	}

	body, _ := json.Marshal(map[string]interface{}{
		"pubkey":  pubkeyHex,
		"balance": balance,
	})

	resp, err := b.httpClient.Post(
		b.healthURL()+"/account",
		"application/json",
		bytes.NewReader(body),
	)
	if err != nil {
		return fmt.Errorf("baals: create account: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		respBody, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("baals: create account failed (status %d): %s", resp.StatusCode, string(respBody))
	}
	return nil
}

// DeployContract deploys WASM bytecode to the chain and returns the hex
// contract ID, or an empty string on failure.
func (b *BaalsClient) DeployContract(deployerHex string, wasm []byte, initPayload []byte, gasLimit uint64) string {
	if !isHex32(deployerHex) {
		return ""
	}

	body, _ := json.Marshal(map[string]interface{}{
		"deployer_hex": deployerHex,
		"wasm_hex":     hex.EncodeToString(wasm),
		"init_hex":     hex.EncodeToString(initPayload),
		"gas_limit":    gasLimit,
	})

	resp, err := b.httpClient.Post(
		b.healthURL()+"/contract/deploy",
		"application/json",
		bytes.NewReader(body),
	)
	if err != nil {
		return ""
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return ""
	}

	var result struct {
		ContractID string `json:"contract_id"`
	}
	if json.NewDecoder(resp.Body).Decode(&result) != nil {
		return ""
	}
	return result.ContractID
}

// CallContract calls a smart-contract method and returns the result as a JSON
// string, or an empty string on failure.
func (b *BaalsClient) CallContract(callerHex, contractIDHex, method string, args []byte, value uint64) string {
	if !isHex32(callerHex) || !isHex32(contractIDHex) {
		return ""
	}

	body, _ := json.Marshal(map[string]interface{}{
		"caller_hex":    callerHex,
		"contract_id":   contractIDHex,
		"method":        method,
		"args_hex":      hex.EncodeToString(args),
		"value":         value,
	})

	resp, err := b.httpClient.Post(
		b.healthURL()+"/contract/call",
		"application/json",
		bytes.NewReader(body),
	)
	if err != nil {
		return ""
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return ""
	}

	var result struct {
		ResultHex string `json:"result_hex"`
		ResultLen int    `json:"result_len"`
	}
	if json.NewDecoder(resp.Body).Decode(&result) != nil {
		return ""
	}
	out, _ := json.Marshal(result)
	return string(out)
}

// QueryContract performs a read-only query against a smart contract and
// returns the result as a JSON string, or an empty string on failure.
func (b *BaalsClient) QueryContract(contractIDHex, method string, payload []byte) string {
	if !isHex32(contractIDHex) {
		return ""
	}

	body, _ := json.Marshal(map[string]interface{}{
		"contract_id":   contractIDHex,
		"method":        method,
		"payload_hex":   hex.EncodeToString(payload),
	})

	resp, err := b.httpClient.Post(
		b.healthURL()+"/contract/query",
		"application/json",
		bytes.NewReader(body),
	)
	if err != nil {
		return ""
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return ""
	}

	var result struct {
		ResultHex string `json:"result_hex"`
	}
	if json.NewDecoder(resp.Body).Decode(&result) != nil {
		return ""
	}
	out, _ := json.Marshal(result)
	return string(out)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// runQueryCmd runs `baalsd --json query <subcmd> [args...] --data-dir <dir>`
// and returns the raw stdout (a JSON string).
func runQueryCmd(binPath, dataDir, subcmd string, args ...string) ([]byte, error) {
	validPath := regexp.MustCompile(`^[a-zA-Z0-9_\-\./\\:]+$`)
	if !validPath.MatchString(binPath) {
		return nil, fmt.Errorf("invalid input")
	}
	if !validPath.MatchString(dataDir) {
		return nil, fmt.Errorf("invalid input")
	}
	validArg := regexp.MustCompile(`^[a-zA-Z0-9_\-\./\\:]+$`)
	for _, arg := range args {
		if !validArg.MatchString(arg) {
			return nil, fmt.Errorf("invalid input")
		}
	}
	argv := []string{
		"--json", "query", subcmd,
		"--data-dir", dataDir,
	}
	argv = append(argv, args...)

	cmd := exec.Command(binPath, argv...)
	cmd.Stdin = nil
	cmd.Stderr = nil

	var out bytes.Buffer
	cmd.Stdout = &out

	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("baalsd query %s: %w", subcmd, err)
	}
	return bytes.TrimSpace(out.Bytes()), nil
}

// isHex32 returns true when s is exactly 64 hex characters (32 bytes).
func isHex32(s string) bool {
	if len(s) != 64 {
		return false
	}
	for _, c := range s {
		if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F')) {
			return false
		}
	}
	return true
}

// findBinary locates the baalsd executable by checking, in order:
//  1. "baalsd" on PATH,
//  2. ../../target/release/baalsd (relative to this source file),
//  3. ../../target/debug/baalsd.
func findBinary() (string, error) {
	if p, err := exec.LookPath(defaultBinName); err == nil {
		return p, nil
	}

	// Resolve relative to the baals.go source file.
	_, file, _, ok := runtime.Caller(0)
	if ok {
		base := filepath.Dir(filepath.Dir(file)) // sdk/go → sdk
		for _, rel := range []string{
			"../target/release/baalsd",
			"../target/debug/baalsd",
		} {
			candidate := filepath.Join(base, rel)
			if _, err := os.Stat(candidate); err == nil {
				abs, err := filepath.Abs(candidate)
				if err == nil {
					return abs, nil
				}
			}
		}
	}

	return "", errors.New("baals: cannot find baalsd binary — build the Rust project first (cargo build --release), or ensure baalsd is on PATH")
}

// marshalByteArray32 serialises 32 bytes as a JSON array of numbers (matching
// serde_json's representation of [u8; 32]).
func marshalByteArray32(b []byte) ([]byte, error) {
	if len(b) != 32 {
		return nil, errors.New("marshalByteArray32: expected 32 bytes")
	}
	var buf bytes.Buffer
	buf.WriteByte('[')
	for i, v := range b {
		if i > 0 {
			buf.WriteByte(',')
		}
		fmt.Fprintf(&buf, "%d", v)
	}
	buf.WriteByte(']')
	return buf.Bytes(), nil
}

// unmarshalByteArray32 reads a JSON array of 32 numbers into the given slice.
func unmarshalByteArray32(data []byte, out []byte) error {
	if len(out) != 32 {
		return errors.New("unmarshalByteArray32: output must be 32 bytes")
	}
	var arr []int
	if err := json.Unmarshal(data, &arr); err != nil {
		return err
	}
	if len(arr) != 32 {
		return fmt.Errorf("expected 32 elements, got %d", len(arr))
	}
	for i, v := range arr {
		out[i] = byte(v)
	}
	return nil
}

// marshalByteArray64 serialises 64 bytes as a JSON array of numbers.
func marshalByteArray64(b []byte) ([]byte, error) {
	if len(b) != 64 {
		return nil, errors.New("marshalByteArray64: expected 64 bytes")
	}
	var buf bytes.Buffer
	buf.WriteByte('[')
	for i, v := range b {
		if i > 0 {
			buf.WriteByte(',')
		}
		fmt.Fprintf(&buf, "%d", v)
	}
	buf.WriteByte(']')
	return buf.Bytes(), nil
}

// unmarshalByteArray64 reads a JSON array of 64 numbers into the given slice.
func unmarshalByteArray64(data []byte, out []byte) error {
	if len(out) != 64 {
		return errors.New("unmarshalByteArray64: output must be 64 bytes")
	}
	var arr []int
	if err := json.Unmarshal(data, &arr); err != nil {
		return err
	}
	if len(arr) != 64 {
		return fmt.Errorf("expected 64 elements, got %d", len(arr))
	}
	for i, v := range arr {
		out[i] = byte(v)
	}
	return nil
}
