/**
 * BaaLS Node.js SDK
 *
 * Wraps the C FFI bindings (src/ffi.rs) via ffi-napi.
 * Requires the compiled cdylib at ../../target/release/libbaals.{so,dylib} or baals.dll.
 */
'use strict';

const path = require('path');

function getLibPath() {
    const platform = process.platform;
    const base = path.join(__dirname, '..', '..', 'target', 'release');
    switch (platform) {
        case 'linux':
            return path.join(base, 'libbaals.so');
        case 'darwin':
            return path.join(base, 'libbaals.dylib');
        case 'win32':
            return path.join(base, 'baals.dll');
        default:
            throw new Error(`Unsupported platform: ${platform}`);
    }
}

let ffi;
try {
    ffi = require('ffi-napi');
} catch (_) {
    ffi = null;
}

let lib = null;

function getLib() {
    if (lib) return lib;
    const libPath = process.env.BAALS_LIB_PATH || getLibPath();
    if (!ffi) {
        throw new Error(
            'ffi-napi is required. Install with: npm install ffi-napi'
        );
    }
    lib = ffi.Library(libPath, {
        baals_sdk_init: ['uint32', ['string']],
        baals_sdk_start: ['uint32', []],
        baals_sdk_stop: ['uint32', []],
        baals_sdk_chain_state_json: ['string', []],
        baals_sdk_get_block_by_height: ['string', ['uint64']],
        baals_sdk_get_transaction: ['string', ['pointer']],
        baals_sdk_get_account: ['string', ['pointer']],
        baals_sdk_submit_tx: ['uint32', ['string']],
        baals_sdk_create_account: ['uint32', ['pointer', 'uint64']],
        baals_sdk_deploy_contract: ['string', ['pointer', 'pointer', 'uint32', 'pointer', 'uint32', 'uint64']],
        baals_sdk_call_contract: ['string', ['pointer', 'pointer', 'string', 'pointer', 'uint32', 'uint64']],
        baals_sdk_query_contract: ['string', ['pointer', 'string', 'pointer', 'uint32']],
        baals_sdk_free_string: ['void', ['string']],
    });
    return lib;
}

function hexToBuffer(hex) {
    if (typeof hex === 'string') {
        return Buffer.from(hex, 'hex');
    }
    return hex;
}

class BaalsClient {
    constructor() {
        this._lib = getLib();
        this._started = false;
    }

    /** Initialize a BaaLS node with the given data directory. */
    static async new(dataDir) {
        const client = new BaalsClient();
        if (!dataDir) {
            dataDir = path.join(process.env.HOME || process.env.USERPROFILE || '.', '.baals', 'data');
        }
        const code = client._lib.baals_sdk_init(dataDir);
        if (code !== 0) {
            throw new Error(`baals_sdk_init failed with code ${code}`);
        }
        return client;
    }

    /** Start block production. */
    async start() {
        const code = this._lib.baals_sdk_start();
        if (code !== 0) {
            throw new Error(`baals_sdk_start failed with code ${code}`);
        }
        this._started = true;
    }

    /** Stop the node. */
    async stop() {
        const code = this._lib.baals_sdk_stop();
        if (code !== 0) {
            throw new Error(`baals_sdk_stop failed with code ${code}`);
        }
        this._started = false;
    }

    /** Get current chain state. */
    async chainState() {
        const json = this._lib.baals_sdk_chain_state_json();
        const free = getLib().baals_sdk_free_string;
        try {
            return JSON.parse(json);
        } finally {
            free(json);
        }
    }

    /** Get a block by height. */
    async getBlockByHeight(height) {
        const json = this._lib.baals_sdk_get_block_by_height(height);
        if (!json) return null;
        const free = getLib().baals_sdk_free_string;
        try {
            return JSON.parse(json);
        } finally {
            free(json);
        }
    }

    /** Get a transaction by hex hash. */
    async getTransaction(hashHex) {
        const buf = hexToBuffer(hashHex);
        const json = this._lib.baals_sdk_get_transaction(buf);
        if (!json) return null;
        const free = getLib().baals_sdk_free_string;
        try {
            return JSON.parse(json);
        } finally {
            free(json);
        }
    }

    /** Get an account by hex public key. */
    async getAccount(pubkeyHex) {
        const buf = hexToBuffer(pubkeyHex);
        const json = this._lib.baals_sdk_get_account(buf);
        if (!json) return null;
        const free = getLib().baals_sdk_free_string;
        try {
            return JSON.parse(json);
        } finally {
            free(json);
        }
    }

    /** Submit a transaction as a JSON string. */
    async submitTx(txJSON) {
        if (typeof txJSON !== 'string') {
            txJSON = JSON.stringify(txJSON);
        }
        const code = this._lib.baals_sdk_submit_tx(txJSON);
        if (code !== 0) {
            throw new Error(`baals_sdk_submit_tx failed with code ${code}`);
        }
    }

    /** Create a wallet account. */
    async createAccount(pubkeyHex, balance) {
        const buf = hexToBuffer(pubkeyHex);
        const code = this._lib.baals_sdk_create_account(buf, balance);
        if (code !== 0) {
            throw new Error(`baals_sdk_create_account failed with code ${code}`);
        }
    }

    /** Deploy a WASM contract. Returns contract ID hex. */
    async deployContract(deployerHex, wasm, initPayload, gasLimit) {
        const deployerBuf = hexToBuffer(deployerHex);
        const wasmBuf = Buffer.from(wasm);
        const initBuf = initPayload ? Buffer.from(initPayload) : Buffer.alloc(0);
        const gas = gasLimit || 1000000;
        const json = this._lib.baals_sdk_deploy_contract(
            deployerBuf,
            wasmBuf,
            wasmBuf.length,
            initBuf,
            initBuf.length,
            gas
        );
        const free = getLib().baals_sdk_free_string;
        try {
            const result = JSON.parse(json);
            return result.contract_id;
        } finally {
            free(json);
        }
    }

    /** Call a contract method. */
    async callContract(callerHex, contractIdHex, method, args, value) {
        const callerBuf = hexToBuffer(callerHex);
        const contractBuf = hexToBuffer(contractIdHex);
        const argsBuf = args ? Buffer.from(args) : Buffer.alloc(0);
        const val = value || 0;
        const json = this._lib.baals_sdk_call_contract(
            callerBuf,
            contractBuf,
            method,
            argsBuf,
            argsBuf.length,
            val
        );
        const free = getLib().baals_sdk_free_string;
        try {
            return JSON.parse(json);
        } finally {
            free(json);
        }
    }

    /** Read-only contract query. */
    async queryContract(contractIdHex, method, payload) {
        const contractBuf = hexToBuffer(contractIdHex);
        const payloadBuf = payload ? Buffer.from(payload) : Buffer.alloc(0);
        const json = this._lib.baals_sdk_query_contract(
            contractBuf,
            method,
            payloadBuf,
            payloadBuf.length
        );
        const free = getLib().baals_sdk_free_string;
        try {
            return JSON.parse(json);
        } finally {
            free(json);
        }
    }
}

module.exports = { BaalsClient };
