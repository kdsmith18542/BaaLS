package baals

/*
#cgo LDFLAGS: -L../../target/release -lbaals
#include <stdlib.h>

extern unsigned int baals_sdk_init(const char* data_dir);
extern unsigned int baals_sdk_start(void);
extern unsigned int baals_sdk_stop(void);
extern char* baals_sdk_chain_state_json(void);
extern char* baals_sdk_get_block_by_height(unsigned long long height);
extern char* baals_sdk_get_transaction(const unsigned char* hash);
extern char* baals_sdk_get_account(const unsigned char* pubkey);
extern unsigned int baals_sdk_submit_tx(const char* tx_json);
extern unsigned int baals_sdk_create_account(const unsigned char* pubkey, unsigned long long balance);
extern char* baals_sdk_deploy_contract(const unsigned char* deployer, const unsigned char* wasm, unsigned int wasm_len, unsigned long long gas_limit);
extern char* baals_sdk_call_contract(const unsigned char* caller, const unsigned char* contract_id, const char* method, const unsigned char* args, unsigned int args_len);
extern char* baals_sdk_query_contract(const unsigned char* contract_id, const unsigned char* payload, unsigned int payload_len);
extern void baals_sdk_free_string(char* s);
*/
import "C"
import (
	"encoding/hex"
	"errors"
	"unsafe"
)

// BaalsClient wraps the C FFI bindings to the BaaLS Rust library.
type BaalsClient struct{}

// New initializes a BaaLS node from the Rust FFI.
func New(dataDir string) (*BaalsClient, error) {
	cDir := C.CString(dataDir)
	defer C.free(unsafe.Pointer(cDir))
	if code := C.baals_sdk_init(cDir); code != 0 {
		return nil, errors.New("baals_sdk_init failed")
	}
	return &BaalsClient{}, nil
}

// Start begins block production.
func (b *BaalsClient) Start() error {
	if code := C.baals_sdk_start(); code != 0 {
		return errors.New("baals_sdk_start failed")
	}
	return nil
}

// Stop shuts down the node.
func (b *BaalsClient) Stop() error {
	if code := C.baals_sdk_stop(); code != 0 {
		return errors.New("baals_sdk_stop failed")
	}
	return nil
}

// ChainStateJSON returns the current chain state as a JSON string.
func (b *BaalsClient) ChainStateJSON() string {
	cStr := C.baals_sdk_chain_state_json()
	if cStr == nil {
		return "{}"
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}

// GetBlockByHeight returns block JSON for the given height, or empty string.
func (b *BaalsClient) GetBlockByHeight(height uint64) string {
	cStr := C.baals_sdk_get_block_by_height(C.ulonglong(height))
	if cStr == nil {
		return ""
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}

// GetTransaction returns transaction JSON for the given 32-byte hex hash.
func (b *BaalsClient) GetTransaction(hashHex string) string {
	hash, err := hex.DecodeString(hashHex)
	if err != nil || len(hash) != 32 {
		return ""
	}
	cStr := C.baals_sdk_get_transaction((*C.uchar)(unsafe.Pointer(&hash[0])))
	if cStr == nil {
		return ""
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}

// GetAccount returns account JSON for the given 32-byte hex public key.
func (b *BaalsClient) GetAccount(pubkeyHex string) string {
	pk, err := hex.DecodeString(pubkeyHex)
	if err != nil || len(pk) != 32 {
		return ""
	}
	cStr := C.baals_sdk_get_account((*C.uchar)(unsafe.Pointer(&pk[0])))
	if cStr == nil {
		return ""
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}

// SubmitTx submits a JSON-serialized transaction.
func (b *BaalsClient) SubmitTx(txJSON string) error {
	cJSON := C.CString(txJSON)
	defer C.free(unsafe.Pointer(cJSON))
	if code := C.baals_sdk_submit_tx(cJSON); code != 0 {
		return errors.New("baals_sdk_submit_tx failed")
	}
	return nil
}

// CreateAccount creates a wallet account with the given public key and balance.
func (b *BaalsClient) CreateAccount(pubkeyHex string, balance uint64) error {
	pk, err := hex.DecodeString(pubkeyHex)
	if err != nil || len(pk) != 32 {
		return err
	}
	if code := C.baals_sdk_create_account((*C.uchar)(unsafe.Pointer(&pk[0])), C.ulonglong(balance)); code != 0 {
		return errors.New("baals_sdk_create_account failed")
	}
	return nil
}

// DeployContract deploys WASM bytecode and returns the contract ID as hex.
func (b *BaalsClient) DeployContract(deployerHex string, wasm []byte, gasLimit uint64) string {
	pk, err := hex.DecodeString(deployerHex)
	if err != nil || len(pk) != 32 {
		return ""
	}
	cStr := C.baals_sdk_deploy_contract(
		(*C.uchar)(unsafe.Pointer(&pk[0])),
		(*C.uchar)(unsafe.Pointer(&wasm[0])),
		C.uint(len(wasm)),
		C.ulonglong(gasLimit),
	)
	if cStr == nil {
		return ""
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}

// CallContract calls a contract method and returns the result JSON.
func (b *BaalsClient) CallContract(callerHex, contractIDHex, method string, args []byte) string {
	caller, _ := hex.DecodeString(callerHex)
	cid, _ := hex.DecodeString(contractIDHex)
	if len(caller) != 32 || len(cid) != 32 {
		return ""
	}
	cMethod := C.CString(method)
	defer C.free(unsafe.Pointer(cMethod))
	cStr := C.baals_sdk_call_contract(
		(*C.uchar)(unsafe.Pointer(&caller[0])),
		(*C.uchar)(unsafe.Pointer(&cid[0])),
		cMethod,
		(*C.uchar)(unsafe.Pointer(&args[0])),
		C.uint(len(args)),
	)
	if cStr == nil {
		return ""
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}

// QueryContract performs a read-only contract call.
func (b *BaalsClient) QueryContract(contractIDHex string, payload []byte) string {
	cid, _ := hex.DecodeString(contractIDHex)
	if len(cid) != 32 {
		return ""
	}
	cStr := C.baals_sdk_query_contract(
		(*C.uchar)(unsafe.Pointer(&cid[0])),
		(*C.uchar)(unsafe.Pointer(&payload[0])),
		C.uint(len(payload)),
	)
	if cStr == nil {
		return ""
	}
	defer C.baals_sdk_free_string(cStr)
	return C.GoString(cStr)
}
