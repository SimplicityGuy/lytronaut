const {EventEmitter}=require('node:events');
const assert=require('node:assert/strict');
const test=require('node:test');
const fence=require('./sidebar-request.cjs');
const url='http://localhost/captures/public-fixture';
const page=()=>Object.assign(new EventEmitter(),{getDefaultTimeout:()=>1000});
const request=(target=url,method='GET',ok=true)=>{
 const req={url:()=>target,method:()=>method};
 req.response=()=>({request:()=>req,status:()=>ok?200:503,url:()=>target});return req;
};
test('ignores pre-arm request even when its response finishes after arming',async()=>{
 const p=page(),old=request();p.emit('request',old);
 const f=fence(p,url);p.emit('requestfinished',old);
 const unrelated=request(url+'?other=1');p.emit('request',unrelated);p.emit('requestfinished',unrelated);
 const post=request(url,'POST');p.emit('request',post);p.emit('requestfinished',post);
 const fresh=request();p.emit('request',fresh);p.emit('requestfinished',fresh);
 assert.equal((await f.promise).request(),fresh);assert.equal(p.eventNames().length,0);
});
test('skips a cancelled object without retrying it, then accepts a different completed request',async()=>{
 const p=page(),f=fence(p,url),cancelled=request();
 p.emit('request',cancelled);p.emit('requestfailed',cancelled);p.emit('requestfinished',cancelled);
 const current=request();p.emit('request',current);p.emit('requestfinished',current);
 assert.equal((await f.promise).request(),current);assert.equal(p.eventNames().length,0);
});
test('headers alone cannot satisfy the fence and HTTP failure is not hidden',async()=>{
 const p=page(),f=fence(p,url),bad=request(url,'GET',false);let settled=false;
 f.promise.finally(()=>{settled=true;}).catch(()=>{});
 p.emit('request',bad);p.emit('response',bad.response());await Promise.resolve();assert.equal(settled,false);
 p.emit('requestfinished',bad);await assert.rejects(f.promise,/successfully/);assert.equal(p.eventNames().length,0);
});
test('deadline rejects missing completion and removes listeners',async()=>{
 const p=page();p.getDefaultTimeout=()=>5;const f=fence(p,url);
 await assert.rejects(f.promise,/deadline/);assert.equal(p.eventNames().length,0);
});
test('failed click cleanup rejects a pending wait and removes listeners',async()=>{
 const p=page(),f=fence(p,url);f.cancel();
 await assert.rejects(f.promise,/interaction ended/);assert.equal(p.eventNames().length,0);
});

test('explicit expected failure status is correlated to its exact completed request',async()=>{
 const p=page(),f=fence(p,url,503),failed=request(url,'GET',false);
 p.emit('request',failed);p.emit('requestfinished',failed);
 assert.equal((await f.promise).request(),failed);assert.equal(p.eventNames().length,0);
});

test('intentional rejected body may be cancelled after exact current-request headers',async()=>{
 const p=page(),old=request(url,'GET',false);p.emit('request',old);
 const f=fence(p,url,503,true);p.emit('response',old.response());
 const fresh=request(url,'GET',false);p.emit('request',fresh);p.emit('response',fresh.response());p.emit('requestfailed',fresh);
 assert.equal((await f.promise).request(),fresh);assert.equal(p.eventNames().length,0);
});

test('successful responses cannot opt out of actual request completion',()=>{
 assert.throws(()=>fence(page(),url,200,true),/limited to rejected/);
});

test('deadline distinguishes a cancelled exact response from an absent request',async()=>{
 const p=page();p.getDefaultTimeout=()=>5;
 const f=fence(p,url),req=request();req.failure=()=>({errorText:'net::ERR_ABORTED'});
 p.emit('request',req);p.emit('response',req.response());p.emit('requestfailed',req);
 await assert.rejects(f.promise,error=>{
  assert.deepEqual(error.requestLifecycle,{seen:1,active:0,requests:[{sequence:1,headers:true,finished:false,failed:true,status:200,fromCache:false,error:'net::ERR_ABORTED'}]});return /deadline/.test(error.message);
 });assert.equal(p.eventNames().length,0);
 const absent=fence(p,url);await assert.rejects(absent.promise,error=>{
  assert.deepEqual(error.requestLifecycle,{seen:0,active:0,requests:[]});return true;
 });assert.equal(p.eventNames().length,0);
});

test('diagnostics retain only eight exact request objects and expose cache completion',async()=>{
 const p=page(),f=fence(p,url);
 for(let i=0;i<12;i++){const req=request();p.emit('request',req);p.emit('requestfailed',req);}
 const req=request();const response=req.response();response.fromCache=()=>true;
 p.emit('request',req);p.emit('response',response);p.emit('requestfinished',req);await f.promise;
 const state=f.snapshot();assert.equal(state.seen,13);assert.equal(state.active,0);assert.equal(state.requests.length,8);
 assert.equal(state.requests[0].sequence,6);assert.equal(state.requests.at(-1).fromCache,true);assert.equal(state.requests.at(-1).finished,true);
 state.requests[0].sequence=0;assert.equal(f.snapshot().requests[0].sequence,6,'diagnostic snapshots cannot mutate correlation state');
 assert.equal(p.eventNames().length,0);
});
