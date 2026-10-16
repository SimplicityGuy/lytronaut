// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Makes individual browser rendering and download APIs
// fail while the client draws grid thumbnails, shows discrete focus
// derivatives, decodes archived H.264 focus and builds a Live Photo download.
// Each failure must be contained: a placeholder thumbnail, a specific status
// message, the last verified frame kept, nothing half-downloaded, and full
// recovery once the API works again.
'use strict';
const assert = require('node:assert/strict');

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    window.__restore = [];
    // Shadow one property (own or inherited) until window.__undo() restores it.
    window.__patch = (owner, key, descriptor) => {
      let holder = owner;
      let original;
      while (holder && !(original = Object.getOwnPropertyDescriptor(holder, key))) holder = Object.getPrototypeOf(holder);
      const own = holder === owner;
      Object.defineProperty(owner, key, { configurable: true, enumerable: original ? original.enumerable : true, ...descriptor });
      window.__restore.push(() => { if (own) Object.defineProperty(owner, key, original); else delete owner[key]; });
      return original;
    };
    window.__break = (owner, key, value) => window.__patch(owner, key, { value, writable: true });
    // Fail only from the nth call onwards; earlier calls use the real member.
    window.__breakFrom = (owner, key, nth, make) => {
      let calls = 0;
      const original = owner[key];
      window.__break(owner, key, function (...args) {
        if (++calls >= nth) return make.apply(this, [original, ...args]);
        return new.target ? Reflect.construct(original, args, new.target) : original.apply(this, args);
      });
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
  const search = async text => {
    await page.$eval('#search', (e, text) => { e.value = text; e.dispatchEvent(new Event('input', { bubbles: true })); }, text);
    await click('Search');
  };
  const pick = async name => {
    await search(name);
    await page.waitForFunction(name => [...document.querySelectorAll('.capture span')].some(s => s.textContent === name), {}, name);
    await page.evaluate(name => [...document.querySelectorAll('.capture')].find(b => b.querySelector('span').textContent === name).click(), name);
    await page.waitForFunction(name => document.querySelector('.viewer-strip span')?.textContent === name, {}, name);
  };
  const only = 'Synthetic 001';
  // The single matching capture shows a drawn thumbnail / the placeholder.
  const thumbnail = drawn => page.waitForFunction((name, drawn) => {
    const cards = [...document.querySelectorAll('.capture')];
    if (cards.length !== 1 || cards[0].querySelector('span')?.textContent !== name) return false;
    const image = cards[0].querySelector('img');
    return drawn ? !!image && image.complete && image.naturalWidth > 0 : !image && !!cards[0].querySelector('.thumb-empty');
  }, {}, only, drawn);
  const focusReady = () => page.waitForFunction(() => !document.querySelector('#focus').disabled && (document.querySelector('.status')?.textContent || '').includes('Discrete focus:'));
  const setFocus = edge => page.$eval('#focus', (e, edge) => { e.value = edge === 'max' ? e.max : e.min; e.dispatchEvent(new Event('input', { bubbles: true })); }, edge);
  const proof = {};
  try {
    await page.goto(origin);
    await page.waitForSelector('#directory');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0, { timeout: 120000 });

    // 1. Grid thumbnails: each failing canvas/Blob step leaves a placeholder,
    //    and the next search draws the real thumbnail again. Every search
    //    clears the grid first, so wait until the injected failure has actually
    //    fired before checking that no thumbnail was drawn.
    proof.thumbnailFaults = [];
    const oversized = 'data:image/jpeg;base64,' + 'A'.repeat(140 * 1024);
    for (const [name, breakIt] of [
      ['canvas-create-throws', () => { const create = Document.prototype.createElement; window.__break(Document.prototype, 'createElement', function (tag, ...rest) { if (String(tag).toLowerCase() === 'canvas') { window.__fired++; throw Error('injected'); } return create.call(this, tag, ...rest); }); }],
      ['canvas-create-wrong-element', () => { const create = Document.prototype.createElement; window.__break(Document.prototype, 'createElement', function (tag, ...rest) { if (String(tag).toLowerCase() !== 'canvas') return create.call(this, tag, ...rest); window.__fired++; return create.call(this, 'div'); }); }],
      ['context-wrong-type', () => window.__break(HTMLCanvasElement.prototype, 'getContext', () => { window.__fired++; return {}; })],
      ['draw-throws', () => window.__break(CanvasRenderingContext2D.prototype, 'drawImage', () => { window.__fired++; throw Error('injected'); })],
      ['encode-throws', () => window.__break(HTMLCanvasElement.prototype, 'toDataURL', () => { window.__fired++; throw Error('injected'); })],
      ['encode-oversized', () => window.__break(HTMLCanvasElement.prototype, 'toDataURL', () => { window.__fired++; return window.__oversized; })],
      ['decode-rejects', () => window.__break(HTMLImageElement.prototype, 'decode', () => { window.__fired++; return Promise.reject(new DOMException('injected', 'EncodingError')); })],
      ['preview-blob-throws', () => window.__breakFrom(window, 'Blob', 1, () => { window.__fired++; throw Error('injected'); })],
      ['display-blob-throws', () => window.__breakFrom(window, 'Blob', 2, () => { window.__fired++; throw Error('injected'); })],
    ]) {
      await search(only);
      await thumbnail(true);
      await page.evaluate(oversized => { window.__oversized = oversized; window.__fired = 0; }, oversized);
      await page.evaluate(breakIt);
      await search(only);
      await page.waitForFunction(() => window.__fired > 0);
      await new Promise(r => setTimeout(r, 300));
      await thumbnail(false);
      await page.evaluate(() => window.__undo());
      proof.thumbnailFaults.push(name);
    }
    await search(only);
    await thumbnail(true);

    // 2. Discrete focus derivatives: each failing step is reported and the
    //    verified frame stays displayed.
    await pick('Synthetic 000');
    await focusReady();
    proof.derivativeFaults = [];
    for (const [name, breakIt, message] of [
      ['metadata-read-throws', () => { const get = Headers.prototype.get; window.__break(Headers.prototype, 'get', function (key) { if (String(key).toLowerCase() === 'x-lytronaut-derivative') throw Error('injected'); return get.call(this, key); }); }, 'Derivative metadata unavailable'],
      ['metadata-oversized', () => { const get = Headers.prototype.get; window.__break(Headers.prototype, 'get', function (key) { return String(key).toLowerCase() === 'x-lytronaut-derivative' ? 'x'.repeat(256 * 1024 + 1) : get.call(this, key); }); }, 'Derivative metadata exceeds budget'],
      ['stream-reader-wrong-type', () => window.__break(ReadableStream.prototype, 'getReader', () => ({})), 'Response stream unavailable'],
      ['blob-throws', () => window.__break(window, 'Blob', function () { throw Error('injected'); }), 'Derivative unavailable'],
      ['object-url-throws', () => window.__break(URL, 'createObjectURL', () => { throw Error('injected'); }), 'Derivative unavailable'],
      ['image-throws', () => window.__break(window, 'Image', function () { throw Error('injected'); }), 'Derivative decode unavailable'],
    ]) {
      const before = await page.$eval('.stage img', e => e.src);
      const edge = await page.$eval('#focus', e => Number(e.value) === Number(e.max) ? 'min' : 'max');
      await page.evaluate(breakIt);
      await setFocus(edge);
      await statusIncludes(message);
      assert.equal(await page.$eval('.stage img', e => e.src), before, `${name}: verified frame retained`);
      await page.evaluate(() => window.__undo());
      // Request the same plane again now that the API works.
      await setFocus(edge);
      await focusReady();
      proof.derivativeFaults.push(name);
    }

    // 3. Archived H.264 focus: platform failures at every decoder and canvas
    //    step fall back to the verified JPEG frame with one message.
    const fallback = 'Browser video decode unavailable. Verified JPEG/static preview retained';
    proof.videoFaults = [];
    for (const [name, breakIt] of [
      ['decoder-lookup-throws', () => { let reads = 0; const real = window.VideoDecoder; window.__patch(window, 'VideoDecoder', { get() { if (++reads >= 2) throw Error('injected'); return real; } }); }],
      ['support-lookup-throws', () => window.__patch(VideoDecoder, 'isConfigSupported', { get() { throw Error('injected'); } })],
      ['support-query-throws', () => window.__break(VideoDecoder, 'isConfigSupported', () => { throw Error('injected'); })],
      ['support-query-not-callable', () => window.__break(VideoDecoder, 'isConfigSupported', 1)],
      ['support-query-not-a-promise', () => window.__break(VideoDecoder, 'isConfigSupported', () => ({ supported: true }))],
      ['frame-canvas-throws', () => { const create = Document.prototype.createElement; window.__break(Document.prototype, 'createElement', function (tag, ...rest) { if (String(tag).toLowerCase() === 'canvas') throw Error('injected'); return create.call(this, tag, ...rest); }); }],
      ['frame-canvas-wrong-element', () => { const create = Document.prototype.createElement; window.__break(Document.prototype, 'createElement', function (tag, ...rest) { return create.call(this, String(tag).toLowerCase() === 'canvas' ? 'div' : tag, ...rest); }); }],
      ['frame-context-throws', () => window.__break(HTMLCanvasElement.prototype, 'getContext', () => { throw Error('injected'); })],
      ['frame-context-wrong-type', () => window.__break(HTMLCanvasElement.prototype, 'getContext', () => ({}))],
      ['frame-draw-throws', () => window.__break(CanvasRenderingContext2D.prototype, 'drawImage', () => { throw Error('injected'); })],
      ['frame-encode-throws', () => window.__break(HTMLCanvasElement.prototype, 'toDataURL', () => { throw Error('injected'); })],
    ]) {
      await page.evaluate(breakIt);
      await click('Try browser H.264 refocus');
      await statusIncludes(fallback);
      await page.evaluate(() => window.__undo());
      // Re-select so the next attempt starts from the verified JPEG stack.
      await pick('Synthetic 002');
      await pick('Synthetic 000');
      await focusReady();
      proof.videoFaults.push(name);
    }
    await click('Try browser H.264 refocus');
    await statusIncludes('Archived discrete browser focus decoded');

    // 4. Live Photo pair: a missing or malformed secure identifier, a failing
    //    pixel/JPEG step and a failing download step each end with a specific
    //    message and no download; the next export succeeds.
    proof.livePhotoFaults = [];
    let downloads = 0;
    await page.exposeFunction('__countDownload', () => { downloads++; });
    await page.evaluate(() => {
      const click = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function () { if (this.download) { window.__countDownload(); return; } return click.call(this); };
    });
    const secureMessage = 'Secure random export identifier unavailable';
    const encodeMessage = 'Use verified JPEG/static still export';
    for (const [name, breakIt, message] of [
      ['crypto-lookup-throws', () => window.__patch(window, 'crypto', { get() { throw Error('injected'); } }), secureMessage],
      ['uuid-lookup-throws', () => window.__patch(Crypto.prototype, 'randomUUID', { get() { throw Error('injected'); } }), secureMessage],
      ['uuid-throws', () => window.__break(Crypto.prototype, 'randomUUID', () => { throw Error('injected'); }), secureMessage],
      ['uuid-malformed', () => window.__break(Crypto.prototype, 'randomUUID', () => 'not-an-identifier'), 'invalid export identifier'],
      ['key-jpeg-unreadable', () => window.__break(window, 'atob', () => { throw Error('injected'); }), encodeMessage],
      ['pixels-unreadable', () => window.__break(CanvasRenderingContext2D.prototype, 'getImageData', () => { throw Error('injected'); }), encodeMessage],
      ['download-blob-throws', () => window.__break(window, 'Blob', function () { throw Error('injected'); }), 'Complete download unavailable'],
      ['download-url-throws', () => window.__break(URL, 'createObjectURL', () => { throw Error('injected'); }), 'Complete download unavailable'],
      ['download-anchor-throws', () => { const create = Document.prototype.createElement; window.__break(Document.prototype, 'createElement', function (tag, ...rest) { if (String(tag).toLowerCase() === 'a') throw Error('injected'); return create.call(this, tag, ...rest); }); }, 'Download unavailable'],
      ['download-anchor-wrong-element', () => { const create = Document.prototype.createElement; window.__break(Document.prototype, 'createElement', function (tag, ...rest) { return create.call(this, String(tag).toLowerCase() === 'a' ? 'span' : tag, ...rest); }); }, 'Download unavailable'],
    ]) {
      await page.evaluate(breakIt);
      await click('Export Live Photo pair (.zip)');
      await statusIncludes(message);
      await page.evaluate(() => window.__undo());
      assert.equal(downloads, 0, `${name}: nothing is offered for download`);
      proof.livePhotoFaults.push(name);
    }
    await click('Export Live Photo pair (.zip)');
    await statusIncludes('Complete Live Photo ZIP downloaded');
    assert.equal(downloads, 1, 'the recovered export is offered once');
    proof.livePhotoRecovered = true;

    assert.deepEqual(errors, [], 'API failures never surface as uncaught page errors');
    return proof;
  } finally {
    await page.close();
  }
};
