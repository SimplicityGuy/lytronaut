// Browser test tooling only. Product behavior is entirely Rust/WASM.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const crypto=require('node:crypto');
const path=require('node:path');
const {spawn}=require('node:child_process');
const pp=require(process.env.LYTRONAUT_PUPPETEER||'./browser/node_modules/puppeteer-core');
const root=process.env.LYTRONAUT_TEST_ROOT;
assert(root,'LYTRONAUT_TEST_ROOT must point at synthetic fixture/scratch');
const gpuMode=process.env.LYTRONAUT_GPU_TEST==='1';
const privateMode=process.env.LYTRONAUT_PRIVATE_TEST==='1';
const gpuLifetimeMode=process.env.LYTRONAUT_GPU_LIFETIME_TEST==='1';
if(gpuLifetimeMode)assert(gpuMode&&!privateMode&&process.env.LYTRONAUT_OWNER_TEST!=='1'&&!process.env.LYTRONAUT_VIDEO_PRIVATE_ROOT&&!process.env.LYTRONAUT_PRIVATE_SEALED_ROOT,'reference GPU lifetime fixture rejects incompatible/private flags');
const binary=process.env.LYTRONAUT_BINARY;
const origin=process.env.LYTRONAUT_TEST_ORIGIN||'http://127.0.0.1:3017';
const data=privateMode?root:path.join(root,'data');
fs.mkdirSync(data,{recursive:true,mode:0o700});
const server=process.env.LYTRONAUT_EXTERNAL_SERVER?{on(){},stderr:{on(){}},kill(){}}:spawn(binary,[],{cwd:process.cwd(),env:{...process.env,LYTRONAUT_DATA:data,LYTRONAUT_DATABASE:privateMode?'app.db':'catalog.db',LYTRONAUT_ORIGIN:origin,LYTRONAUT_LISTEN:new URL(origin).host},stdio:['ignore','ignore','pipe']});
let serverFailure=false;server.stderr.on("data",d=>{if(!privateMode)process.stderr.write(d);});server.on('exit',code=>{if(code)serverFailure=true;});
// The native service writes its LLVM profile (coverage builds) only on a normal
// exit, so shutdown awaits the graceful SIGINT path instead of abandoning it.
const serverExit=new Promise(resolve=>server.on('exit',(code,signal)=>resolve({code,signal})));
const stopServer=async()=>{
 if(process.env.LYTRONAUT_EXTERNAL_SERVER)return;
 if(server.exitCode===null&&server.signalCode===null)server.kill('SIGINT');
 let timer;const exit=await Promise.race([serverExit,new Promise(resolve=>{timer=setTimeout(()=>resolve(null),30000);})]);clearTimeout(timer);
 if(!exit){server.kill('SIGKILL');throw Error('service did not exit within 30s of SIGINT');}
 if(exit.signal||exit.code!==0)throw Error(`service exited uncleanly after SIGINT (code ${exit.code}, signal ${exit.signal})`);
};
let checkpoint='startup';
const delay=ms=>new Promise(r=>setTimeout(r,ms));
// Extra Chrome switches, e.g. --enable-unsafe-webgpu for the CI software WebGPU adapter.
const chromeArgs=(process.env.LYTRONAUT_CHROME_ARGS||'').split(/\s+/).filter(Boolean);
(async()=>{
 let browser;
 try{
  const startupStart=Date.now();let ready=false;for(let i=0;i<1200;i++){try{if((await fetch(origin+'/healthz')).ok){ready=true;break;}}catch{}if(serverFailure)throw Error('test service startup failed');await delay(250);}assert(ready,'service must become healthy within 300s');const startupMs=Date.now()-startupStart;
  if(!privateMode)console.error("starting isolated Chrome");
  browser=await require('./coverage.cjs').launch(pp,{executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:process.env.LYTRONAUT_GPU_TRACE!=='1'&&process.env.LYTRONAUT_CHROME_HEADLESS!=='0',userDataDir:path.join(root,'browser-profile-'+process.pid),args:chromeArgs});
  let page=await browser.newPage();
  const click=async text=>{await page.waitForFunction(text=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===text&&!b.disabled);if(!b)return false;b.click();return true;},{polling:50},text);};await page.setViewport({width:1440,height:1000});
  const instrument=async p=>p.evaluateOnNewDocument(gpuMode=>{
   if(!gpuMode)Object.defineProperty(navigator,'gpu',{value:undefined});
   else {
    window.__gpuAPI=navigator.gpu;window.__gpuFault='';window.__gpuCounts={created:0,destroyed:0};
    const requestAdapter=navigator.gpu.requestAdapter.bind(navigator.gpu);
    navigator.gpu.requestAdapter=async (...args)=>{
      if(window.__gpuFault==='adapter')return null;
      const a=await requestAdapter(...args);if(!a)return a;
      const requestDevice=a.requestDevice.bind(a);
      a.requestDevice=async (...args)=>{
        if(window.__gpuFault==='device')throw Error('Injected device rejection');
        if(window.__gpuFault==='late-device')await new Promise(r=>setTimeout(r,1200));
        const d=await requestDevice(...args);window.__gpuDevice=d;window.__gpuCounts.created++;const destroy=d.destroy.bind(d);let destroyed=false;d.destroy=()=>{if(!destroyed){destroyed=true;window.__gpuCounts.destroyed++;const callback=window.__gpuDestroyCallback;window.__gpuDestroyCallback=null;if(callback)callback();}return destroy();};
        const shader=d.createShaderModule.bind(d);
        d.createShaderModule=desc=>shader(window.__gpuFault==='pipeline'?{...desc,code:'invalid WGSL'}:desc);
        return d;
      };return a;
    };
   }window.__nativeVideoDecoder=window.VideoDecoder;Object.defineProperty(window,'VideoDecoder',{value:undefined,writable:true});
   window.__urls={created:0,revoked:0};window.__activeURLs=new Set();const create=URL.createObjectURL,revoke=URL.revokeObjectURL;
   URL.createObjectURL=function(...args){const url=create.apply(this,args);window.__urls.created++;window.__activeURLs.add(url);window.__lastCreatedUrl=url;return url;};
   URL.revokeObjectURL=function(...args){const result=revoke.apply(this,args);window.__urls.revoked++;window.__activeURLs.delete(args[0]);if(window.__gpuDestroyProbe?.url===args[0])window.__gpuDestroyProbe.revoked=true;return result;};
  },gpuMode);await instrument(page);
  const errors=[],responses=[];const attach=p=>{p.on('pageerror',e=>{errors.push(e.message);if(!privateMode)console.error(e.message);});p.on('response',r=>{if(/\/assets\//.test(r.url()))responses.push(r.status());});};attach(page);
  if(!privateMode)console.error('opening app');await page.goto(origin);checkpoint='direct-entry';await page.waitForSelector('#directory');
  assert.equal(await page.$('#username'),null);assert.equal(await page.$('#password'),null);
  assert(!fs.existsSync(path.join(data,'bootstrap.secret')));
  if(!privateMode){
   const directory=await page.$('#directory');await directory.uploadFile(path.join(root,'Synthetic.lytrolibrary'));
   await page.waitForFunction(async()=>{const id=sessionStorage.getItem('lytronaut-import');if(!id||document.querySelector('#directory').disabled)return false;return (await (await fetch('/imports/'+id)).json()).phase==='catalog_complete';},{timeout:120000});
   await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24,{timeout:120000});
  }
  await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length&&document.querySelectorAll('.capture').length>0,{timeout:120000});
  await page.waitForFunction(()=>document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
  assert(await page.$eval('.stage img',e=>e.complete&&e.naturalWidth>0));
  if(gpuLifetimeMode){
   checkpoint='gpu-acquisition-lifetime';await page.close();
   const proof=await require('./gpu-lifetime-test.cjs')({browser,origin});
   const clientDir=process.env.LYTRONAUT_CLIENT;assert(clientDir);
   const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
   const report={...proof,revision:process.env.LYTRONAUT_TEST_REVISION,artifactFeature:'test-harness',productionArtifact:false,browser:await browser.version(),applicationWasmSha256:hash(path.join(clientDir,'application.wasm')),generatedWasmSha256:hash(path.join(clientDir,'lytronaut_client_bg.wasm')),generatedGlueSha256:hash(path.join(clientDir,'lytronaut_client.js'))};
   assert(/^[0-9a-f]{40}$/.test(report.revision));
   const serialized=JSON.stringify(report);assert(Buffer.byteLength(serialized)<=16*1024);
   fs.writeFileSync(path.join(root,'gpu-lifetime-results.json'),serialized+'\n',{mode:0o600,flag:'wx'});console.log(serialized);return;
  }
  if(process.env.LYTRONAUT_OWNER_TEST==='1') {
   assert(!privateMode&&!gpuMode,'owner harness is public synthetic only');
   checkpoint='live-photo-actual-owner';
   await page.evaluate(()=>{if(typeof window.__nativeVideoDecoder!=='function')throw Error('actual decoder required for owner harness');window.VideoDecoder=window.__nativeVideoDecoder;});
   const shape=process.env.LYTRONAUT_OWNER_SHAPE||'small';
   const shapes={small:{width:32,height:32},large:{width:2048,height:1024},padded:{width:1138,height:1842}};
   assert(Object.hasOwn(shapes,shape),'closed public owner shape enum');
   const proof=await require('./live-photo-owner-test.cjs')({page,click,delay,shape,expectedGeometry:shapes[shape]});
   assert.deepEqual(errors,[],'actual owner cleanup and late futures cannot panic');
   const clientDir=process.env.LYTRONAUT_CLIENT;
   const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
   const report={...proof,revision:process.env.LYTRONAUT_TEST_REVISION||'working-tree',browser:await browser.version(),artifactFeature:'test-harness',productionArtifact:false,applicationWasmSha256:hash(path.join(clientDir,'application.wasm')),generatedWasmSha256:hash(path.join(clientDir,'lytronaut_client_bg.wasm')),photosGuiAcceptancePending:true,countedApplicationMemoryAcceptancePending:true,opaqueBrowserMemoryMeasured:false};
   fs.writeFileSync(path.join(root,'owner-results.json'),JSON.stringify(report,null,2)+'\n',{mode:0o600});
   console.log(JSON.stringify(report));return;
  }
  let sidebarProof=null;
  if(!privateMode&&!gpuMode){checkpoint='bounded-sidebars';sidebarProof=await require('./sidebar-test.cjs')({browser,origin});}
  // Separate public fault-injection flows (scripts/flows), each on its own page.
  const flowProofs={};if(!privateMode&&!gpuMode)for(const flow of (process.env.LYTRONAUT_FLOWS??'fault-injection,focus-video-faults,browse-admission,video-container-faults,cancellation-races,browser-api-faults,render-faults').split(',').filter(Boolean)){checkpoint='flow-'+flow;flowProofs[flow]=await require('./flows/'+flow+'.cjs')({browser,origin});}
  checkpoint='keyboard-navigation';const firstCapture=await page.$eval('.viewer-strip span',e=>e.textContent);
  await page.focus('.stage');await page.keyboard.press('ArrowRight');
  await page.waitForFunction(first=>document.querySelector('.viewer-strip span').textContent!==first,{},firstCapture);
  await page.keyboard.press('ArrowLeft');await page.waitForFunction(first=>document.querySelector('.viewer-strip span').textContent===first,{},firstCapture);
  checkpoint='discrete-focus-corpus';
  // Aggregate-only actual native generation + Chrome decoding; never log source IDs/pixels.
  const focusProof=await page.evaluate(async()=>{
   const libraries=(await (await fetch('/libraries')).json()).items.filter(l=>l.state==='ready');
   let stacks=0,planes=0,maxSourceBytes=0;const times=[];
   for(const library of libraries){let offset=0;for(;;){const captures=await (await fetch(`/libraries/${library.id}/captures?limit=100&offset=${offset}`)).json();
    for(const capture of captures.items){const detail=await (await fetch(`/captures/${capture.id}`)).json();
     const file=detail.originals.find(f=>f.role==='jpeg_focus_stack');if(!file)continue;
     const response=await fetch(`/assets/${file.id}/focus`);if(!response.ok)throw Error('focus metadata failed');
     const stack=await response.json();if(stack.lambdas.some((v,i)=>!Number.isFinite(v)||(i&&v<=stack.lambdas[i-1])))throw Error('invalid focus order');
     stacks++;maxSourceBytes=Math.max(maxSourceBytes,file.encoded_bytes);
     for(let i=0;i<stack.lambdas.length;i++){
      const start=performance.now();const response=await fetch(`/assets/${file.id}/focus/${i}/1`);if(!response.ok)throw Error('focus derivative failed');
      const meta=JSON.parse(response.headers.get('X-Lytronaut-Derivative'));const bytes=await response.arrayBuffer();
      const digest=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(v=>v.toString(16).padStart(2,'0')).join('');
      if(meta.representation!=='jpeg_focus_plane'||meta.plane_index!==i||meta.lambda!==stack.lambdas[i]||meta.encoded_bytes!==bytes.byteLength||meta.content_sha256!==digest||meta.orientation!==1)throw Error('derivative metadata or integrity failed');
      const bitmap=await createImageBitmap(new Blob([bytes],{type:'image/jpeg'}));
      if(bitmap.width!==meta.output_dimensions.width||bitmap.height!==meta.output_dimensions.height)throw Error('derivative decoded geometry failed');bitmap.close();
      times.push(performance.now()-start);planes++;
     }
    }if(captures.next_offset==null)break;offset=captures.next_offset;
   }}
   times.sort((a,b)=>a-b);return {stacks,planes,maxSourceBytes,p95NativeHttpDecodeMs:times[Math.ceil(times.length*.95)-1]};
  });
  assert.equal(focusProof.stacks,privateMode?58:1);assert.equal(focusProof.planes,privateMode?420:3);assert(focusProof.maxSourceBytes<=8*1024*1024);
  if(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1')assert(focusProof.p95NativeHttpDecodeMs<=100);
  checkpoint='focus-ui-benchmark';
  await click('All captures');
  const focusCaptureIndex=await page.evaluate(async()=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');let offset=0;for(;;){const page=await (await fetch(`/libraries/${l.id}/captures?limit=24&offset=${offset}`)).json();for(let i=0;i<page.items.length;i++){const d=await (await fetch(`/captures/${page.items[i].id}`)).json();if(d.capabilities.discrete_focus.state==='available')return offset+i;}if(page.next_offset==null)return -1;offset=page.next_offset;}});
  assert(focusCaptureIndex>=0);await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24);
  for(let offset=0;offset<Math.floor(focusCaptureIndex/24);offset++){await click('Next page');await page.waitForFunction(p=>document.querySelector('.pagination span').textContent===`Page ${p}`,{},offset+2);}
  await page.evaluate(i=>document.querySelectorAll('.capture')[i].click(),focusCaptureIndex%24);
  await page.waitForFunction(()=>!document.querySelector('#focus').disabled&&document.querySelector('.status').textContent.includes('Discrete focus:'));
  const measureFocusUi=async()=>page.evaluate(async()=>{const times=[];const pendingGridAtStart=document.querySelectorAll('.capture').length-[...document.querySelectorAll('.capture img')].filter(i=>i.complete&&i.naturalWidth>0).length;const range=document.querySelector('#focus');for(let i=0;i<24;i++){
   const image=document.querySelector('.stage img'),old=image.src;const start=performance.now();range.value=i%2?range.min:range.max;
   const displayed=new Promise((resolve,reject)=>{const timer=setTimeout(()=>{observer.disconnect();reject(Error('focus interaction timeout'));},5000);const observer=new MutationObserver(()=>{if(image.src!==old){clearTimeout(timer);observer.disconnect();resolve();}});observer.observe(image,{attributes:true,attributeFilter:['src']});});
   range.dispatchEvent(new Event('input',{bubbles:true}));await displayed;await image.decode();await new Promise(requestAnimationFrame);times.push(performance.now()-start);
  }times.sort((a,b)=>a-b);return {samples:times.length,pendingGridAtStart,p50InputToDecodedFrameReadyMs:times[Math.ceil(times.length*.5)-1],p95InputToDecodedFrameReadyMs:times[Math.ceil(times.length*.95)-1],maxInputToDecodedFrameReadyMs:times[times.length-1]};});
  const focusUiLoadContendedTimes=await measureFocusUi();
  if(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1')assert(focusUiLoadContendedTimes.pendingGridAtStart>0,'release grid-load gate requires pending grid images; a warm first batch is not load-contended evidence');
  await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length&&[...document.querySelectorAll('.capture img')].every(i=>i.complete&&i.naturalWidth>0),{timeout:120000});
  const focusUiTimes=await measureFocusUi();
  if(privateMode)console.log(JSON.stringify({focusProof,focusUiLoadContendedTimes,focusUiTimes}));
  if(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1')assert(focusUiTimes.p95InputToDecodedFrameReadyMs<=100&&focusUiLoadContendedTimes.p95InputToDecodedFrameReadyMs<=100,JSON.stringify({focusProof,focusUiLoadContendedTimes,focusUiTimes}));
  let gpuProof=null;if(gpuMode){checkpoint='GPU';
   const tracePath=path.join(root,'gpu-trace.json');
   if(process.env.LYTRONAUT_GPU_TRACE==='1')await page.tracing.start({path:tracePath,categories:['blink.user_timing','devtools.timeline','cc','benchmark','viz','gpu','disabled-by-default-devtools.timeline.frame'],screenshots:true});
   gpuProof=await require('./gpu-test.cjs')(page,click);
   fs.writeFileSync(path.join(root,'gpu-results.json'),JSON.stringify(gpuProof,null,2),{mode:0o600});
   if(process.env.LYTRONAUT_GPU_TRACE==='1'){
    await page.tracing.stop();const events=JSON.parse(fs.readFileSync(tracePath,'utf8')).traceEvents;
    gpuProof.compositorTrace={submitMarkers:events.filter(e=>e.name.startsWith('lytronaut-gpu-submit-')).length,framePresented:events.filter(e=>e.name==='FramePresented').length,frameDisplayed:events.filter(e=>e.name==='Display::FrameDisplayed').length,drawAndSwap:events.filter(e=>e.name==='Graphics.Pipeline.DrawAndSwap').length,matchedCanvasPresentationSamples:0,status:'Canvas-frame identity correlation unavailable; unrelated presentation events are not latency evidence'};
    fs.writeFileSync(path.join(root,'gpu-results.json'),JSON.stringify(gpuProof,null,2),{mode:0o600});
   }
  }
  if(!privateMode){
   checkpoint='focus-controls';await page.waitForFunction(()=>!document.querySelector('#focus').disabled&&document.querySelector('.status').textContent.includes('Discrete focus:'));
   assert.deepEqual(await page.$eval('.stage img',e=>[e.naturalWidth,e.naturalHeight]),[360,240]);
   const corners=async()=>page.evaluate(async()=>{const img=document.querySelector('.stage img');await img.decode();const c=document.createElement('canvas');c.width=img.naturalWidth;c.height=img.naturalHeight;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);const samples=[[20,20],[c.width-20,20],[20,c.height-20],[c.width-20,c.height-20]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data].slice(0,3));c.width=c.height=0;return samples.map(rgb=>rgb.map(v=>v>180?'H':'L').join(''));});
   assert.deepEqual(await corners(),['HLL','LHL','LLH','HHL'],'baked8/sidecar6 conflict defaults to identity');
   checkpoint='focus-keyboard-end';await page.focus('#focus');await page.keyboard.press('End');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('stored plane 3 of 3'));
   checkpoint='focus-keyboard-home';await page.keyboard.press('Home');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('stored plane 1 of 3'));
   checkpoint='focus-orientation6';await page.select('#orientation','6');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===240&&document.querySelector('.status').textContent.includes('EXIF 6'));
   assert.deepEqual(await corners(),['LLH','HLL','HHL','LHL']);
   checkpoint='focus-orientation1';await page.select('#orientation','1');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===360&&document.querySelector('.status').textContent.includes('EXIF 1'));
   checkpoint='export-png-ui';await page.select('#orientation','6');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===240&&document.querySelector('.status').textContent.includes('EXIF 6'));
   await click('Export PNG');await page.waitForFunction(()=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&document.querySelector('#export-download'));
   const exportProof=await page.evaluate(async()=>{
    const recipe=await (await fetch(document.querySelector('#export-recipe').getAttribute('href'))).json();const r=await fetch(document.querySelector('#export-download').getAttribute('href'));const bytes=await r.arrayBuffer();
    const hash=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(b=>b.toString(16).padStart(2,'0')).join('');
    const image=await createImageBitmap(new Blob([bytes],{type:'image/png'}));const c=document.createElement('canvas');c.width=image.width;c.height=image.height;const ctx=c.getContext('2d');ctx.drawImage(image,0,0);const colors=[[20,20],[c.width-20,20],[20,c.height-20],[c.width-20,c.height-20]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data].slice(0,3).map(v=>v>180?'H':'L').join(''));image.close();c.width=c.height=0;
    return {recipe,bytes:bytes.byteLength,hash,colors};
   });
   assert.equal(exportProof.recipe.source_variant,'stored_jpeg_focus_plane');assert.equal(exportProof.recipe.plane_index,0);assert.equal(exportProof.recipe.orientation,6);assert.deepEqual(exportProof.recipe.output_dimensions,{width:240,height:360});assert.deepEqual(exportProof.recipe.visible_geometry,[0,0,240,360]);assert.equal(exportProof.recipe.refocus_loss,true);assert.equal(exportProof.recipe.color_space,'unknown');assert.equal(exportProof.recipe.content_sha256,exportProof.hash);assert.equal(exportProof.recipe.encoded_bytes,exportProof.bytes);assert.deepEqual(exportProof.colors,['LLH','HLL','HHL','LHL']);
   const pngHref=await page.$eval('#export-download',e=>e.getAttribute('href'));
   checkpoint='export-jpeg-ui';await click('Export JPEG');await page.waitForFunction(old=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&document.querySelector('#export-download')?.getAttribute('href')!==old,{},pngHref);
   assert.equal(await page.evaluate(async()=> (await (await fetch(document.querySelector('#export-recipe').getAttribute('href'))).json()).format),'jpeg');
   const jpegHref=await page.$eval('#export-download',e=>e.getAttribute('href'));
   checkpoint='lossless-backup-ui';await click('Back up originals and catalog');await page.waitForFunction(old=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&document.querySelector('#export-download')?.getAttribute('href')!==old,{},jpegHref);
   assert.equal(await page.$('#export-recipe'),null);
   const bundleBytes=await page.evaluate(async()=>[...new Uint8Array(await (await fetch(document.querySelector('#export-download').getAttribute('href'))).arrayBuffer())]);const bundlePath=path.join(root,'synthetic-backup.lytronaut');fs.writeFileSync(bundlePath,Buffer.from(bundleBytes),{mode:0o600});
   checkpoint='isolated-restore-ui';await (await page.$('#restore-bundle')).uploadFile(bundlePath);await page.waitForFunction(()=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&!document.querySelector('#export-download')&&document.querySelector('.status').textContent.includes('restore complete'));
   assert.equal(await page.evaluate(async()=> (await (await fetch('/libraries')).json()).items.filter(l=>l.state==='ready').length),1,'isolated restore must not replace the live library');
   await page.select('#orientation','1');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===360&&document.querySelector('.status').textContent.includes('EXIF 1'));
   // Corrupt a real derivative while retaining its genuine typed header. Old verified frame stays usable.
   checkpoint='focus-integrity';const focusFile=await page.evaluate(async()=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');const c=(await (await fetch(`/libraries/${l.id}/captures?limit=100`)).json()).items.find(c=>c.name==='Synthetic 000');return (await (await fetch(`/captures/${c.id}`)).json()).originals.find(f=>f.role==='jpeg_focus_stack').id;});
   const genuine=await page.evaluate(async id=>{const r=await fetch(`/assets/${id}/focus/2/1`);return {meta:r.headers.get('X-Lytronaut-Derivative'),size:JSON.parse(r.headers.get('X-Lytronaut-Derivative')).encoded_bytes};},focusFile);
   await page.setRequestInterception(true);let corruptFocus=true;
   const badFocus=r=>{if(corruptFocus&&r.url().endsWith('/focus/2/1'))r.respond({status:200,headers:{'Content-Type':'image/jpeg','X-Lytronaut-Derivative':genuine.meta},body:Buffer.alloc(genuine.size)}).catch(()=>{});else r.continue().catch(()=>{});};page.on('request',badFocus);
   await page.focus('#focus');await page.keyboard.press('End');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Derivative integrity check failed'));
   assert.deepEqual(await corners(),['HLL','LHL','LLH','HHL']);corruptFocus=false;await page.setRequestInterception(false);page.off('request',badFocus);
   // Cancel held focus work by changing capture; a late response cannot replace the static frame.
   checkpoint='focus-cancellation';let heldFocus;await page.setRequestInterception(true);const staleFocus=r=>{if(r.url().includes('/focus/1/')&&!heldFocus)heldFocus=r;else r.continue().catch(()=>{});};page.on('request',staleFocus);
   await page.$eval('#focus',e=>{e.value='0';e.dispatchEvent(new Event('input',{bubbles:true}));});for(let i=0;i<100&&!heldFocus;i++)await delay(10);assert(heldFocus);
   const selectedStatic=await page.evaluate(()=>{const button=[...document.querySelectorAll('.capture')].find(b=>b.textContent!=='Synthetic 000');const name=button.textContent;button.click();return name;});await heldFocus.continue().catch(()=>{});await page.waitForFunction(name=>document.querySelector('#focus').disabled&&document.querySelector('.viewer-strip span').textContent===name,{},selectedStatic);await delay(100);
   assert(await page.$eval('#focus',e=>e.disabled));assert(await page.$eval('.stage img',e=>e.naturalWidth===640));await page.setRequestInterception(false);page.off('request',staleFocus);
  }
  const contrast=await page.evaluate(()=>{const luminance=c=>{const rgb=c.match(/[\d.]+/g).slice(0,3).map(x=>{let v=Number(x)/255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;});return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722;};const ratio=(a,b)=>{a=luminance(a);b=luminance(b);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);};const root=getComputedStyle(document.documentElement),button=getComputedStyle(document.querySelector('.search button'));return {text:ratio(root.color,root.backgroundColor),control:ratio(button.borderTopColor,button.backgroundColor)};});
  assert(contrast.text>=4.5);assert(contrast.control>=3);
  const selectNamedCapture=async name=>{
   await click('All captures');
   const firstNames=await page.evaluate(async()=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');return (await (await fetch(`/libraries/${l.id}/captures?limit=24&offset=0`)).json()).items.map(c=>c.name);});
   await page.waitForFunction(names=>JSON.stringify([...document.querySelectorAll('.capture span')].map(b=>b.textContent))===JSON.stringify(names)&&document.querySelectorAll('.capture span').length>0,{},firstNames);
   for(let i=0;i<8;i++){
    const found=await page.evaluate(name=>{const b=[...document.querySelectorAll('.capture')].find(b=>b.querySelector('span').textContent===name);if(!b)return false;b.click();return true;},name);if(found)return;
    const old=await page.$$eval('.capture span',b=>b.map(e=>e.textContent));const nav=await page.evaluate(()=>({disabled:[...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='Next page').disabled,page:document.querySelector('.pagination span').textContent,status:document.querySelector('.status').textContent,search:document.querySelector('#search').value,sort:document.querySelector('#sort').value}));assert(!nav.disabled,JSON.stringify({name,firstNames,old,i,...nav}));
    await click('Next page');await page.waitForFunction(old=>JSON.stringify([...document.querySelectorAll('.capture span')].map(b=>b.textContent))!==JSON.stringify(old)&&document.querySelectorAll('.capture span').length>0,{},old);
   }throw Error('named fixture exceeds bounded page search');
  };
  let videoUiProof=null;
  if(!privateMode){
   let gpuVideoFailureRecovery=null,rendererDestroyReentry=null;
   checkpoint='video-ui';
   // Prior intentional derivative faults select CPU fallback for this session.
   // Reload establishes a fresh automatic GPU attempt before testing the join.
   await page.reload();await page.waitForSelector('#directory');await click('All captures');
   await page.waitForFunction(()=>document.querySelector('.pagination span').textContent==='Page 1'&&document.querySelectorAll('.capture').length>0&&[...document.querySelectorAll('.capture img')].every(i=>i.complete&&i.naturalWidth>0));
   await page.evaluate(()=>window.VideoDecoder=window.__nativeVideoDecoder);
   await selectNamedCapture('Synthetic 000');
   await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Discrete focus:'));
   if(gpuMode){
    await page.waitForFunction(()=>getComputedStyle(document.querySelector('#gpu-stage')).display!=='none'&&window.__gpuCounts.created-window.__gpuCounts.destroyed===1);
    await page.evaluate(()=>window.VideoDecoder=undefined);await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Browser video decode unavailable'));
    assert(await page.$eval('#gpu-stage',e=>getComputedStyle(e).display!=='none'),'failed video preserves current verified GPU frame');
    await page.evaluate(()=>window.VideoDecoder=window.__nativeVideoDecoder);
   }
   await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Archived discrete browser focus decoded'));
   assert.deepEqual(await page.$eval('.stage img',e=>[e.naturalWidth,e.naturalHeight]),[32,32]);
   assert.deepEqual(await page.$eval('#focus',e=>[Number(e.min),Number(e.max)]),[-2,3]);
   assert(await page.$eval('.stage img',e=>getComputedStyle(e).display!=='none'),'new verified video image is visible');
   assert(await page.$eval('#gpu-stage',e=>getComputedStyle(e).display==='none'),'old GPU canvas cannot obscure video');
   if(gpuMode){const devices=await page.evaluate(()=>window.__gpuCounts);assert.equal(devices.created,devices.destroyed,'video adoption releases obsolete GPU device');}
   assert(await page.$eval('#export-png',e=>e.disabled));assert(await page.$eval('#export-jpeg',e=>e.disabled));assert(!(await page.$eval('#backup-library',e=>e.disabled)));
   assert((await page.$eval('#video-export-unavailable',e=>e.textContent)).includes('Select a verified static/JPEG preview'));
   let videoExportPosts=0;const countVideoExport=r=>{if(r.method()==='POST'&&r.url().endsWith('/exports'))videoExportPosts++;};page.on('request',countVideoExport);
   await page.$eval('#export-png',e=>{e.disabled=false;e.dispatchEvent(new MouseEvent('click',{bubbles:true}));});await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('PNG/JPEG export from browser AVC planes is unavailable'));await delay(100);page.off('request',countVideoExport);assert.equal(videoExportPosts,0,'injected flattened export handler fails closed on video');

   await page.$eval('#focus',e=>{e.value='-2';e.dispatchEvent(new Event('input',{bubbles:true}));});
   await page.waitForFunction(()=>document.querySelector('output[for=focus]').textContent.includes('plane 3')&&document.querySelector('.stage img').complete);
   assert(await page.evaluate(async()=>{const img=document.querySelector('.stage img');await img.decode();const c=document.createElement('canvas');c.width=c.height=32;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);const p=ctx.getImageData(16,16,1,1).data;c.width=c.height=0;return p[2]>200&&p[0]<40&&p[1]<40;}),'archived nonmonotonic lambda retains selected blue frame');
   assert(await page.$eval('#orientation',e=>e.disabled));
   checkpoint='live-photo-browser-ui';
   await page.evaluate(()=>{window.__liveDownloadCount=0;window.__liveAnchorClick=HTMLAnchorElement.prototype.click;HTMLAnchorElement.prototype.click=function(){if(this.download==='lytronaut-live-photo.zip'){window.__liveDownloadCount++;return;}return window.__liveAnchorClick.call(this);};});
   await click('Export Live Photo pair (.zip)');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Complete Live Photo ZIP downloaded'));
   assert.equal(await page.evaluate(()=>window.__liveDownloadCount),1,'only the complete pair reaches download');
   assert(!(await page.$eval('#export-live-photo',e=>e.disabled)));
   await page.evaluate(()=>{window.__liveNative=window.VideoDecoder;window.__liveLateFlush=null;window.VideoDecoder=class extends window.__liveNative{flush(){return new Promise(resolve=>window.__liveLateFlush=resolve);}};});
   checkpoint='live-photo-cancel-late-flush';await click('Export Live Photo pair (.zip)');await page.waitForFunction(()=>typeof window.__liveLateFlush==='function');
   await click('Cancel browser Live Photo export');await page.waitForFunction(()=>!document.querySelector('#export-live-photo').disabled);
   const cancelledStatus=await page.$eval('.status',e=>e.textContent);
   await page.evaluate(()=>{window.__liveLateFlush();window.__liveLateFlush=null;});await delay(100);
   assert.equal(await page.$eval('.status',e=>e.textContent),cancelledStatus,'late flush cannot change cancellation status');
   assert.equal(await page.evaluate(()=>window.__liveDownloadCount),1,'cancelled export never downloads a half pair');
   checkpoint='live-photo-navigate-late-flush';await click('Export Live Photo pair (.zip)');await page.waitForFunction(()=>typeof window.__liveLateFlush==='function');
   await selectNamedCapture('Synthetic 002');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===640);
   const navigatedStatus=await page.$eval('.status',e=>e.textContent);await page.evaluate(()=>{window.__liveLateFlush();window.__liveLateFlush=null;});await delay(100);
   assert.equal(await page.$eval('.status',e=>e.textContent),navigatedStatus,'late export cannot overwrite a new capture');
   assert.equal(await page.evaluate(()=>window.__liveDownloadCount),1);
   await page.evaluate(()=>window.VideoDecoder=window.__liveNative);await selectNamedCapture('Synthetic 000');await page.waitForFunction(()=>document.querySelector('.viewer-strip span').textContent==='Synthetic 000');
   await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===32);
   await page.evaluate(()=>{window.__liveLateFlush=null;window.VideoDecoder=class extends window.__liveNative{flush(){return new Promise(resolve=>window.__liveLateFlush=resolve);}};});
   checkpoint='live-photo-pagehide';await click('Export Live Photo pair (.zip)');await page.waitForFunction(()=>typeof window.__liveLateFlush==='function');
   await page.evaluate(()=>window.dispatchEvent(new PageTransitionEvent('pagehide',{persisted:true})));await delay(100);
   await page.evaluate(()=>{window.__liveLateFlush();window.__liveLateFlush=null;});await delay(100);
   assert.equal(await page.evaluate(()=>window.__liveDownloadCount),1,'pagehide cancels unpublished pair');
   await page.evaluate(()=>{window.VideoDecoder=window.__liveNative;HTMLAnchorElement.prototype.click=window.__liveAnchorClick;window.dispatchEvent(new PageTransitionEvent('pageshow',{persisted:true}));});
   await page.waitForFunction(()=>document.querySelectorAll('.capture span').length>0&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
   await selectNamedCapture('Synthetic 000');await page.waitForFunction(()=>document.querySelector('.viewer-strip span').textContent==='Synthetic 000');await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===32);
   await page.$eval('#focus',e=>{e.value='-2';e.dispatchEvent(new Event('input',{bubbles:true}));});await page.waitForFunction(()=>document.querySelector('output[for=focus]').textContent.includes('plane 3'));
   checkpoint='video-corruption';const videoFile=await page.evaluate(async()=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');const c=(await (await fetch(`/libraries/${l.id}/captures?limit=100`)).json()).items.find(c=>c.name==='Synthetic 000');return (await (await fetch(`/captures/${c.id}`)).json()).originals.find(f=>f.role==='refocus_stack');});
   const retainedVideo=await page.$eval('.stage img',e=>e.src);await page.setRequestInterception(true);
   const badVideo=r=>{if(r.url().endsWith(`/assets/${videoFile.id}/download`))r.respond({status:200,headers:{'Content-Type':'application/octet-stream'},body:Buffer.alloc(videoFile.encoded_bytes)}).catch(()=>{});else r.continue().catch(()=>{});};page.on('request',badVideo);
   await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Video original integrity failed'));assert.equal(await page.$eval('.stage img',e=>e.src),retainedVideo);
   await page.setRequestInterception(false);page.off('request',badVideo);
   checkpoint='video-stale-capture';let heldVideo;await page.setRequestInterception(true);const staleVideo=r=>{if(r.url().endsWith(`/assets/${videoFile.id}/download`)&&!heldVideo)heldVideo=r;else r.continue().catch(()=>{});};page.on('request',staleVideo);
   await click('Try browser H.264 refocus');for(let i=0;i<100&&!heldVideo;i++)await delay(10);assert(heldVideo);
   await selectNamedCapture('Synthetic 002');await heldVideo.continue().catch(()=>{});
   await page.waitForFunction(()=>document.querySelector('#focus').disabled&&document.querySelector('.stage img')?.naturalWidth===640);await delay(100);assert.equal(await page.$eval('.stage img',e=>e.naturalWidth),640);await page.setRequestInterception(false);page.off('request',staleVideo);
   checkpoint='video-absence-fallback';await page.evaluate(()=>window.VideoDecoder=undefined);await selectNamedCapture('Synthetic 001');await page.waitForFunction(()=>document.querySelector('.viewer-strip span').textContent==='Synthetic 001'&&document.querySelector('.stage img')?.naturalWidth===640);
   const staticVideo=await page.$eval('.stage img',e=>e.src);await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Browser video decode unavailable'));assert.equal(await page.$eval('.stage img',e=>e.src),staticVideo);assert(await page.$eval('#focus',e=>e.disabled));
   await page.evaluate(()=>window.VideoDecoder=window.__nativeVideoDecoder);await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===32);
   if(gpuMode){
    await selectNamedCapture('Synthetic 000');await page.waitForFunction(()=>getComputedStyle(document.querySelector('#gpu-stage')).display!=='none');
    await page.evaluate(()=>window.VideoDecoder=undefined);await click('Try browser H.264 refocus');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Browser video decode unavailable'));
    assert(await page.$eval('#gpu-stage',e=>getComputedStyle(e).display!=='none'));
    const lossStart=Date.now();await page.evaluate(()=>window.__gpuDevice.destroy());await page.waitForFunction(()=>document.querySelector('#gpu-status').textContent.includes('device lost'));
    const lossRecoveryMs=Date.now()-lossStart;assert(lossRecoveryMs<=1000);assert(await page.$eval('.stage img',e=>getComputedStyle(e).display!=='none'&&e.naturalWidth>0));
    gpuVideoFailureRecovery={priorGpuPreserved:true,lossRecoveryMs,verifiedImageFallback:true};await page.evaluate(()=>window.VideoDecoder=window.__nativeVideoDecoder);
    // A separate successful AVC pass above already proves ordinary adoption.
    // Here an actual acquired device synchronously selects a newer capture while
    // the old video's unpublished JPEG guard is held across Renderer::drop.
    checkpoint='gpu-destroy-reentrant-video';await page.reload();await page.waitForSelector('#directory');await click('All captures');await page.select('#sort','name');
    await page.waitForFunction(()=>['Synthetic 000','Synthetic 002'].every(name=>[...document.querySelectorAll('.capture span')].some(span=>span.textContent===name))&&[...document.querySelectorAll('.capture img')].every(image=>image.complete&&image.naturalWidth>0));
    await page.evaluate(()=>{window.VideoDecoder=window.__nativeVideoDecoder;[...document.querySelectorAll('.capture')].find(button=>button.querySelector('span')?.textContent==='Synthetic 002').click();});
    await page.waitForFunction(()=>document.querySelector('.viewer-strip span')?.textContent==='Synthetic 002'&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth===640&&document.querySelector('.stage img')?.naturalHeight===360);
    const sampleReentryPixels=()=>page.$eval('.stage img',image=>{const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;try{const context=canvas.getContext('2d');context.drawImage(image,0,0);return [[0,0],[320,180],[205,118],[639,359]].flatMap(([x,y])=>Array.from(context.getImageData(x,y,1,1).data));}finally{canvas.width=canvas.height=0;}});
    const expectedReentryPixels=await sampleReentryPixels();
    await page.evaluate(()=>[...document.querySelectorAll('.capture')].find(button=>button.querySelector('span')?.textContent==='Synthetic 000').click());
    await page.waitForFunction(()=>getComputedStyle(document.querySelector('#gpu-stage')).display!=='none'&&window.__gpuCounts.created>0&&window.__gpuCounts.created-window.__gpuCounts.destroyed===1);
    await page.evaluate(()=>{
      const probe=window.__gpuDestroyProbe={calls:0,url:null,published:false,revoked:false};
      const inspect=records=>{if(probe.url)for(const record of records)if(record.attributeName==='src'&&(record.oldValue===probe.url||record.target.getAttribute('src')===probe.url))probe.published=true;};
      probe.observer=new MutationObserver(inspect);probe.inspect=inspect;probe.observer.observe(document.querySelector('.stage'),{subtree:true,attributes:true,attributeOldValue:true,attributeFilter:['src']});
      window.__gpuDestroyCallback=()=>{probe.calls++;probe.url=window.__lastCreatedUrl;const capture=[...document.querySelectorAll('.capture')].find(button=>button.querySelector('span')?.textContent==='Synthetic 002');if(!capture)throw Error('public destroy target missing');capture.click();};
    });
    try{
      await click('Try browser H.264 refocus');
      await page.waitForFunction(()=>window.__gpuDestroyProbe.calls===1&&document.querySelector('.viewer-strip span')?.textContent==='Synthetic 002'&&document.querySelector('.capture[aria-pressed="true"] span')?.textContent==='Synthetic 002'&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth===640&&document.querySelector('.stage img')?.naturalHeight===360);
      assert.deepEqual(await sampleReentryPixels(),expectedReentryPixels,'newer static capture pixels survive obsolete AVC destruction');
      assert(await page.$eval('#gpu-stage',element=>getComputedStyle(element).display==='none'));
      rendererDestroyReentry=await page.evaluate(()=>{const p=window.__gpuDestroyProbe;p.inspect(p.observer.takeRecords());return {actualDeviceCreated:window.__gpuCounts.created>0,callbackCalls:p.calls,newerCaptureWon:true,unpublishedVideoRevoked:p.revoked,obsoleteVideoPublished:p.published};});
      assert.deepEqual(rendererDestroyReentry,{actualDeviceCreated:true,callbackCalls:1,newerCaptureWon:true,unpublishedVideoRevoked:true,obsoleteVideoPublished:false});
    }finally{await page.evaluate(()=>{window.__gpuDestroyCallback=null;window.__gpuDestroyProbe?.observer.disconnect();window.__gpuDestroyProbe=null;});}
    await page.select('#sort','order');
    const restoredOrder=await page.evaluate(async()=>{const libraries=await fetch('/libraries',{signal:AbortSignal.timeout(10000)});if(!libraries.ok)throw Error('public library response unavailable');const library=(await libraries.json()).items.find(item=>item.state==='ready');const response=await fetch(`/libraries/${library.id}/captures?limit=24&offset=0`,{signal:AbortSignal.timeout(10000)});if(!response.ok)throw Error('public capture response unavailable');return (await response.json()).items.map(item=>item.name);});
    await page.waitForFunction(names=>JSON.stringify([...document.querySelectorAll('.capture span')].map(span=>span.textContent))===JSON.stringify(names)&&[...document.querySelectorAll('.capture img')].every(image=>image.complete&&image.naturalWidth>0),{},restoredOrder);
   }
   await selectNamedCapture('Synthetic 002');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===640);
   videoUiProof={actualRustHttp:true,visibleVideo:true,oldGpuHidden:true,obsoleteDeviceReleased:gpuMode,gpuVideoFailureRecovery,rendererDestroyReentry,flattenedButtonsDisabled:true,injectedFlattenedExportPosts:videoExportPosts,backupAvailable:true,nonmonotonicSelectedBlue:true,corruptOriginalRetainsFrame:true,staleCaptureIgnored:true,absentCodecStaticFallback:true,retryRecovered:true};
  }
  checkpoint='capture-pagination';await click('All captures');await page.waitForFunction(()=>document.querySelector('.pagination span').textContent==='Page 1'&&document.querySelectorAll('.capture img').length===24,{timeout:120000});let seen=0;
  for(;;){
   const count=await page.$$eval('.capture img',imgs=>imgs.filter(i=>i.complete&&i.naturalWidth>0).length);seen+=count;
   const next=await page.evaluateHandle(()=>[...document.querySelectorAll('button')].find(b=>b.textContent==='Next page'));
   if(await next.evaluate(e=>e.disabled)){await next.dispose();break;}
   const old=await page.$eval('.pagination span',e=>e.textContent);await next.asElement().click();await next.dispose();
   await page.waitForFunction(old=>document.querySelector('.pagination span').textContent!==old,{},old);
   await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length,{timeout:120000});
  }
  assert.equal(seen,privateMode?109:30);
  checkpoint='ordered-albums';const libraryCount=await page.evaluate(async()=> (await (await fetch('/libraries')).json()).items.length);const albumCount=await page.$$eval('.rail-item',(buttons,libraryCount)=>buttons.length-libraryCount-1,libraryCount);assert.equal(albumCount,3);
  const albums=await page.evaluate(async()=>{const libraries=await (await fetch('/libraries')).json();const library=libraries.items.find(l=>l.state==='ready');return (await (await fetch(`/libraries/${library.id}/albums`)).json()).items;});
  const memberships=[];
  for(let index=0;index<albumCount;index++){
   const expected=await page.evaluate(async id=>{let offset=0,all=[];for(;;){let p=await (await fetch(`/albums/${id}/captures?limit=24&offset=${offset}`)).json();all.push(...p.items);if(p.next_offset==null)break;offset=p.next_offset;}return all.map(c=>c.name+(c.name_truncated?' [truncated]':''));},albums[index].id);
   const buttons=await page.$$('.rail-item');await buttons[index+libraryCount+1].click();await Promise.all(buttons.map(b=>b.dispose()));
   let actual=[];
   for(let offset=0;;offset+=24){
    const names=expected.slice(offset,offset+24);
    await page.waitForFunction(names=>JSON.stringify([...document.querySelectorAll('.capture span')].map(e=>e.textContent))===JSON.stringify(names),{timeout:120000},names);
    await page.waitForFunction(()=>document.querySelectorAll('.capture img').length>0&&document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length,{timeout:120000});
    actual.push(...await page.$$eval('.capture span',nodes=>nodes.map(e=>e.textContent)));
    if(offset+24>=expected.length)break;
    await click('Next page');
   }
   assert.deepEqual(actual,expected);memberships.push(actual.length);
  }
  assert.deepEqual([...memberships].sort((a,b)=>a-b),privateMode?[12,39,58]:[10,10,10]);
  if(!privateMode){
   await click('All captures');await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24&&!document.querySelector('.album-item[aria-pressed="true"]'));
   await page.type('#search','Synthetic 029');await click('Search');
   await page.waitForFunction(()=>document.querySelectorAll('.capture').length===1);assert((await page.$eval('.capture',e=>e.textContent)).includes('029'));
   await page.$eval('#search',e=>{e.value='';e.dispatchEvent(new Event('input',{bubbles:true}));});await click('Search');
   await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
   checkpoint='sorting';for(const sort of ['name','rating','order']){
    const expected=await page.evaluate(async sort=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');return (await (await fetch(`/libraries/${l.id}/captures?limit=24&offset=0&sort=${sort}`)).json()).items.map(c=>c.name);},sort);
    await page.$eval('#sort',(e,sort)=>{e.value=sort;e.dispatchEvent(new Event('change',{bubbles:true}));},sort);
    await page.waitForFunction(expected=>JSON.stringify([...document.querySelectorAll('.capture span')].map(e=>e.textContent))===JSON.stringify(expected),{},expected);
    await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
   }
   await page.screenshot({path:path.join(root,'synthetic-browser.png'),fullPage:true});
   for(const zipName of ['synthetic.zip','synthetic-zip64.zip']){const zip=await page.$('#zip');await zip.uploadFile(path.join(root,zipName));
   await page.waitForFunction(async()=>{const id=sessionStorage.getItem('lytronaut-import');if(!id||document.querySelector('#zip').disabled)return false;return (await (await fetch('/imports/'+id)).json()).phase==='catalog_complete';},{timeout:120000});}
  }
  if(!privateMode){
   checkpoint='upload-cancellation';
   // Abort a real in-progress XHR and recover its durable typed state.
   await page.setRequestInterception(true);let holdUploads=true;const heldUploads=[];
   const intercept=request=>{if(holdUploads&&request.method()==='PUT'&&request.url().includes('/file')){heldUploads.push(request);return;}request.continue().catch(()=>{});};page.on('request',intercept);
   await (await page.$('#directory')).uploadFile(path.join(root,'Synthetic.lytrolibrary'));
   await page.waitForFunction(()=>document.querySelector('#directory').disabled&&[...document.querySelectorAll('button')].some(b=>b.textContent==='Cancel import'&&!b.disabled));
   for(let i=0;i<500&&!heldUploads.length;i++)await delay(20);assert(heldUploads.length>0,'real upload has started');
   await click('Cancel import');holdUploads=false;await Promise.all(heldUploads.map(r=>r.abort().catch(()=>{})));
   await page.waitForFunction(async()=>{const id=sessionStorage.getItem('lytronaut-import');return (await (await fetch('/imports/'+id)).json()).phase==='cancelled';});
   await page.setRequestInterception(false);page.off('request',intercept);
   const priorImport=await page.evaluate(()=>sessionStorage.getItem('lytronaut-import'));
   let recoveryRequest=null,recoveryFinished=false,recoveryFailed=false;
   const recoveryStarted=request=>{if(!recoveryRequest&&request.method()==='GET'&&new URL(request.url()).pathname==='/imports/'+priorImport)recoveryRequest=request;};
   const recoveryEnded=request=>{if(request===recoveryRequest)recoveryFinished=true;};
   const recoveryErrored=request=>{if(request===recoveryRequest)recoveryFailed=true;};
   page.on('request',recoveryStarted);page.on('requestfinished',recoveryEnded);page.on('requestfailed',recoveryErrored);
   try{
    await page.evaluate(()=>{window.__importBusyObserved=false;window.__importStatusObserver=new MutationObserver(()=>{if(document.querySelector('.status')?.textContent.includes('Previous operation still pending'))window.__importBusyObserved=true;});window.__importStatusObserver.observe(document.querySelector('.status'),{subtree:true,childList:true,characterData:true});});
    // The cancel lane reports first; then hold the actual recovery response so a
    // pending recovery is observable instead of racing an already-true status.
    await page.waitForFunction(()=>document.querySelector('.status').textContent.toLowerCase().includes('cancelled'));
    let holdRecovery=null;const holdRecoveryRequest=request=>{if(!holdRecovery&&request.method()==='GET'&&new URL(request.url()).pathname==='/imports/'+priorImport){holdRecovery=request;return;}request.continue().catch(()=>{});};
    await page.setRequestInterception(true);page.on('request',holdRecoveryRequest);
    try{
     await click('Recover import status');
     for(let i=0;i<500&&!holdRecovery;i++)await delay(10);assert(holdRecovery,'recovery request issued');
     checkpoint='recovery-pending-controls';
     assert.deepEqual(await page.evaluate(()=>({directory:document.querySelector('#directory').disabled,zip:document.querySelector('#zip').disabled,recover:[...document.querySelectorAll('button')].find(b=>b.textContent==='Recover import status').disabled})),{directory:true,zip:true,recover:true},'import controls stay disabled while recovery is pending');
    }finally{page.off('request',holdRecoveryRequest);if(holdRecovery)await holdRecovery.continue().catch(()=>{});await page.setRequestInterception(false);}
    await page.waitForFunction(()=>document.querySelector('.status').textContent.toLowerCase().includes('cancelled')&&!document.querySelector('#directory').disabled);
    checkpoint='preserved-only';
    // Unknown schema stays useful through preserved-only original links.
    await (await page.$('#directory')).uploadFile(path.join(root,'Preserved.lytrolibrary'));
    try{await page.waitForFunction(async()=>['catalog_complete','catalog_failed'].includes((await (await fetch('/imports/'+sessionStorage.getItem('lytronaut-import'))).json()).phase)&&!document.querySelector('#directory').disabled,{timeout:120000});}
    catch(error){
     let state={diagnosticReadFailed:true};
     try{state=await page.evaluate(async prior=>{const id=sessionStorage.getItem('lytronaut-import');let phase='unavailable';try{const response=await fetch('/imports/'+id,{signal:AbortSignal.timeout(10000)});if(response.ok){const value=(await response.json()).phase;phase=['intake','catalog_pending','catalog_complete','catalog_failed','failed','cancelled','cancelling'].includes(value)?value:'other';}}catch{}const status=document.querySelector('.status')?.textContent||'';return {diagnosticReadFailed:false,importIdentityChanged:id!==prior,directoryDisabled:!!document.querySelector('#directory')?.disabled,busyFeedbackObserved:window.__importBusyObserved===true,phase,statusKind:status.includes('Previous operation still pending')?'busy':status.toLowerCase().includes('cancelled')?'cancelled':'other'};},priorImport);}catch{}
     console.error(JSON.stringify({preservedImportDiagnostic:true,recoveryRequested:recoveryRequest!==null,recoveryFinished,recoveryFailed,...state}));throw error;
    }
   }finally{
    page.off('request',recoveryStarted);page.off('requestfinished',recoveryEnded);page.off('requestfailed',recoveryErrored);
    await page.evaluate(()=>{window.__importStatusObserver?.disconnect();delete window.__importStatusObserver;delete window.__importBusyObserved;}).catch(()=>{});
   }
   const preservedPhase=await page.evaluate(async()=> (await (await fetch('/imports/'+sessionStorage.getItem('lytronaut-import'))).json()).phase);
   if(preservedPhase==='catalog_failed')await click('Retry catalog adaptation');
   await page.waitForFunction(()=>document.querySelectorAll('.browser-panel ul a').length===2,{timeout:120000});
   const original=await page.$eval('.browser-panel ul a',e=>e.getAttribute('href'));
   assert.equal((await page.evaluate(async url=>(await fetch(url)).status,original)),200);
   checkpoint='stale-page';
   // Delayed library/albums responses from a replaced page cannot repopulate the new workspace.
   const allLibraries=await page.evaluate(async()=> (await (await fetch('/libraries')).json()).items);
   const readyIndex=allLibraries.findIndex(l=>l.state==='ready');
   let held;await page.setRequestInterception(true);
   const delayed=request=>{if(request.url().includes('/albums')&&!held){held=request;}else{request.continue().catch(()=>{});}};page.on('request',delayed);
   await click('Library '+(readyIndex+1));
   for(let i=0;i<50&&!held;i++)await delay(20);assert(held);
   await page.reload();await held.continue().catch(()=>{});await delay(200);
   assert.equal(await page.$('#username'),null);
   await page.setRequestInterception(false);page.off('request',delayed);
   await page.waitForSelector('#directory');
   await click('Library '+(readyIndex+1));
   await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
   checkpoint='corrupt-preview';
   await page.evaluate(()=>[...document.querySelectorAll('.capture')].find(b=>b.textContent!=='Synthetic 000').click());await page.waitForFunction(()=>document.querySelector('#focus').disabled&&document.querySelector('.stage img')?.naturalWidth===640);
   // Corrupt transfer bytes reject preview integrity while original downloads work.
   let corrupt=true;await page.setRequestInterception(true);
   const corruptResponse=request=>{if(corrupt&&request.url().includes('/chunks/'))request.respond({status:200,body:Buffer.alloc(10154)}).catch(()=>{});else request.continue().catch(()=>{});};page.on('request',corruptResponse);
   await click('Retry preview');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('integrity check failed'));
   const intact=await page.$$eval('.inspector a',links=>links.find(e=>e.textContent.includes('Thumbnail')).getAttribute('href'));const originalBytes=await page.evaluate(async url=>{const r=await fetch(url);if(r.status!==200)throw Error('original unavailable');return [...new Uint8Array(await r.arrayBuffer())];},intact);
   assert(Buffer.from(originalBytes).equals(fs.readFileSync(path.join(root,'synthetic.jpg'))),'preserved thumbnail bytes match independent fixture');
   corrupt=false;await page.setRequestInterception(false);page.off('request',corruptResponse);await click('Retry preview');
   await page.waitForFunction(()=>document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
   checkpoint='canvas-cleanup';
   await page.waitForFunction(()=>{const images=[...document.querySelectorAll('.capture img')];return images.length===24&&images.every(image=>image.complete&&image.naturalWidth>0);},{timeout:120000});
   // The synthetic second page has six thumbnails. Each downsample creates a
   // 128² canvas; Rust's error path zeroes it and revokes its source URL before
   // returning Unavailable. Fence those exact attempts, not unrelated polling.
   await page.evaluate(()=>{
    window.__canvasGet=HTMLCanvasElement.prototype.getContext;
    window.__failedThumbnailCanvases=new Set();
    HTMLCanvasElement.prototype.getContext=function(kind){
     if(kind==='2d'&&this.width===128&&this.height===128)window.__failedThumbnailCanvases.add(this);
     return null;
    };
   });
   try{
    await click('Next page');
    await page.waitForFunction(()=>{
     const canvases=window.__failedThumbnailCanvases,stage=document.querySelector('.stage img');
     return document.querySelector('.pagination span').textContent==='Page 2'
      &&document.querySelectorAll('.capture').length===6
      &&document.querySelectorAll('.capture .thumb-empty').length===6
      &&document.querySelectorAll('.capture img').length===0
      &&canvases.size===6&&[...canvases].every(canvas=>canvas.width===0&&canvas.height===0)
      &&stage?.complete&&stage.naturalWidth>0&&stage.src.startsWith('blob:')
      &&window.__activeURLs.size===1&&window.__activeURLs.has(stage.src)
      &&window.__urls.created-window.__urls.revoked===1;
    },{timeout:120000});
   }finally{
    await page.evaluate(()=>{HTMLCanvasElement.prototype.getContext=window.__canvasGet;delete window.__canvasGet;delete window.__failedThumbnailCanvases;});
   }
   await click('Previous page');await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
  }
  await page.waitForFunction(()=>document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
  // Each accepted grid JPEG and the selected stage own a guarded Blob URL.
  const preZoomURLProof=await page.evaluate(()=>{const stage=[...document.querySelectorAll('.stage img')].map(img=>img.src).filter(src=>src.startsWith('blob:'));const gridImages=[...document.querySelectorAll('.capture img')];const grid=gridImages.map(img=>img.src).filter(src=>src.startsWith('blob:'));const displayed=new Set([...stage,...grid]);return {created:window.__urls.created,revoked:window.__urls.revoked,owned:window.__activeURLs.size,stage:stage.length,grid:grid.length,gridImages:gridImages.length,distinctDisplayed:displayed.size,exactDisplayedOwnership:displayed.size===window.__activeURLs.size&&[...displayed].every(url=>window.__activeURLs.has(url))};});
  assert(preZoomURLProof.created>0);assert.equal(preZoomURLProof.stage,1);assert(preZoomURLProof.grid<=24);assert.equal(preZoomURLProof.grid,preZoomURLProof.gridImages);assert.equal(preZoomURLProof.distinctDisplayed,1+preZoomURLProof.grid);assert(preZoomURLProof.exactDisplayedOwnership);assert.equal(preZoomURLProof.created-preZoomURLProof.revoked,preZoomURLProof.owned);
  await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));await page.waitForFunction(()=>window.__urls.created===window.__urls.revoked);assert.equal(await page.evaluate(()=>window.__activeURLs.size),0,'pagehide releases every owned Blob URL');
  const preZoomGPUDevices=gpuMode?await page.evaluate(()=>window.__gpuCounts):null;if(gpuMode)assert.equal(preZoomGPUDevices.created,preZoomGPUDevices.destroyed,'all GPU devices released before browser restart');
  // Actual browser lifecycle callbacks restore a bfcache page without account state.
  await page.evaluate(()=>window.dispatchEvent(new PageTransitionEvent('pageshow',{persisted:true})));
  await page.waitForFunction(()=>document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0,{timeout:120000});
  await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));
  await page.waitForFunction(()=>window.__urls.created===window.__urls.revoked);assert.equal(await page.evaluate(()=>window.__activeURLs.size),0,'pagehide releases every owned Blob URL');
  await page.waitForSelector('#directory');
  // Chromium stores default zoom as log(factor)/log(1.2), partition key x.
  // Restart the isolated profile with actual browser zoom; do not use page scaling.
  checkpoint='zoom200';const baseline=await page.evaluate(()=>({inner:innerWidth,dpr:devicePixelRatio}));
  await browser.close();
  const preferences=path.join(root,'browser-profile-'+process.pid,'Default','Preferences');
  const prefs=JSON.parse(fs.readFileSync(preferences,'utf8'));prefs.partition??={};
  prefs.partition.default_zoom_level={x:Math.log(2)/Math.log(1.2)};
  prefs.partition.per_host_zoom_levels={};fs.writeFileSync(preferences,JSON.stringify(prefs));
  browser=await require('./coverage.cjs').launch(pp,{executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:process.env.LYTRONAUT_GPU_TRACE!=='1'&&process.env.LYTRONAUT_CHROME_HEADLESS!=='0',userDataDir:path.join(root,'browser-profile-'+process.pid),args:chromeArgs});
  page=await browser.newPage();attach(page);await instrument(page);await page.setViewport({width:1440,height:1000});await page.goto(origin);await page.waitForSelector('#directory');
  const zoom=await page.evaluate(()=>({inner:innerWidth,outer:outerWidth,dpr:devicePixelRatio}));
  assert(Math.abs(zoom.dpr/baseline.dpr-2)<0.01,'actual browser200% DPR');
  assert(Math.abs(baseline.inner/zoom.inner-2)<0.01,'actual browser200% CSS viewport');
  if(!privateMode)console.error(await page.evaluate(()=>({viewport:innerWidth,scroll:document.documentElement.scrollWidth,overflow:[...document.querySelectorAll('*')].filter(e=>e.getBoundingClientRect().right>innerWidth+1).map(e=>e.tagName+'.'+e.className).slice(0,8)})));
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  const cdp=await page.createCDPSession();
  await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'reduce'}]);
  const reduced=await page.evaluate(()=>matchMedia('(prefers-reduced-motion: reduce)').matches);assert(reduced);
  await cdp.send('Emulation.setEmulatedMedia',{features:[{name:'forced-colors',value:'active'}]});
  assert(await page.evaluate(()=>matchMedia('(forced-colors: active)').matches));
  const ax=await cdp.send('Accessibility.getFullAXTree');
  assert(ax.nodes.some(n=>n.role?.value==='navigation'&&n.name?.value==='Library and albums'));
  assert(ax.nodes.some(n=>n.role?.value==='status'));assert(!ax.nodes.some(n=>n.role?.value==='button'&&n.name?.value==='Sign out'));assert(ax.nodes.some(n=>n.role?.value==='searchbox'&&n.name?.value==='Search captures'));
  checkpoint='responsive';await page.setViewport({width:390,height:844});
  if(!privateMode)console.error(await page.evaluate(()=>({viewport:innerWidth,scroll:document.documentElement.scrollWidth,overflow:[...document.querySelectorAll('*')].filter(e=>e.getBoundingClientRect().right>innerWidth+1).map(e=>e.tagName+'.'+e.className).slice(0,8)})));
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.keyboard.press('Tab');const focus=await page.evaluate(()=>({name:document.activeElement.textContent||document.activeElement.getAttribute('aria-label'),outline:getComputedStyle(document.activeElement).outlineWidth}));
  assert.notEqual(focus.outline,'0px');
  checkpoint='final-direct-entry';assert.equal(await page.$('#username'),null);
  const direct=await page.evaluate(async()=> (await fetch('/libraries')).status);assert.equal(direct,200);
  await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));await page.waitForFunction(()=>window.__urls.created===window.__urls.revoked);assert.equal(await page.evaluate(()=>window.__activeURLs.size),0,'pagehide releases every owned Blob URL');
  assert.equal(errors.length,0);assert(!responses.includes(429));const urls=await page.evaluate(()=>window.__urls);assert.equal(urls.created,urls.revoked);
  const result={mode:privateMode?'private-aggregate':'synthetic',revision:process.env.LYTRONAUT_TEST_REVISION||'working-tree',artifactProfiles:process.env.LYTRONAUT_ARTIFACT_PROFILES||(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1'?'native release; wasm release':'native debug; wasm debug'),performanceGateEnabled:process.env.LYTRONAUT_RELEASE_BENCHMARK==='1',startupMs,gpuProof,preZoomGPUDevices,sidebarProof,focusProof,focusUiLoadContendedTimes,focusUiTimes,faultSuite:!privateMode,flowProofs,videoHttpRustUi:!privateMode,videoUiProof,keyboardNavigation:true,contrast,preZoomURLProof,displayedBlobOwnershipVerified:true,captures:seen,albums:albumCount,orderedMemberships:memberships,assets429:responses.filter(s=>s===429).length,blobURLsReleased:true,absentGPUAndCodecInjected:!gpuMode,pageErrors:errors.length,reducedMotion:reduced,forcedColors:true,AXTree:true,responsive:true,zoom,browser:await browser.version(),screenReader:'AX inspection only; VoiceOver not manually certified'};
  const clientDir=process.env.LYTRONAUT_CLIENT||'dist';const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
  result.applicationWasmSha256=hash(path.join(clientDir,'lytronaut_client_bg.wasm'));result.generatedGlueSha256=hash(path.join(clientDir,'lytronaut_client.js'));if(binary)result.nativeServiceSha256=hash(binary);
  console.log(JSON.stringify(result));
  fs.writeFileSync(process.env.LYTRONAUT_BROWSER_REPORT||path.join(root,privateMode?'browser-aggregate.json':'browser-results.json'),JSON.stringify(result,null,2),{mode:0o600});
 }finally{if(browser)await browser.close();fs.rmSync(path.join(root,'browser-profile-'+process.pid),{recursive:true,force:true});await stopServer().catch(e=>{console.error(e.message);process.exitCode=1;});}
})().catch(e=>{console.error(privateMode?'Browser validation failed at '+checkpoint+' '+(e.stack?.match(/browser-test.cjs:\d+:\d+/)?.[0]||'')+' (details redacted)':'checkpoint '+checkpoint+' '+e.stack);process.exitCode=1;});
