'use strict';

const fs = require('fs');
const path = require('path');

function resolveBinding() {
  const candidates = [
    path.join(__dirname, 'baals-nodejs-native.node'),
    path.join(__dirname, 'baals_nodejs_native.node'),
    path.join(__dirname, '..', 'target', 'debug', 'baals_nodejs_native.node'),
    path.join(__dirname, '..', 'target', 'release', 'baals_nodejs_native.node'),
  ];

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      return require(candidate);
    }
  }

  throw new Error(
    'Unable to locate baals-nodejs-native addon (.node). ' +
      'Run `npm run build` inside sdk/nodejs-native first.'
  );
}

module.exports = resolveBinding();
