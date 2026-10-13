// MIT standalone PUBLIC real-encoder experiment; requires the existing feature build.
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {spawn,execFile}=require('node:child_process');const {promisify}=require('node:util');
const pp=require('./browser/node_modules/puppeteer-core');
if(process.env.LYTRONAUT_VIDEO_PRIVATE_ROOT||process.env.LYTRONAUT_PRIVATE_SEALED_ROOT||process.env.LYTRONAUT_PRIVATE_TEST==='1')throw Error('public stress rejects private mode');
const evidence=path.resolve(process.env.LYTRONAUT_OWNER_EVIDENCE||'owner-evidence'),dist=path.resolve(process.env.LYTRONAUT_CLIENT||'dist-owner');
const target=path.resolve(process.env.CARGO_TARGET_DIR||'target');
const revision=process.env.LYTRONAUT_TEST_REVISION;assert(/^[0-9a-f]{40}$/.test(revision),'exact source required');
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const identity={revision,applicationWasmSha256:hash(path.join(dist,'application.wasm')),generatedWasmSha256:hash(path.join(dist,'lytronaut_client_bg.wasm')),artifactFeature:'test-harness',productionArtifact:false};
const ownerFile=path.join(evidence,'runtime.json');assert(fs.statSync(ownerFile).size<=256*1024);
const owner=JSON.parse(fs.readFileSync(ownerFile,'utf8'));
for(const key of ['revision','applicationWasmSha256','generatedWasmSha256','artifactFeature','productionArtifact'])assert.equal(owner[key],identity[key],'stress reuses exact successful owner feature/source identity');
const retryBytes=Buffer.from(JSON.parse(fs.readFileSync(path.join(__dirname,'../fixtures/public/avc-colors.json'),'utf8')).container_base64,'base64');assert(retryBytes.length>0&&retryBytes.length<=8*1024*1024);
const bounded=async(operation,ms)=>{let timer;try{return await Promise.race([operation,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('stress timeout')),ms);timer.unref();})]);}finally{clearTimeout(timer);}};
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
// Test observer only: public Node resource types, never handle objects or names.
const resourceTypes=['Timeout','Immediate','FSReqCallback','FSReqPromise','GetAddrInfoReqWrap','GetNameInfoReqWrap','PipeWrap','ProcessWrap','TCPServerWrap','TCPSocketWrap','TTYWrap','UDPWrap','MessagePort'];
function resourceSnapshot(){
  const resources=process.getActiveResourcesInfo(),counts=Object.fromEntries(resourceTypes.map(type=>[type,0]));let unknownCount=0;
  for(const type of resources.slice(0,256)){if(Object.hasOwn(counts,type))counts[type]++;else unknownCount++;}
  return {counts,unknownCount,complete:resources.length<=256};
}
const runFile=promisify(execFile);
async function stopProcess(child,signal,ms){
  if(child.exitCode!==null||child.signalCode!==null)return;
  if(!child.kill(signal))throw Error('owned process termination unavailable');
  await bounded(new Promise(resolve=>{if(child.exitCode!==null||child.signalCode!==null)resolve();else child.once('exit',resolve);}),ms);
}
async function catalog(root,bytes,libraryUUID){
  assert(['public-jpeg-stress-library','public-jpeg-retry-library'].includes(libraryUUID));
  fs.mkdirSync(root,{mode:0o700});fs.writeFileSync(path.join(root,'synthetic.video.lfp'),bytes,{mode:0o600,flag:'wx'});
  await runFile(path.join(target,'debug/examples/synthetic_jpeg'),[path.join(root,'synthetic.jpg')],{timeout:30000,maxBuffer:1024});
  await runFile('python3',[path.join(__dirname,'synthetic-library.py'),root],{timeout:30000,maxBuffer:1024});
  // Own generated fixture only: different original bytes belong to independent libraries.
  await runFile('python3',['-c','import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); n=c.execute("UPDATE event SET library_uuid=? WHERE library_uuid=?",(sys.argv[2],"synthetic-library")).rowcount; assert n==1; c.commit(); c.close()',path.join(root,'Synthetic.lytrolibrary','lytrolibrary.db'),libraryUUID],{timeout:30000,maxBuffer:1024});
  return path.join(root,'Synthetic.lytrolibrary');
}
async function trial(index,bitrate){
  const scratch=fs.mkdtempSync(path.join(os.tmpdir(),'lytronaut-jpeg-stress-'));fs.chmodSync(scratch,0o700);
  const origin=`http://127.0.0.1:${3020+index}`,data=path.join(scratch,'data');fs.mkdirSync(data,{mode:0o700});
  const report={trial:index,bitrate,width:2048,height:1024,sourceBytes:0,encodedBytes:0,authoredFrames:0,keyChunks:0,deltaChunks:0,outcome:'setup_failure',accepted:false,elapsedMs:0,...identity};
  let browser,service,exited=false,stopped=false,phase='setup_failure',cleanupFailed=false;const start=Date.now();
  try{
    service=spawn(path.join(target,'debug/lytronaut-service'),[],{env:{...process.env,LYTRONAUT_DATA:data,LYTRONAUT_DATABASE:'catalog.db',LYTRONAUT_ORIGIN:origin,LYTRONAUT_LISTEN:new URL(origin).host,LYTRONAUT_CLIENT:dist},stdio:'ignore'});
    service.on('exit',()=>{exited=true;});service.on('error',()=>{exited=true;});
    await bounded((async()=>{for(;;){if(stopped)throw Error('stress startup stopped');if(exited)throw Error('stress service unavailable');try{const response=await fetch(origin+'/healthz',{signal:AbortSignal.timeout(1000)});await response.body?.cancel();if(response.ok)return;}catch{}await wait(100);}})(),30000);
    browser=await require('./coverage.cjs').launch(pp,{executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,userDataDir:path.join(scratch,'profile'),timeout:30000});
    const page=await browser.newPage();page.setDefaultTimeout(30000);page.setDefaultNavigationTimeout(30000);
    let requestFailed=false,pageFailed=false;
    await page.setRequestInterception(true);page.on('request',request=>{try{const url=new URL(request.url());const localImage=url.protocol==='data:'&&request.url().startsWith('data:image/jpeg;base64,')&&request.url().length<=2*1024*1024;const operation=url.origin===origin||localImage?request.continue():request.abort();operation.catch(()=>{requestFailed=true;});}catch{requestFailed=true;request.abort().catch(()=>{});}});
    page.on('pageerror',()=>{pageFailed=true;});
    await page.evaluateOnNewDocument(()=>Object.defineProperty(navigator,'gpu',{value:undefined}));
    await page.goto(origin);await page.waitForSelector('#directory');
    await page.evaluate(()=>{window.__stressJson=async path=>{const response=await fetch(path,{signal:AbortSignal.timeout(10000)});if(!response.ok)throw Error('public experiment API unavailable');return response.json();};});
    await page.addScriptTag({content:fs.readFileSync(path.join(__dirname,'live-photo-fixture.js'),'utf8')});
    phase='author_failure';
    const authored=await bounded(page.evaluate(async bitrate=>{
      let result;
      try{result=await window.authorLivePhotoStressFixture(bitrate);}
      catch(error){return {unproven:['public fixture output budget','public fixture container budget'].includes(error.message)?'source_admission_unproven':'author_failure'};}
      if(result.bytes.length>8*1024*1024||result.authoredFrames!==5)throw Error('admitted real fixture required');
      window.__stressBytes=result.bytes;
      return {bytes:Array.from(result.bytes),encodedBytes:result.encodedBytes,authoredFrames:result.authoredFrames,keyChunks:result.keyChunks,deltaChunks:result.deltaChunks};
    },bitrate),60000);
    if(authored.unproven){report.outcome=authored.unproven;return report;}
    assert(Array.isArray(authored.bytes)&&authored.bytes.length>0&&authored.bytes.length<=8*1024*1024);
    assert(authored.bytes.every(v=>Number.isInteger(v)&&v>=0&&v<=255));
    for(const name of ['encodedBytes','authoredFrames','keyChunks','deltaChunks'])assert(Number.isSafeInteger(authored[name])&&authored[name]>=0&&authored[name]<=8*1024*1024);
    assert.equal(authored.authoredFrames,5);assert.equal(authored.keyChunks+authored.deltaChunks,5);
    Object.assign(report,{sourceBytes:authored.bytes.length,encodedBytes:authored.encodedBytes,authoredFrames:authored.authoredFrames,keyChunks:authored.keyChunks,deltaChunks:authored.deltaChunks});
    const stressBuffer=Buffer.from(authored.bytes),stressDigest=crypto.createHash('sha256').update(stressBuffer).digest('hex');
    const stressCatalog=await catalog(path.join(scratch,'stress'),stressBuffer,'public-jpeg-stress-library');authored.bytes=null;
    const retryCatalog=await catalog(path.join(scratch,'retry'),retryBytes,'public-jpeg-retry-library');
    const click=async text=>page.waitForFunction(text=>{const button=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===text&&!b.disabled);if(!button)return false;button.click();return true;},{polling:50},text);
    const importCatalog=async directory=>{
      Object.assign(report,{importKind:directory===stressCatalog?'stress':'retry',importPhase:'upload',completedLibraryFound:false,catalogState:'unobserved',targetCapturePresent:false,sourcePresent:false,digestMatch:false,sizeMatch:false});
      const previous=await page.evaluate(()=>sessionStorage.getItem('lytronaut-import'));
      await (await page.$('#directory')).uploadFile(directory);
      report.importPhase='commit';
      await page.waitForFunction(async previous=>{const id=sessionStorage.getItem('lytronaut-import');return id&&id!==previous&&!document.querySelector('#directory').disabled&&(await window.__stressJson('/imports/'+id)).phase==='catalog_complete';},{timeout:120000},previous);
      report.importPhase='library';
      const selected=await page.evaluate(async()=>{
        const progress=await window.__stressJson('/imports/'+sessionStorage.getItem('lytronaut-import'));
        const libraries=await window.__stressJson('/libraries?limit=24&offset=0');
        const index=libraries.items.findIndex(item=>item.id===progress.library);
        return {index,library:progress.library,catalogState:index<0?'unobserved':libraries.items[index].state};
      });
      report.completedLibraryFound=selected.index>=0;
      assert(['ready','quarantined','preserved_only','unobserved'].includes(selected.catalogState));report.catalogState=selected.catalogState;
      assert(report.completedLibraryFound,'completed import library required');
      report.importPhase='select';
      await click('Library '+(selected.index+1));
      await page.waitForFunction(index=>[...document.querySelectorAll('.rail-item')].some(button=>button.textContent==='Library '+(index+1)&&button.getAttribute('aria-pressed')==='true'),{},selected.index);
      await click('All captures');await page.select('#sort','name');
      const expectedDigest=directory===stressCatalog?stressDigest:crypto.createHash('sha256').update(retryBytes).digest('hex');
      report.importPhase='source';
      const verified=await page.evaluate(async({library,expectedDigest})=>{
        const captures=await window.__stressJson('/libraries/'+library+'/captures?sort=name&limit=24&offset=0');
        const target=captures.items.find(item=>item.name==='Synthetic 001');
        if(!target)return {targetCapturePresent:false,sourcePresent:false,digestMatch:false,sizeMatch:false};
        const detail=await window.__stressJson('/captures/'+target.id);const file=detail.originals.find(item=>item.role==='refocus_stack');
        if(!file)return {targetCapturePresent:true,sourcePresent:false,digestMatch:false,sizeMatch:false};
        const manifest=await window.__stressJson('/assets/'+file.id+'/manifest');
        return {targetCapturePresent:true,sourcePresent:true,digestMatch:manifest.content_sha256===expectedDigest,sizeMatch:manifest.encoded_bytes===file.encoded_bytes};
      },{library:selected.library,expectedDigest});
      Object.assign(report,verified);assert(verified.targetCapturePresent,'fresh target required');assert(verified.sourcePresent,'fresh source required');
      assert.equal(verified.digestMatch&&verified.sizeMatch,true,'fresh imported target source identity');
      report.importPhase='target';
      await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24&&[...document.querySelectorAll('.capture')].some(b=>b.querySelector('span')?.textContent==='Synthetic 001'));
      await page.evaluate(()=>[...document.querySelectorAll('.capture')].find(b=>b.querySelector('span')?.textContent==='Synthetic 001').click());
      await page.waitForFunction(()=>document.querySelector('.viewer-strip span')?.textContent==='Synthetic 001'&&[...document.querySelectorAll('.capture')].some(b=>b.getAttribute('aria-pressed')==='true'&&b.querySelector('span')?.textContent==='Synthetic 001')&&document.querySelector('#focus').disabled&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img').naturalWidth>0);
      report.importPhase='complete';
      return true;
    };
    phase='validation_failure';
    const proof=await require('./live-photo-jpeg-stress-page.cjs')({page,click,importCatalog,stressCatalog,retryCatalog,retryBytes:Array.from(retryBytes)});
    Object.assign(report,proof);
    assert(!requestFailed&&!pageFailed,'bounded public browser has no harness errors');
    report.accepted=authored.keyChunks===1&&authored.deltaChunks===4&&proof.crossed&&proof.viewerFallback&&proof.directRejected&&proof.cancelled&&proof.retryViewer&&proof.retryExport&&proof.complete;
    report.outcome=report.accepted?'proven_rejection':authored.keyChunks!==1||authored.deltaChunks!==4?'predictive_encoding_unproven':'jpeg_rejection_unproven';
  }catch(error){if(error.experiment)Object.assign(report,error.experiment);report.outcome=phase;report.errorKind=report.importPhase&&report.importPhase!=='complete'?'import_failure':'validation_failure';}
  finally{
    stopped=true;
    try{if(browser){try{await bounded(browser.close(),10000);}catch{cleanupFailed=true;const child=browser.process();if(child)await stopProcess(child,'SIGKILL',5000);}}}
    catch{cleanupFailed=true;}
    finally{
      try{if(service&&!exited)await stopProcess(service,'SIGINT',10000);}
      catch{cleanupFailed=true;try{if(service)await stopProcess(service,'SIGKILL',5000);}catch{cleanupFailed=true;}}
      finally{try{fs.rmSync(scratch,{recursive:true,force:true});}catch{cleanupFailed=true;}}
    }
    if(cleanupFailed){report.accepted=false;report.outcome='cleanup_failure';}
    report.elapsedMs=Date.now()-start;
  }
  return report;
}
(async()=>{
  const resourceBaseline=resourceSnapshot();
  // Always retain both fixed trial outcomes; no larger/adaptive search or injected output.
  const trials=[];for(const [index,bitrate] of [32000000,64000000].entries())trials.push(await trial(index,bitrate));
  const result={version:1,publicSynthetic:true,realEncoder:true,inputPath:'fixture-js-rust',totalMemoryProof:false,opaqueBrowserMemoryMeasured:false,accepted:trials.some(t=>t.accepted)&&trials.every(t=>['proven_rejection','jpeg_rejection_unproven','predictive_encoding_unproven','source_admission_unproven'].includes(t.outcome)),trials};
  await new Promise(resolve=>setImmediate(resolve));
  result.nodeResources={baseline:resourceBaseline,afterCleanup:resourceSnapshot()};
  const text=JSON.stringify(result);assert(text.length<=32*1024,'bounded numeric/fixed-enum stress report');
  fs.writeFileSync(path.join(evidence,'jpeg-stress.json'),text+'\n',{mode:0o600,flag:'wx'});
  console.log(text);if(!result.accepted)process.exitCode=1;
})().catch(()=>{console.error('Public JPEG stress experiment failed');process.exitCode=1;});
