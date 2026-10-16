// MIT opt-in local sealed-corpus TEST tooling. No private data is written or
// returned from the browser; stdout/errors contain only aggregate counters.
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const pp=require('./browser/node_modules/puppeteer-core');
const sealed=process.env.LYTRONAUT_PRIVATE_SEALED_ROOT,dist=path.resolve(process.env.LYTRONAUT_CLIENT||'dist');
const expected=109,sourceCap=8*1024*1024,started=performance.now(),snapshot=new Map();let sources=[],fixtures=[],browser,server,checkpoint='admission';
const scratch=fs.mkdtempSync(path.join(os.tmpdir(),'lytronaut-private-live-photo-'));fs.chmodSync(scratch,0o700);
const report={denominator:expected,complete:false,photosGuiAcceptancePending:true,countedApplicationMemoryAcceptancePending:true,opaqueBrowserMemoryMeasured:false,privateResourcesSaved:false};
async function integrity(after=false){
  let checked=0,failed=0;
  for(const name of new Set(sources)){
    try{
      if(!/^[0-9a-f]{64}-[0-9]+$/.test(name))throw Error('sealed integrity failure');
      const file=path.join(sealed,'originals',name),stat=fs.lstatSync(file),split=name.lastIndexOf('-'),hash=crypto.createHash('sha256');
      if(!stat.isFile()||stat.isSymbolicLink()||stat.size!==Number(name.slice(split+1))||(stat.mode&0o777)!==0o400)throw Error('sealed integrity failure');
      for await(const chunk of fs.createReadStream(file))hash.update(chunk);
      if(hash.digest('hex')!==name.slice(0,split))throw Error('sealed integrity failure');
      const fingerprint=[stat.size,stat.mtimeMs,stat.mode].join(':');if(after&&snapshot.get(name)!==fingerprint)throw Error('sealed integrity failure');
      if(!after)snapshot.set(name,fingerprint);checked++;
    }catch{failed++;}
  }
  return {logicalFiles:sources.length,uniqueBlobs:new Set(sources).size,checkedBlobs:checked,failedBlobs:failed,sha256SizeMtimeUnchanged:failed===0};
}
(async()=>{
  try{
    if(!sealed||! /^[0-9a-f]{40}$/.test(process.env.LYTRONAUT_TEST_REVISION||''))throw Error('explicit sealed root and frozen revision required');
    for(const dir of [sealed,path.join(sealed,'originals'),dist]){const stat=fs.lstatSync(dir);if(!stat.isDirectory()||stat.isSymbolicLink())throw Error('directory admission failed');}
    const database=fs.lstatSync(path.join(sealed,'app.db'));if(!database.isFile()||database.isSymbolicLink())throw Error('catalog admission failed');
    // Operator checks the frozen manifest's exact source/feature/hash binding.
    // Hash agreement is required before any sealed catalog/source is opened.
    const programs=[['lytronaut_client_bg.wasm','LYTRONAUT_TEST_WASM_SHA256'],['lytronaut_client.js','LYTRONAUT_TEST_GLUE_SHA256']];
    for(const [name,key]of programs){const file=path.join(dist,name),stat=fs.lstatSync(file),expectedHash=process.env[key];if(!stat.isFile()||stat.isSymbolicLink()||stat.size>64*1024*1024||! /^[0-9a-f]{64}$/.test(expectedHash||'')||crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')!==expectedHash)throw Error('frozen program admission failed');}
    // Same read-only catalog discovery as video-test. Private rows remain in
    // process memory; captured stdout is never logged or persisted.
    const query="import sqlite3,json,sys\nc=sqlite3.connect('file:'+sys.argv[1]+'/app.db?mode=ro&immutable=1',uri=True)\nc.execute('PRAGMA query_only=ON')\nrows=list(c.execute('SELECT digest,length,path FROM catalog_sources ORDER BY path'))\nprint(json.dumps({'sources':[str(d)+'-'+str(n) for d,n,p in rows], 'fixtures':[str(d)+'-'+str(n) for d,n,p in rows if p.endswith('/stack.lfp')]}))";
    const catalog=JSON.parse(execFileSync('python3',['-c',query,sealed],{encoding:'utf8',stdio:['ignore','pipe','pipe']}));sources=catalog.sources;fixtures=catalog.fixtures;
    report.discovered=fixtures.length;if(fixtures.length!==expected)throw Error('corpus denominator mismatch');
    checkpoint='sealed-precheck';report.sealedPrecheck=await integrity();if(!report.sealedPrecheck.sha256SizeMtimeUnchanged)throw Error('sealed integrity failure');
    server=http.createServer((req,res)=>{
      res.setHeader('Cache-Control','no-store');if(req.method!=='GET'){res.writeHead(405);res.end();return;}
      if(req.url==='/'){res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Local sealed Live Photo verification</title>');return;}
      if(req.url==='/pair.js'){res.setHeader('Content-Type','text/javascript');fs.createReadStream(path.join(__dirname,'live-photo-pair.js')).on('error',()=>res.destroy()).pipe(res);return;}
      const match=/^\/fixture\/(\d+)$/.exec(req.url);
      if(match&&Number(match[1])<fixtures.length){const file=path.join(sealed,'originals',fixtures[Number(match[1])]);let stat;try{stat=fs.lstatSync(file);}catch{res.writeHead(404);res.end();return;}if(!stat.isFile()||stat.isSymbolicLink()||stat.size<=0||stat.size>sourceCap){res.writeHead(413);res.end();return;}res.setHeader('Content-Length',stat.size);res.setHeader('Content-Type','application/octet-stream');fs.createReadStream(file,{start:0,end:stat.size-1}).on('error',()=>res.destroy()).pipe(res);return;}
      const name=req.url.slice(1);if(!['lytronaut_client.js','lytronaut_client_bg.wasm'].includes(name)){res.writeHead(404);res.end();return;}
      res.setHeader('Content-Type',name.endsWith('.wasm')?'application/wasm':'text/javascript');fs.createReadStream(path.join(dist,name)).on('error',()=>res.destroy()).pipe(res);
    });
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const origin=`http://127.0.0.1:${server.address().port}`;
    browser=await require('./coverage.cjs').launch(pp,{executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,userDataDir:path.join(scratch,'profile'),protocolTimeout:3600000,args:['--disable-background-networking','--disable-component-update','--no-first-run']});
    const page=await browser.newPage();let pageErrors=0,externalRequests=0;page.on('pageerror',()=>pageErrors++);await page.setRequestInterception(true);
    page.on('request',request=>{if(request.method()!=='GET'||(!['data:','blob:'].includes(new URL(request.url()).protocol)&&new URL(request.url()).origin!==origin)){externalRequests++;request.abort().catch(()=>{});}else request.continue().catch(()=>{});});
    await page.goto(origin);await page.addScriptTag({url:origin+'/pair.js'});checkpoint='private-export';
    Object.assign(report,await page.evaluate(async({count})=>{
      const module=await import('/lytronaut_client.js');if('harness_dispose'in module)throw Error('production artifact required');
      const wasm=await module.default(),initialWasmBytes=wasm.memory.buffer.byteLength;
      const exportStarted=performance.now(),native=VideoDecoder,close=VideoFrame.prototype.close;let decoderCreated=0,decoderClosed=0,outputs=0,closed=0,maxQueue=0,maxImageDataBytes=0;
      const getPixels=CanvasRenderingContext2D.prototype.getImageData;CanvasRenderingContext2D.prototype.getImageData=function(x,y,w,h,...rest){maxImageDataBytes=Math.max(maxImageDataBytes,w*h*4);return getPixels.call(this,x,y,w,h,...rest);};
      VideoFrame.prototype.close=function(){closed++;return close.call(this);};
      class Tracked extends native{constructor(init){super({...init,output:f=>{outputs++;init.output(f);}});decoderCreated++;this.closedByHarness=false;}close(){if(!this.closedByHarness){decoderClosed++;this.closedByHarness=true;}return super.close();}decode(chunk){super.decode(chunk);maxQueue=Math.max(maxQueue,this.decodeQueueSize);if(this.decodeQueueSize>2)throw Error('queue bound');}}
      window.VideoDecoder=Tracked;
      const summary={attemptedStreams:0,processed:0,failedStreams:0,failedExports:0,failurePhases:{},exportElapsedMs:0,minExportElapsedMs:null,maxExportElapsedMs:0,exportAttempts:0,exportsVerified:0,cancelledBefore:0,cancelledDuring:0,recovered:0,sourceFrames:0,maxSourceBytes:0,maxBundleBytes:0,dpbHistogram:{},geometryHistogram:{},outputFrameHistogram:{},stillLoss:{absoluteSum:0,squaredSum:0,samples:0,maxDifference:0},motionLoss:{absoluteSum:0,squaredSum:0,samples:0,maxDifference:0}};
      const failure=phase=>{summary.failurePhases[phase]=(summary.failurePhases[phase]||0)+1;};
      const loss=(target,value)=>{for(const key of ['absoluteSum','squaredSum','samples'])target[key]+=value[key];target.maxDifference=Math.max(target.maxDifference,value.maxDifference);};
      const invoke=(bytes,selected,signal=new AbortController().signal)=>module.export_archived_live_photo(bytes,selected,crypto.randomUUID(),signal);
      const readSource=async(index,signal)=>{
        const response=await fetch('/fixture/'+index,{signal});if(!response.ok)throw Error('fixture unavailable');
        const length=Number(response.headers.get('content-length'));if(!Number.isSafeInteger(length)||length<=0||length>8*1024*1024||!response.body)throw Error('source extent');
        const reader=response.body.getReader(),bytes=new Uint8Array(length);let offset=0;
        try{for(;;){const part=await reader.read();if(part.done)break;if(part.value.length>length-offset)throw Error('source extent');bytes.set(part.value,offset);offset+=part.value.length;}if(offset!==length)throw Error('source extent');return bytes;}
        finally{await reader.cancel().catch(()=>{});reader.releaseLock();}
      };
      const cancelled=async(operation,signal)=>{try{await operation();}catch(error){if(String(error)==='cancelled'&&signal.aborted)return;throw Error('unexpected cancellation error');}throw Error('cancelled export returned resources');};
      for(let index=0;index<count;index++){
        summary.attemptedStreams++;let bytes,first,streamFailed=false,phase='source-fetch';
        try{
          const fetchAbort=new AbortController(),timer=setTimeout(()=>fetchAbort.abort(),30000);
          try{bytes=await readSource(index,fetchAbort.signal);}finally{clearTimeout(timer);}
          summary.maxSourceBytes=Math.max(summary.maxSourceBytes,bytes.length);phase='source-decode';
          first=JSON.parse(await module.decode_archived_refocus(bytes,0,new AbortController().signal));first.image='';summary.sourceFrames+=first.frames;
          const dpb=[first.coded_width,first.coded_height,first.reference_frames,first.bidirectional_frames>0].join('x');summary.dpbHistogram[dpb]=(summary.dpbHistogram[dpb]||0)+1;
          const geometry=[first.width,first.height].join('x');summary.geometryHistogram[geometry]=(summary.geometryHistogram[geometry]||0)+1;
          phase='pre-cancellation';const aborted=new AbortController();aborted.abort();await cancelled(()=>invoke(bytes,0,aborted.signal),aborted.signal);summary.cancelledBefore++;
          phase='active-cancellation';const active=new AbortController();class Cancel extends Tracked{decode(chunk){super.decode(chunk);active.abort();}}window.VideoDecoder=Cancel;
          try{await cancelled(()=>invoke(bytes,0,active.signal),active.signal);summary.cancelledDuring++;}finally{window.VideoDecoder=Tracked;}
          let streamExports=0;
          for(const selected of new Set([0,Math.floor(first.frames/2),first.frames-1])){
            summary.exportAttempts++;let reference,zip;const attemptStarted=performance.now();
            try{
              phase='selected-source-decode';reference=JSON.parse(await module.decode_archived_refocus(bytes,selected,new AbortController().signal));const identifier=crypto.randomUUID();
              phase='pair-export';zip=await module.export_archived_live_photo(bytes,selected,identifier,new AbortController().signal);
              phase='independent-pair';const proof=await window.verifyLivePhotoPair(zip,{identifier,referenceImage:reference.image});
              phase='source-order';if(proof.selected!==selected||proof.width!==first.width||proof.height!==first.height||!proof.sweep.includes(selected)||!proof.sweep.every((v,i)=>Number.isInteger(v)&&v>=0&&v<first.frames&&(!i||first.lambdas[proof.sweep[i-1]]<first.lambdas[v])))throw Error('selected source order mismatch');
              const ordered=first.lambdas.map((lambda,i)=>({lambda,i})).sort((a,b)=>a.lambda-b.lambda);if(proof.sweep[0]!==ordered[0].i||proof.sweep.at(-1)!==ordered.at(-1).i)throw Error('sweep endpoints mismatch');
              summary.maxBundleBytes=Math.max(summary.maxBundleBytes,zip.length);summary.outputFrameHistogram[proof.frames]=(summary.outputFrameHistogram[proof.frames]||0)+1;loss(summary.stillLoss,proof.stillLoss);loss(summary.motionLoss,proof.motionLoss);summary.exportsVerified++;streamExports++;
            }catch{streamFailed=true;summary.failedExports++;failure(phase);}
            finally{const elapsed=performance.now()-attemptStarted;summary.exportElapsedMs+=elapsed;summary.minExportElapsedMs=Math.min(summary.minExportElapsedMs??elapsed,elapsed);summary.maxExportElapsedMs=Math.max(summary.maxExportElapsedMs,elapsed);if(reference)reference.image='';reference=null;zip=null;}
          }
          if(streamExports)summary.recovered++;summary.processed++;
        }catch{streamFailed=true;failure(phase);}
        finally{
          window.VideoDecoder=Tracked;if(first)first.image='';first=null;bytes=null;
          if(decoderCreated!==decoderClosed||outputs!==closed){streamFailed=true;failure('decoder-ownership');}
          if(streamFailed)summary.failedStreams++;
        }
      }
      summary.corpusComplete=summary.attemptedStreams===count&&summary.processed===count&&summary.failedStreams===0&&summary.sourceFrames===654&&summary.cancelledBefore===count&&summary.cancelledDuring===count&&summary.recovered===count&&maxImageDataBytes<=128*1024&&decoderCreated===decoderClosed&&outputs===closed;
      return {...summary,corpusElapsedMs:performance.now()-exportStarted,decoderCreated,decoderClosed,decodedFrames:outputs,closedFrames:closed,maxDecodeQueue:maxQueue,maxImageDataBytes,initialWasmBytes,finalWasmHighWaterBytes:wasm.memory.buffer.byteLength,actualRustWasm:true};
    },{count:expected}));
    if(pageErrors||externalRequests)throw Error('browser isolation failure');report.pageErrors=pageErrors;report.externalRequests=externalRequests;
    report.browser=await browser.version();report.revision=process.env.LYTRONAUT_TEST_REVISION||'working-tree';report.productionArtifact=true;
    report.applicationWasmSha256=crypto.createHash('sha256').update(fs.readFileSync(path.join(dist,'lytronaut_client_bg.wasm'))).digest('hex');report.generatedGlueSha256=crypto.createHash('sha256').update(fs.readFileSync(path.join(dist,'lytronaut_client.js'))).digest('hex');report.complete=report.corpusComplete;if(!report.complete){report.failureCheckpoint='private-export';process.exitCode=1;}
  }catch{report.failureCheckpoint=checkpoint;process.exitCode=1;}
  finally{
    if(sources.length){try{report.sealedIntegrity=await integrity(true);}catch{report.sealedIntegrity={sha256SizeMtimeUnchanged:false};}if(!report.sealedIntegrity.sha256SizeMtimeUnchanged){report.complete=false;process.exitCode=1;}}
    if(browser)await browser.close().catch(()=>{});if(server?.listening)await new Promise(resolve=>server.close(resolve));fs.rmSync(scratch,{recursive:true,force:true});
    report.elapsedMs=performance.now()-started;console.log(JSON.stringify(report));
  }
})();
