export class BaalsClient {
  constructor(dataDir: string);
  start(): void;
  stop(): void;
  chainStateJson(): string;
  getBlockByHeight(height: number): string | null;
  getTransaction(hashHex: string): string | null;
  getAccount(pubkeyHex: string): string | null;
  submitTx(txJson: string): void;
  createAccount(pubkeyHex: string, balance: number): void;
}
