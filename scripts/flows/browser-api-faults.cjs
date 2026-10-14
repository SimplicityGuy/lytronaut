// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Makes individual browser platform APIs fail (file
// reads, XMLHttpRequest, canvas contexts, secure random identifiers) and
// asserts that the client reports the failure, starts nothing it cannot finish
// and recovers once the API works again.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');

// Small stored ZIP and ZIP64 archives (independently written APPNOTE framing).
function zip(name, data, zip64) {
  const u16 = v => { const b = Buffer.alloc(2); b.writeUInt16LE(v); return b; };
  const u32 = v => { const b = Buffer.alloc(4); b.writeUInt32LE(v >>> 0); return b; };
  const u64 = v => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; };
  const n = Buffer.from(name);
  const crc = zlib.crc32(data);
  const local = Buffer.concat([Buffer.from('PK\x03\x04', 'latin1'), u16(20), u16(0), u16(0), u32(0), u32(crc), u32(data.length), u32(data.length), u16(n.length), u16(0), n, data]);
  const central = Buffer.concat([Buffer.from('PK\x01\x02', 'latin1'), u16(0x0314), u16(20), u16(0), u16(0), u32(0), u32(crc), u32(data.length), u32(data.length), u16(n.length), u16(0), u16(0), u16(0), u16(0), u32(0o100644 << 16), u32(0), n]);
  const parts = [local, central];
  if (zip64) {
    parts.push(Buffer.concat([Buffer.from('PK\x06\x06', 'latin1'), u64(44), u16(45), u16(45), u32(0), u32(0), u64(1), u64(1), u64(central.length), u64(local.length)]));
    parts.push(Buffer.concat([Buffer.from('PK\x06\x07', 'latin1'), u32(0), u64(local.length + central.length), u32(1)]));
  }
  parts.push(Buffer.concat([Buffer.from('PK\x05\x06', 'latin1'), u16(0), u16(0), u16(zip64 ? 0xffff : 1), u16(zip64 ? 0xffff : 1),
    u32(zip64 ? 0xffffffff : central.length), u32(zip64 ? 0xffffffff : local.length), u16(0)]));
  return Buffer.concat(parts);
}

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    window.__nativeVideoDecoder = window.VideoDecoder;
    window.__restore = [];
    // Replace one prototype member until window.__undo() restores it.
    window.__break = (owner, key, replacement) => {
      const original = Object.getOwnPropertyDescriptor(owner, key);
      Object.defineProperty(owner, key, { ...original, value: replacement, configurable: true, writable: true });
      window.__restore.push(() => Object.defineProperty(owner, key, original));
    };
    window.__undo = () => { while (window.__restore.length) window.__restore.pop()(); };
  });
  const statusIncludes = text => page.waitForFunction(text => (document.querySelector('.status')?.textContent || '').includes(text), {}, text)
    .catch(async error => {
      const status = await page.$eval('.status', e => e.textContent).catch(() => null);
      throw Error(`status never included ${JSON.stringify(text)}; last status ${JSON.stringify(status)}`, { cause: error });
    });
  const click = text => page.waitForFunction(text => {
    const button = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === text && !b.disabled);
    if (!button) return false;
    button.click();
    return true;
  }, { polling: 50 }, text);
  const pick = async name => {
    await page.$eval('#search', (e, name) => { e.value = name; e.dispatchEvent(new Event('input', { bubbles: true })); }, name);
    await click('Search');
    await page.waitForFunction(name => [...document.querySelectorAll('.capture span')].some(s => s.textContent === name), {}, name);
    await page.evaluate(name => [...document.querySelectorAll('.capture')].find(b => b.querySelector('span').textContent === name).click(), name);
    await page.waitForFunction(name => document.querySelector('.viewer-strip span')?.textContent === name, {}, name);
  };
  const proof = {};
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'lytronaut-flow-api-'));
  try {
    await page.goto(origin);
    await page.waitForSelector('#directory');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0, { timeout: 120000 });
    let imports = 0;
    page.on('request', request => { if (request.method() === 'POST' && new URL(request.url()).pathname === '/imports') imports++; });

    // 1. Unreadable ZIP selections are refused before any import is created.
    const classic = path.join(scratch, 'classic.zip');
    const wide = path.join(scratch, 'wide.zip');
    fs.writeFileSync(classic, zip('synthetic.bin', Buffer.from('synthetic'), false));
    fs.writeFileSync(wide, zip('synthetic.bin', Buffer.from('synthetic'), true));
    proof.zipReads = [];
    for (const [name, file, breakIt, message] of [
      ['slice-throws', classic, () => window.__break(Blob.prototype, 'slice', () => { throw Error('injected'); }), 'ZIP read unavailable'],
      ['read-rejects', classic, () => window.__break(Blob.prototype, 'arrayBuffer', () => Promise.reject(Error('injected'))), 'ZIP read unavailable'],
      ['zip64-record-slice-throws', wide, () => { let calls = 0; const slice = Blob.prototype.slice; window.__break(Blob.prototype, 'slice', function (...a) { if (++calls === 2) throw Error('injected'); return slice.apply(this, a); }); }, 'ZIP64 read unavailable'],
      ['zip64-record-read-rejects', wide, () => { let calls = 0; const read = Blob.prototype.arrayBuffer; window.__break(Blob.prototype, 'arrayBuffer', function () { return ++calls === 2 ? Promise.reject(Error('injected')) : read.call(this); }); }, 'ZIP64 read unavailable'],
      ['central-slice-throws', classic, () => { let calls = 0; const slice = Blob.prototype.slice; window.__break(Blob.prototype, 'slice', function (...a) { if (++calls === 2) throw Error('injected'); return slice.apply(this, a); }); }, 'ZIP metadata unavailable'],
      ['central-read-rejects', classic, () => { let calls = 0; const read = Blob.prototype.arrayBuffer; window.__break(Blob.prototype, 'arrayBuffer', function () { return ++calls === 2 ? Promise.reject(Error('injected')) : read.call(this); }); }, 'ZIP metadata unavailable'],
    ]) {
      await page.waitForFunction(() => !document.querySelector('#zip').disabled);
      await page.evaluate(breakIt);
      await (await page.$('#zip')).uploadFile(file);
      await statusIncludes(message);
      await page.evaluate(() => window.__undo());
      proof.zipReads.push(name);
    }
    assert.equal(imports, 0, 'no import is created when the selection cannot be read');

    // 2. A transfer the browser cannot start is reported, not shown as progress.
    const folder = path.join(scratch, 'Synthetic transfer');
    fs.mkdirSync(folder);
    fs.writeFileSync(path.join(folder, 'synthetic.bin'), 'synthetic transfer payload');
    proof.transfers = [];
    for (const [name, breakIt, message] of [
      ['open-throws', () => window.__break(XMLHttpRequest.prototype, 'open', () => { throw Error('injected'); }), 'Upload unavailable'],
      ['header-throws', () => window.__break(XMLHttpRequest.prototype, 'setRequestHeader', () => { throw Error('injected'); }), 'Upload unavailable'],
      ['send-throws', () => window.__break(XMLHttpRequest.prototype, 'send', () => { throw Error('injected'); }), 'Upload could not start'],
    ]) {
      await page.waitForFunction(() => !document.querySelector('#directory').disabled);
      await page.evaluate(breakIt);
      await (await page.$('#directory')).uploadFile(folder);
      await statusIncludes(message);
      await page.evaluate(() => window.__undo());
      proof.transfers.push(name);
    }

    // 3. While canvas contexts cannot be created, grid thumbnails cannot be
    //    downsampled; browsing and selection keep working and every thumbnail
    //    is drawn again once canvases work.
    await page.evaluate(() => window.__break(HTMLCanvasElement.prototype, 'getContext', () => { throw Error('injected'); }));
    await page.$eval('#search', e => { e.value = 'Synthetic 01'; e.dispatchEvent(new Event('input', { bubbles: true })); });
    await click('Search');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0);
    // Selection keeps working while thumbnails cannot be drawn.
    const name = await page.$eval('.capture span', s => s.textContent);
    await page.evaluate(() => document.querySelector('.capture').click());
    await page.waitForFunction(name => document.querySelector('.viewer-strip span')?.textContent === name, {}, name);
    await page.evaluate(() => window.__undo());
    await page.$eval('#search', e => { e.value = ''; e.dispatchEvent(new Event('input', { bubbles: true })); });
    await click('Search');
    await page.waitForFunction(() => document.querySelectorAll('.capture img').length > 0 && [...document.querySelectorAll('.capture img')].every(i => i.complete && i.naturalWidth > 0), { timeout: 120000 });
    proof.canvasUnavailable = true;

    // 4. Browser Live Photo export needs a secure random identifier; without
    //    one nothing is generated or offered.
    await pick('Synthetic 000');
    await page.waitForFunction(() => !document.querySelector('#focus').disabled && (document.querySelector('.status')?.textContent || '').includes('Discrete focus:'));
    await click('Try browser H.264 refocus');
    await statusIncludes('Archived discrete browser focus decoded');
    await page.evaluate(() => window.__break(Crypto.prototype, 'randomUUID', undefined));
    await click('Export Live Photo pair (.zip)');
    await statusIncludes('Secure random export identifier unavailable');
    await page.evaluate(() => window.__undo());
    proof.secureRandomRequired = true;

    assert.deepEqual(errors, [], 'API failures never surface as uncaught page errors');
    return proof;
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
    await page.close();
  }
};
