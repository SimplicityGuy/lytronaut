const {test}=require('node:test');
const assert=require('node:assert/strict');
const install=require('./focus-readiness.cjs');
const names=Array.from({length:24},(_,i)=>`Synthetic ${String(i).padStart(3,'0')}`);
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(){
 const notices=[],listeners=new Map();let callback,disconnected=0;
 const image={src:'blob:static',complete:true,naturalWidth:640,decode:async()=>{}};
 const range={disabled:false,value:'-1',min:'-1',max:'1'};
 const selected={textContent:'Synthetic 000'},status={textContent:'Static JPEG preview ready'};
 const labels=names.map(textContent=>({textContent}));
 const doc={querySelectorAll(selector){if(selector==='.capture span')return labels;if(selector==='.capture')return Array(24).fill({});if(selector==='.capture img')return Array(23).fill({complete:true,naturalWidth:128});throw Error(selector);},querySelector(selector){return {'#focus':range,'.stage img':image,'.viewer-strip span':selected,'.status':status}[selector];}};
 const scope={document:doc,performance:{now:()=>42},queueMicrotask,MutationObserver:class{constructor(fn){callback=fn;}observe(){}disconnect(){disconnected++;}},addEventListener:(name,fn)=>listeners.set(name,fn),removeEventListener:(name)=>listeners.delete(name),notify:state=>{notices.push(state);}};
 const api=install({names,selectedName:'Synthetic 000',notify:'notify'},scope);
 return {api,image,range,selected,status,labels,notices,listeners,mutate:()=>callback(),disconnected:()=>disconnected};
}
test('passive metadata readiness announces once after existing image decode, without requiring initial focus completion',async()=>{
 const f=fixture();await tick();assert.equal(f.notices.length,1);assert.equal(f.notices[0].stageKind,'static-or-initial');assert.equal(f.notices[0].pendingGrid,1);assert.equal(f.notices[0].pageReadyMs,42);assert.equal(f.notices[0].existingStageDecoded,true);
 assert.equal(f.listeners.size,0);assert.equal(f.disconnected(),1);f.mutate();await tick();assert.equal(f.notices.length,1);f.api.dispose();
});
test('wrong identity, ordered labels or disabled control cannot arm the observer',async()=>{
 for(const change of [f=>{f.selected.textContent='Synthetic 016';},f=>{f.labels[1].textContent='other';},f=>{f.range.disabled=true;},f=>{f.range.max='NaN';}]){
  const f=fixture();change(f);await tick();assert.equal(f.notices.length,0);f.api.dispose();assert.equal(f.listeners.size,0);
 }
});
test('a changed image source during decode cannot announce stale readiness',async()=>{
 const g=fixture();g.image.complete=false;await tick();let decoded;
 g.image.decode=()=>new Promise(resolve=>{decoded=resolve;});g.image.complete=true;g.mutate();g.image.src='blob:new';decoded();await tick();assert.equal(g.notices.length,0);g.api.dispose();
});
test('disposal while decode is pending suppresses a late notification',async()=>{
 const f=fixture();f.image.complete=false;await tick();let decoded;f.image.decode=()=>new Promise(resolve=>{decoded=resolve;});f.image.complete=true;f.mutate();f.api.dispose();decoded();await tick();assert.equal(f.notices.length,0);assert.equal(f.listeners.size,0);
});
test('requested-frame witness rejects the late default plane, stale identities and unchanged stages',()=>{
 const f=fixture(),expected={name:'Synthetic 000',plane:3,lambda:1};
 const state={changed:true,selected:expected.name,alt:'Stored JPEG focus derivative: Synthetic 000',output:'Displayed: 360 × 240 source · plane 3 · relative lambda 1 · EXIF 1 · lossy JPEG',status:'Discrete focus: stored plane 3 of 3, lambda 1 · 360 × 240 source'};
 assert.equal(f.api.frameMatches(state,expected),true);
 for(const change of [{changed:false},{selected:'Synthetic 016'},{output:state.output.replace('plane 3','plane 1')},{status:state.status.replace('lambda 1','lambda -1')},{alt:'Static JPEG preview: Synthetic 000'}])assert.equal(f.api.frameMatches({...state,...change},expected),false);
 f.api.dispose();
});

test('passive image-load observation catches readiness without a DOM mutation',async()=>{
 const f=fixture();f.image.complete=false;await tick();assert.equal(f.notices.length,0);f.image.complete=true;f.listeners.get('load')();await tick();assert.equal(f.notices.length,1);assert.equal(f.listeners.size,0);f.api.dispose();
});
