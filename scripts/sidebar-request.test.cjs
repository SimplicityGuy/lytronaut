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
