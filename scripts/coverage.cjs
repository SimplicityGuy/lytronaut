// Coverage tooling only (MIT). Inactive unless LYTRONAUT_COVERAGE_DIR is set.
//
// Every page that loaded the Rust/WASM client is dumped through the
// instrumented build's `window.__lytronautCoverage` hook (crates/lytronaut-coverage)
// immediately before page.goto/page.reload/page.close/browser.close, because a
// navigation or close discards that WASM instance and its LLVM counters.
// A page that loaded the client but yields no hook, an empty profile, a
// signature mismatch, or a navigation the driver did not dump first is a
// failure: the process exits non-zero and the failure lands in the driver summary.
'use strict';
const fs = require('node:fs');
const path = require('node:path');

const dir = process.env.LYTRONAUT_COVERAGE_DIR ? path.resolve(process.env.LYTRONAUT_COVERAGE_DIR) : null;
const script = path.basename(process.argv[1] || 'node', '.cjs');
const CLIENT_WASM = /\/lytronaut_client_bg\.wasm(?:[?#]|$)/;
const summary = { script, pid: process.pid, pagesWithClient: 0, profiles: 0, bytes: 0, signatures: [], failures: [] };
let sequence = 0;

const fail = message => {
  summary.failures.push(message);
  console.error(`coverage: ${message}`);
};

// Recompute minicov's lprofGetLoadModuleSignature() from the raw profile header.
// minicov statically reserves INSTR_PROF_VNODE_POOL_SIZE (1024) value nodes.
function profileSignature(buffer) {
  const mask = (1n << 64n) - 1n;
  const word = index => buffer.readBigUInt64LE(index * 8);
  const version = word(1);
  const raw = Number(version & 0xffffffffn);
  const fields = raw >= 11
    ? ['Magic', 'Version', 'BinaryIdsSize', 'NumData', 'PadBeforeCounters', 'NumCounters', 'PadAfterCounters', 'NumBitmapBytes', 'PadAfterBitmap', 'NumUniformCounters', 'PadAfterUniform', 'UniformCountersDelta', 'NamesSize', 'CountersDelta', 'BitmapDelta', 'NamesDelta', 'NumVTables', 'VNamesSize', 'ValueKindLast']
    : ['Magic', 'Version', 'BinaryIdsSize', 'NumData', 'PadBeforeCounters', 'NumCounters', 'PadAfterCounters', 'NumBitmapBytes', 'PadAfterBitmap', 'NamesSize', 'CountersDelta', 'BitmapDelta', 'NamesDelta', 'NumVTables', 'VNamesSize', 'ValueKindLast'];
  if (buffer.length < fields.length * 8) throw Error('truncated raw profile header');
  const header = Object.fromEntries(fields.map((name, index) => [name, word(index)]));
  const dataOffset = fields.length * 8 + Number(header.BinaryIdsSize);
  const firstNameRef = header.NumData > 0n ? buffer.readBigUInt64LE(dataOffset) : 0n;
  return {
    rawVersion: raw,
    signature: (((header.NamesSize << 40n) + (header.NumCounters << 30n) + (header.NumData << 20n)
      + (1024n << 10n) + firstNameRef + version + header.Magic) & mask).toString(),
  };
}

async function dump(page, reason) {
  const state = page.__lytronautCoverage;
  if (!state || !state.client) return;
  state.dumped = true;
  if (page.isClosed()) { fail(`${reason}: page closed before its client coverage was dumped`); return; }
  let result;
  try {
    result = await page.evaluate(() => {
      const hook = globalThis.__lytronautCoverage;
      if (!hook) return { missing: true };
      const bytes = hook.dump();
      let binary = '';
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
      return { signature: hook.signature(), length: bytes.length, base64: btoa(binary) };
    });
  } catch (error) {
    fail(`${reason}: coverage dump failed: ${error.message}`);
    return;
  }
  if (result.missing) { fail(`${reason}: page loaded the client without the coverage hook (uninstrumented build?)`); return; }
  const buffer = Buffer.from(result.base64, 'base64');
  if (!result.length || buffer.length !== result.length) { fail(`${reason}: empty or truncated profile`); return; }
  let computed;
  try { computed = profileSignature(buffer); } catch (error) { fail(`${reason}: ${error.message}`); return; }
  if (computed.signature !== result.signature) { fail(`${reason}: profile signature ${computed.signature} != module ${result.signature}`); return; }
  if (!summary.signatures.includes(result.signature)) summary.signatures.push(result.signature);
  if (summary.signatures.length > 1) fail('profiles from more than one WASM build in one coverage directory');
  const file = path.join(dir, `${script}-${process.pid}-${String(++sequence).padStart(4, '0')}-${reason.replace(/[^a-z0-9]+/gi, '_')}.profraw`);
  fs.writeFileSync(file, buffer, { mode: 0o600, flag: 'wx' });
  summary.profiles++;
  summary.bytes += buffer.length;
}

function track(page) {
  if (!dir || page.__lytronautCoverage) return page;
  const state = { client: false, dumped: false };
  Object.defineProperty(page, '__lytronautCoverage', { value: state });
  page.on('response', response => {
    if (CLIENT_WASM.test(response.url()) && response.frame() === page.mainFrame() && !state.client) {
      state.client = true;
      state.dumped = false;
      summary.pagesWithClient++;
    }
  });
  page.on('framenavigated', frame => {
    if (frame !== page.mainFrame()) return;
    if (state.client && !state.dumped) fail(`main frame navigated to a new document without a coverage dump (${frame.url().split('?')[0]})`);
    state.client = false;
    state.dumped = false;
  });
  for (const method of ['goto', 'reload', 'close']) {
    const original = page[method].bind(page);
    page[method] = async (...args) => {
      await dump(page, method);
      return original(...args);
    };
  }
  return page;
}

// Drop-in replacement for puppeteer.launch().
async function launch(puppeteer, options) {
  const browser = await puppeteer.launch(options);
  if (!dir) return browser;
  const newPage = browser.newPage.bind(browser);
  browser.newPage = async (...args) => track(await newPage(...args));
  const close = browser.close.bind(browser);
  browser.close = async (...args) => {
    for (const page of await browser.pages().catch(() => [])) if (page.__lytronautCoverage && !page.isClosed()) await dump(page, 'browser_close');
    return close(...args);
  };
  return browser;
}

if (dir) {
  fs.mkdirSync(dir, { recursive: true });
  // A navigation the driver did not dump first is detected after the fact, so
  // the final verdict is applied when the process exits.
  process.on('exit', () => {
    fs.writeFileSync(path.join(dir, `driver-${script}-${process.pid}.json`), JSON.stringify(summary, null, 2) + '\n');
    if (summary.failures.length) {
      console.error(`coverage: ${summary.failures.length} WASM coverage failure(s) in ${script}`);
      process.exitCode = 1;
    }
  });
}

module.exports = { launch, enabled: dir !== null };
