// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Tampers with genuine service responses for discrete
// focus derivatives, focus stacks, video originals, export job reports and
// restore input, and asserts that the client rejects them with a specific
// message while the last verified frame stays displayed.
'use strict';
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    window.__nativeVideoDecoder = window.VideoDecoder;
  });
  let rule = null;
  await page.setRequestInterception(true);
  page.on('request', request => {
    if (!(rule && rule(request))) request.continue().catch(() => {});
  });
  // Serve a tampered copy of the genuine response for one exact path.
  const tamper = (pathname, edit) => {
    rule = request => {
      if (new URL(request.url()).pathname !== pathname) return false;
      (async () => {
        const genuine = await fetch(request.url(), { signal: AbortSignal.timeout(10000) });
        const headers = Object.fromEntries(genuine.headers);
        const body = Buffer.from(await genuine.arrayBuffer());
        const changed = edit({ status: genuine.status, headers, body });
        // The body may change length; let the browser frame it.
        for (const name of ['content-length', 'content-encoding', 'transfer-encoding', 'connection']) delete changed.headers[name];
        await request.respond(changed);
      })().catch(() => request.abort().catch(() => {}));
      return true;
    };
  };
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
  const proof = {};
  try {
    await page.goto(origin);
    await page.waitForSelector('#directory');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0, { timeout: 120000 });
    const ids = await page.evaluate(async () => {
      const library = (await (await fetch('/libraries')).json()).items.find(l => l.state === 'ready');
      const captures = (await (await fetch(`/libraries/${library.id}/captures?limit=100`)).json()).items;
      const detail = await (await fetch(`/captures/${captures.find(c => c.name === 'Synthetic 000').id}`)).json();
      const stack = detail.originals.find(f => f.role === 'jpeg_focus_stack').id;
      return {
        lambdas: (await (await fetch(`/assets/${stack}/focus`)).json()).lambdas,
        stack,
        video: detail.originals.find(f => f.role === 'refocus_stack').id,
      };
    });

    // 1. Tampered focus derivatives: header problems and undecodable but
    //    integrity-consistent bytes are each rejected with their own message.
    await selectNamed('Synthetic 000');
    await focusReady();
    const derivativeFaults = [
      ['missing-metadata', r => { delete r.headers['x-lytronaut-derivative']; return r; }, 'Derivative metadata missing'],
      ['malformed-metadata', r => { r.headers['x-lytronaut-derivative'] = '{"not":"metadata"'; return r; }, 'Invalid derivative metadata'],
      ['inconsistent-provenance', r => { const m = JSON.parse(r.headers['x-lytronaut-derivative']); m.plane_index = (m.plane_index + 1) % 3; r.headers['x-lytronaut-derivative'] = JSON.stringify(m); return r; }, 'Invalid derivative extent or provenance'],
      // A self-consistent derivative for a different plane than the one requested.
      ['other-plane', r => { const m = JSON.parse(r.headers['x-lytronaut-derivative']); m.plane_index = 1; m.lambda = ids.lambdas[1]; r.headers['x-lytronaut-derivative'] = JSON.stringify(m); return r; }, 'Derivative selection mismatch'],
      ['undecodable', r => {
        const m = JSON.parse(r.headers['x-lytronaut-derivative']);
        const body = Buffer.alloc(r.body.length, 0x5a);
        m.content_sha256 = crypto.createHash('sha256').update(body).digest('hex');
        r.headers['x-lytronaut-derivative'] = JSON.stringify(m);
        return { ...r, body };
      }, 'Derivative decode or geometry failed'],
    ];
    proof.derivativeFaults = [];
    for (const [name, edit, message] of derivativeFaults) {
      const before = await page.$eval('.stage img', e => e.src);
      const target = await page.$eval('#focus', e => Number(e.value) === Number(e.max) ? e.min : e.max);
      tamper(`/assets/${ids.stack}/focus/${target === (await page.$eval('#focus', e => e.min)) ? 0 : 2}/1`, edit);
      await page.$eval('#focus', (e, v) => { e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }, target);
      await statusIncludes(message);
      assert.equal(await page.$eval('.stage img', e => e.src), before, `${name}: verified frame retained`);
      rule = null;
      // Restore a verified plane before the next case.
      await page.$eval('#focus', (e, v) => { e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }, target);
      await focusReady();
      proof.derivativeFaults.push(name);
    }

    // 2. A focus stack naming a different asset disables discrete focus only.
    tamper(`/assets/${ids.stack}/focus`, r => {
      const stack = JSON.parse(r.body);
      stack.id = 'someone-elses-stack';
      return { ...r, body: JSON.stringify(stack) };
    });
    await selectNamed('Synthetic 001');
    await selectNamed('Synthetic 000');
    await statusIncludes('Discrete focus unavailable. Static preview and originals remain usable');
    assert(await page.$eval('.stage img', e => e.complete && e.naturalWidth > 0), 'static preview stays usable');
    rule = null;
    proof.foreignFocusStack = true;

    // 3. A video original manifest that names another asset is refused before download.
    await selectNamed('Synthetic 001');
    await selectNamed('Synthetic 000');
    await focusReady();
    await page.evaluate(() => { window.VideoDecoder = window.__nativeVideoDecoder; });
    tamper(`/assets/${ids.video}/manifest`, r => {
      const manifest = JSON.parse(r.body);
      manifest.encoded_bytes += 1;
      return { ...r, body: JSON.stringify(manifest) };
    });
    let downloads = 0;
    const countDownloads = request => { if (new URL(request.url()).pathname === `/assets/${ids.video}/download`) downloads++; };
    page.on('request', countDownloads);
    await click('Try browser H.264 refocus');
    await statusIncludes('Video original identity mismatch');
    page.off('request', countDownloads);
    assert.equal(downloads, 0, 'mismatched manifest stops before the original is fetched');
    rule = null;
    proof.videoManifestMismatch = true;

    // 4. Terminal export reports are shown as failed or cancelled, never as
    //    success. Creation is untouched; the genuine job's status poll reports
    //    the injected terminal state.
    await selectNamed('Synthetic 002');
    await page.waitForFunction(() => document.querySelector('.stage img')?.naturalWidth === 640);
    proof.exportStates = [];
    for (const [state, message] of [['failed', 'Export / restore failed'], ['cancelled', 'Export / restore cancelled. Originals are unchanged.']]) {
      rule = request => {
        if (request.method() !== 'GET' || !/^\/exports\/[^/]+$/.test(new URL(request.url()).pathname)) return false;
        (async () => {
          const report = await (await fetch(request.url(), { signal: AbortSignal.timeout(10000) })).json();
          report.state = state;
          report.error = state === 'failed' ? { version: 1, code: 'storage_unavailable', retryable: false } : null;
          await request.respond({ status: 200, contentType: 'application/json', body: JSON.stringify(report) });
        })().catch(() => request.abort().catch(() => {}));
        return true;
      };
      await click('Export JPEG');
      await statusIncludes(message);
      rule = null;
      proof.exportStates.push(state);
    }

    // 5. An empty bundle is refused by the client before any upload.
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'lytronaut-flow-restore-'));
    try {
      const empty = path.join(scratch, 'empty.lytronaut');
      fs.writeFileSync(empty, Buffer.alloc(0));
      let restores = 0;
      const countRestores = request => { if (request.method() === 'POST' && new URL(request.url()).pathname.startsWith('/exports')) restores++; };
      page.on('request', countRestores);
      await page.waitForFunction(() => !document.querySelector('#restore-bundle').disabled);
      await (await page.$('#restore-bundle')).uploadFile(empty);
      await statusIncludes('The selected bundle is empty');
      page.off('request', countRestores);
      assert.equal(restores, 0);
      proof.emptyBundleRefused = true;
    } finally {
      fs.rmSync(scratch, { recursive: true, force: true });
    }

    // 6. Recovering an export with nothing saved is an explicit message.
    await page.evaluate(() => sessionStorage.removeItem('lytronaut-export'));
    await click('Recover export / restore status');
    await statusIncludes('No saved export or restore job');
    proof.noSavedExport = true;

    assert.deepEqual(errors, [], 'tampered responses never surface as uncaught page errors');
    return proof;
  } finally {
    rule = null;
    await page.close();
  }
};
