// Drives the wasm-bindgen module in headless Chrome, calls hit() (both
// branches), dumps coverage, writes the .profraw and checks the module
// signature recomputed from the profraw header against cov_signature().
import { writeFileSync } from 'node:fs';
import puppeteer from 'puppeteer-core';

const [url, outProfraw, outJson] = process.argv.slice(2);

const browser = await puppeteer.launch({
  executablePath: '/usr/bin/google-chrome',
  headless: true,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});
const page = await browser.newPage();
const consoleLines = [];
page.on('console', (m) => consoleLines.push(`[console.${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => consoleLines.push(`[pageerror] ${e.message}`));
await page.goto(url, { waitUntil: 'load' });

const res = await page.evaluate(async () => {
  const m = await import('/pkg/covsmoke.js');
  await m.default();
  const sigBefore = m.cov_signature().toString();
  const h1 = m.hit(12);
  const h2 = m.hit(3);
  const bytes = m.cov_dump();
  const sigAfter = m.cov_signature().toString();
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return { sigBefore, sigAfter, h1, h2, len: bytes.length, b64: btoa(bin) };
});
await browser.close();
for (const l of consoleLines) console.log(l);

const buf = Buffer.from(res.b64, 'base64');
writeFileSync(outProfraw, buf);

// Recompute lprofGetLoadModuleSignature() from the profraw header.
const M = (1n << 64n) - 1n;
const u64 = (i) => buf.readBigUInt64LE(i * 8);
const magic = u64(0);
const version = u64(1);
const rawVer = Number(version & 0xffffffffn);
const fields = rawVer >= 11
  ? ['Magic', 'Version', 'BinaryIdsSize', 'NumData', 'PadBeforeCounters', 'NumCounters', 'PadAfterCounters', 'NumBitmapBytes', 'PadAfterBitmap', 'NumUniformCounters', 'PadAfterUniform', 'UniformCountersDelta', 'NamesSize', 'CountersDelta', 'BitmapDelta', 'NamesDelta', 'NumVTables', 'VNamesSize', 'ValueKindLast']
  : ['Magic', 'Version', 'BinaryIdsSize', 'NumData', 'PadBeforeCounters', 'NumCounters', 'PadAfterCounters', 'NumBitmapBytes', 'PadAfterBitmap', 'NamesSize', 'CountersDelta', 'BitmapDelta', 'NamesDelta', 'NumVTables', 'VNamesSize', 'ValueKindLast'];
const h = Object.fromEntries(fields.map((f, i) => [f, u64(i)]));
const dataOff = fields.length * 8 + Number(h.BinaryIdsSize);
const firstNameRef = h.NumData > 0n ? buf.readBigUInt64LE(dataOff) : 0n;
const sigFromProfraw = ((h.NamesSize << 40n) + (h.NumCounters << 30n) + (h.NumData << 20n)
  + firstNameRef + version + magic) & M;

const summary = {
  hit12: res.h1,
  hit3: res.h2,
  profrawBytes: res.len,
  rawProfileVersion: rawVer,
  header: Object.fromEntries(Object.entries(h).map(([k, v]) => [k, v.toString()])),
  sigBefore: res.sigBefore,
  sigAfter: res.sigAfter,
  sigFromProfraw: sigFromProfraw.toString(),
};
summary.signatureMatch = summary.sigBefore === summary.sigFromProfraw && summary.sigAfter === summary.sigFromProfraw;
writeFileSync(outJson, JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary, null, 2));
if (!summary.signatureMatch) { console.error('SIGNATURE MISMATCH'); process.exit(1); }
if (res.len === 0) { console.error('EMPTY PROFRAW'); process.exit(1); }
