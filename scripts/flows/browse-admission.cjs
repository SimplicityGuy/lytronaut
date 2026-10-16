// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Client-side ZIP admission of hostile archives (refused
// before any import is created), capture navigation, a capture without a
// verified preview, and a flattened export of a static preview.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');

const u16 = v => { const b = Buffer.alloc(2); b.writeUInt16LE(v); return b; };
const u32 = v => { const b = Buffer.alloc(4); b.writeUInt32LE(v >>> 0); return b; };
const u64 = v => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; };
const sig = text => Buffer.from(text, 'latin1');

// Independently written ZIP structures (PKWARE APPNOTE): one stored entry with
// optional overrides of the central size/extra fields and the end records.
function archive({ name = 'synthetic.bin', data = Buffer.from('synthetic'), centralSize, extra = Buffer.alloc(0), extraLength, centralSignature = 'PK\x01\x02', end = {}, zip64 } = {}) {
  const nameBytes = Buffer.from(name);
  const crc = zlib.crc32(data);
  const local = Buffer.concat([sig('PK\x03\x04'), u16(20), u16(0), u16(0), u32(0), u32(crc), u32(data.length), u32(data.length), u16(nameBytes.length), u16(0), nameBytes, data]);
  const central = Buffer.concat([sig(centralSignature), u16(0x0314), u16(20), u16(0), u16(0), u32(0), u32(crc), u32(data.length), u32(centralSize ?? data.length),
    u16(nameBytes.length), u16(extraLength ?? extra.length), u16(0), u16(0), u16(0), u32(0o100644 << 16), u32(0), nameBytes, extra]);
  const parts = [local, central];
  if (zip64) {
    const offset = local.length + central.length;
    parts.push(Buffer.concat([sig(zip64.recordSignature ?? 'PK\x06\x06'), u64(44), u16(45), u16(45), u32(0), u32(0), u64(1), u64(zip64.total ?? 1), u64(central.length), u64(local.length)]));
    parts.push(Buffer.concat([sig('PK\x06\x07'), u32(0), u64(zip64.locatorOffset ?? offset), u32(zip64.disks ?? 1)]));
  }
  const count = end.count ?? (zip64 ? 0xffff : 1);
  parts.push(Buffer.concat([sig('PK\x05\x06'), u16(end.disk ?? 0), u16(0), u16(count), u16(count),
    u32(zip64 ? 0xffffffff : central.length), u32(zip64 ? 0xffffffff : local.length), u16(0)]));
  return Buffer.concat(parts);
}

const ZIP_CASES = [
  ['no-end-record', Buffer.from('not a zip archive at all, only synthetic bytes'), 'ZIP end record missing'],
  // Consecutive cases never share a message, so each status change is attributable.
  ['multi-disk-classic', archive({ end: { disk: 1 } }), 'Multi-volume ZIP is unsupported'],
  ['zip64-record-beyond-archive', archive({ zip64: { locatorOffset: 1 << 30 } }), 'ZIP64 end record exceeds archive'],
  ['multi-disk-zip64', archive({ zip64: { disks: 2 } }), 'Multi-volume ZIP is unsupported'],
  ['invalid-zip64-record', archive({ zip64: { recordSignature: 'PK\x06\x05' } }), 'Invalid ZIP64 end record'],
  ['zip64-split-entry-counts', archive({ zip64: { total: 2 } }), 'Multi-volume ZIP is unsupported'],
  ['zip64-sentinel-without-locator', archive({ end: { count: 0xffff } }), 'ZIP64 locator missing'],
  ['bad-central-signature', archive({ centralSignature: 'PK\x01\x03' }), 'Invalid ZIP central directory'],
  ['central-extra-overrun', archive({ extraLength: 40 }), 'Truncated ZIP central directory'],
  ['zip64-size-missing', archive({ centralSize: 0xffffffff }), 'ZIP64 size missing'],
  ['zip64-extra-truncated', archive({ centralSize: 0xffffffff, extra: Buffer.concat([u16(1), u16(16), u64(5)]) }), 'Truncated ZIP extra field'],
  ['oversized-member', archive({ centralSize: 600 * 1024 * 1024 }), 'ZIP individual file exceeds 512 MiB'],
  ['directories-only', archive({ name: 'only-a-directory/', data: Buffer.alloc(0) }), 'ZIP library is empty'],
];

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    Object.defineProperty(window, 'VideoDecoder', { value: undefined, writable: true });
  });
  let rule = null;
  await page.setRequestInterception(true);
  page.on('request', request => {
    if (!(rule && rule(request))) request.continue().catch(() => {});
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
  const viewer = () => page.$eval('.viewer-strip span', e => e.textContent);
  const proof = {};
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'lytronaut-flow-zip-'));
  try {
    await page.goto(origin);
    await page.waitForSelector('#directory');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0 && document.querySelector('.stage img')?.complete, { timeout: 120000 });

    // 1. Hostile ZIP framing is refused in the browser before any import exists.
    let imports = 0;
    const countImports = request => { if (request.method() === 'POST' && new URL(request.url()).pathname === '/imports') imports++; };
    page.on('request', countImports);
    proof.zipAdmission = [];
    for (const [name, bytes, message] of ZIP_CASES) {
      const file = path.join(scratch, `${name}.zip`);
      fs.writeFileSync(file, bytes);
      await page.waitForFunction(() => !document.querySelector('#zip').disabled);
      await (await page.$('#zip')).uploadFile(file);
      await statusIncludes(message);
      proof.zipAdmission.push(name);
    }
    // The control accepts one archive; a multi-file selection is refused too.
    await page.$eval('#zip', e => { e.multiple = true; });
    await (await page.$('#zip')).uploadFile(path.join(scratch, 'no-end-record.zip'), path.join(scratch, 'directories-only.zip'));
    await statusIncludes('Select one ZIP archive');
    proof.zipAdmission.push('two-archives');
    page.off('request', countImports);
    assert.equal(imports, 0, 'no import is created for a refused archive');

    // 1b. Directory selections are admitted in the browser before any import:
    //     unsafe relative names, members over 512 MiB (sparse synthetic file) and
    //     selections without bytes are refused.
    page.on('request', countImports);
    proof.directoryAdmission = [];
    for (const [name, populate, message] of [
      ['unsafe-name', dir => fs.writeFileSync(path.join(dir, 'drive:name.bin'), 'x'), 'Selection contains an unsafe or oversized relative path'],
      ['oversized-member', dir => { const fd = fs.openSync(path.join(dir, 'large.bin'), 'w'); fs.ftruncateSync(fd, 513 * 1024 * 1024); fs.closeSync(fd); }, 'An individual file exceeds 512 MiB'],
      ['no-bytes', dir => fs.writeFileSync(path.join(dir, 'empty.bin'), ''), 'Selection must contain bytes and fit the 20 GiB import budget'],
    ]) {
      const dir = path.join(scratch, `directory-${name}`);
      fs.mkdirSync(dir);
      populate(dir);
      await page.waitForFunction(() => !document.querySelector('#directory').disabled);
      await (await page.$('#directory')).uploadFile(dir);
      await statusIncludes(message);
      proof.directoryAdmission.push(name);
    }
    page.off('request', countImports);
    assert.equal(imports, 0, 'no import is created for a refused directory selection');

    // 2. Previous/next capture navigation follows the grid order.
    await click('All captures');
    await page.waitForFunction(() => document.querySelectorAll('.capture span').length > 1);
    const names = await page.$$eval('.capture span', spans => spans.map(s => s.textContent));
    await page.evaluate(name => [...document.querySelectorAll('.capture')].find(b => b.querySelector('span').textContent === name).click(), names[0]);
    await page.waitForFunction(name => document.querySelector('.viewer-strip span').textContent === name, {}, names[0]);
    await click('Next capture');
    await page.waitForFunction(name => document.querySelector('.viewer-strip span').textContent === name, {}, names[1]);
    await click('Previous capture');
    await page.waitForFunction(name => document.querySelector('.viewer-strip span').textContent === name, {}, names[0]);
    await click('Previous capture'); // already first: reloads the same capture
    await page.waitForFunction(name => document.querySelector('.viewer-strip span').textContent === name && document.querySelector('.stage img')?.complete, {}, names[0]);
    assert.equal(await viewer(), names[0]);
    proof.navigation = true;

    // 3. A capture without a verified preview says so; originals stay listed.
    const target = names[1];
    rule = request => {
      if (request.method() !== 'GET' || !/^\/captures\/[^/]+$/.test(new URL(request.url()).pathname)) return false;
      (async () => {
        const detail = await (await fetch(request.url(), { signal: AbortSignal.timeout(10000) })).json();
        if (detail.capture.name === target) detail.preview = null;
        await request.respond({ status: 200, contentType: 'application/json', body: JSON.stringify(detail) });
      })().catch(() => request.abort().catch(() => {}));
      return true;
    };
    await click('Next capture');
    await page.waitForFunction(name => document.querySelector('.viewer-strip span').textContent === name, {}, target);
    await statusIncludes('Preview unavailable. Preserved originals remain downloadable');
    assert((await page.$$eval('.inspector ul a', links => links.length)) > 0, 'originals remain downloadable');
    rule = null;
    proof.previewlessCapture = true;

    // 4. Flattened PNG export of a verified static preview (no focus stack).
    await click('All captures');
    await page.waitForFunction(() => document.querySelectorAll('.capture span').length > 0);
    for (let i = 0; i < 4; i++) {
      if (await page.evaluate(() => { const b = [...document.querySelectorAll('.capture')].find(c => c.querySelector('span').textContent === 'Synthetic 002'); if (!b) return false; b.click(); return true; })) break;
      const before = await page.$eval('.pagination span', e => e.textContent);
      await click('Next page');
      await page.waitForFunction(before => document.querySelector('.pagination span').textContent !== before, {}, before);
    }
    await page.waitForFunction(() => document.querySelector('.viewer-strip span').textContent === 'Synthetic 002' && document.querySelector('.stage img')?.naturalWidth === 640);
    assert(await page.$eval('#focus', e => e.disabled), 'static preview has no discrete focus');
    await click('Export PNG');
    await page.waitForFunction(() => document.querySelector('#export-progress')?.textContent.includes('Succeeded') && document.querySelector('#export-download'), { timeout: 120000 });
    await statusIncludes('Flattened export complete');
    const recipe = await page.evaluate(async () => (await (await fetch(document.querySelector('#export-recipe').getAttribute('href'))).json()));
    assert.equal(recipe.format, 'png');
    assert.notEqual(recipe.source_variant, 'stored_jpeg_focus_plane');
    proof.staticPngExport = { sourceVariant: recipe.source_variant };

    // 5. A whole-library backup refused for lack of server work space reports
    //    the exact shortfall from the retryable 429 instead of a generic limit.
    rule = request => {
      if (request.method() !== 'POST' || new URL(request.url()).pathname !== '/exports') return false;
      request.respond({ status: 429, contentType: 'application/json', body: JSON.stringify({ version: 1, code: 'resource_limit', retryable: true, shortfall: { required_bytes: 5 * 1024 ** 3, available_bytes: 1024 ** 3 } }) }).catch(() => {});
      return true;
    };
    await click('Back up originals and catalog');
    await statusIncludes('Not enough free work space on the server: this needs 5.0 GiB and 1.0 GiB is available');
    // Sizes below one KiB are shown in bytes.
    rule = request => {
      if (request.method() !== 'POST' || new URL(request.url()).pathname !== '/exports') return false;
      request.respond({ status: 429, contentType: 'application/json', body: JSON.stringify({ version: 1, code: 'resource_limit', retryable: true, shortfall: { required_bytes: 900, available_bytes: 100 } }) }).catch(() => {});
      return true;
    };
    await click('Back up originals and catalog');
    await statusIncludes('this needs 900 bytes and 100 bytes is available');
    rule = null;
    proof.backupShortfall = true;

    // 6. Recovering the saved export job: a status read that stays rate-limited
    //    after the client's bounded retries is reported, and unavailable session
    //    storage is reported instead of throwing.
    let limited = 0;
    rule = request => {
      if (request.method() !== 'GET' || !/^\/exports\/[^/]+$/.test(new URL(request.url()).pathname)) return false;
      limited++;
      request.respond({ status: 429, contentType: 'application/json', body: '{"version":1,"code":"resource_limit","retryable":true}' }).catch(() => {});
      return true;
    };
    await click('Recover export / restore status');
    await statusIncludes('Resource limit reached');
    rule = null;
    assert.equal(limited, 3, 'a rate-limited status read is retried a bounded number of times');
    await page.evaluate(() => {
      const original = Object.getOwnPropertyDescriptor(window, 'sessionStorage');
      Object.defineProperty(window, 'sessionStorage', { configurable: true, get() { throw Error('injected'); } });
      window.__restoreStorage = () => { if (original) Object.defineProperty(window, 'sessionStorage', original); else delete window.sessionStorage; };
    });
    await click('Recover export / restore status');
    await statusIncludes('Job recovery unavailable');
    await page.evaluate(() => window.__restoreStorage());
    proof.exportRecovery = ['rate-limited', 'storage-unavailable'];

    // 7. A restore upload answered with something other than a job report is
    //    reported, not followed.
    const bundle = path.join(scratch, 'synthetic.lytronaut');
    fs.writeFileSync(bundle, Buffer.from('synthetic bundle placeholder'));
    rule = request => {
      if (request.method() !== 'POST' || new URL(request.url()).pathname !== '/restores') return false;
      request.respond({ status: 200, contentType: 'application/json', body: 'not a job report' }).catch(() => {});
      return true;
    };
    await page.waitForFunction(() => !document.querySelector('#restore-bundle').disabled);
    await (await page.$('#restore-bundle')).uploadFile(bundle);
    await statusIncludes('Invalid restore report');
    rule = null;
    proof.invalidRestoreReport = true;

    assert.deepEqual(errors, [], 'no uncaught page errors');
    return proof;
  } finally {
    rule = null;
    fs.rmSync(scratch, { recursive: true, force: true });
    await page.close();
  }
};
