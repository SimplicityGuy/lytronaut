// MIT independently authored public synthetic acceptance; production is Rust/WASM.
// LYTRONAUT_LIVE_PHOTO_OUT is optional, public synthetic complete ZIP only.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const pp = require('./browser/node_modules/puppeteer-core');
const dist = path.resolve(process.env.LYTRONAUT_CLIENT || 'dist');
const fixture = Buffer.from(JSON.parse(fs.readFileSync(path.join(__dirname, '../fixtures/public/avc-colors.json'))).container_base64, 'base64');
const scratch = fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'lytronaut-live-photo-'));
fs.chmodSync(scratch, 0o700);
const server = http.createServer((req, res) => {
  if (req.url === '/') { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><title>Rust Live Photo acceptance</title>'); return; }
  if (req.url === '/synthetic') { res.end(fixture); return; }
  const name = req.url.slice(1);
  if (!['lytronaut_client.js', 'lytronaut_client_bg.wasm'].includes(name)) { res.statusCode = 404; res.end(); return; }
  res.setHeader('Content-Type', name.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
  fs.createReadStream(path.join(dist, name)).on('error', () => { res.statusCode = 500; res.end(); }).pipe(res);
});
(async () => {
  let browser;
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    browser = await pp.launch({ executablePath: process.env.LYTRONAUT_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true, userDataDir: path.join(scratch, 'profile') });
    const page = await browser.newPage();
    const errors = []; page.on('pageerror', () => errors.push('uncaught browser error'));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    const result = await page.evaluate(async () => {
      const module = await import('/lytronaut_client.js'); const wasm = await module.default(); const initialWasmBytes = wasm.memory.buffer.byteLength;
      const bytes = new Uint8Array(await (await fetch('/synthetic')).arrayBuffer());
      const identifier = 'A8A8A8A8-1111-4444-8888-121212121212';
      const invoke = (source, selected, signal = new AbortController().signal) => module.export_archived_live_photo(source, selected, identifier, signal);
      const reject = async operation => { let failed = false; try { await operation(); } catch { failed = true; } if (!failed) throw Error('unsupported export accepted'); };
      let outputs = 0, closed = 0, queue = 0, decoderCreated = 0, decoderClosed = 0, maxImageDataBytes = 0, authoredFrames = 0;
      const getPixels = CanvasRenderingContext2D.prototype.getImageData;
      CanvasRenderingContext2D.prototype.getImageData = function(x,y,width,height,...rest) { maxImageDataBytes = Math.max(maxImageDataBytes, width * height * 4); return getPixels.call(this,x,y,width,height,...rest); };
      const Native = VideoDecoder, close = VideoFrame.prototype.close;
      VideoFrame.prototype.close = function () { closed++; return close.call(this); };
      class Tracked extends Native {
        constructor(init) { super({ ...init, output: frame => { outputs++; init.output(frame); } }); decoderCreated++; this.closedByHarness=false; }
        close() { if(!this.closedByHarness) { decoderClosed++; this.closedByHarness=true; } return super.close(); }
        decode(chunk) { super.decode(chunk); queue = Math.max(queue, this.decodeQueueSize); if (queue > 2) throw Error('unbounded decoder queue'); }
      }
      window.VideoDecoder = Tracked;
      const zip = await invoke(bytes, 0);
      if (zip.length > 18 * 1024 * 1024) throw Error('bundle budget');
      const entries = {}; const view = new DataView(zip.buffer, zip.byteOffset, zip.byteLength);
      let offset = 0;
      while (view.getUint32(offset, true) === 0x04034b50) {
        if (view.getUint16(offset + 8, true) !== 0) throw Error('unexpected ZIP compression');
        const size = view.getUint32(offset + 18, true), nameLength = view.getUint16(offset + 26, true), extra = view.getUint16(offset + 28, true);
        const name = new TextDecoder().decode(zip.slice(offset + 30, offset + 30 + nameLength));
        const start = offset + 30 + nameLength + extra; entries[name] = zip.slice(start, start + size); offset = start + size;
      }
      if (Object.keys(entries).sort().join(',') !== 'focus.JPG,focus.MOV,provenance.json') throw Error('incomplete pair');
      const provenance = JSON.parse(new TextDecoder().decode(entries['provenance.json']));
      if (provenance.motion !== 'generated_focus_animation' || provenance.selected_stored_plane !== 0 || provenance.key_seconds !== 0.6 || provenance.sweep_stored_planes.join(',') !== '2,0,3,4,1') throw Error('selected key/order/provenance mismatch');
      for (const resource of ['focus.JPG', 'focus.MOV']) if (!new TextDecoder().decode(entries[resource]).includes(identifier)) throw Error('pairing identifier missing');
      const url = URL.createObjectURL(new Blob([entries['focus.JPG']], { type: 'image/jpeg' }));
      try {
        const image = new Image(); image.src = url; await image.decode();
        if (image.width !== 32 || image.height !== 32) throw Error('still geometry');
        const canvas = document.createElement('canvas'); canvas.width = 32; canvas.height = 32;
        const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
        const rgb = context.getImageData(16, 16, 1, 1).data;
        if (rgb[0] < 200 || rgb[1] > 40 || rgb[2] > 40) throw Error('selected still pixels');
      } finally { URL.revokeObjectURL(url); }
      const aborted = new AbortController(); aborted.abort(); await reject(() => invoke(bytes, 0, aborted.signal));
      await reject(() => invoke(bytes, 63)); await reject(() => invoke(new Uint8Array([1, 2, 3]), 0));
      window.VideoDecoder = undefined; await reject(() => invoke(bytes, 0));
      class Unsupported extends Tracked { static async isConfigSupported() { return { supported: false }; } }
      window.VideoDecoder = Unsupported; await reject(() => invoke(bytes, 0));
      class Retry extends Tracked { configure(config) { this.preference = config.hardwareAcceleration; super.configure(config); } decode(chunk) { if (this.preference === 'no-preference') throw Error('actual configuration cannot decode'); super.decode(chunk); } }
      window.VideoDecoder = Retry; await invoke(bytes, 2);
      class Cancel extends Tracked { decode(chunk) { super.decode(chunk); cancel.abort(); } }
      const cancel = new AbortController(); window.VideoDecoder = Cancel; await reject(() => invoke(bytes, 0, cancel.signal));
      window.VideoDecoder = Tracked; await invoke(bytes, 4);
      // Test tooling only: independently authored maximum-geometry flat colors.
      // The production path never depends on VideoEncoder or this fixture writer.
      const largeCanvas = document.createElement('canvas'); largeCanvas.width = 2048; largeCanvas.height = 1024;
      const context = largeCanvas.getContext('2d'), chunks = []; let encoderFailed = false;
      const encoder = new VideoEncoder({ output: chunk => { const data = new Uint8Array(chunk.byteLength); chunk.copyTo(data); chunks.push(data); }, error: () => encoderFailed = true });
      try {
        encoder.configure({ codec: 'avc1.420032', width: 2048, height: 1024, framerate: 1, latencyMode: 'realtime', hardwareAcceleration: 'prefer-software', avc: { format: 'annexb' } });
        for (let i = 0; i < 5; i++) {
          context.fillStyle = ['red', 'green', 'blue', 'yellow', 'white'][i]; context.fillRect(0, 0, 2048, 1024);
          const frame = new VideoFrame(largeCanvas, { timestamp: i * 1000 });
          try { encoder.encode(frame, { keyFrame: true }); authoredFrames++; } finally { frame.close(); }
        }
        await encoder.flush();
      } finally { encoder.close(); largeCanvas.width = 0; largeCanvas.height = 0; }
      if (encoderFailed || chunks.length !== 5) throw Error('maximum-geometry synthetic fixture failed');
      const stream = new Uint8Array(chunks.reduce((n, chunk) => n + chunk.length + 6, 0)); let position = 0;
      for (const chunk of chunks) { stream.set([0, 0, 0, 1, 9, 0xf0], position); position += 6; stream.set(chunk, position); position += chunk.length; }
      chunks.length = 0;
      const sha = async data => 'sha1-' + [...new Uint8Array(await crypto.subtle.digest('SHA-1', data))].map(v => v.toString(16).padStart(2, '0')).join('');
      const directory = new TextEncoder().encode(JSON.stringify({ version: { major: 1, minor: 0 }, picture: { accelerationArray: [{ type: 'com.lytro.acceleration.refocusStack', vendorContent: { blockOfImages: { representation: 'h264', blockOfImagesRef: await sha(stream), metadataArray: [0,3,-2,1,2].map(lambda => ({ width: 2048, height: 1024, lambda })) } } }] } }));
      const parts = [new Uint8Array([137,76,70,80,13,10,26,10,0,0,0,1,0,0,0,0])]; let total = 16;
      for (const [isDirectory, data] of [[true, directory], [false, stream]]) {
        const header = new Uint8Array(96); header.set([137,76,70,isDirectory?77:67,13,10,26,10]); new DataView(header.buffer).setUint32(12, data.length); header.set(new TextEncoder().encode(await sha(data)), 16);
        parts.push(header, data); total += 96 + data.length; const padding = (16 - total % 16) % 16; parts.push(new Uint8Array(padding)); total += padding;
      }
      const large = new Uint8Array(total); position = 0; for (const part of parts) { large.set(part, position); position += part.length; }
      const metadata = JSON.parse(await module.decode_archived_refocus(large, 0, new AbortController().signal)); metadata.image = '';
      const largeZip = await invoke(large, 0);
      if (largeZip.length > 18 * 1024 * 1024 || largeZip.length < 8 * 1024 * 1024) throw Error('maximum-geometry resource budget');
      const largeResources = { width: metadata.width, height: metadata.height, codedWidth: metadata.coded_width, codedHeight: metadata.coded_height, referenceFrames: metadata.reference_frames, bidirectionalFrames: metadata.bidirectional_frames, sourceBytes: large.length, bundleBytes: largeZip.length };
      if (largeResources.width !== 2048 || largeResources.height !== 1024) throw Error('maximum visible geometry');
      if (decoderCreated !== decoderClosed) throw Error('decoder leak');
      if (maxImageDataBytes > 128 * 1024) throw Error('full-frame platform pixel copy');
      if (closed !== outputs + authoredFrames) throw Error('decoder frame leak');
      return { largeResources, initialWasmBytes, finalWasmHighWaterBytes:wasm.memory.buffer.byteLength, maxImageDataBytes, decoderCreated, decoderClosed, actualRustWasm: true, completePair: true, selectedStillCorrect: true, softwareRetry: true, cancellation: true, unsupportedFallback: true, maxDecodeQueue: queue, decodedFrames: outputs, closedFrames: closed, authoredFrames, zip: Array.from(zip) };
    });
    if (errors.length) throw Error('uncaught page error');
    if (process.env.LYTRONAUT_LIVE_PHOTO_OUT) fs.writeFileSync(process.env.LYTRONAUT_LIVE_PHOTO_OUT, Buffer.from(result.zip), { mode: 0o600 });
    delete result.zip;
    const report = { ...result, browser: await browser.version(), revision: process.env.LYTRONAUT_TEST_REVISION || 'working-tree', applicationWasmSha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(dist, 'lytronaut_client_bg.wasm'))).digest('hex'), photosGuiAcceptancePending: true };
    fs.writeFileSync(process.env.LYTRONAUT_LIVE_PHOTO_REPORT || path.join(scratch, 'aggregate.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 });
    console.log(JSON.stringify(report));
  } catch (error) { console.error(String(error)); process.exitCode = 1; }
  finally { if (browser) await browser.close(); if (server.listening) await new Promise(resolve => server.close(resolve)); fs.rmSync(path.join(scratch, 'profile'), { recursive: true, force: true }); }
})();
