// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Serves integrity-consistent but hostile archived video
// containers (identity, manifest digest and length all match the served bytes)
// and misbehaving platform decoders. The Rust Annex-B/stack checks must refuse
// hostile containers before any VideoDecoder exists, decoder misbehaviour must
// fall back, and the verified frame must stay displayed throughout.
'use strict';
const assert = require('node:assert/strict');
const crypto = require('node:crypto');

const sha1 = b => 'sha1-' + crypto.createHash('sha1').update(b).digest('hex');
// Independently authored baseline AVC syntax: AUD, SPS (30x30 visible), PPS, IDR.
const BASELINE = Buffer.from('0000000109f00000016742001ef44bd54000000168e000000165b800', 'hex');

function container(blocks) {
  const directory = Buffer.from(JSON.stringify({
    version: { major: 1, minor: 0 },
    picture: { accelerationArray: blocks.map(({ payload, frames }) => ({
      type: 'com.lytro.acceleration.refocusStack',
      vendorContent: { blockOfImages: { representation: 'h264', blockOfImagesRef: sha1(payload), metadataArray: frames } },
    })) },
  }));
  const parts = [Buffer.from('\x89LFP\r\n\x1a\n\0\0\0\x01\0\0\0\0', 'latin1')];
  const seen = new Set();
  for (const [isDirectory, payload] of [[true, directory], ...blocks.map(b => [false, b.payload])]) {
    if (seen.has(sha1(payload))) continue;
    seen.add(sha1(payload));
    const header = Buffer.alloc(96);
    header.write(isDirectory ? '\x89LFM\r\n\x1a\n' : '\x89LFC\r\n\x1a\n', 0, 'latin1');
    header.writeUInt32BE(payload.length, 12);
    header.write(sha1(payload), 16, 'latin1');
    parts.push(header, payload, Buffer.alloc((16 - (payload.length % 16)) % 16));
  }
  return Buffer.concat(parts);
}
const frame = (width, height, lambda) => ({ width, height, lambda });
const withProfile = profile => { const b = Buffer.from(BASELINE); b[BASELINE.indexOf(0x67) + 1] = profile; return b; };
const oversizedSps = Buffer.concat([Buffer.from([0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67]), Buffer.alloc(65 * 1024, 0x55)]);

// Each hostile container is rejected by Rust before a decoder is constructed.
const CONTAINERS = [
  ['no-start-code', container([{ payload: Buffer.from('synthetic bytes without Annex-B framing'), frames: [frame(30, 30, 0)] }])],
  ['empty-nal', container([{ payload: Buffer.from([0, 0, 0, 1, 9, 0xf0, 0, 0, 1]), frames: [frame(30, 30, 0)] }])],
  ['declared-geometry-lie', container([{ payload: BASELINE, frames: [frame(32, 32, 0)] }])],
  ['frame-count-lie', container([{ payload: BASELINE, frames: [frame(30, 30, -1), frame(30, 30, 1)] }])],
  ['unsupported-profile', container([{ payload: withProfile(99), frames: [frame(30, 30, 0)] }])],
  ['oversized-parameter-set', container([{ payload: oversizedSps, frames: [frame(30, 30, 0)] }])],
  ['persistent-image-budget', container([{ payload: BASELINE, frames: [frame(2048, 2048, 0)] }])],
  ['split-stack', container([{ payload: BASELINE, frames: [frame(30, 30, -1)] }, { payload: withProfile(77), frames: [frame(30, 30, 1)] }])],
];

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  // Platform-timeout cases wait out the client's 10 s decode bounds.
  page.setDefaultTimeout(90000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    const Native = window.VideoDecoder;
    window.__nativeVideoDecoder = Native;
    window.__decoders = 0;
    window.__countingDecoder = class extends Native { constructor(init) { super(init); window.__decoders++; } };
    window.VideoDecoder = window.__countingDecoder;
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
  const focusReady = () => page.waitForFunction(() => !document.querySelector('#focus').disabled && (document.querySelector('.status')?.textContent || '').includes('Discrete focus:'));
  const reselect = async () => {
    await selectNamed('Synthetic 001');
    await selectNamed('Synthetic 000');
    await focusReady();
  };
  const proof = {};
  try {
    await page.goto(origin);
    await page.waitForSelector('#directory');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0, { timeout: 120000 });
    const ids = await page.evaluate(async () => {
      const library = (await (await fetch('/libraries')).json()).items.find(l => l.state === 'ready');
      const captures = (await (await fetch(`/libraries/${library.id}/captures?limit=100`)).json()).items;
      const capture = captures.find(c => c.name === 'Synthetic 000').id;
      const detail = await (await fetch(`/captures/${capture}`)).json();
      return { capture, video: detail.originals.find(f => f.role === 'refocus_stack').id };
    });

    // 1. Integrity-consistent hostile containers never reach the platform decoder.
    proof.hostileContainers = [];
    for (const [name, bytes] of CONTAINERS) {
      const digest = crypto.createHash('sha256').update(bytes).digest('hex');
      rule = request => {
        const pathname = new URL(request.url()).pathname;
        if (request.method() !== 'GET') return false;
        if (pathname === `/assets/${ids.video}/download`) {
          request.respond({ status: 200, contentType: 'application/octet-stream', body: bytes }).catch(() => {});
          return true;
        }
        if (pathname !== `/assets/${ids.video}/manifest` && pathname !== `/captures/${ids.capture}`) return false;
        (async () => {
          const value = await (await fetch(request.url(), { signal: AbortSignal.timeout(10000) })).json();
          if (pathname.startsWith('/captures/')) {
            value.originals.find(f => f.id === ids.video).encoded_bytes = bytes.length;
          } else {
            value.encoded_bytes = bytes.length;
            value.content_sha256 = digest;
          }
          await request.respond({ status: 200, contentType: 'application/json', body: JSON.stringify(value) });
        })().catch(() => request.abort().catch(() => {}));
        return true;
      };
      await reselect();
      const before = await page.$eval('.stage img', e => e.src);
      const decoders = await page.evaluate(() => window.__decoders);
      await click('Try browser H.264 refocus');
      await statusIncludes('Browser video decode unavailable');
      assert.equal(await page.evaluate(() => window.__decoders), decoders, `${name}: rejected before any decoder`);
      assert.equal(await page.$eval('.stage img', e => e.src), before, `${name}: verified frame retained`);
      rule = null;
      proof.hostileContainers.push(name);
    }

    // 1b. A video original beyond the viewer budget is refused before download,
    //     and a malformed manifest stops it too.
    for (const [name, message, edit] of [
      ['over-budget', 'Video original exceeds viewer budget. Static preview remains available',
        (pathname, value) => { if (pathname.startsWith('/captures/')) value.originals.find(f => f.id === ids.video).encoded_bytes = 8 * 1024 * 1024 + 1; return JSON.stringify(value); }],
      ['malformed-manifest', 'Invalid video original manifest',
        (pathname, value) => pathname.startsWith('/captures/') ? JSON.stringify(value) : '{"not":"a manifest"}'],
    ]) {
      let downloads = 0;
      rule = request => {
        const pathname = new URL(request.url()).pathname;
        if (request.method() !== 'GET') return false;
        if (pathname === `/assets/${ids.video}/download`) { downloads++; return false; }
        if (pathname !== `/assets/${ids.video}/manifest` && pathname !== `/captures/${ids.capture}`) return false;
        (async () => {
          const value = await (await fetch(request.url(), { signal: AbortSignal.timeout(10000) })).json();
          await request.respond({ status: 200, contentType: 'application/json', body: edit(pathname, value) });
        })().catch(() => request.abort().catch(() => {}));
        return true;
      };
      await reselect();
      const before = await page.$eval('.stage img', e => e.src);
      await click('Try browser H.264 refocus');
      await statusIncludes(message);
      assert.equal(downloads, 0, `${name}: original never downloaded`);
      assert.equal(await page.$eval('.stage img', e => e.src), before, `${name}: verified frame retained`);
      rule = null;
      proof.hostileContainers.push(name);
    }

    // 2. Misbehaving platform decoders on the genuine archived video.
    const decoderFaults = {
      'frames-never-delivered': () => {
        window.VideoDecoder = class extends window.__nativeVideoDecoder {
          constructor(init) { super({ output: f => f.close(), error: init.error }); }
        };
      },
      'flush-never-settles': () => {
        window.VideoDecoder = class extends window.__nativeVideoDecoder {
          flush() { return new Promise(() => {}); }
        };
      },
      'queue-never-drains': () => {
        window.VideoDecoder = class extends window.__nativeVideoDecoder {
          get decodeQueueSize() { return 64; }
        };
      },
      'error-while-queue-full': () => {
        window.VideoDecoder = class extends window.__nativeVideoDecoder {
          constructor(init) { super(init); setTimeout(() => init.error(new DOMException('injected', 'EncodingError')), 200); }
          get decodeQueueSize() { return 64; }
        };
      },
    };
    proof.decoderFaults = [];
    for (const [name, install] of Object.entries(decoderFaults)) {
      await reselect();
      await page.evaluate(() => { window.VideoDecoder = window.__countingDecoder; });
      await click('Try browser H.264 refocus');
      await statusIncludes('Archived discrete browser focus decoded');
      const before = await page.$eval('.stage img', e => e.src);
      await page.evaluate(install);
      await click('Try browser H.264 refocus');
      await statusIncludes('Browser video decode unavailable');
      assert.equal(await page.$eval('.stage img', e => e.src), before, `${name}: verified frame retained`);
      proof.decoderFaults.push(name);
    }
    await page.evaluate(() => { window.VideoDecoder = window.__countingDecoder; });
    await click('Try browser H.264 refocus');
    await statusIncludes('Archived discrete browser focus decoded');

    // 3. Browser Live Photo export re-verifies its archived source: a malformed
    //    or foreign manifest and tampered bytes are refused before any pair is
    //    generated, and nothing is offered for download.
    await page.evaluate(() => {
      window.__pairDownloads = 0;
      const original = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function () { if (this.download === 'lytronaut-live-photo.zip') { window.__pairDownloads++; return; } return original.call(this); };
    });
    proof.livePhotoSource = [];
    for (const [name, message, tamper] of [
      ['malformed-manifest', 'Invalid source manifest', (pathname, body) => pathname.endsWith('/manifest') ? '{"not":"a manifest"}' : null],
      ['foreign-manifest', 'Source identity mismatch', (pathname, body) => { if (!pathname.endsWith('/manifest')) return null; const m = JSON.parse(body); m.encoded_bytes += 1; return JSON.stringify(m); }],
      ['tampered-bytes', 'Source integrity failed', (pathname, body) => pathname.endsWith('/download') ? Buffer.from(body).fill(0x5a, 0, 64) : null],
    ]) {
      rule = request => {
        const pathname = new URL(request.url()).pathname;
        if (request.method() !== 'GET' || (pathname !== `/assets/${ids.video}/manifest` && pathname !== `/assets/${ids.video}/download`)) return false;
        (async () => {
          const genuine = await fetch(request.url(), { signal: AbortSignal.timeout(10000) });
          const body = Buffer.from(await genuine.arrayBuffer());
          const changed = tamper(pathname, pathname.endsWith('/manifest') ? body.toString() : body);
          await request.respond({ status: 200, contentType: pathname.endsWith('/manifest') ? 'application/json' : 'application/octet-stream', body: changed ?? body });
        })().catch(() => request.abort().catch(() => {}));
        return true;
      };
      await click('Export Live Photo pair (.zip)');
      await statusIncludes(message);
      rule = null;
      proof.livePhotoSource.push(name);
    }
    // 3b. Integrity-consistent hostile sources of exactly the original's length
    //     (so identity and digest checks pass) are refused by the export's own
    //     stack and Annex-B checks.
    const genuineLength = await page.evaluate(async video => (await (await fetch(`/assets/${video}/manifest`)).json()).encoded_bytes, ids.video);
    // Grow one filler payload until the framed container has the genuine length.
    const sized = build => {
      for (let n = 1; n <= genuineLength; n++) {
        const bytes = build(Buffer.alloc(n, 0x41));
        if (bytes.length === genuineLength) return bytes;
        if (bytes.length > genuineLength) break;
      }
      throw Error('no filler gives the genuine container length');
    };
    for (const [name, bytes, message] of [
      ['split-stack', sized(filler => container([{ payload: filler, frames: [frame(30, 30, -1)] }, { payload: withProfile(77), frames: [frame(30, 30, 1)] }])), 'unsupported video stack'],
      ['no-start-code', sized(filler => container([{ payload: filler, frames: [frame(30, 30, 0)] }])), 'invalid Annex-B stream'],
    ]) {
      const digest = crypto.createHash('sha256').update(bytes).digest('hex');
      rule = request => {
        const pathname = new URL(request.url()).pathname;
        if (request.method() !== 'GET') return false;
        if (pathname === `/assets/${ids.video}/download`) {
          request.respond({ status: 200, contentType: 'application/octet-stream', body: bytes }).catch(() => {});
          return true;
        }
        if (pathname !== `/assets/${ids.video}/manifest`) return false;
        (async () => {
          const value = await (await fetch(request.url(), { signal: AbortSignal.timeout(10000) })).json();
          value.content_sha256 = digest;
          await request.respond({ status: 200, contentType: 'application/json', body: JSON.stringify(value) });
        })().catch(() => request.abort().catch(() => {}));
        return true;
      };
      await click('Export Live Photo pair (.zip)');
      await statusIncludes(message);
      rule = null;
      proof.livePhotoSource.push(name);
    }
    assert.equal(await page.evaluate(() => window.__pairDownloads), 0, 'no half-verified pair is offered');

    assert.deepEqual(errors, [], 'hostile video never surfaces as uncaught page errors');
    return proof;
  } finally {
    rule = null;
    await page.close();
  }
};
