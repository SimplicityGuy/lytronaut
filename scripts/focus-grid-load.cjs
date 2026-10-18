// Opt-in public synthetic acceptance tooling; no production code or mock bytes.
const assert=require('node:assert/strict');

function requireOverlap(row){
 assert(Number.isFinite(row.start)&&Number.isFinite(row.end)&&row.end>row.start,'finite focus interval required');
 assert(Number.isFinite(row.bodyFinished)&&Number.isFinite(row.gridDecoded),'completed response and decode timestamps required');
 assert(row.pendingGridAtStart>0,'pending grid required');
 assert(row.requestedFrameVerified===true&&row.requestedPlane===3&&Number.isFinite(row.requestedLambda),'actual requested endpoint frame required');
 assert(row.bodyBytes>0&&row.bodyFinished>=row.start&&row.bodyFinished<=row.end,'a real grid response body must complete during the focus interval');
 assert(row.gridDecoded>=row.bodyFinished&&row.gridDecoded<=row.end,'the same grid image must decode during the focus interval');
 return row.end-row.start;
}

function gridPreviewIdentities(previews){
 const selected=previews.find(p=>p.name==='Synthetic 000');
 assert(selected&&previews.length===24,'unchanged 24-capture public synthetic page required');
 assert.equal(previews[0],selected,'public first capture must be the default-selected focus capture');
 assert(previews.every(p=>typeof p.id==='string'&&p.id.length>0),'actual preview asset identities required');
 assert.equal(new Set(previews.map(p=>p.id)).size,24,'selected and nonselected preview identities must be distinct');
 assert.equal(new Set(previews.map(p=>p.name)).size,24,'unique capture labels required for same-image attribution');
 assert.deepEqual(previews.map(p=>p.name),previews.map(p=>p.name).sort(),'complete preview page must follow name order');
 return new Map(previews.filter(p=>p!==selected).map(p=>[p.id,p]));
}

async function selectNamePage(page,libraryPath,names){
 const response=page.waitForResponse(response=>{
  const url=new URL(response.url());
  return response.ok()&&url.pathname===libraryPath&&url.searchParams.get('sort')==='name'&&url.searchParams.get('offset')==='0'&&url.searchParams.get('limit')==='24';
 });
 response.catch(()=>{});
 // Use the existing product control: change resets page/selection naturally.
 await page.select('#sort','name');
 // The exact response fences the post-change request. The complete label
 // vector below witnesses Rust's real JSON consumption/render; avoid a CDP
 // body read after Rust has consumed or cancelled that response stream.
 await response;
 await page.waitForFunction(expected=>{
  const actual=[...document.querySelectorAll('.capture span')].map(e=>e.textContent);
  return actual.length===expected.length&&actual.every((name,i)=>name===expected[i]);
 },{},names);
}

async function run({browser,page,origin,instrument}){
 // Discover only the existing public synthetic grid. The caller refuses private
 // mode before opening this helper. No source names/IDs are emitted in results.
 const {previews,libraryPath}=await page.evaluate(async()=>{
  const library=(await(await fetch('/libraries')).json()).items.find(l=>l.state==='ready');
  if(!library)throw Error('ready synthetic library required');
  const found=[];
  const libraryPath=`/libraries/${library.id}/captures`;
  const batch=await(await fetch(`${libraryPath}?limit=24&offset=0&sort=name`)).json();
  for(const capture of batch.items){
   const detail=await(await fetch(`/captures/${capture.id}`)).json();
   if(detail.preview)found.push({name:capture.name,id:detail.preview});
  }
  return {previews:found,libraryPath};
 });
 const byId=gridPreviewIdentities(previews);
 const names=previews.map(p=>p.name);
 const workload='name-sorted public 24-capture page; initial focus interaction after metadata becomes usable; may supersede initial default derivative';
 await selectNamePage(page,libraryPath,names);
 const rows=[];
 for(let sample=0;sample<24;sample++){
  const probe=await browser.newPage(),held=[],errors=[];
  let released=false,armed=false,lastInterval=null,heldReady,metadataReady=null,announceReady;
  const readiness=new Promise(resolve=>{announceReady=resolve;});
  const timeline=[],releaseBounds={};let events=0;
  const record=(kind,extra={})=>{events++;if(timeline.length===32)timeline.shift();timeline.push({kind,driverEpochMs:Date.now(),...extra});};
  const pendingCandidate=new Promise(resolve=>{heldReady=resolve;});
  try{
   await probe.setViewport({width:1440,height:1000});
   probe.on('pageerror',error=>errors.push(error.message));
   await probe.setCacheEnabled(false);
   await instrument(probe);
   await probe.exposeFunction('__reportFocusMetadataReady',state=>{
    if(metadataReady)return;
    metadataReady=state;armed=state.pendingGrid>0;
    record('metadata-ready',{pageReadyMs:state.pageReadyMs,pendingGrid:state.pendingGrid,stageKind:state.stageKind});announceReady(state);
   });
   await probe.evaluateOnNewDocument(require('./focus-readiness.cjs'),{names,selectedName:'Synthetic 000',notify:'__reportFocusMetadataReady'});
   await probe.setRequestInterception(true);
   probe.on('request',request=>{
    const match=new URL(request.url()).pathname.match(/^\/assets\/([^/]+)\/chunks\/0$/);
    const preview=match&&byId.get(match[1]);
    // Hold one real nonselected response; holding the entire grid can exhaust
    // admission before the selected stage is ready, preventing measurement.
    if(preview)record('natural-grid-request',{eligible:armed&&!released,held:held.length});
    if(preview&&armed&&!released&&held.length===0){held.push({request,preview});record('grid-held');heldReady();}
    else request.continue().catch(()=>{});
   });
   await probe.goto(origin,{waitUntil:'domcontentloaded'});
   await probe.waitForFunction(()=>document.querySelectorAll('.capture').length===24);
   await selectNamePage(probe,libraryPath,names);
   // The passive in-page observer binds the exact name-sorted page and an
   // enabled real focus control before initial focus rendering must finish.
   let readyTimer;
   const ready=await Promise.race([readiness,new Promise(resolve=>{readyTimer=setTimeout(()=>resolve(null),10000);})]);
   clearTimeout(readyTimer);
   if(!ready)return {available:false,workload,reason:'no passive metadata-ready announcement',samples:rows.length,intervals:rows,timeline,events};
   const pendingAtArming=ready.pendingGrid;
   if(pendingAtArming===0)return {available:false,workload,reason:'all grid images decoded before focus metadata became usable',samples:rows.length,intervals:rows,metadataReady,timeline,events};
   let candidateTimer;
   await Promise.race([pendingCandidate,new Promise(resolve=>{candidateTimer=setTimeout(resolve,5000);})]);
   clearTimeout(candidateTimer);
   if(held.length===0)return {available:false,workload,reason:'no naturally issued metadata-ready grid chunk',pendingAtArming,samples:rows.length,intervals:rows,metadataReady,timeline,events};
   const candidate=held[0];
   const url=candidate.request.url();
   // One chunk is deliberate: the same response must account for the decoded
   // grid image, rather than attributing unrelated bytes to its completion.
   const manifest=await probe.evaluate(async id=>(await(await fetch(`/assets/${id}/manifest`)).json()),candidate.preview.id);
   assert.equal(manifest.chunks.length,1,'single-chunk public grid preview required');
   const measuring=probe.evaluate(async({name,url,sample})=>{
    const range=document.querySelector('#focus'),image=document.querySelector('.stage img');
    const grid=[...document.querySelectorAll('.capture')].find(e=>e.querySelector('span')?.textContent===name);
    if(!range||range.disabled||!grid||grid.querySelector('img'))throw Error('fresh pending grid and ready focus required');
    let gridDecoded=null;
    const gridObserver=new MutationObserver(()=>{
     const img=grid.querySelector('img');if(!img)return;
     gridObserver.disconnect();img.decode().then(()=>{gridDecoded=performance.now();}).catch(()=>{});
    });
    gridObserver.observe(grid,{childList:true,subtree:true,attributes:true});
    const minimum=Number(range.min),maximum=Number(range.max);
    if(!Number.isFinite(minimum)||!Number.isFinite(maximum)||maximum<=minimum||Number(range.value)!==minimum)throw Error('fresh initial focus must start at minimum endpoint');
    const expected={name:'Synthetic 000',plane:3,lambda:maximum};
    let stageObserver,timer;
    const old=image.src;
    const frameState=()=>({changed:image.src!==old,selected:document.querySelector('.viewer-strip span')?.textContent,alt:image.alt,output:document.querySelector('output[for="focus"]')?.textContent,status:document.querySelector('.status')?.textContent});
    const displayed=new Promise((resolve,reject)=>{
     timer=setTimeout(()=>reject(Error('focus interval timed out')),5000);
     stageObserver=new MutationObserver(()=>{if(window.__focusReadinessObserver.frameMatches(frameState(),expected))resolve();});
     stageObserver.observe(document.body,{subtree:true,childList:true,characterData:true,attributes:true});
    });
    const pendingGridAtStart=document.querySelectorAll('.capture').length-document.querySelectorAll('.capture img').length;
    performance.clearResourceTimings();
    const start=performance.now();window.__gridFocusIntervalRunning=true;
    const interval={sample,start,end:null,pendingGridAtStart,requestedPlane:expected.plane,requestedLambda:expected.lambda,requestedFrameVerified:false};window.__lastFocusInterval=interval;
    try{
     // Each fresh page starts on the selected stored plane; choose the opposite
     // endpoint to require a real stage update rather than an unchanged value.
     range.value=range.max;
     range.dispatchEvent(new Event('input',{bubbles:true}));
     await displayed;const requestedSrc=image.src;await image.decode();await new Promise(requestAnimationFrame);
     if(image.src!==requestedSrc||!window.__focusReadinessObserver.frameMatches(frameState(),expected))throw Error('requested focus frame changed before measurement completed');
     const end=performance.now();
     const resource=performance.getEntriesByName(url).find(e=>e.entryType==='resource'&&e.responseEnd>=start);
     Object.assign(interval,{end,bodyBytes:resource?.decodedBodySize||0,bodyFinished:resource?.responseEnd??null,gridDecoded,requestedFrameVerified:true,displayedFrame:frameState()});return interval;
    }finally{
     if(interval.end===null){interval.failureObservedAtMs=performance.now();interval.displayedFrame=frameState();interval.gridDecoded=gridDecoded;}
     window.__gridFocusIntervalRunning=false;clearTimeout(timer);stageObserver.disconnect();gridObserver.disconnect();
    }
   },{name:candidate.preview.name,url,sample});
   // Begin actual network service only after the exact page-clock interval
   // starts. Awaiting the measured promise never waits for grid completion.
   measuring.catch(()=>{});
   await probe.waitForFunction(()=>window.__gridFocusIntervalRunning===true,{polling:1,timeout:5000});
   released=true;releaseBounds.commandDriverEpochMs=Date.now();record('grid-release-command');
   await candidate.request.continue();releaseBounds.ackDriverEpochMs=Date.now();record('grid-release-ack');
   const row=await measuring;row.metadataReady=metadataReady;row.timeline=timeline;row.timelineEvents=events;row.releaseBounds=releaseBounds;row.clocks={page:'performance.now in this page',driver:'Date.now epoch; not subtracted from page clock'};lastInterval=row;requireOverlap(row);assert.deepEqual(errors,[],'contention page must not have application errors');rows.push(row);
  }catch(error){
   const state=await probe.evaluate(()=>({captures:document.querySelectorAll('.capture').length,gridImages:document.querySelectorAll('.capture img').length,focusDisabled:document.querySelector('#focus')?.disabled??null,stageReady:!!document.querySelector('.stage img')?.naturalWidth})).catch(()=>null);
   if(!lastInterval)lastInterval=await probe.evaluate(()=>window.__lastFocusInterval||null).catch(()=>null);
   error.publicGridEvidence={available:false,workload,reason:'grid setup or interval failed',samples:rows.length,intervals:rows,lastInterval,heldRequests:held.length,armed,released,state,metadataReady,timeline,events,releaseBounds};
   error.message+=`; public grid state ${JSON.stringify(error.publicGridEvidence)}`;
   throw error;
  }finally{
   // Aborting other held requests is cleanup outside the measured interval.
   for(const {request}of held)if(!request.isInterceptResolutionHandled())await request.abort().catch(()=>{});
   await probe.evaluate(()=>window.__focusReadinessObserver?.dispose()).catch(()=>{});
   await probe.close();
  }
 }
 const times=rows.map(requireOverlap).sort((a,b)=>a-b);
 return {samples:rows.length,pendingGridAtStart:Math.min(...rows.map(r=>r.pendingGridAtStart)),p50InputToDecodedFrameReadyMs:times[11],p95InputToDecodedFrameReadyMs:times[22],maxInputToDecodedFrameReadyMs:times[23],actualGridResponseAndDecodeOverlap:true,experiment:workload+'; fresh page per interaction; one real pending grid chunk released during focus',intervals:rows};
}
module.exports={run,requireOverlap,gridPreviewIdentities};
