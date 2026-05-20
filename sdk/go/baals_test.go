package baals

import (
	"encoding/hex"
	"encoding/json"
	"math"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// newTestClient creates a minimal BaalsClient for unit tests without requiring
// a baalsd binary on PATH or built from source.  The returned client has no
// running daemon — methods that exec the binary or connect to HTTP will fail.
func newTestClient(t *testing.T, dataDir string) *BaalsClient {
	t.Helper()
	if dataDir == "" {
		dataDir = t.TempDir()
	}
	abs, err := filepath.Abs(dataDir)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(abs, 0755); err != nil {
		t.Fatal(err)
	}
	return &BaalsClient{
		dataDir:    abs,
		httpPort:   defaultHTTPPort,
		binPath:    "baalsd",
		httpClient: &http.Client{Timeout: 10 * time.Second},
	}
}

// ---------------------------------------------------------------------------
// Client creation & configuration
// ---------------------------------------------------------------------------

func TestNewClient_CreatesDir(t *testing.T) {
	dir := t.TempDir()
	dataDir := filepath.Join(dir, "sub", "dir")

	c, err := New(dataDir)
	if err != nil {
		if strings.Contains(err.Error(), "cannot find baalsd binary") {
			t.Skip("baalsd binary not available")
		}
		t.Fatalf("New(%q) unexpected error: %v", dataDir, err)
	}
	defer c.Close()

	info, err := os.Stat(dataDir)
	if err != nil {
		t.Fatalf("data dir was not created: %v", err)
	}
	if !info.IsDir() {
		t.Fatal("data dir is not a directory")
	}
}

func TestNewClient_Defaults(t *testing.T) {
	c, err := New(t.TempDir())
	if err != nil {
		if strings.Contains(err.Error(), "cannot find baalsd binary") {
			t.Skip("baalsd binary not available")
		}
		t.Fatal(err)
	}
	defer c.Close()

	if c.httpPort != defaultHTTPPort {
		t.Errorf("httpPort = %d, want %d", c.httpPort, defaultHTTPPort)
	}
	if c.dataDir == "" {
		t.Error("dataDir is empty")
	}
	if c.httpClient == nil {
		t.Error("httpClient is nil")
	}
	if c.binPath == "" {
		t.Error("binPath is empty — baalsd binary not found")
	}
}

func TestNewClient_RejectsInvalidPath(t *testing.T) {
	invalid := []string{
		"<script>",
		"dir;rm -rf /",
		"`backtick`",
		"$(whoami)",
		"|pipe",
	}
	for _, p := range invalid {
		_, err := New(p)
		if err == nil {
			t.Errorf("New(%q) expected error, got nil", p)
		}
	}
}

func TestSetHTTPPort(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	c.SetHTTPPort(9999)
	if c.httpPort != 9999 {
		t.Errorf("httpPort = %d, want 9999", c.httpPort)
	}
}

func TestSetBinaryPath(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	c.SetBinaryPath("/custom/path/baalsd")
	if c.binPath != "/custom/path/baalsd" {
		t.Errorf("binPath = %q, want /custom/path/baalsd", c.binPath)
	}
}

func TestSetBinaryPath_RejectsInvalid(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	orig := c.binPath
	c.SetBinaryPath("<invalid>")
	if c.binPath != orig {
		t.Error("binPath was changed despite invalid input")
	}
}

// ---------------------------------------------------------------------------
// healthURL
// ---------------------------------------------------------------------------

func TestHealthURL(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	want := "http://127.0.0.1:8080"
	if got := c.healthURL(); got != want {
		t.Errorf("healthURL() = %q, want %q", got, want)
	}

	c.SetHTTPPort(9090)
	want = "http://127.0.0.1:9090"
	if got := c.healthURL(); got != want {
		t.Errorf("healthURL() after SetHTTPPort = %q, want %q", got, want)
	}
}

// ---------------------------------------------------------------------------
// isHex32 utility
// ---------------------------------------------------------------------------

func TestIsHex32(t *testing.T) {
	tests := []struct {
		input string
		want  bool
	}{
		{strings.Repeat("a", 64), true},
		{strings.Repeat("A", 64), true},
		{strings.Repeat("0", 64), true},
		{"abcdef0123456789" + strings.Repeat("0", 48), true},
		{"", false},
		{strings.Repeat("a", 63), false},
		{strings.Repeat("a", 65), false},
		{strings.Repeat("g", 64), false},  // 'g' is not hex
		{strings.Repeat("x", 64), false},  // 'x' is not hex
		{strings.Repeat("-", 64), false},
		{"abcdef0123456789" + strings.Repeat("0", 47) + "g", false},
	}
	for _, tc := range tests {
		got := isHex32(tc.input)
		if got != tc.want {
			t.Errorf("isHex32(%q) = %v, want %v", tc.input, got, tc.want)
		}
	}
}

// ---------------------------------------------------------------------------
// PublicKey
// ---------------------------------------------------------------------------

func TestNewPublicKeyFromHex(t *testing.T) {
	_, err := NewPublicKeyFromHex(strings.Repeat("ab", 32))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	_, err = NewPublicKeyFromHex("")
	if err == nil {
		t.Error("expected error for empty hex")
	}

	_, err = NewPublicKeyFromHex("abc") // odd length, < 32 bytes
	if err == nil {
		t.Error("expected error for short hex")
	}

	_, err = NewPublicKeyFromHex(strings.Repeat("ab", 33)) // 33 bytes
	if err == nil {
		t.Error("expected error for long hex")
	}

	_, err = NewPublicKeyFromHex(strings.Repeat("gg", 32))
	if err == nil {
		t.Error("expected error for invalid hex chars")
	}
}

func TestPublicKey_Hex(t *testing.T) {
	pk, err := NewPublicKeyFromHex(strings.Repeat("ab", 32))
	if err != nil {
		t.Fatal(err)
	}
	if pk.Hex() != strings.Repeat("ab", 32) {
		t.Errorf("Hex() = %q, want %q", pk.Hex(), strings.Repeat("ab", 32))
	}
}

func TestPublicKey_JSONRoundTrip(t *testing.T) {
	orig, err := NewPublicKeyFromHex("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
	if err != nil {
		t.Fatal(err)
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatalf("Marshal error: %v", err)
	}

	var decoded PublicKey
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded != orig {
		t.Errorf("round-trip: got %v, want %v", decoded, orig)
	}
}

func TestPublicKey_JSONFormat(t *testing.T) {
	pk := PublicKey{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31}
	b, err := json.Marshal(pk)
	if err != nil {
		t.Fatal(err)
	}
	want := "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31]"
	if string(b) != want {
		t.Errorf("Marshal = %s, want %s", string(b), want)
	}
}

// ---------------------------------------------------------------------------
// Signature
// ---------------------------------------------------------------------------

func TestSignature_JSONRoundTrip(t *testing.T) {
	var orig Signature
	for i := range orig {
		orig[i] = byte(i)
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatalf("Marshal error: %v", err)
	}

	var decoded Signature
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded != orig {
		t.Errorf("round-trip: got %v, want %v", decoded, orig)
	}
}

// ---------------------------------------------------------------------------
// ContractID
// ---------------------------------------------------------------------------

func TestContractID_JSONRoundTrip(t *testing.T) {
	orig, err := NewContractIDFromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
	if err != nil {
		t.Fatal(err)
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatalf("Marshal error: %v", err)
	}

	var decoded ContractID
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded != orig {
		t.Errorf("round-trip: got %v, want %v", decoded, orig)
	}
}

func TestContractID_Hex(t *testing.T) {
	cid, err := NewContractIDFromHex(strings.Repeat("ff", 32))
	if err != nil {
		t.Fatal(err)
	}
	if cid.Hex() != strings.Repeat("ff", 32) {
		t.Errorf("Hex() = %q, want %q", cid.Hex(), strings.Repeat("ff", 32))
	}
}

func TestNewContractIDFromHex_Errors(t *testing.T) {
	_, err := NewContractIDFromHex("")
	if err == nil {
		t.Error("expected error for empty")
	}
	_, err = NewContractIDFromHex(strings.Repeat("ab", 33))
	if err == nil {
		t.Error("expected error for >32 bytes")
	}
}

// ---------------------------------------------------------------------------
// Transaction
// ---------------------------------------------------------------------------

func TestTransaction_CalculateHash_Deterministic(t *testing.T) {
	tx := Transaction{
		Sender:    PublicKey{1, 2, 3},
		Nonce:     42,
		Timestamp: 1000,
		GasLimit:  21000,
		Priority:  5,
		Recipient: Address{
			Wallet: &PublicKey{4, 5, 6},
		},
		Payload: TransactionPayload{
			Transfer: &TransferPayload{Amount: 500},
		},
	}

	h1 := tx.CalculateHash()
	h2 := tx.CalculateHash()
	if h1 != h2 {
		t.Error("CalculateHash is not deterministic")
	}
}

func TestTransaction_CalculateHash_ChangesWithFields(t *testing.T) {
	base := Transaction{
		Sender:    PublicKey{1},
		Nonce:     0,
		Timestamp: 0,
		GasLimit:  0,
		Priority:  0,
		Recipient: Address{Wallet: &PublicKey{2}},
		Payload:   TransactionPayload{Transfer: &TransferPayload{Amount: 0}},
	}
	h1 := base.CalculateHash()

	tx2 := base
	tx2.Nonce = 1
	h2 := tx2.CalculateHash()
	if h1 == h2 {
		t.Error("hash should change when Nonce changes")
	}

	tx3 := base
	tx3.Payload = TransactionPayload{Data: &DataPayload{Data: []byte("hello")}}
	h3 := tx3.CalculateHash()
	if h1 == h3 {
		t.Error("hash should change when Payload changes")
	}
}

func TestTransaction_CalculateHash_IncludesMetadata(t *testing.T) {
	tx := Transaction{
		Sender:    PublicKey{1},
		Nonce:     0,
		Timestamp: 0,
		GasLimit:  0,
		Priority:  0,
		Recipient: Address{Wallet: &PublicKey{2}},
		Payload:   TransactionPayload{Transfer: &TransferPayload{Amount: 0}},
	}

	h1 := tx.CalculateHash()

	tx.Metadata = map[string]string{"key": "value"}
	h2 := tx.CalculateHash()
	if h1 == h2 {
		t.Error("hash should change when Metadata is added")
	}
}

func TestTransaction_Verify_FailsOnBadSignature(t *testing.T) {
	tx := Transaction{
		Sender:    PublicKey{1},
		Nonce:     0,
		Timestamp: 0,
		GasLimit:  0,
		Priority:  0,
		Recipient: Address{Wallet: &PublicKey{2}},
		Payload:   TransactionPayload{Transfer: &TransferPayload{Amount: 0}},
		Hash:      [32]byte{},
		Signature: Signature{},
	}

	// Hash doesn't match, so verification should fail.
	if tx.Verify() {
		t.Error("Verify returned true for transaction with mismatched hash")
	}
}

// ---------------------------------------------------------------------------
// Address JSON serialization
// ---------------------------------------------------------------------------

func TestAddress_JSON_Wallet(t *testing.T) {
	pk := PublicKey{1}
	addr := Address{Wallet: &pk}

	b, err := json.Marshal(addr)
	if err != nil {
		t.Fatal(err)
	}

	var decoded Address
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Wallet == nil || *decoded.Wallet != pk {
		t.Error("Wallet field did not survive JSON round-trip")
	}
	if decoded.Contract != nil {
		t.Error("Contract field should be nil for wallet address")
	}
}

func TestAddress_JSON_Contract(t *testing.T) {
	cid, _ := NewContractIDFromHex(strings.Repeat("01", 32))
	addr := Address{Contract: &cid}

	b, err := json.Marshal(addr)
	if err != nil {
		t.Fatal(err)
	}

	var decoded Address
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Contract == nil || *decoded.Contract != cid {
		t.Error("Contract field did not survive JSON round-trip")
	}
	if decoded.Wallet != nil {
		t.Error("Wallet field should be nil for contract address")
	}
}

// ---------------------------------------------------------------------------
// TransactionPayload JSON serialization
// ---------------------------------------------------------------------------

func TestTransactionPayload_TransferJSON(t *testing.T) {
	payload := TransactionPayload{
		Transfer: &TransferPayload{Amount: 12345},
	}

	b, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}

	var decoded TransactionPayload
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Transfer == nil || decoded.Transfer.Amount != 12345 {
		t.Errorf("Transfer payload did not survive round-trip: %+v", decoded)
	}
}

func TestTransactionPayload_DataJSON(t *testing.T) {
	payload := TransactionPayload{
		Data: &DataPayload{Data: []byte("hello world")},
	}

	b, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}

	var decoded TransactionPayload
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Data == nil || string(decoded.Data.Data) != "hello world" {
		t.Errorf("Data payload did not survive round-trip: %+v", decoded)
	}
}

func TestTransactionPayload_ContractDeployJSON(t *testing.T) {
	payload := TransactionPayload{
		ContractDeploy: &ContractDeployPayload{
			WasmBytes:   []byte{0, 1, 2},
			InitPayload: []byte{3, 4, 5},
		},
	}

	b, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}

	var decoded TransactionPayload
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.ContractDeploy == nil {
		t.Fatal("ContractDeploy is nil after round-trip")
	}
	if len(decoded.ContractDeploy.WasmBytes) != 3 || decoded.ContractDeploy.WasmBytes[0] != 0 {
		t.Error("WasmBytes did not survive round-trip")
	}
}

func TestTransactionPayload_ContractCallJSON(t *testing.T) {
	val := uint64(99)
	payload := TransactionPayload{
		ContractCall: &ContractCallPayload{
			Method: "transfer",
			Args:   [][]byte{{1, 2}, {3, 4, 5}},
			Value:  &val,
		},
	}

	b, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}

	var decoded TransactionPayload
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.ContractCall == nil {
		t.Fatal("ContractCall is nil after round-trip")
	}
	if decoded.ContractCall.Method != "transfer" {
		t.Errorf("Method = %q, want %q", decoded.ContractCall.Method, "transfer")
	}
	if len(decoded.ContractCall.Args) != 2 {
		t.Errorf("len(Args) = %d, want 2", len(decoded.ContractCall.Args))
	}
	if decoded.ContractCall.Value == nil || *decoded.ContractCall.Value != 99 {
		t.Error("Value did not survive round-trip")
	}
}

// ---------------------------------------------------------------------------
// Block
// ---------------------------------------------------------------------------

func TestBlock_JSONRoundTrip(t *testing.T) {
	orig := Block{
		Index:     1,
		Timestamp: 1234,
		PrevHash:  [32]byte{1},
		Hash:      [32]byte{2},
		Nonce:     99,
		Transactions: []Transaction{
			{
				Sender: PublicKey{10},
				Nonce:  0,
				Recipient: Address{
					Wallet: &PublicKey{20},
				},
				Payload: TransactionPayload{
					Transfer: &TransferPayload{Amount: 100},
				},
			},
		},
		Metadata: map[string]string{"key": "value"},
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatalf("Marshal error: %v", err)
	}

	var decoded Block
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Index != orig.Index {
		t.Errorf("Index = %d, want %d", decoded.Index, orig.Index)
	}
	if decoded.Hash != orig.Hash {
		t.Errorf("Hash mismatch")
	}
	if len(decoded.Transactions) != 1 {
		t.Fatalf("len(Transactions) = %d, want 1", len(decoded.Transactions))
	}
	if decoded.Transactions[0].Sender != orig.Transactions[0].Sender {
		t.Error("Transaction Sender mismatch")
	}
}

// ---------------------------------------------------------------------------
// ChainState
// ---------------------------------------------------------------------------

func TestChainState_JSONRoundTrip(t *testing.T) {
	orig := ChainState{
		LatestBlockHash:  [32]byte{1, 2, 3},
		LatestBlockIndex: 100,
		AccountsRootHash: [32]byte{4, 5, 6},
		TotalSupply:      1_000_000,
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatalf("Marshal error: %v", err)
	}

	var decoded ChainState
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded != orig {
		t.Errorf("round-trip: got %+v, want %+v", decoded, orig)
	}
}

// ---------------------------------------------------------------------------
// Account JSON
// ---------------------------------------------------------------------------

func TestAccount_WalletJSON(t *testing.T) {
	orig := Account{
		Wallet: &WalletAccount{
			Balance: 5000,
			Nonce:   3,
		},
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatal(err)
	}

	var decoded Account
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Wallet == nil {
		t.Fatal("Wallet is nil after round-trip")
	}
	if decoded.Wallet.Balance != 5000 || decoded.Wallet.Nonce != 3 {
		t.Errorf("WalletAccount mismatch: %+v", decoded.Wallet)
	}
}

func TestAccount_ContractJSON(t *testing.T) {
	orig := Account{
		Contract: &ContractAccount{
			CodeHash:        [32]byte{1},
			StorageRootHash: [32]byte{2},
			Nonce:           7,
		},
	}

	b, err := json.Marshal(orig)
	if err != nil {
		t.Fatal(err)
	}

	var decoded Account
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}

	if decoded.Contract == nil {
		t.Fatal("Contract is nil after round-trip")
	}
	if decoded.Contract.Nonce != 7 {
		t.Errorf("Nonce = %d, want 7", decoded.Contract.Nonce)
	}
}

// ---------------------------------------------------------------------------
// putUint64LE
// ---------------------------------------------------------------------------

func TestPutUint64LE(t *testing.T) {
	tests := []struct {
		val  uint64
		want []byte
	}{
		{0, []byte{0, 0, 0, 0, 0, 0, 0, 0}},
		{1, []byte{1, 0, 0, 0, 0, 0, 0, 0}},
		{0xFF, []byte{0xFF, 0, 0, 0, 0, 0, 0, 0}},
		{0x0102030405060708, []byte{0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01}},
		{math.MaxUint64, []byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}},
	}

	for _, tc := range tests {
		buf := make([]byte, 8)
		putUint64LE(buf, tc.val)
		for i := range buf {
			if buf[i] != tc.want[i] {
				t.Errorf("putUint64LE(%d) = %v, want %v", tc.val, buf, tc.want)
				break
			}
		}
	}
}

// ---------------------------------------------------------------------------
// marshalByteArray32 / unmarshalByteArray32
// ---------------------------------------------------------------------------

func TestMarshalByteArray32(t *testing.T) {
	b := make([]byte, 32)
	for i := range b {
		b[i] = byte(i)
	}

	got, err := marshalByteArray32(b)
	if err != nil {
		t.Fatal(err)
	}

	want := "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31]"
	if string(got) != want {
		t.Errorf("marshalByteArray32 = %s, want %s", string(got), want)
	}
}

func TestMarshalByteArray32_WrongSize(t *testing.T) {
	_, err := marshalByteArray32([]byte{1, 2, 3})
	if err == nil {
		t.Error("expected error for wrong size")
	}
}

func TestUnmarshalByteArray32_RoundTrip(t *testing.T) {
	orig := make([]byte, 32)
	for i := range orig {
		orig[i] = byte(i * 7)
	}

	json, err := marshalByteArray32(orig)
	if err != nil {
		t.Fatal(err)
	}

	var out [32]byte
	if err := unmarshalByteArray32(json, out[:]); err != nil {
		t.Fatalf("unmarshalByteArray32 error: %v", err)
	}

	for i := range orig {
		if orig[i] != out[i] {
			t.Errorf("byte %d: got %d, want %d", i, out[i], orig[i])
			break
		}
	}
}

func TestUnmarshalByteArray32_WrongSize(t *testing.T) {
	var out [32]byte
	err := unmarshalByteArray32([]byte("[1,2,3]"), out[:])
	if err == nil {
		t.Error("expected error for too few elements")
	}

	err = unmarshalByteArray32([]byte("invalid"), out[:])
	if err == nil {
		t.Error("expected error for invalid JSON")
	}
}

// ---------------------------------------------------------------------------
// marshalByteArray64 / unmarshalByteArray64
// ---------------------------------------------------------------------------

func TestMarshalByteArray64(t *testing.T) {
	b := make([]byte, 64)
	for i := range b {
		b[i] = byte(i)
	}

	got, err := marshalByteArray64(b)
	if err != nil {
		t.Fatal(err)
	}

	var decoded []int
	if err := json.Unmarshal(got, &decoded); err != nil {
		t.Fatalf("Unmarshal error: %v", err)
	}
	if len(decoded) != 64 {
		t.Errorf("len = %d, want 64", len(decoded))
	}
}

func TestMarshalByteArray64_WrongSize(t *testing.T) {
	_, err := marshalByteArray64([]byte{1, 2, 3})
	if err == nil {
		t.Error("expected error for wrong size")
	}
}

// ---------------------------------------------------------------------------
// Query method input validation
// ---------------------------------------------------------------------------

func TestGetTransaction_RejectsNonHex(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	if got := c.GetTransaction(""); got != "" {
		t.Errorf("GetTransaction('') = %q, want ''", got)
	}
	if got := c.GetTransaction("short"); got != "" {
		t.Errorf("GetTransaction('short') = %q, want ''", got)
	}
	if got := c.GetTransaction(strings.Repeat("zz", 32)); got != "" {
		t.Errorf("GetTransaction with invalid hex = %q, want ''", got)
	}
}

func TestGetAccount_RejectsNonHex(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	if got := c.GetAccount(""); got != "" {
		t.Errorf("GetAccount('') = %q, want ''", got)
	}
	if got := c.GetAccount(strings.Repeat("zz", 32)); got != "" {
		t.Errorf("GetAccount with invalid hex = %q, want ''", got)
	}
}

func TestCreateAccount_RejectsNonHex(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	err := c.CreateAccount("bad", 100)
	if err == nil {
		t.Error("expected error for non-hex pubkey")
	}
}

func TestDeployContract_RejectsNonHex(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	if got := c.DeployContract("bad", nil, nil, 0); got != "" {
		t.Errorf("DeployContract with invalid hex = %q, want ''", got)
	}
}

func TestCallContract_RejectsNonHex(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	if got := c.CallContract("bad", strings.Repeat("ab", 32), "", nil, 0); got != "" {
		t.Errorf("CallContract with invalid caller = %q, want ''", got)
	}
	if got := c.CallContract(strings.Repeat("ab", 32), "bad", "", nil, 0); got != "" {
		t.Errorf("CallContract with invalid contract ID = %q, want ''", got)
	}
}

func TestQueryContract_RejectsNonHex(t *testing.T) {
	c := newTestClient(t, "")
	defer c.Close()

	if got := c.QueryContract("bad", "", nil); got != "" {
		t.Errorf("QueryContract with invalid hex = %q, want ''", got)
	}
}

// ---------------------------------------------------------------------------
// Integration tests — require a running BaaLS node
// ---------------------------------------------------------------------------

func TestIntegration_SubmitTx(t *testing.T) {
	t.Skip("requires running BaaLS node")
	c, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	err = c.SubmitTx(`{"hash":[],"sender":[],"nonce":0,"timestamp":0,"recipient":{"Wallet":[]},"payload":{"Transfer":{"amount":0}},"signature":[],"gas_limit":0,"priority":0,"metadata":null}`)
	if err != nil {
		t.Fatalf("SubmitTx failed: %v", err)
	}
}

func TestIntegration_ChainStateJSON(t *testing.T) {
	t.Skip("requires running BaaLS node")
	c, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	state := c.ChainStateJSON()
	if state == "" || state == "{}" {
		t.Fatal("ChainStateJSON returned empty")
	}

	var cs ChainState
	if err := json.Unmarshal([]byte(state), &cs); err != nil {
		t.Fatalf("ChainStateJSON is not valid JSON: %v", err)
	}
}

func TestIntegration_StartStop(t *testing.T) {
	t.Skip("requires baalsd binary")
	c, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	if err := c.Start(); err != nil {
		t.Fatalf("Start failed: %v", err)
	}
	if err := c.Stop(); err != nil {
		t.Fatalf("Stop failed: %v", err)
	}
}

func TestIntegration_CreateAccount(t *testing.T) {
	t.Skip("requires running BaaLS node")
	c, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	pubkey := strings.Repeat("ab", 32)
	if err := c.CreateAccount(pubkey, 1000); err != nil {
		t.Fatalf("CreateAccount failed: %v", err)
	}
}

func TestIntegration_DeployAndCallContract(t *testing.T) {
	t.Skip("requires running BaaLS node")
	c, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	// Deploy a minimal WASM contract (just an empty module).
	deployer := strings.Repeat("aa", 32)
	wasm := []byte{
		0x00, 0x61, 0x73, 0x6d, // magic
		0x01, 0x00, 0x00, 0x00, // version
	}
	contractID := c.DeployContract(deployer, wasm, nil, 100000)
	if contractID == "" {
		t.Fatal("DeployContract returned empty contract ID")
	}
	t.Logf("deployed contract: %s", contractID)

	result := c.CallContract(deployer, contractID, "ping", nil, 0)
	if result == "" {
		t.Fatal("CallContract returned empty result")
	}
	t.Logf("call result: %s", result)
}

func TestIntegration_GetBlockByHeight(t *testing.T) {
	t.Skip("requires running BaaLS node")
	c, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	blockJSON := c.GetBlockByHeight(0)
	if blockJSON == "" {
		t.Fatal("GetBlockByHeight returned empty for height 0")
	}
	var block Block
	if err := json.Unmarshal([]byte(blockJSON), &block); err != nil {
		t.Fatalf("GetBlockByHeight returned invalid JSON: %v", err)
	}
	if block.Index != 0 {
		t.Errorf("block index = %d, want 0", block.Index)
	}
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

func TestTransaction_EmptyFields(t *testing.T) {
	tx := Transaction{}
	hash := tx.CalculateHash()
	if hash == [32]byte{} {
		t.Error("CalculateHash of zero-value tx should not be all zeros")
	}
}

func TestNewClient_AbsolutePath(t *testing.T) {
	c, err := New(t.TempDir())
	if err != nil {
		if strings.Contains(err.Error(), "cannot find baalsd binary") {
			t.Skip("baalsd binary not available")
		}
		t.Fatal(err)
	}
	defer c.Close()

	if !filepath.IsAbs(c.dataDir) {
		t.Errorf("dataDir = %q, want absolute path", c.dataDir)
	}
}

func TestConcurrentClose(t *testing.T) {
	c, err := New(t.TempDir())
	if err != nil {
		if strings.Contains(err.Error(), "cannot find baalsd binary") {
			t.Skip("baalsd binary not available")
		}
		t.Fatal(err)
	}
	c.Close()
	c.Close()
}

func TestMarshalByteArray32_Boundary(t *testing.T) {
	// All zeros
	b := make([]byte, 32)
	got, err := marshalByteArray32(b)
	if err != nil {
		t.Fatal(err)
	}
	want := "[" + strings.Repeat("0,", 31) + "0]"
	if string(got) != want {
		t.Errorf("all-zero marshal = %s, want %s", string(got), want)
	}

	// All 255s
	for i := range b {
		b[i] = 255
	}
	got, err = marshalByteArray32(b)
	if err != nil {
		t.Fatal(err)
	}
	want = "[" + strings.Repeat("255,", 31) + "255]"
	if string(got) != want {
		t.Errorf("all-255 marshal = %s, want %s", string(got), want)
	}
}

// ---------------------------------------------------------------------------
// Example-style tests
// ---------------------------------------------------------------------------

func TestPublicKeyFromHex_Example(t *testing.T) {
	// Example from documentation: 64 hex chars = 32 bytes.
	pk, err := NewPublicKeyFromHex("deadbeef" + strings.Repeat("00", 28))
	if err != nil {
		t.Fatal(err)
	}
	if pk[0] != 0xde || pk[1] != 0xad || pk[2] != 0xbe || pk[3] != 0xef {
		t.Errorf("unexpected first 4 bytes: %x", pk[:4])
	}
}

func TestTransaction_HashExample(t *testing.T) {
	tx := Transaction{
		Sender:    PublicKey{0x01, 0x02},
		Nonce:     1,
		Timestamp: 100,
		GasLimit:  50000,
		Priority:  1,
		Recipient: Address{Wallet: &PublicKey{0x03}},
		Payload:   TransactionPayload{Transfer: &TransferPayload{Amount: 1000}},
		Metadata:  map[string]string{"note": "test"},
	}
	hash := tx.CalculateHash()
	hexHash := hex.EncodeToString(hash[:])
	if len(hexHash) != 64 {
		t.Errorf("hash hex length = %d, want 64", len(hexHash))
	}
}
