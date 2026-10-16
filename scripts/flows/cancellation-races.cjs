// MIT public synthetic browser flow (test tooling only; behaviour under test is
// the Rust/WASM client). Holds one in-flight response at each stage of loading a
// capture (detail, preview manifest, preview chunk, focus derivative), moves the
// user to another capture, then releases the stale response. The newer
// selection must win: the late response never replaces it or its status.
'use strict';
const assert = require('node:assert/strict');

module.exports = async ({ browser, origin }) => {
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    Object.defineProperty(window, 'VideoDecoder', { value: undefined, writable: true });
  });
  let hold = null;
  let held = null;
  await page.setRequestInterception(true);
  page.on('request', request => {
    if (!held && hold && request.method() === 'GET' && new URL(request.url()).pathname === hold) {
      held = request;
      return;
    }
    request.continue().catch(() => {});
  });
  const click = text => page.waitForFunction(text => {
    const button = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === text && !b.disabled);
    if (!button) return false;
    button.click();
    return true;
  }, { polling: 50 }, text);
  // Find a capture through the real search control, then select it.
  const pick = async name => {
    await page.$eval('#search', (e, name) => { e.value = name; e.dispatchEvent(new Event('input', { bubbles: true })); }, name);
    await click('Search');
    await page.waitForFunction(name => [...document.querySelectorAll('.capture span')].some(s => s.textContent === name), {}, name);
    return page.evaluate(name => {
      const button = [...document.querySelectorAll('.capture')].find(b => b.querySelector('span')?.textContent === name);
      if (!button) return false;
      button.click();
      return true;
    }, name);
  };
  const settled = name => page.waitForFunction(name => document.querySelector('.viewer-strip span')?.textContent === name
    && document.querySelector('.stage img')?.complete && document.querySelector('.stage img')?.naturalWidth === 640
    && (document.querySelector('.status')?.textContent || '').includes('Static JPEG preview ready'), {}, name);
  const waitHeld = async () => {
    for (let i = 0; i < 500 && !held; i++) await new Promise(r => setTimeout(r, 10));
    assert(held, `request ${hold} was issued and held`);
  };
  const release = async () => {
    const late = held;
    held = null;
    hold = null;
    await late.continue().catch(() => {});
    await new Promise(r => setTimeout(r, 300));
  };
  const proof = {};
  try {
    await page.goto(origin);
    await page.waitForSelector('#directory');
    await click('All captures');
    await page.waitForFunction(() => document.querySelectorAll('.capture').length > 0, { timeout: 120000 });
    const ids = await page.evaluate(async () => {
      const library = (await (await fetch('/libraries')).json()).items.find(l => l.state === 'ready');
      const captures = (await (await fetch(`/libraries/${library.id}/captures?limit=100`)).json()).items;
      const out = {};
      for (const name of ['Synthetic 000', 'Synthetic 001']) {
        const capture = captures.find(c => c.name === name).id;
        const detail = await (await fetch(`/captures/${capture}`)).json();
        out[name] = { capture, preview: detail.preview, stack: detail.originals.find(f => f.role === 'jpeg_focus_stack')?.id };
      }
      return out;
    });
    const stale = ids['Synthetic 001'];
    proof.staleStages = [];
    for (const [stage, path] of [
      ['detail', `/captures/${stale.capture}`],
      ['preview-manifest', `/assets/${stale.preview}/manifest`],
      ['preview-chunk', `/assets/${stale.preview}/chunks/0`],
    ]) {
      assert(await pick('Synthetic 002'));
      await settled('Synthetic 002');
      hold = path;
      assert(await pick('Synthetic 001'));
      await waitHeld();
      // The user moves on before the stale response arrives.
      assert(await pick('Synthetic 002'));
      await settled('Synthetic 002');
      await release();
      await settled('Synthetic 002');
      proof.staleStages.push(stage);
    }

    // A held focus derivative is superseded by selecting a static capture.
    const focus = ids['Synthetic 000'];
    assert(await pick('Synthetic 000'));
    await page.waitForFunction(() => !document.querySelector('#focus').disabled && (document.querySelector('.status')?.textContent || '').includes('Discrete focus:'));
    hold = `/assets/${focus.stack}/focus/2/1`;
    await page.$eval('#focus', e => { e.value = e.max; e.dispatchEvent(new Event('input', { bubbles: true })); });
    await waitHeld();
    assert(await pick('Synthetic 002'));
    await settled('Synthetic 002');
    await release();
    await settled('Synthetic 002');
    assert(await page.$eval('#focus', e => e.disabled), 'static capture keeps focus disabled');
    proof.staleStages.push('focus-derivative');

    // A newer search supersedes a capture listing that is still in flight.
    const library = await page.evaluate(async () => (await (await fetch('/libraries')).json()).items.find(l => l.state === 'ready').id);
    const searchFor = text => page.$eval('#search', (e, text) => { e.value = text; e.dispatchEvent(new Event('input', { bubbles: true })); }, text);
    hold = `/libraries/${library}/captures`;
    await searchFor('Synthetic 01');
    await click('Search');
    await waitHeld();
    await searchFor('Synthetic 02');
    await click('Search');
    const newer = () => page.waitForFunction(() => {
      const names = [...document.querySelectorAll('.capture span')].map(s => s.textContent);
      return names.length === 10 && names.every(n => n.startsWith('Synthetic 02'));
    });
    await newer();
    await release();
    await newer();
    proof.staleStages.push('capture-listing');

    assert.deepEqual(errors, [], 'stale responses never surface as uncaught page errors');
    return proof;
  } finally {
    if (held) await held.continue().catch(() => {});
    await page.close();
  }
};
