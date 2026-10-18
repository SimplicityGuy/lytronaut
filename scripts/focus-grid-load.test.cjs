// Evidence acceptance must reject warm, held-only and late-completing work.
const {test}=require('node:test'),assert=require('node:assert/strict');
const {requireOverlap,gridPreviewIdentities}=require('./focus-grid-load.cjs');
const previews=()=>Array.from({length:24},(_,i)=>({name:`Synthetic ${String(i).padStart(3,'0')}`,id:`asset-${i}`}));
test('held grid identities exclude the selected preview',()=>{
 const grid=gridPreviewIdentities(previews());
 assert.equal(grid.size,23);assert(!grid.has('asset-0'));assert.equal(grid.get('asset-1').name,'Synthetic 001');
});
test('experiment requires the focus capture to be the natural default selection',()=>{
 const rows=previews();[rows[0],rows[1]]=[rows[1],rows[0]];
 assert.throws(()=>gridPreviewIdentities(rows),/default-selected focus capture/);
});
test('all capture labels must match the explicit name-ordered page',()=>{
 const rows=previews();[rows[1],rows[2]]=[rows[2],rows[1]];
 assert.throws(()=>gridPreviewIdentities(rows),/follow name order/);
});
test('selected preview identity alias cannot be counted as nonselected grid work',()=>{
 const rows=previews();rows[1].id=rows[0].id;assert.throws(()=>gridPreviewIdentities(rows),/identities must be distinct/);
});
test('duplicate grid identity or label cannot establish same-image attribution',()=>{
 for(const field of ['id','name']){const rows=previews();rows[2][field]=rows[1][field];assert.throws(()=>gridPreviewIdentities(rows));}
});
const valid={start:10,end:80,pendingGridAtStart:2,bodyBytes:4000,bodyFinished:30,gridDecoded:50};
test('completed response and decoded grid pixels overlap the exact focus interval',()=>assert.equal(requireOverlap(valid),70));
for(const [name,change]of [
 ['warm page',{pendingGridAtStart:0}],
 ['held request without bytes',{bodyBytes:0}],
 ['body completed before focus',{bodyFinished:9}],
 ['body completed after focus',{bodyFinished:81}],
 ['grid never decoded',{gridDecoded:null}],
 ['grid decoded before body',{gridDecoded:29}],
 ['grid decoded after focus',{gridDecoded:81}],
 ['invalid interval',{end:NaN}],
])test(`reject ${name}`,()=>assert.throws(()=>requireOverlap({...valid,...change})));
