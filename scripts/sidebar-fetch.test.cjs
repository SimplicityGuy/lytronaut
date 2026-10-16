const assert=require('node:assert/strict');
const test=require('node:test');
const {EventEmitter}=require('node:events');
const install=require('./sidebar-fetch.cjs');
const fence=require('./sidebar-request.cjs');
const origin='http://localhost',id='public-fixture',url=origin+'/captures/'+id;
const payload=(captureId=id)=>JSON.stringify({capture:{id:captureId},originals:[{role:'jpeg_focus_stack'}]});
const response=(body=payload(),status=200)=>new Response(body,{status});
async function settled(probe,token){
 const deadline=Date.now()+2000;
 while(probe.snapshot(token)?.requests.some(row=>!row.done)){
  assert(Date.now()<deadline,'bounded body observer must settle');
  await new Promise(resolve=>setImmediate(resolve));
 }
}

test('complete logical Fetch body remains provable after the observed late CDP abort',async()=>{
 const scope={fetch:async()=>response()},probe=install(origin,scope),token=probe.arm(id);
 const page=Object.assign(new EventEmitter(),{getDefaultTimeout:()=>10});
 const request={url:()=>url,method:()=> 'GET',failure:()=>({errorText:'net::ERR_ABORTED'})};
 const headers={request:()=>request,status:()=>200};request.response=()=>headers;
 const transport=fence(page,url);page.emit('request',request);
 const actual=await scope.fetch(new Request(url));page.emit('response',headers);
 assert.equal((await actual.json()).capture.id,id,'application receives the original complete response');
 await settled(probe,token);page.emit('requestfailed',request);
 assert.equal(probe.ready(token).captureId,id);assert.equal(probe.ready(token).focus,true);
 assert.equal(probe.snapshot(token).seen,1,'no replacement request or retry');
 await assert.rejects(transport.promise,/deadline/);assert.equal(page.eventNames().length,0);probe.dispose();
});

test('pre-arm calls, wrong URL/query and non-GET calls cannot supply a body witness',async()=>{
 let release;let calls=0;
 const scope={fetch:()=>++calls===1?new Promise(resolve=>{release=resolve;}):Promise.resolve(response())};
 const probe=install(origin,scope),old=scope.fetch(new Request(url));const token=probe.arm(id);
 release(response());await (await old).text();
 for(const [target,init] of [[url+'?other=1',undefined],[origin+'/other',undefined],[url,{method:'POST'}]])await (await scope.fetch(new Request(target),init)).text();
 assert.equal(probe.snapshot(token).seen,0);assert.equal(probe.ready(token),null);
 await (await scope.fetch(new Request(url))).text();await settled(probe,token);
 assert.equal(probe.ready(token).invocation,1);probe.dispose();
});

test('superseding a pending observation cannot attach its later response to the new action',async()=>{
 let release;let calls=0;
 const scope={fetch:()=>++calls===1?new Promise(resolve=>{release=resolve;}):Promise.resolve(response())};
 const probe=install(origin,scope),oldToken=probe.arm(id),old=scope.fetch(new Request(url));
 const current=probe.arm(id);release(response());await (await old).text();
 assert.equal(probe.ready(oldToken),null);assert.equal(probe.ready(current),null);assert.equal(probe.snapshot(current).seen,0);
 await (await scope.fetch(new Request(url))).text();await settled(probe,current);
 assert.equal(probe.ready(current).captureId,id);assert.equal(probe.ready(oldToken),null);probe.dispose();
});

test('HTTP errors, truncated JSON, wrong identities and excessive bodies are rejected',async()=>{
 for(const [body,status,reason] of [[payload(),503,'http'],['{"capture":',200,'json'],[payload('other'),200,'identity'],['x'.repeat(256*1024+1),200,'budget']]){
  const scope={fetch:async()=>response(body,status)},probe=install(origin,scope),token=probe.arm(id);
  const original=await scope.fetch(new Request(url));assert.equal(await original.text(),body);
  await settled(probe,token);assert.equal(probe.ready(token),null);
  assert.equal(probe.snapshot(token).requests[0].error,reason);probe.dispose();
 }
});

test('a genuinely aborted response body cannot be mistaken for consumed metadata',async()=>{
 const abort=new AbortController();let stream;
 const body=new ReadableStream({start(controller){stream=controller;}});
 abort.signal.addEventListener('abort',()=>stream.error(new DOMException('aborted','AbortError')),{once:true});
 const scope={fetch:async()=>response(body)},probe=install(origin,scope),token=probe.arm(id);
 const original=await scope.fetch(new Request(url,{signal:abort.signal}));abort.abort();
 await assert.rejects(original.text(),/aborted/);await settled(probe,token);
 assert.equal(probe.ready(token),null);assert.equal(probe.snapshot(token).requests[0].error,'cancelled');probe.dispose();
});

test('observer cleanup never cancels the application response branch',async()=>{
 let stream;const body=new ReadableStream({start(controller){stream=controller;}});
 const scope={fetch:async()=>response(body)},probe=install(origin,scope),token=probe.arm(id);
 const original=await scope.fetch(new Request(url));probe.cancel(token);
 stream.enqueue(new TextEncoder().encode(payload()));stream.close();
 assert.equal((await original.json()).capture.id,id);assert.equal(probe.ready(token),null);probe.dispose();
});

test('matching request observations are capped and overflow cannot produce a passing witness',async()=>{
 const scope={fetch:async()=>response(payload(),503)},probe=install(origin,scope),token=probe.arm(id);
 for(let i=0;i<9;i++)await (await scope.fetch(new Request(url))).text();
 const state=probe.snapshot(token);assert.equal(state.seen,9);assert.equal(state.requests.length,8);assert.equal(state.overflow,true);
 assert.equal(probe.ready(token),null);probe.dispose();
});
