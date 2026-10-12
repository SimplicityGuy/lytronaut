// Synthetic integration tooling: the tested application/renderer is Rust/WASM.
const assert=require('node:assert/strict');
const {PNG}=require('./browser/node_modules/pngjs');
module.exports=async function gpuTest(page,click){
 const ready=()=>page.waitForFunction(()=>document.querySelector('#gpu-status').textContent.startsWith('GPU verified derivative'));
 const render=async()=>{await click('Try GPU presentation');await ready();};
 await render();
 const hardware=await page.evaluate(async()=>{const a=await navigator.gpu.requestAdapter();return {info:{vendor:a.info.vendor,architecture:a.info.architecture,device:a.info.device},maxTextureDimension2D:a.limits.maxTextureDimension2D};});
 assert.equal(hardware.info.vendor,'apple');assert.equal(hardware.info.architecture,'metal-3');
 const compare=async orientation=>{const expected=await page.evaluate(async orientation=>{
  const gpu=document.querySelector('#gpu-stage');
  const library=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');
  const captures=await (await fetch(`/libraries/${library.id}/captures?limit=100`)).json();
  const capture=captures.items.find(c=>c.name===document.querySelector('.viewer-strip span').textContent);
  const d=await (await fetch(`/captures/${capture.id}`)).json();const id=d.originals.find(f=>f.role==='jpeg_focus_stack').id;
  const range=document.querySelector('#focus');const stack=await (await fetch(`/assets/${id}/focus`)).json();
  const index=stack.lambdas.reduce((best,v,i)=>Math.abs(v-Number(range.value))<Math.abs(stack.lambdas[best]-Number(range.value))?i:best,0);
  const blob=await (await fetch(`/assets/${id}/focus/${index}/1`)).blob();const url=URL.createObjectURL(blob),image=new Image();image.src=url;await image.decode();
  const b=document.createElement('canvas');b.width=image.naturalWidth;b.height=image.naturalHeight;
  const bc=b.getContext('2d');bc.imageSmoothingEnabled=false;bc.drawImage(image,0,0);
  const original=bc.getImageData(0,0,b.width,b.height).data,expected=new Uint8ClampedArray(gpu.width*gpu.height*4);
  const w=b.width,h=b.height,ow=orientation>=5?h:w,oh=orientation>=5?w:h;
  for(let dy=0;dy<gpu.height;dy++)for(let dx=0;dx<gpu.width;dx++){
   const x=Math.floor(dx*ow/gpu.width),y=Math.floor(dy*oh/gpu.height);
   const coordinates=[[x,y],[w-1-x,y],[w-1-x,h-1-y],[x,h-1-y],[y,x],[y,h-1-x],[w-1-y,h-1-x],[w-1-y,x]][orientation-1];
   const source=(coordinates[1]*w+coordinates[0])*4,destination=(dy*gpu.width+dx)*4;
   for(let c=0;c<4;c++)expected[destination+c]=original[source+c];
  }URL.revokeObjectURL(url);image.src='';
  gpu.style.width=gpu.width+'px';gpu.style.height=gpu.height+'px';
  return {pixels:[...expected],width:gpu.width,height:gpu.height,cpuBytes:Number(gpu.dataset.cpuBytes),gpuBytes:Number(gpu.dataset.gpuBytes)};
 },orientation);
 const png=PNG.sync.read(Buffer.from(await (await page.$('#gpu-stage')).screenshot({path:require('node:path').join(process.env.LYTRONAUT_TEST_ROOT,'gpu-orientation-'+orientation+'.png')})));assert.equal(png.width,expected.width);assert.equal(png.height,expected.height);
 let maximum=0,squared=0;for(let i=0;i<png.data.length;i++){const d=Math.abs(png.data[i]-expected.pixels[i]);maximum=Math.max(maximum,d);squared+=d*d;}
 const {pixels,...metadata}=expected;return {maximum,rmse:Math.sqrt(squared/png.data.length),...metadata};
 };
 const transforms=[];
 for(let orientation=1;orientation<=8;orientation++){
  await page.$eval('#orientation',(e,n)=>{e.value=String(n);e.dispatchEvent(new Event('change',{bubbles:true}));},orientation);
  await page.waitForFunction(n=>document.querySelector('output[for=focus]').textContent.includes('EXIF '+n),{},orientation);await ready();
  await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  const c=await compare(orientation);assert.equal(c.maximum,0,'exact integer texture-to-canvas pixels');assert.equal(c.rmse,0);assert(c.cpuBytes<=96*1024*1024&&c.gpuBytes<=64*1024*1024);transforms.push({orientation,...c});
 }
 await page.$eval('#gpu-quality',e=>{e.value='256';e.dispatchEvent(new Event('change',{bubbles:true}));});
 await page.waitForFunction(()=>document.querySelector('#gpu-stage').height===256);await ready();
 await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 const scaling=await compare(8);assert.equal(scaling.maximum,0);assert.equal(scaling.height,256);
 const timings=await page.evaluate(async()=>{const times=[];const range=document.querySelector('#focus'),canvas=document.querySelector('#gpu-stage');
  for(let i=0;i<35;i++){
   const old=canvas.dataset.submittedMs;range.value=i%2?range.min:range.max;range.dispatchEvent(new Event('input',{bubbles:true}));
   await new Promise((resolve,reject)=>{const start=performance.now();function poll(){if(canvas.dataset.submittedMs!==old&&document.querySelector('#gpu-status').textContent.startsWith('GPU verified derivative'))resolve();else if(performance.now()-start>5000)reject(Error('GPU submit timeout'));else setTimeout(poll,1);}poll();});
   const submit=Number(canvas.dataset.submittedMs);await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
   if(i>=5)times.push(performance.now()-submit);
  }times.sort((a,b)=>a-b);return {samples:times.length,p50SubmitToPostFrameCallbackMs:times[14],p95SubmitToPostFrameCallbackMs:times[28],maxMs:times[29],physicalPresentationMeasured:false};
 });
 // Observed device destruction: recover the already verified JPEG without input.
 const lossStart=Date.now();await page.evaluate(()=>window.__gpuDevice.destroy());
 await page.waitForFunction(()=>document.querySelector('#gpu-status').textContent.includes('CPU fallback')&&getComputedStyle(document.querySelector('.stage img')).display!=='none');
 const lossFallbackMs=Date.now()-lossStart;assert(lossFallbackMs<=1000);assert(await page.$eval('.stage img',e=>e.complete&&e.naturalWidth>0));
 const oversized=await page.evaluate(async()=>{
  const library=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');const captures=(await (await fetch(`/libraries/${library.id}/captures?limit=100`)).json()).items;
  const c=captures.find(c=>c.name===document.querySelector('.viewer-strip span').textContent);const d=await (await fetch(`/captures/${c.id}`)).json();const id=d.originals.find(f=>f.role==='jpeg_focus_stack').id;
  const stack=await (await fetch(`/assets/${id}/focus`)).json(),value=Number(document.querySelector('#focus').value);const index=stack.lambdas.indexOf(value);const orientation=document.querySelector('#orientation').value;
  const url=`/assets/${id}/focus/${index}/${orientation}`;const r=await fetch(url),metadata=JSON.parse(r.headers.get('X-Lytronaut-Derivative'));metadata.output_dimensions.width=2049;return {url,metadata,bytes:[...new Uint8Array(await r.arrayBuffer())]};
 });
 await page.setRequestInterception(true);const oversizeResponse=r=>{if(r.url().endsWith(oversized.url))r.respond({status:200,headers:{'Content-Type':'image/jpeg','X-Lytronaut-Derivative':JSON.stringify(oversized.metadata)},body:Buffer.from(oversized.bytes)}).catch(()=>{});else r.continue().catch(()=>{});};page.on('request',oversizeResponse);
 const previous=await page.$eval('.stage img',e=>e.src),oversizeStart=Date.now();await click('Try GPU presentation');await page.waitForFunction(()=>document.querySelector('#gpu-status').textContent.includes('CPU fallback'));
 const oversizeFallbackMs=Date.now()-oversizeStart;assert(oversizeFallbackMs<=1000);assert.equal(await page.$eval('.stage img',e=>e.src),previous);
 await page.setRequestInterception(false);page.off('request',oversizeResponse);
 const faults=[];
 for(const fault of ['missing-api','adapter','device','pipeline','late-device']){
  await page.evaluate(f=>{window.__gpuFault=f;if(f==='missing-api')Object.defineProperty(navigator,'gpu',{value:undefined,configurable:true});else Object.defineProperty(navigator,'gpu',{value:window.__gpuAPI,configurable:true});},fault);
  const start=Date.now();await click('Try GPU presentation');
  await page.waitForFunction(()=>document.querySelector('#gpu-status').textContent.includes('CPU fallback'));
  const elapsed=Date.now()-start;assert(elapsed<=1000);assert(await page.$eval('.stage img',e=>e.complete&&e.naturalWidth>0));faults.push({fault,fallbackMs:elapsed});
 }
 await new Promise(r=>setTimeout(r,1500));
 const released=await page.evaluate(()=>window.__gpuCounts);assert.equal(released.created,released.destroyed,'failed and late devices explicitly destroyed');
 await page.evaluate(()=>{window.__gpuFault='';Object.defineProperty(navigator,'gpu',{value:window.__gpuAPI,configurable:true});});
 await render();
 for(let i=0;i<6;i++){
  await click('Next capture');await page.waitForFunction(()=>document.querySelector('#focus').disabled&&document.querySelector('.stage img')?.naturalWidth===640);
  await click('Previous capture');await page.waitForFunction(()=>!document.querySelector('#focus').disabled&&document.querySelector('.status').textContent.includes('Discrete focus:'));await ready();
 }
 await click('Use CPU presentation');
 const finalDevices=await page.evaluate(()=>window.__gpuCounts);assert.equal(finalDevices.created,finalDevices.destroyed,'capture switches and CPU selection release devices');
 await page.$eval('#orientation',e=>{e.value='1';e.dispatchEvent(new Event('change',{bubbles:true}));});
 await page.$eval('#focus',e=>{e.value=e.min;e.dispatchEvent(new Event('input',{bubbles:true}));});
 await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('EXIF 1')&&document.querySelector('.status').textContent.includes('stored plane 1 of 3')&&document.querySelector('.stage img')?.naturalWidth===360);
 return {hardware,transforms,scaling,timings,lossFallbackMs,oversizeFallbackMs,faults,released,finalDevices,repeatedSwitches:12,physicalPresentation:'unmeasured; post-frame callback does not certify scanout',target:'16.7ms physical submit-to-present unverified'};
};
