'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

let bindingLoadError = null;
let addon = null;
try {
  addon = require('../index.js');
} catch (err) {
  bindingLoadError = err;
}

function requireIntegration(t) {
  if (process.env.BAALS_NODE_INTEGRATION !== '1') {
    t.skip('set BAALS_NODE_INTEGRATION=1 to run Node SDK integration tests');
    return false;
  }
  if (!addon) {
    t.skip(`native addon not available: ${bindingLoadError}`);
    return false;
  }
  return true;
}

function createClient() {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'baals-node-sdk-'));
  return new addon.BaalsClient(dataDir, 'sled');
}

test('node_sdk_integration_lifecycle_and_queries', (t) => {
  if (!requireIntegration(t)) return;
  const client = createClient();
  try {
    client.start();

    const chainState = JSON.parse(client.chainStateJson());
    assert.ok(chainState);
    assert.equal(typeof chainState.latest_block_index, 'number');

    const genesis = client.getBlockByHeight(0);
    assert.ok(genesis, 'expected genesis block at height 0');
    const block = JSON.parse(genesis);
    assert.equal(block.index, 0);
  } finally {
    client.stop();
  }
});

test('node_sdk_integration_submit_tx_validation_error', (t) => {
  if (!requireIntegration(t)) return;
  const client = createClient();
  try {
    client.start();
    assert.throws(
      () => client.submitTx('{"hash":[]}'),
      /Invalid transaction|Invalid input|missing field|invalid type|signature/i
    );
  } finally {
    client.stop();
  }
});
