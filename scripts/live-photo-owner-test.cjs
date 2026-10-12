// MIT public synthetic test tooling. Only an explicit test-harness WASM exposes
// real mount disposal; ordinary application artifacts do not expose these hooks.
const assert = require('node:assert/strict');
module.exports = async ({page, click, delay, shape, expectedGeometry}) => {
  await page.evaluate(async () => {
    // Same URL as assets/index.html: reuse the already initialized module.
    window.__ownerModule = await import('/client/lytronaut_client.js');
    if (['harness_dispose','harness_remount','harness_capacity_start','harness_capacity_take'].some(name=>typeof window.__ownerModule[name]!=='function')) throw Error('test-harness artifact required');
  });
  const responseReaderProof=shape==='small'?await require('./response-read-test.cjs')(page):null;
  const browseLifetimeProof=shape==='small'?await require('./browser-lifetime-test.cjs')({browser:page.browser(),origin:new URL(page.url()).origin}):null;
  await page.evaluate(()=>{
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
    let phase='directory';
    try {
      await page.waitForSelector('#directory');
      phase='all-captures';
      await click('All captures');
      // The library has 30 captures and a 24-item page. Default library order
      // falls back to opaque IDs, so Synthetic 000 need not be on page one.
      // Exercise the real name-sort UI rather than assuming identity ordering.
      phase='name-sort';
      await page.select('#sort','name');
      phase='synthetic-capture';
      await page.waitForFunction(()=>[...document.querySelectorAll('.capture')].some(b=>b.querySelector('span')?.textContent==='Synthetic 000'));
      phase='select-capture';
      await page.evaluate(()=>[...document.querySelectorAll('.capture')].find(b=>b.querySelector('span')?.textContent==='Synthetic 000').click());
      await page.waitForFunction(()=>document.querySelector('.viewer-strip span')?.textContent==='Synthetic 000'&&!document.querySelector('#focus').disabled);
      phase='decode-video';
      await click('Try browser H.264 refocus');
      await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Archived discrete browser focus decoded')&&!document.querySelector('#export-live-photo').disabled);
    } catch(error) {
      // Synthetic-only diagnostics: fixed categories, counts and booleans. Never
      // relay status text, capture names, library IDs, URLs or search contents.
      const diagnostic=await page.evaluate(()=>{
        const status=document.querySelector('.status')?.textContent||'';
        const statusCategory=status==='Loading captures…'?'loading-captures':status==='No captures match this search'?'empty-search':/^Showing \d+ captures on this page$/.test(status)?'showing-captures':status==='Library ready'?'library-ready':status.includes('Connection interrupted')?'connection-interrupted':status.includes('Select a library')?'library-missing':'other';
        const sort=document.querySelector('#sort')?.value;
        const pagination=/^Page (\d+)$/.exec(document.querySelector('.pagination span')?.textContent||'');
        const pageNumber=pagination?Number(pagination[1]):0;
        return {
          statusCategory,
          captureCount:Math.min(document.querySelectorAll('.capture').length,1024),
          targetPresent:[...document.querySelectorAll('.capture span')].some(s=>s.textContent==='Synthetic 000'),
          searchEmpty:document.querySelector('#search')?.value==='',
          sort:['order','name','rating'].includes(sort)?sort:'unknown',
          pageNumber:Number.isSafeInteger(pageNumber)&&pageNumber<=1000000?pageNumber:0,
          previousDisabled:document.querySelector('.pagination button')?.disabled===true,
          nextDisabled:document.querySelector('.pagination button:last-child')?.disabled===true,
          importBusy:document.querySelector('#directory')?.disabled===true,
          selectedTarget:document.querySelector('.viewer-strip span')?.textContent==='Synthetic 000',
          focusEnabled:document.querySelector('#focus')?.disabled===false,
          mounted:document.querySelector('#directory')!==null,
        };
      });
      console.error(JSON.stringify({publicOwnerReadiness:{phase,...diagnostic}}));
      throw error;
    }
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
  await page.waitForFunction(expected=>{const image=document.querySelector('.stage img');return image?.complete&&image.naturalWidth===expected.width&&image.naturalHeight===expected.height;},{},expectedGeometry);
  const geometry=await page.$eval('.stage img',image=>({width:image.naturalWidth,height:image.naturalHeight}));
  assert.deepEqual(geometry,expectedGeometry,'declared synthetic UI fixture scope');
  await page.evaluate(()=>{
    const nativeCanvas=HTMLCanvasElement.prototype.toDataURL, nativeBlob=window.Blob;
    const observations=[];let dropped=0;
    const record=(stage,bytes)=>{
      if(!bytes.every(value=>Number.isSafeInteger(value)&&value>=0&&value<=128*1024*1024))throw Error('bounded platform observation required');
      if(observations.length===128){dropped=Math.min(dropped+1,128*1024*1024);return;}
      observations.push({stage,bytes});
    };
    const restore=()=>{HTMLCanvasElement.prototype.toDataURL=nativeCanvas;window.Blob=nativeBlob;delete window.__ownerPlatformTake;};
    window.__ownerPlatformTake=()=>{
      restore();
      return {observations,dropped,complete:dropped===0,physicalHeapMeasured:false};
    };
    try {
      HTMLCanvasElement.prototype.toDataURL=function(...args){
        const result=Reflect.apply(nativeCanvas,this,args);
        record('canvas_result',[this.width,this.height,result.length,2*result.length]);
        return result;
      };
      window.Blob=new Proxy(nativeBlob,{construct(target,args,newTarget){
        const blob=Reflect.construct(target,args,newTarget);
        if(args[1]?.type==='application/zip'){
          const parts=args[0];
          if(!Array.isArray(parts)||parts.length!==1||!(parts[0] instanceof Uint8Array))throw Error('fixed ZIP parts required');
          record('zip_blob',[parts.length,parts[0].byteLength,parts[0].buffer.byteLength,blob.size]);
        }
        return blob;
      }});
    } catch(error) {restore();throw error;}
  });
  let capacityText,platform;
  try {
    await page.evaluate(()=>window.__ownerModule.harness_capacity_start());
    await click('Export Live Photo pair (.zip)');
    await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Complete Live Photo ZIP downloaded'));
  } finally {
    const result=await page.evaluate(()=>{
      let text,platform;
      try {text=window.__ownerModule.harness_capacity_take();}
      finally {platform=window.__ownerPlatformTake();}
      if(typeof text!=='string'||text.length>192*1024)throw Error('bounded capacity report required');
      const serialized=JSON.stringify(platform);
      if(serialized.length>32*1024)throw Error('bounded platform report required');
      return {text,platform};
    });
    capacityText=result.text;platform=result.platform;
  }
  const capacities=JSON.parse(capacityText);
  const keys=(value,expected)=>assert.deepEqual(Object.keys(value).sort(),[...expected].sort(),'fixed capacity record schema');
  keys(capacities,['observations','dropped','complete','observer_storage_bytes']);
  assert.equal(capacities.complete,true,'partial trace must not overflow');
  assert.equal(capacities.dropped,0);
  assert(Number.isSafeInteger(capacities.observer_storage_bytes)&&capacities.observer_storage_bytes>0&&capacities.observer_storage_bytes<=256*1024);
  assert(Array.isArray(capacities.observations)&&capacities.observations.length>0&&capacities.observations.length<=1024);
  const retainedStages=['retained_snapshot','retained_layout_strings','retained_layout_lists','retained_layout_detail','retained_layout_metadata','retained_layout_options','retained_layout_ui','retained_stage','retained_thumbs','retained_lists','retained_detail','retained_focus','retained_controls','retained_counts'];
  const stages=new Set([...retainedStages,'fetch_category','fetch_before_extend','fetch_after_extend','ui_source_begin','ui_source_end','metadata_json_storage','metadata_json_counts','metadata_references','metadata_planes','metadata_export','jpeg_platform_result','selected_jpeg','pixel_strip','pcm_input','pcm_escape','segments','paired_jpeg','assembly','package','download_array','download_array_live']);
  for(const observation of capacities.observations) {
    keys(observation,['stage','bytes']);
    assert(stages.has(observation.stage),'only fixed UI capacity stages are allowed');
    assert(Array.isArray(observation.bytes)&&observation.bytes.length===4);
    for(const bytes of observation.bytes)assert(Number.isSafeInteger(bytes)&&bytes>=0&&bytes<=128*1024*1024,'bounded numeric capacity only');
  }
  for(const stage of stages)assert(capacities.observations.some(o=>o.stage===stage),`required partial UI stage: ${stage}`);
  const retainedSnapshots=[];
  for(const observation of capacities.observations) {
    if(observation.stage==='retained_snapshot')retainedSnapshots.push([]);
    if(observation.stage.startsWith('retained_')) {
      assert(retainedSnapshots.length>0,'inventory rows belong to a synchronous snapshot');
      retainedSnapshots[retainedSnapshots.length-1].push(observation);
    }
  }
  assert.equal(retainedSnapshots.length,2,'entry and complete-download inventories');
  for(const [phase,snapshot] of retainedSnapshots.entries()) {
    assert.deepEqual(snapshot.map(row=>row.stage),retainedStages,'fixed snapshot field order');
    assert.deepEqual(snapshot[0].bytes,[phase,0,0,0]);
    const strings=snapshot.find(row=>row.stage==='retained_layout_strings').bytes;
    assert.equal(strings[0],4,'actual wasm32 pointer width');
    assert.equal(strings[1],3*strings[0],'pinned target String header receipt');
    assert.equal(strings[2],3*strings[0],'pinned target Vec header receipt');
    for(const row of snapshot.filter(row=>row.stage.startsWith('retained_layout_'))) {
      assert(row.bytes[0]>0,'actual target type size is recorded');
    }
    const stage=snapshot.find(row=>row.stage==='retained_stage').bytes;
    assert.equal(stage[3],1,'one actual stage String owner');
    assert(stage[1]>0&&stage[2]<=2*1024*1024,'actual capacity and admitted ASCII stage length are distinct');
    // Stage is an ASCII Blob URL; capacity remains distinct from URL UTF16 length.
    // The positive JPEG payload is tested separately; a short URL is not zero bytes.
    assert(stage[1]>=stage[2],'capacity covers recorded UTF16 units for ASCII stage');
    const counts=snapshot.find(row=>row.stage==='retained_counts').bytes;
    assert(counts[0]>=0&&counts[0]<=24&&counts[1]===24,'current synthetic capture-page and actual completed thumbnail owners');
    const thumbs=snapshot.find(row=>row.stage==='retained_thumbs').bytes;
    assert.equal(thumbs[3],2*counts[0],'each map key and value is counted separately');
  }
  for(let index=0;index<capacities.observations.length;index++) {
    const observation=capacities.observations[index];
    if(observation.stage==='fetch_category') {
      assert(observation.bytes[0]<=4&&observation.bytes.slice(1).every(v=>v===0),'fixed request category');
      assert(['fetch_before_extend','fetch_after_extend'].includes(capacities.observations[index+1]?.stage),'category adjacent to chunk snapshot');
    }
    if(['fetch_before_extend','fetch_after_extend'].includes(observation.stage))assert.equal(capacities.observations[index-1]?.stage,'fetch_category');
  }
  for(const category of [1,2])assert(capacities.observations.some(o=>o.stage==='fetch_category'&&o.bytes[0]===category),'actual export download and manifest attributed');
  const metadataStages=['metadata_json_storage','metadata_json_counts','metadata_references','metadata_planes','metadata_export'];
  const metadataRows=new Map();
  for(const stage of metadataStages) {
    const rows=capacities.observations.filter(row=>row.stage===stage);
    assert.equal(rows.length,1,'one completed export metadata phase per actual UI source');
    metadataRows.set(stage,rows[0].bytes);
  }
  const metadataIndices=metadataStages.map(stage=>capacities.observations.findIndex(row=>row.stage===stage));
  assert(metadataIndices[0]>capacities.observations.findIndex(row=>row.stage==='ui_source_begin'),'completed metadata is inside source ownership');
  for(let index=1;index<metadataIndices.length;index++)assert(metadataIndices[index]>metadataIndices[index-1],'JSON/references/planes/export lifetime order');
  assert(metadataIndices.at(-1)<capacities.observations.findIndex(row=>row.stage==='jpeg_platform_result'),'parser owners observed before codec JPEG allocation');
  assert(metadataRows.get('metadata_json_storage')[0]>0&&metadataRows.get('metadata_json_storage')[3]<=64);
  assert(metadataRows.get('metadata_json_counts')[0]>0&&metadataRows.get('metadata_json_counts')[1]>0);
  assert(metadataRows.get('metadata_references')[3]>0,'validated references observed before their drop');
  assert(metadataRows.get('metadata_references')[0]>0&&metadataRows.get('metadata_planes')[0]>0,'actual allocated reference/plane slots observed');
  assert(metadataRows.get('metadata_export')[1]>0&&metadataRows.get('metadata_export')[2]>=5*8,'actual section slots and at least five f64 lambda slots');
  assert.equal(metadataRows.get('metadata_planes')[2],5,'five verified archived descriptors');
  assert.equal(metadataRows.get('metadata_planes')[3],0);
  assert.equal(metadataRows.get('metadata_export')[3],5);
  keys(platform,['observations','dropped','complete','physicalHeapMeasured']);
  assert.equal(platform.complete,true);assert.equal(platform.dropped,0);assert.equal(platform.physicalHeapMeasured,false);
  assert(Array.isArray(platform.observations)&&platform.observations.length>0&&platform.observations.length<=128);
  for(const observation of platform.observations) {
    keys(observation,['stage','bytes']);assert(['canvas_result','zip_blob'].includes(observation.stage));
    assert(Array.isArray(observation.bytes)&&observation.bytes.length===4);
    for(const value of observation.bytes)assert(Number.isSafeInteger(value)&&value>=0&&value<=128*1024*1024);
    if(observation.stage==='canvas_result') {
      assert(observation.bytes[0]>0&&observation.bytes[0]<=2048&&observation.bytes[1]>0&&observation.bytes[1]<=2048);
      assert(observation.bytes[0]*observation.bytes[1]<=2*1024*1024,'admitted export and concurrent preview canvas extents');
      assert.equal(observation.bytes[3],2*observation.bytes[2]);
    }
  }
  const blobs=platform.observations.filter(o=>o.stage==='zip_blob');
  assert.equal(blobs.length,1);assert.equal(blobs[0].bytes[0],1);
  assert.equal(blobs[0].bytes[1],blobs[0].bytes[3]);assert(blobs[0].bytes[2]>=blobs[0].bytes[1]);
  const jpegResults=capacities.observations.filter(o=>o.stage==='jpeg_platform_result');
  assert(jpegResults.length>0,'selected export JPEG observed');
  const canvasCounts=new Map();
  for(const observation of platform.observations.filter(o=>o.stage==='canvas_result')) {
    const key=observation.bytes.slice(0,3).join(':');canvasCounts.set(key,(canvasCounts.get(key)||0)+1);
  }
  for(const result of jpegResults) {
    const jpeg=result.bytes;
    assert.equal(jpeg[2],geometry.width);assert.equal(jpeg[3],geometry.height);assert(jpeg[0]>=jpeg[1]);
    const key=[jpeg[2],jpeg[3],jpeg[1]].join(':');
    assert((canvasCounts.get(key)||0)>0,'selected ASCII JPEG result agrees across JS and Rust with retry multiplicity');
    canvasCounts.set(key,canvasCounts.get(key)-1);
  }
  const arrays=capacities.observations.filter(o=>o.stage==='download_array_live');
  assert.equal(arrays.length,1);assert.equal(arrays[0].bytes[1],blobs[0].bytes[1]);assert.equal(arrays[0].bytes[2],blobs[0].bytes[2]);
  const sources=capacities.observations.filter(o=>o.stage==='ui_source_begin');
  assert.equal(sources.length,1,'one actual UI export observed');
  assert(sources[0].bytes[1]>0&&sources[0].bytes[1]<=8*1024*1024,'existing source admission scope');
  assert.equal(metadataRows.get('metadata_export')[0],sources[0].bytes[1],'borrowed metadata source length matches actual UI source length');
  assert.equal(capacities.observations.filter(o=>o.stage==='download_array').length,1,'one complete UI download observed');
  await page.evaluate(()=>window.__ownerModule.harness_dispose());await released();
  const counts=await page.evaluate(()=>window.__owner);
  assert.equal(counts.downloads,3,'complete export works after every actual owner disposal');
  return {actualMountedOwnerDisposal:true,lateFetch:true,queuedCodecCallbacks:true,lateFlush:true,downloadTimerDisposal:true,remountRecovery:true,responseReaderProof,browseLifetimeProof,partialRetainedInventory:{snapshotCount:retainedSnapshots.length,complete:true,totalMemoryProof:false,unobserved:["tachys-cached-strings","btree-nodes","signal-infrastructure","dom-storage","physical-blob-sharing","gpu-resources","temporary-future-owners"],targetLayout:retainedSnapshots[0].filter(row=>row.stage.startsWith("retained_layout_"))},partialCapacityTrace:{path:"mounted-ui",shape,geometry,sourceLengthBytes:sources[0].bytes[1],sourceAdmissionBytes:8*1024*1024,maximumGeometryProof:false,totalMemoryProof:false,platform,...capacities},...counts};
};
