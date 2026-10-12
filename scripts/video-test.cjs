// MIT, independently authored test harness. Product parsing/decoding is Rust/WASM.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const crypto = require('node:crypto');
const {execFileSync} = require('node:child_process');
const pp = require('./browser/node_modules/puppeteer-core');
const privateRoot = process.env.LYTRONAUT_VIDEO_PRIVATE_ROOT;
const scratch = fs.mkdtempSync(path.join(os.tmpdir(),'lytronaut-video-')); fs.chmodSync(scratch,0o700);
const dist = path.resolve(process.env.LYTRONAUT_CLIENT || 'dist');
const fixturePath=path.join(__dirname,'../fixtures/public/avc-colors.json');
const authoredFixture=fs.existsSync(fixturePath)?Buffer.from(JSON.parse(fs.readFileSync(fixturePath,'utf8')).container_base64,'base64'):null;
let fixtures = [], sealedSources = [], sourceSnapshot = new Map();
if(privateRoot) {
 try {
  // Read-only sealed catalog. Private rows remain in process memory, never printed.
  const source = `import sqlite3,json,sys\nc=sqlite3.connect('file:'+sys.argv[1]+'/app.db?mode=ro',uri=True)\nc.execute('PRAGMA query_only=ON')\nmetadata={}\nfor (raw,) in c.execute("SELECT original FROM catalog_records WHERE kind='picture_metadata'"):\n f=json.loads(raw)['fields'];metadata[bytes(f['uuid']['Text']).decode()]=int(f['lfe_major']['Integer'])\nrows=list(c.execute("SELECT digest,length,path FROM catalog_sources ORDER BY path"))\nprint(json.dumps({'sources':[(str(d)+'-'+str(n)) for d,n,p in rows], 'fixtures':[{'name':str(d)+'-'+str(n),'variant':metadata.get(p.split('/')[-2],0)} for d,n,p in rows if p.endswith('/stack.lfp')]}))`;
  const data=JSON.parse(execFileSync('python3',['-c',source,privateRoot],{encoding:'utf8'}));
  fixtures=data.fixtures.map(f=>({file:path.join(privateRoot,'originals',f.name),variant:f.variant}));
  sealedSources=data.sources;
 } catch { console.error(JSON.stringify({checkpoint:'private-catalog',error:'sealed catalog unreadable'}));process.exit(1); }
}
const server = http.createServer((req,res)=>{
  if(req.url==='/') {res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Rust codec acceptance</title>');return;}
  if(req.url==='/synthetic'&&authoredFixture){res.setHeader('Content-Type','application/octet-stream');res.end(authoredFixture);return;}
  const match = /^\/fixture\/(\d+)$/.exec(req.url);
  if(match && fixtures[+match[1]]) {res.setHeader('Content-Type','application/octet-stream');fs.createReadStream(fixtures[+match[1]].file).on('error',()=>{res.statusCode=500;res.end();}).pipe(res);return;}
  const name = req.url.slice(1);
  if(!['lytronaut_client.js','lytronaut_client_bg.wasm'].includes(name)) {res.statusCode=404;res.end();return;}
  res.setHeader('Content-Type',name.endsWith('.wasm')?'application/wasm':'text/javascript');fs.createReadStream(path.join(dist,name)).on('error',()=>{res.statusCode=500;res.end();}).pipe(res);
});
async function verifySealed(after=false) {
  for(const name of new Set(sealedSources)) {
    const file=path.join(privateRoot,'originals',name),stat=fs.statSync(file), split=name.lastIndexOf('-');
    const expectedSize=Number(name.slice(split+1));if(stat.size!==expectedSize||(stat.mode&0o777)!==0o400)throw Error('sealed original admission failed');
    const hash=crypto.createHash('sha256');for await(const chunk of fs.createReadStream(file))hash.update(chunk);
    if(hash.digest('hex')!==name.slice(0,split))throw Error('sealed original integrity failed');
    const fingerprint=[stat.size,stat.mtimeMs,stat.mode].join(':');
    if(after&&sourceSnapshot.get(name)!==fingerprint)throw Error('sealed original changed');sourceSnapshot.set(name,fingerprint);
  }
  return {logicalFiles:sealedSources.length,uniqueBlobs:new Set(sealedSources).size,logicalBytes:sealedSources.reduce((n,name)=>n+Number(name.slice(name.lastIndexOf('-')+1)),0),sha256SizeMtimeUnchanged:true};
}
(async()=>{
  let browser; let checkpoint='startup';
  try {
    if(privateRoot){checkpoint='sealed-precheck';await verifySealed();}
    await new Promise(r=>server.listen(0,'127.0.0.1',r));
    browser=await pp.launch({executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,userDataDir:path.join(scratch,'profile')});
    const page=await browser.newPage();const errors=[];page.on('pageerror',()=>errors.push('pageerror'));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    checkpoint=privateRoot?'private-corpus':'public-codec';
    const result=await page.evaluate(async({count,privateMode,variants,authored})=>{
      const module=await import('/lytronaut_client.js');await module.default();
      const invoke=async(bytes,index=0,signal=new AbortController().signal)=>JSON.parse(await module.decode_archived_refocus(bytes,index,signal));
      let closed=0,outputs=0,maxQueue=0,configs=[];
      const nativeDecoder=VideoDecoder, nativeClose=VideoFrame.prototype.close;
      VideoFrame.prototype.close=function(){closed++;return nativeClose.call(this);};
      class Tracked extends nativeDecoder {
        constructor(init){super({output:f=>{outputs++;init.output(f);},error:e=>{init.error(e);}});}
        configure(c){if('description' in c)throw Error('Annex-B description forbidden');configs.push(c.hardwareAcceleration);super.configure(c);}
        decode(chunk){super.decode(chunk);maxQueue=Math.max(maxQueue,this.decodeQueueSize);if(this.decodeQueueSize>2)throw Error('unbounded queue');}
      }
      window.VideoDecoder=Tracked;
      let blocks=0,frames=0,retries=0,reps=0,syntheticContainer=null,encoderFrames=0;const repeatedVariants=new Set(); const dpbHistogram={}; const recordDpb=r=>{const key=[r.coded_width,r.coded_height,r.reference_frames,r.bidirectional_frames>0].join("x");dpbHistogram[key]=(dpbHistogram[key]||0)+1;};
      if(privateMode) {
        for(let i=0;i<count;i++) {
          const bytes=new Uint8Array(await (await fetch('/fixture/'+i)).arrayBuffer());
          let result;try {result=await invoke(bytes);} catch(e) {return {blocks,frames,failure:String(e),privateMode:true,actualRustWasm:true};}recordDpb(result);blocks++;frames+=result.frames;retries+=Number(result.software_retry);
          if(result.frames===5&&[10,11].includes(variants[i])&&!repeatedVariants.has(variants[i])){await invoke(bytes);repeatedVariants.add(variants[i]);reps++;}
          result.image=''; // private pixels never cross the browser-tool boundary.
        }
        if(blocks!==109||frames!==654||reps!==2)throw Error('private full corpus count mismatch');
      } else {
        // Checked-in independently authored fixture avoids requiring a browser encoder.
        let bytes;const canvas=document.createElement('canvas');canvas.width=32;canvas.height=32;
        const context=canvas.getContext('2d');
        if(authored){bytes=new Uint8Array(await(await fetch('/synthetic')).arrayBuffer());}
        else {const chunks=[];let encoderError=false;
        const encoder=new VideoEncoder({output:c=>{const b=new Uint8Array(c.byteLength);c.copyTo(b);chunks.push(b);},error:()=>encoderError=true});
        encoder.configure({codec:'avc1.42001e',width:32,height:32,framerate:30,latencyMode:'realtime',hardwareAcceleration:'prefer-software',avc:{format:'annexb'}});
        for(let i=0;i<5;i++){context.fillStyle=['red','green','blue','yellow','white'][i];context.fillRect(0,0,32,32);const f=new VideoFrame(canvas,{timestamp:i*1000});encoder.encode(f,{keyFrame:i===0});f.close();encoderFrames++;}
        await encoder.flush();encoder.close();if(encoderError||chunks.length!==5)throw Error('synthetic encoder failed');
        const data=new Uint8Array(chunks.reduce((n,c)=>n+c.length+6,0));let pos=0;for(const c of chunks){data.set([0,0,0,1,9,0xf0],pos);pos+=6;data.set(c,pos);pos+=c.length;}
        const sha=async b=>'sha1-'+[...new Uint8Array(await crypto.subtle.digest('SHA-1',b))].map(v=>v.toString(16).padStart(2,'0')).join('');
        const directory=new TextEncoder().encode(JSON.stringify({version:{major:1,minor:0},picture:{accelerationArray:[{type:'com.lytro.acceleration.refocusStack',vendorContent:{blockOfImages:{representation:'h264',blockOfImagesRef:await sha(data),metadataArray:chunks.map((_,i)=>({width:32,height:32,lambda:[0,3,-2,1,2][i]}))}}}]}}));
        let framed=[137,76,70,80,13,10,26,10,0,0,0,1,0,0,0,0];
        for(const [isDir,bytes] of [[true,directory],[false,data]]) {const header=new Uint8Array(96);header.set([137,76,70,isDir?77:67,13,10,26,10]);new DataView(header.buffer).setUint32(12,bytes.length);header.set(new TextEncoder().encode(await sha(bytes)),16);framed.push(...header,...bytes);while(framed.length%16)framed.push(0);}
        bytes=new Uint8Array(framed);}
        syntheticContainer=Array.from(bytes);const first=await invoke(bytes,2);if(first.frames!==5||first.width!==32||first.height!==32)throw Error('synthetic decode count');
        const raw=Uint8Array.from(atob(first.image.split(',')[1]),c=>c.charCodeAt(0));const generatedHash=[...new Uint8Array(await crypto.subtle.digest('SHA-256',raw))].map(v=>v.toString(16).padStart(2,'0')).join('');if(first.content_sha256!==generatedHash||first.encoded_bytes!==raw.length)throw Error('generated JPEG integrity metadata mismatch');
        const image=new Image();image.src=first.image;await image.decode();context.drawImage(image,0,0);const pixel=context.getImageData(16,16,1,1).data;if(pixel[2]<200||pixel[0]>40||pixel[1]>40)throw Error('selected colored plane mismatch');
        recordDpb(first);blocks=1;frames=5;retries=Number(first.software_retry);
        const reject=async fn=>{let rejected=false;try{await fn();}catch{rejected=true;}if(!rejected)throw Error('fault accepted');};
        window.VideoDecoder=undefined;await reject(()=>invoke(bytes));window.VideoDecoder=Tracked;
        class Unsupported extends Tracked {static async isConfigSupported(){return {supported:false};}}window.VideoDecoder=Unsupported;await reject(()=>invoke(bytes));
        class ThrowSupport extends Tracked {static isConfigSupported(){throw Error('injected synchronous support error');}}window.VideoDecoder=ThrowSupport;await reject(()=>invoke(bytes));
        const probeAbort=new AbortController();class PendingSupport extends Tracked {static isConfigSupported(){return new Promise(()=>{});}}window.VideoDecoder=PendingSupport;setTimeout(()=>probeAbort.abort(),20);await reject(()=>invoke(bytes,0,probeAbort.signal()));
        class Retry extends Tracked {configure(c){this.preference=c.hardwareAcceleration;super.configure(c);}decode(c){if(this.preference==='no-preference')throw new DOMException('injected actual decode failure','EncodingError');super.decode(c);}}window.VideoDecoder=Retry;if(!(await invoke(bytes)).software_retry)throw Error('software retry missing');
        class Late extends Tracked {constructor(init){super({...init,output:f=>{init.output(f);init.error(new DOMException('injected late error','EncodingError'));}});}}window.VideoDecoder=Late;await reject(()=>invoke(bytes));window.VideoDecoder=Tracked;
        class Geometry extends Tracked {constructor(init){super({...init,output:f=>{Object.defineProperty(f,'displayWidth',{value:31});init.output(f);}});}}window.VideoDecoder=Geometry;await reject(()=>invoke(bytes));window.VideoDecoder=Tracked;
        const flushAbort=new AbortController();class PendingFlush extends Tracked {flush(){return new Promise(()=>{});}}window.VideoDecoder=PendingFlush;setTimeout(()=>flushAbort.abort(),20);await reject(()=>invoke(bytes,0,flushAbort.signal()));window.VideoDecoder=Tracked;
        const getContext=HTMLCanvasElement.prototype.getContext;HTMLCanvasElement.prototype.getContext=()=>null;await reject(()=>invoke(bytes));HTMLCanvasElement.prototype.getContext=getContext;
        const abort=new AbortController();abort.abort();await reject(()=>invoke(bytes,0,abort.signal()));
        const active=new AbortController();class Cancel extends Tracked {decode(c){super.decode(c);active.abort();}}window.VideoDecoder=Cancel;await reject(()=>invoke(bytes,0,active.signal()));window.VideoDecoder=Tracked;
        const corrupt=bytes.slice();corrupt[corrupt.length-10]^=1;await reject(()=>invoke(corrupt));
        const final=await invoke(bytes,4);if(final.index!==4||final.frames!==5)throw Error('reset recovery failed');
      }
      // Encoder-created frames are separate from decoder outputs.
      if(privateMode&&closed!==outputs)throw Error('VideoFrame leak');
      if(!privateMode&&closed!==outputs+encoderFrames)throw Error('VideoFrame leak');
      return {blocks,frames,retries,repeatedFiveFrameRepresentatives:reps,repeatedFiveFrameVariants:[...repeatedVariants].sort(),dpbHistogram,decoderOutputs:outputs,closedFrames:closed,maxDecodeQueue:maxQueue,pageErrors:0,actualRustWasm:true,privateMode,syntheticContainer,userAgent:navigator.userAgent};
    },{count:fixtures.length,privateMode:!!privateRoot,variants:fixtures.map(f=>f.variant),authored:!!authoredFixture});
    const sealedIntegrity=privateRoot?await verifySealed(true):null;
    if(errors.length)throw Error('uncaught page errors');
    if(result.syntheticContainer&&process.env.LYTRONAUT_VIDEO_FIXTURE_OUT)fs.writeFileSync(process.env.LYTRONAUT_VIDEO_FIXTURE_OUT,Buffer.from(result.syntheticContainer),{mode:0o600});delete result.syntheticContainer;
    const report={...result,sealedIntegrity,browser:await browser.version(),artifactProfiles:process.env.LYTRONAUT_VIDEO_PROFILE||'wasm debug; native service not used',os:process.platform==='darwin'?{version:execFileSync('sw_vers',['-productVersion'],{encoding:'utf8'}).trim(),build:execFileSync('sw_vers',['-buildVersion'],{encoding:'utf8'}).trim(),hardware:execFileSync('sysctl',['-n','hw.model'],{encoding:'utf8'}).trim(),cpu:execFileSync('sysctl',['-n','machdep.cpu.brand_string'],{encoding:'utf8'}).trim()}: {platform:process.platform,arch:process.arch},revision:process.env.LYTRONAUT_TEST_REVISION||'working-tree',applicationWasmSha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(dist,'lytronaut_client_bg.wasm'))).digest('hex'),generatedGlueSha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(dist,'lytronaut_client.js'))).digest('hex')};
    const destination=process.env.LYTRONAUT_VIDEO_REPORT||path.join(scratch,'aggregate.json');fs.writeFileSync(destination,JSON.stringify(report,null,2)+'\n',{mode:0o600});console.log(JSON.stringify(report));if(report.failure)process.exitCode=1;
  } catch (e) {if(!privateRoot)console.error(String(e));console.error(JSON.stringify({checkpoint,error:'video acceptance failed',reason:['invalid video container','invalid video metadata','video stack missing','unsupported video stack','invalid Annex-B stream','video stream extent mismatch','video stream metadata mismatch','video stream unsupported syntax','video stream budget','video stream truncated','decode failed','incomplete decode','unsupported config','selected frame missing','frame encode failed'].includes(String(e))?String(e):'browser or harness failure',privateMode:!!privateRoot}));process.exitCode=1;}
  finally {if(browser)await browser.close();if(server.listening)await new Promise(r=>server.close(r));fs.rmSync(path.join(scratch,'profile'),{recursive:true,force:true});}
})();
