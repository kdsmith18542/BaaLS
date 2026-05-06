/**
 * BaaLS SDK — TypeScript definitions for the Node.js native addon.
 *
 * The JS bindings load a native Node-API addon compiled from the Rust FFI layer.
 * For now, this provides a Promise-based API that mirrors the Go SDK surface.
 */

declare module '@baals/sdk' {
  export class BaalsClient {
    /** Initialize a BaaLS node with the given data directory. */
    static new(dataDir: string): Promise<BaalsClient>;

    /** Start block production. */
    start(): Promise<void>;

    /** Stop the node. */
    stop(): Promise<void>;

    /** Get the current chain state as a parsed object. */
    chainState(): Promise<ChainState>;

    /** Get a block by height. */
    getBlockByHeight(height: number): Promise<Block | null>;

    /** Get a transaction by 32-byte hex hash. */
    getTransaction(hashHex: string): Promise<Transaction | null>;

    /** Get an account by 32-byte hex public key. */
    getAccount(pubkeyHex: string): Promise<Account | null>;

    /** Submit a JSON transaction. */
    submitTx(txJSON: string): Promise<void>;

    /** Create a wallet account with initial balance. */
    createAccount(pubkeyHex: string, balance: number): Promise<void>;

    /** Deploy WASM bytecode, returns contract ID hex. */
    deployContract(deployerHex: string, wasm: Uint8Array, gasLimit: number): Promise<string>;

    /** Call a contract method. */
    callContract(callerHex: string, contractIdHex: string, method: string, args: Uint8Array): Promise<ContractResult>;

    /** Read-only contract query. */
    queryContract(contractIdHex: string, payload: Uint8Array): Promise<ContractResult>;
  }

  interface ChainState {
    latest_block_hash: string;
    latest_block_index: number;
    accounts_root_hash: string;
    total_supply: number;
  }

  interface Block {
    index: number;
    timestamp: number;
    prev_hash: string;
    hash: string;
    nonce: number;
    transactions: Transaction[];
  }

  interface Transaction {
    hash: string;
    sender: string;
    nonce: number;
    timestamp: number;
    recipient: string;
    payload: Record<string, unknown>;
    signature: string;
    gas_limit: number;
    priority: number;
  }

  interface Account {
    type: 'wallet' | 'contract';
    balance?: number;
    nonce: number;
    code_hash?: string;
    storage_root_hash?: string;
  }

  interface ContractResult {
    result_hex: string;
  }
}
