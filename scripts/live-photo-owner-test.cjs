// MIT public synthetic test tooling. Only an explicit test-harness WASM exposes
// real mount disposal; ordinary application artifacts do not expose these hooks.
const assert = require('node:assert/strict');
module.exports = async ({page, click, delay}) => {
  await page.evaluate(async () => {
    // Same URL as assets/index.html: reuse the already initialized module.
    window.__ownerModule = await import('/client/lytronaut_client.js');
    if (['harness_dispose','harness_remount','harness_capacity_start','harness_capacity_take'].some(name=>typeof window.__ownerModule[name]!=='function')) throw Error('test-harness artifact required');
    window.__owner = {created:0, closed:0, outputs:0, framesClosed:0, downloads:0, aborts:0};
    window.__ownerNative = {decoder:VideoDecoder, fetch:window.fetch.bind(window), timer:window.setTimeout.bind(window), anchor:HTMLAnchorElement.prototype.click};
    const close = VideoFrame.prototype.close, abort = AbortController.prototype.abort;
    VideoFrame.prototype.close = function() { window.__owner.framesClosed++; return close.call(this); };
    AbortController.prototype.abort = function(...args) { window.__owner.aborts++; return abort.apply(this,args); };
    HTMLAnchorElement.prototype.click = function() { if(this.download==='lytronaut-live-photo.zip') { window.__owner.downloads++; return; } return window.__ownerNative.anchor.call(this); };
    window.__ownerTracked = class extends window.__ownerNative.decoder {
      constructor(init) {
        const callbacks = {...init, output:frame=>{window.__owner.outputs++;init.output(frame);}};
        super(callbacks);this.ownerClosed=false;window.__owner.created++;window.__ownerCallbacks=callbacks;
      }
      close() { if(!this.ownerClosed) {this.ownerClosed=true;window.__owner.closed++;} return super.close(); }
    };
    window.VideoDecoder = window.__ownerTracked;
  });
  const readyVideo = async () => {
    await page.waitForSelector('#directory');
    await click('All captures');
    await page.waitForFunction(()=>[...document.querySelectorAll('.capture')].some(b=>b.querySelector('span')?.textContent==='Synthetic 000'));
    await page.evaluate(()=>[...document.querySelectorAll('.capture')].find(b=>b.querySelector('span')?.textContent==='Synthetic 000').click());
    await page.waitForFunction(()=>document.querySelector('.viewer-strip span')?.textContent==='Synthetic 000'&&!document.querySelector('#focus').disabled);
    await click('Try browser H.264 refocus');
    await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Archived discrete browser focus decoded')&&!document.querySelector('#export-live-photo').disabled);
  };
  const dispose = async (lateCallbacks=false) => {
    const before = await page.evaluate(()=>window.__owner.aborts);
    await page.evaluate(late=>{
      window.__ownerModule.harness_dispose();
      // Simulate already queued codec callbacks while their pending Rust future
      // still owns its closures. Closed decoders cannot schedule new callbacks.
      if(late) {
        const canvas=document.createElement('canvas');canvas.width=canvas.height=32;
        const frame=new VideoFrame(canvas,{timestamp:999999});canvas.width=canvas.height=0;
        window.__ownerCallbacks.output(frame);
        window.__ownerCallbacks.error(new DOMException('queued late codec error','EncodingError'));
      }
    },lateCallbacks);
    assert.equal(await page.$('#directory'),null,'real view is unmounted');
    assert((await page.evaluate(()=>window.__owner.aborts))>before,'owner cleanup aborts outstanding work');
  };
  const released = async () => {
    await page.waitForFunction(()=>window.__owner.created===window.__owner.closed&&window.__owner.outputs===window.__owner.framesClosed&&window.__urls.created===window.__urls.revoked);
  };
  const remount = async () => {
    await page.evaluate(()=>window.__ownerModule.harness_remount());
    await readyVideo();
  };
  await readyVideo();
  // Hold an actual complete synthetic network response across disposal. Returning
  // it late exercises abort checks after fetch, rather than only fetch rejection.
  await page.evaluate(()=>{
    window.__ownerFetchCounts={calls:0,requests:0,downloads:0};
    window.fetch=function(input,...args) {
      window.__ownerFetchCounts.calls++;
      if(input instanceof Request)window.__ownerFetchCounts.requests++;
      if(input instanceof Request&&input.url.endsWith('/download')) {
        window.__ownerFetchCounts.downloads++;
        window.__ownerFetchSignal=input.signal;
        window.__ownerFetchPromise=window.__ownerNative.fetch(new Request(input,{signal:new AbortController().signal}));
        return new Promise(resolve=>window.__ownerFetchResolve=resolve);
      }
      return window.__ownerNative.fetch(input,...args);
    };
  });
  await click('Export Live Photo pair (.zip)');
  try {await page.waitForFunction(()=>typeof window.__ownerFetchResolve==='function');}
  catch(error) {console.error(JSON.stringify(await page.evaluate(()=>({publicOwnerPhase:'held-source-fetch',status:document.querySelector('.status')?.textContent,counts:window.__ownerFetchCounts,browserAvcActive:!document.querySelector('#cancel-live-photo')?.disabled,exportDisabled:document.querySelector('#export-live-photo')?.disabled}))));throw error;}
  await dispose();
  assert(await page.evaluate(()=>window.__ownerFetchSignal.aborted),'fetch signal actually aborted');
  await page.evaluate(async()=>{window.fetch=window.__ownerNative.fetch;window.__ownerFetchResolve(await window.__ownerFetchPromise);});
  await delay(100);await released();
  assert.equal(await page.evaluate(()=>window.__owner.downloads),0);
  await remount();
  // Leave flush unresolved, dispose actual owner, and mount a new owner while the
  // old export future still exists. Its guard must not disturb new work.
  await page.evaluate(()=>{
    window.VideoDecoder=class extends window.__ownerTracked {flush(){return new Promise(resolve=>window.__ownerFlushResolve=resolve);}};
  });
  await click('Export Live Photo pair (.zip)');
  await page.waitForFunction(()=>typeof window.__ownerFlushResolve==='function');
  await dispose(true);
  await page.evaluate(()=>{window.VideoDecoder=window.__ownerTracked;window.__ownerModule.harness_remount();});
  await readyVideo();
  await page.evaluate(()=>window.__ownerFlushResolve());
  await click('Export Live Photo pair (.zip)');
  await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Complete Live Photo ZIP downloaded'));
  assert.equal(await page.evaluate(()=>window.__owner.downloads),1,'fresh owner recovers after late old flush');
  // Dispose during the final download URL lifetime, before the Rust future's
  // 250 ms cleanup timer settles. The already published pair remains complete.
  await page.evaluate(()=>{
    window.setTimeout=function(callback,ms,...args){if(ms===250){window.__ownerTimer=()=>callback(...args);return 0;}return window.__ownerNative.timer(callback,ms,...args);};
  });
  await click('Export Live Photo pair (.zip)');
  await page.waitForFunction(()=>window.__owner.downloads===2&&typeof window.__ownerTimer==='function');
  await dispose();
  await page.evaluate(()=>{window.setTimeout=window.__ownerNative.timer;window.__ownerTimer();});
  await delay(100);await released();
  assert.equal(await page.evaluate(()=>window.__owner.downloads),2,'timer cleanup creates no later download');
  await remount();
  // Observe the existing final successful UI export only. readyVideo has settled;
  // no public JS input clone or independent pair-verifier work enters this trace.
  await page.waitForFunction(()=>{const image=document.querySelector('.stage img');return image?.complete&&image.naturalWidth===32&&image.naturalHeight===32;});
  const geometry=await page.$eval('.stage img',image=>({width:image.naturalWidth,height:image.naturalHeight}));
  assert.deepEqual(geometry,{width:32,height:32},'tiny synthetic UI fixture scope');
  await page.evaluate(()=>window.__ownerModule.harness_capacity_start());
  let capacityText;
  try {
    await click('Export Live Photo pair (.zip)');
    await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Complete Live Photo ZIP downloaded'));
  } finally {
    capacityText=await page.evaluate(()=>{
      const text=window.__ownerModule.harness_capacity_take();
      if(typeof text!=='string'||text.length>192*1024)throw Error('bounded capacity report required');
      return text;
    });
  }
  const capacities=JSON.parse(capacityText);
  const keys=(value,expected)=>assert.deepEqual(Object.keys(value).sort(),[...expected].sort(),'fixed capacity record schema');
  keys(capacities,['observations','dropped','complete','observer_storage_bytes']);
  assert.equal(capacities.complete,true,'partial trace must not overflow');
  assert.equal(capacities.dropped,0);
  assert(Number.isSafeInteger(capacities.observer_storage_bytes)&&capacities.observer_storage_bytes>0&&capacities.observer_storage_bytes<=256*1024);
  assert(Array.isArray(capacities.observations)&&capacities.observations.length>0&&capacities.observations.length<=1024);
  const stages=new Set(['fetch_before_extend','fetch_after_extend','ui_source_begin','ui_source_end','selected_jpeg','pixel_strip','pcm_input','pcm_escape','segments','paired_jpeg','assembly','package','download_array']);
  for(const observation of capacities.observations) {
    keys(observation,['stage','bytes']);
    assert(stages.has(observation.stage),'only fixed UI capacity stages are allowed');
    assert(Array.isArray(observation.bytes)&&observation.bytes.length===4);
    for(const bytes of observation.bytes)assert(Number.isSafeInteger(bytes)&&bytes>=0&&bytes<=128*1024*1024,'bounded numeric capacity only');
  }
  for(const stage of stages)assert(capacities.observations.some(o=>o.stage===stage),`required partial UI stage: ${stage}`);
  const sources=capacities.observations.filter(o=>o.stage==='ui_source_begin');
  assert.equal(sources.length,1,'one actual UI export observed');
  assert(sources[0].bytes[1]>0&&sources[0].bytes[1]<=8*1024*1024,'existing source admission scope');
  assert.equal(capacities.observations.filter(o=>o.stage==='download_array').length,1,'one complete UI download observed');
  await page.evaluate(()=>window.__ownerModule.harness_dispose());await released();
  const counts=await page.evaluate(()=>window.__owner);
  assert.equal(counts.downloads,3,'complete export works after every actual owner disposal');
  return {actualMountedOwnerDisposal:true,lateFetch:true,queuedCodecCallbacks:true,lateFlush:true,downloadTimerDisposal:true,remountRecovery:true,partialCapacityTrace:{path:"mounted-ui",geometry,sourceLengthBytes:sources[0].bytes[1],sourceAdmissionBytes:8*1024*1024,maximumGeometryProof:false,totalMemoryProof:false,...capacities},...counts};
};
