// Opt-in public synthetic acceptance tooling; no production code or mock bytes.
const assert=require('node:assert/strict');

function requireOverlap(row){
 assert(Number.isFinite(row.start)&&Number.isFinite(row.end)&&row.end>row.start,'finite focus interval required');
 assert(Number.isFinite(row.bodyFinished)&&Number.isFinite(row.gridDecoded),'completed response and decode timestamps required');
 assert(row.pendingGridAtStart>0,'pending grid required');
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
 return new Map(previews.filter(p=>p!==selected).map(p=>[p.id,p]));
}

async function run({browser,page,origin,instrument}){
 // Discover only the existing public synthetic grid. The caller refuses private
 // mode before opening this helper. No source names/IDs are emitted in results.
 const previews=await page.evaluate(async()=>{
  const names=[...document.querySelectorAll('.capture span')].map(e=>e.textContent);
  const library=(await(await fetch('/libraries')).json()).items.find(l=>l.state==='ready');
  if(!library)throw Error('ready synthetic library required');
  const found=[];
  for(let offset=0;;){
   const batch=await(await fetch(`/libraries/${library.id}/captures?limit=24&offset=${offset}`)).json();
   for(const capture of batch.items){
    if(!names.includes(capture.name))continue;
    const detail=await(await fetch(`/captures/${capture.id}`)).json();
    if(detail.preview)found.push({name:capture.name,id:detail.preview});
   }
   if(batch.next_offset==null)break;offset=batch.next_offset;
  }
  return found;
 });
 const byId=gridPreviewIdentities(previews);
 const rows=[];
 for(let sample=0;sample<24;sample++){
  const probe=await browser.newPage(),held=[],errors=[];
  let released=false,armed=false,lastInterval=null,heldReady;
  const pendingCandidate=new Promise(resolve=>{heldReady=resolve;});
  try{
   await probe.setViewport({width:1440,height:1000});
   probe.on('pageerror',error=>errors.push(error.message));
   await probe.setCacheEnabled(false);
   await instrument(probe);
   await probe.setRequestInterception(true);
   probe.on('request',request=>{
    const match=new URL(request.url()).pathname.match(/^\/assets\/([^/]+)\/chunks\/0$/);
    const preview=match&&byId.get(match[1]);
    // Hold one real nonselected response; holding the entire grid can exhaust
    // admission before the selected stage is ready, preventing measurement.
    if(preview&&armed&&!released&&held.length===0){held.push({request,preview});heldReady();}
    else request.continue().catch(()=>{});
   });
   await probe.goto(origin,{waitUntil:'domcontentloaded'});
   await probe.waitForFunction(()=>document.querySelectorAll('.capture').length===24);
   // Rust page() already selects the first capture. Clicking it again here
   // cancels and requeues initial metadata while thumbnail lanes are active.
   await probe.waitForFunction(()=>document.querySelector('.viewer-strip span')?.textContent==='Synthetic 000'&&!document.querySelector('#focus')?.disabled&&document.querySelector('.status')?.textContent.includes('Discrete focus:')&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
   // Initial metadata requires the entire shared 64 MiB lease. Do not hold a
   // positive-byte thumbnail lease until the selected focus is fully ready.
   // Only naturally issued later grid traffic is eligible; no image reloads.
   const pendingAtArming=await probe.evaluate(()=>document.querySelectorAll('.capture').length-[...document.querySelectorAll('.capture img')].filter(i=>i.complete&&i.naturalWidth>0).length);
   if(pendingAtArming===0)return {available:false,reason:'selected focus readiness drained all grid images',samples:rows.length,intervals:rows};
   armed=true;
   let candidateTimer;
   await Promise.race([pendingCandidate,new Promise(resolve=>{candidateTimer=setTimeout(resolve,5000);})]);
   clearTimeout(candidateTimer);
   if(held.length===0)return {available:false,reason:'no naturally issued post-readiness grid chunk',pendingAtArming,samples:rows.length,intervals:rows};
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
    let stageObserver,timer;
    const old=image.src;
    const displayed=new Promise((resolve,reject)=>{
     timer=setTimeout(()=>reject(Error('focus interval timed out')),5000);
     stageObserver=new MutationObserver(()=>{if(image.src!==old)resolve();});
     stageObserver.observe(image,{attributes:true,attributeFilter:['src']});
    });
    const pendingGridAtStart=document.querySelectorAll('.capture').length-document.querySelectorAll('.capture img').length;
    performance.clearResourceTimings();
    const start=performance.now();window.__gridFocusIntervalRunning=true;
    try{
     // Each fresh page starts on the selected stored plane; choose the opposite
     // endpoint to require a real stage update rather than an unchanged value.
     range.value=Number(range.value)===Number(range.min)?range.max:range.min;
     range.dispatchEvent(new Event('input',{bubbles:true}));
     await displayed;await image.decode();await new Promise(requestAnimationFrame);
     const end=performance.now();
     const resource=performance.getEntriesByName(url).find(e=>e.entryType==='resource'&&e.responseEnd>=start);
     return {sample,start,end,pendingGridAtStart,bodyBytes:resource?.decodedBodySize||0,bodyFinished:resource?.responseEnd??null,gridDecoded};
    }finally{
     window.__gridFocusIntervalRunning=false;clearTimeout(timer);stageObserver.disconnect();gridObserver.disconnect();
    }
   },{name:candidate.preview.name,url,sample});
   // Begin actual network service only after the exact page-clock interval
   // starts. Awaiting the measured promise never waits for grid completion.
   measuring.catch(()=>{});
   await probe.waitForFunction(()=>window.__gridFocusIntervalRunning===true,{polling:1,timeout:5000});
   released=true;
   await candidate.request.continue();
   const row=await measuring;lastInterval=row;requireOverlap(row);assert.deepEqual(errors,[],'contention page must not have application errors');rows.push(row);
  }catch(error){
   const state=await probe.evaluate(()=>({captures:document.querySelectorAll('.capture').length,gridImages:document.querySelectorAll('.capture img').length,focusDisabled:document.querySelector('#focus')?.disabled??null,stageReady:!!document.querySelector('.stage img')?.naturalWidth})).catch(()=>null);
   error.publicGridEvidence={available:false,reason:'grid setup or interval failed',samples:rows.length,intervals:rows,lastInterval,heldRequests:held.length,armed,released,state};
   error.message+=`; public grid state ${JSON.stringify(error.publicGridEvidence)}`;
   throw error;
  }finally{
   // Aborting other held requests is cleanup outside the measured interval.
   for(const {request}of held)if(!request.isInterceptResolutionHandled())await request.abort().catch(()=>{});
   await probe.close();
  }
 }
 const times=rows.map(requireOverlap).sort((a,b)=>a-b);
 return {samples:rows.length,pendingGridAtStart:Math.min(...rows.map(r=>r.pendingGridAtStart)),p50InputToDecodedFrameReadyMs:times[11],p95InputToDecodedFrameReadyMs:times[22],maxInputToDecodedFrameReadyMs:times[23],actualGridResponseAndDecodeOverlap:true,experiment:'fresh public synthetic page per interaction; one real pending grid chunk released during focus',intervals:rows};
}
module.exports={run,requireOverlap,gridPreviewIdentities};
