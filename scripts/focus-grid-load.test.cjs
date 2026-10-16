// Evidence acceptance must reject warm, held-only and late-completing work.
const {test}=require('node:test'),assert=require('node:assert/strict');
const {requireOverlap}=require('./focus-grid-load.cjs');
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
