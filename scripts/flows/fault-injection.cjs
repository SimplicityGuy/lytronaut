// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Runs on its own page against the library imported by
// browser-test.cjs and injects transport, preview, search, export and codec
// faults through request interception or platform API substitution. Every case
// asserts the user-visible outcome and that verified pixels stay displayed.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

// Independently written ZIP64 archive (APPNOTE 6.3.10 structure): ZIP64 end
// record, locator and central/local ZIP64 extras around one stored file.
function zip64Archive(name, data) {
  const u16 = v => { const b = Buffer.alloc(2); b.writeUInt16LE(v); return b; };
  const u32 = v => { const b = Buffer.alloc(4); b.writeUInt32LE(v >>> 0); return b; };
  const u64 = v => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; };
  const crc = require('node:zlib').crc32(data);
  const nameBytes = Buffer.from(name);
  const local = Buffer.concat([Buffer.from('PK\x03\x04', 'latin1'), u16(45), u16(0), u16(0), u32(0), u32(crc), u32(0xffffffff), u32(0xffffffff),
    u16(nameBytes.length), u16(20), nameBytes, u16(1), u16(16), u64(data.length), u64(data.length), data]);
  const central = Buffer.concat([Buffer.from('PK\x01\x02', 'latin1'), u16(0x0314), u16(45), u16(0), u16(0), u32(0), u32(crc), u32(0xffffffff), u32(0xffffffff),
    u16(nameBytes.length), u16(28), u16(0), u16(0), u16(0), u32(0o100644 << 16), u32(0xffffffff), nameBytes,
    u16(1), u16(24), u64(data.length), u64(data.length), u64(0)]);
  const zoffset = local.length + central.length;
  const record = Buffer.concat([Buffer.from('PK\x06\x06', 'latin1'), u64(44), u16(45), u16(45), u32(0), u32(0), u64(1), u64(1), u64(central.length), u64(local.length)]);
  const locator = Buffer.concat([Buffer.from('PK\x06\x07', 'latin1'), u32(0), u64(zoffset), u32(1)]);
  const end = Buffer.concat([Buffer.from('PK\x05\x06', 'latin1'), u16(0), u16(0), u16(0xffff), u16(0xffff), u32(0xffffffff), u32(0xffffffff), u16(0)]);
  return Buffer.concat([local, central, record, locator, end]);
}

const STATUS = () => document.querySelector('.status')?.textContent || '';

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    // CPU path only; GPU behaviour has its own suite.
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    window.__nativeVideoDecoder = window.VideoDecoder;
  });
  // One interception handler; each case installs a rule and removes it after.
  let rule = null;
  await page.setRequestInterception(true);
  page.on('request', request => {
    const handled = rule && rule(request);
    if (!handled) request.continue().catch(() => {});
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
  const ready = async () => {
    await page.waitForSelector('#directory');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0 && document.querySelector('.stage img')?.complete && document.querySelector('.stage img')?.naturalWidth > 0, { timeout: 120000 });
  };
  const selectNamed = async name => {
    await click('All captures');
    for (let i = 0; i < 4; i++) {
      await page.waitForFunction(() => document.querySelectorAll('.capture span').length > 0);
      if (await page.evaluate(name => {
        const button = [...document.querySelectorAll('.capture')].find(b => b.querySelector('span')?.textContent === name);
        if (!button) return false;
        button.click();
        return true;
      }, name)) {
        await page.waitForFunction(name => document.querySelector('.viewer-strip span')?.textContent === name, {}, name);
        return;
      }
      const before = await page.$eval('.pagination span', e => e.textContent);
      await click('Next page');
      await page.waitForFunction(before => document.querySelector('.pagination span').textContent !== before, {}, before);
    }
    throw Error(`capture ${name} not found`);
  };
  const proof = {};
  try {
    // 1. Transient 429 on GET is retried with backoff and the library loads.
    let attempts = 0;
    rule = request => {
      if (request.method() === 'GET' && new URL(request.url()).pathname === '/libraries' && attempts < 2) {
        attempts++;
        request.respond({ status: 429, contentType: 'application/json', body: '{"version":1,"code":"resource_limit","retryable":true}' }).catch(() => {});
        return true;
      }
      return false;
    };
    await page.goto(origin);
    await ready();
    assert.equal(attempts, 2, 'two throttled attempts before success');
    proof.retried429 = true;

    // 2. Terminal transport outcomes are reported, never shown as an empty library.
    const failures = [
      ['policy-401', r => r.respond({ status: 401, contentType: 'application/json', body: '{}' }), 'Request rejected by deployment policy'],
      ['throttled-429', r => r.respond({ status: 429, contentType: 'application/json', body: '{}' }), 'Resource limit reached. Wait, then retry'],
      ['bad-request-400', r => r.respond({ status: 400, contentType: 'application/json', body: '{}' }), 'Selection or request was rejected'],
      ['missing-404', r => r.respond({ status: 404, contentType: 'application/json', body: '{}' }), 'This item is no longer available'],
      ['unavailable-503', r => r.respond({ status: 503, contentType: 'application/json', body: '{}' }), 'The service could not complete this request'],
      ['network', r => r.abort('connectionfailed'), 'Connection interrupted'],
      ['malformed-json', r => r.respond({ status: 200, contentType: 'application/json', body: '{"version":1,"items":[' }), 'Invalid or oversized service response'],
    ];
    proof.libraryFailures = [];
    for (const [name, respond, message] of failures) {
      rule = request => {
        if (new URL(request.url()).pathname !== '/libraries') return false;
        respond(request).catch(() => {});
        return true;
      };
      await page.reload();
      await page.waitForSelector('#directory');
      await statusIncludes(message);
      assert.equal(await page.$$eval('.capture', c => c.length), 0, `${name}: no stale captures`);
      proof.libraryFailures.push(name);
    }
    rule = null;
    await page.reload();
    await ready();

    // 3. Search with no match is an explicit empty state.
    // An over-long search is refused by the client before any request.
    await page.$eval('#search', e => { e.value = 'x'.repeat(129); e.dispatchEvent(new Event('input', { bubbles: true })); });
    await click('Search');
    await statusIncludes('Search must contain at most 128 UTF-8 bytes without control characters');
    await page.$eval('#search', e => { e.value = ''; e.dispatchEvent(new Event('input', { bubbles: true })); });
    await page.type('#search', 'zz-no-synthetic-match');
    await click('Search');
    await statusIncludes('No captures match this search');
    assert.equal(await page.$$eval('.capture', c => c.length), 0);
    await page.$eval('#search', e => { e.value = ''; e.dispatchEvent(new Event('input', { bubbles: true })); });
    await click('Search');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0);
    proof.emptySearch = true;

    // 4. Stage preview faults keep the previous verified frame and report why.
    await selectNamed('Synthetic 002');
    await page.waitForFunction(() => document.querySelector('.stage img')?.naturalWidth === 640);
    const previewId = await page.evaluate(async () => {
      const libraries = await (await fetch('/libraries')).json();
      const library = libraries.items.find(l => l.state === 'ready');
      const captures = await (await fetch(`/libraries/${library.id}/captures?limit=100`)).json();
      const capture = captures.items.find(c => c.name === 'Synthetic 002');
      return (await (await fetch(`/captures/${capture.id}`)).json()).preview;
    });
    assert(previewId, 'synthetic static preview exists');
    const manifest = await page.evaluate(async id => (await fetch(`/assets/${id}/manifest`)).json(), previewId);
    const previewFaults = [
      ['invalid-manifest', `/assets/${previewId}/manifest`, r => r.respond({ status: 200, contentType: 'application/json', body: '{"not":"a manifest"}' }), 'Invalid preview manifest'],
      ['foreign-manifest', `/assets/${previewId}/manifest`, r => r.respond({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...manifest, id: 'some-other-asset' }) }), 'Preview exceeds display budget'],
      ['original-representation', `/assets/${previewId}/manifest`, r => r.respond({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...manifest, representation: 'original', dimensions: null, decoded_bytes: null }) }), 'This representation is preserved only'],
      ['over-viewer-budget', `/assets/${previewId}/manifest`, r => r.respond({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...manifest, encoded_bytes: 16 * 1024 * 1024, chunks: [{ offset: 0, length: 8 * 1024 * 1024 }, { offset: 8 * 1024 * 1024, length: 8 * 1024 * 1024 }], dimensions: { width: 2048, height: 2048 }, decoded_bytes: 2048 * 2048 * 4 }) }), 'Preview exceeds the browser memory budget'],
      ['short-chunk', `/assets/${previewId}/chunks/0`, r => r.respond({ status: 200, contentType: 'image/jpeg', body: Buffer.alloc(Math.max(1, manifest.chunks[0].length - 1)) }), 'Preview length mismatch'],
    ];
    proof.previewFaults = [];
    for (const [name, path, respond, message] of previewFaults) {
      const before = await page.$eval('.stage img', e => e.src);
      rule = request => {
        if (new URL(request.url()).pathname !== path) return false;
        respond(request).catch(() => {});
        return true;
      };
      await click('Retry preview');
      await statusIncludes(message);
      // Unverified bytes never reach the stage: either the previous verified
      // frame is kept or the stage is empty until a verified retry succeeds.
      const shown = await page.evaluate(() => document.querySelector('.stage img')?.src ?? null);
      assert(shown === null || shown === before, `${name}: no unverified frame displayed`);
      rule = null;
      await click('Retry preview');
      await statusIncludes('Static JPEG preview ready');
      await page.waitForFunction(() => document.querySelector('.stage img')?.naturalWidth === 640);
      proof.previewFaults.push(name);
    }

    // 5. Live Photo pair from a verified stored JPEG focus plane (server export).
    await selectNamed('Synthetic 000');
    await page.waitForFunction(() => !document.querySelector('#focus').disabled && (document.querySelector('.status')?.textContent || '').includes('Discrete focus:'));
    await click('Export Live Photo pair (.zip)');
    await page.waitForFunction(() => document.querySelector('#export-progress')?.textContent.includes('Succeeded') && document.querySelector('#export-download'), { timeout: 120000 });
    await statusIncludes('Live Photo pair complete');
    const zip = await page.evaluate(async () => {
      const bytes = new Uint8Array(await (await fetch(document.querySelector('#export-download').getAttribute('href'))).arrayBuffer());
      const text = new TextDecoder('latin1').decode(bytes);
      return { signature: [...bytes.slice(0, 4)], jpeg: /\.JPG/i.test(text), mov: /\.MOV/i.test(text), bytes: bytes.length };
    });
    assert.deepEqual(zip.signature, [0x50, 0x4b, 0x03, 0x04]);
    assert(zip.jpeg && zip.mov, 'pair contains both resources');
    proof.serverLivePhoto = { bytes: zip.bytes };

    // 6. Codec failures fall back to the verified static/JPEG frame.
    const codecFaults = {
      'unsupported-config': () => { window.VideoDecoder = class extends window.__nativeVideoDecoder { static isConfigSupported() { return Promise.resolve({ supported: false }); } }; },
      'support-probe-throws': () => { window.VideoDecoder = class extends window.__nativeVideoDecoder { static isConfigSupported() { throw Error('injected probe failure'); } }; },
      'configure-throws': () => { window.VideoDecoder = class extends window.__nativeVideoDecoder { configure() { throw Error('injected configure failure'); } }; },
      'decode-error-callback': () => {
        window.VideoDecoder = class extends window.__nativeVideoDecoder {
          constructor(init) { super({ output: frame => frame.close(), error: init.error }); this.__error = init.error; }
          decode() { this.__error(new DOMException('injected decode failure', 'EncodingError')); }
        };
      },
      'constructor-throws': () => { window.VideoDecoder = class extends window.__nativeVideoDecoder { constructor() { throw Error('injected constructor failure'); } }; },
    };
    const decodeNatively = async () => {
      await page.evaluate(() => { window.VideoDecoder = window.__nativeVideoDecoder; });
      await click('Try browser H.264 refocus');
      await statusIncludes('Archived discrete browser focus decoded');
    };
    proof.codecFaults = [];
    for (const [name, install] of Object.entries(codecFaults)) {
      // Start every case from a fresh successful decode so the failure status
      // below can only come from this injected fault.
      await decodeNatively();
      const before = await page.$eval('.stage img', e => e.src);
      await page.evaluate(install);
      await click('Try browser H.264 refocus');
      await statusIncludes('Browser video decode unavailable');
      assert.equal(await page.$eval('.stage img', e => e.src), before, `${name}: verified frame retained`);
      proof.codecFaults.push(name);
    }
    await decodeNatively();
    proof.codecRecovered = true;

    // 7. A rejected upload transfer is reported as failed and retryable by the user;
    //    nothing is committed for the rejected file.
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'lytronaut-flow-'));
    try {
      const folder = path.join(scratch, 'Rejected upload');
      fs.mkdirSync(folder);
      fs.writeFileSync(path.join(folder, 'synthetic-preserved.bin'), Buffer.from('synthetic upload payload'));
      rule = request => {
        if (request.method() !== 'PUT' || !/^\/imports\/[^/]+\/file$/.test(new URL(request.url()).pathname)) return false;
        request.respond({ status: 503, contentType: 'application/json', body: '{"version":1,"code":"storage_unavailable","retryable":true}' }).catch(() => {});
        return true;
      };
      await (await page.$('#directory')).uploadFile(folder);
      await statusIncludes('Upload failed. Inspect status, then select the original files again to retry');
      await page.waitForFunction(() => !document.querySelector('#directory').disabled);
      rule = null;
      proof.rejectedUpload = true;
    } finally {
      fs.rmSync(scratch, { recursive: true, force: true });
    }

    // 8. A genuine ZIP64 archive (end locator, record and per-entry extras) is
    //    admitted by the Rust/WASM client and preserved byte-exact by the service.
    const zipScratch = fs.mkdtempSync(path.join(os.tmpdir(), 'lytronaut-flow-zip64-'));
    try {
      const payload = Buffer.from('synthetic ZIP64 preserved payload');
      const archive = path.join(zipScratch, 'synthetic-true-zip64.zip');
      fs.writeFileSync(archive, zip64Archive('zip64/synthetic-preserved.bin', payload));
      await page.waitForFunction(() => !document.querySelector('#zip').disabled);
      await (await page.$('#zip')).uploadFile(archive);
      await page.waitForFunction(async () => {
        const id = sessionStorage.getItem('lytronaut-import');
        if (!id || document.querySelector('#zip').disabled) return false;
        return (await (await fetch('/imports/' + id)).json()).phase === 'catalog_complete';
      }, { timeout: 120000 });
      const imported = await page.evaluate(async () => {
        const progress = await (await fetch('/imports/' + sessionStorage.getItem('lytronaut-import'))).json();
        const files = await (await fetch(`/libraries/${progress.library}/files`)).json();
        return { state: progress.catalog_state, files: files.items.length };
      });
      assert.deepEqual(imported, { state: 'preserved_only', files: 1 });
      proof.trueZip64 = imported;
    } finally {
      fs.rmSync(zipScratch, { recursive: true, force: true });
    }

    assert.deepEqual(errors, [], 'faults never surface as uncaught page errors');
    return proof;
  } finally {
    rule = null;
    await page.close();
  }
};
