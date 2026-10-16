// Coverage driver contract tests; mocked browser, no product or private data.
'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {spawnSync}=require('node:child_process');

function run(scenario){
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'lytronaut-coverage-test-'));
  try{
    const program=`
      const {EventEmitter}=require('node:events');
      const scenario=process.argv[1];
      const profile=Buffer.alloc(160);
      const word=(n,v)=>profile.writeBigUInt64LE(BigInt(v),n*8);
      word(0,0xff6c70726f665281n);word(1,11);word(3,1);word(5,1);word(12,1);word(19,7);
      const signature=(((1n<<40n)+(1n<<30n)+(1n<<20n)+(1024n<<10n)+7n+11n+0xff6c70726f665281n)&((1n<<64n)-1n)).toString();
      class Page extends EventEmitter{
        isClosed(){return false;}
        mainFrame(){return this;}
        url(){return 'http://fixture/';}
        async waitForNetworkIdle(){}
        async evaluate(fn){
          if(!String(fn).includes('hook.dump'))return;
          if(scenario==='missing')return {missing:true};
          let bytes=Buffer.from(profile);
          if(scenario==='truncated')bytes=bytes.subarray(0,8);
          if(scenario==='magic')bytes.writeBigUInt64LE(0n);
          if(scenario==='version')bytes.writeBigUInt64LE(99n,8);
          if(scenario==='bounds')bytes.writeBigUInt64LE(10000n,16);
          return {signature:scenario==='signature'?'0':signature,length:bytes.length,base64:bytes.toString('base64')};
        }
        async goto(){this.emit('framenavigated',this);}
        async reload(){this.emit('framenavigated',this);}
        async close(){}
      }
      const page=new Page(),browser=new EventEmitter();let enumeration=0,popup=null;
      browser.pages=async()=>{if(scenario==='enumeration'&&enumeration++)throw Error('enumeration failed');return popup?[page,popup]:[page];};
      browser.newPage=async()=>new Page();browser.close=async()=>{};
      (async()=>{
        const driver=require(${JSON.stringify(path.resolve(__dirname,'coverage.cjs'))});
        await driver.launch({launch:async()=>browser},{});
        page.emit('response',{url:()=> 'http://fixture/lytronaut_client_bg.wasm',frame:()=>page});
        if(scenario==='popup'){
          popup=new Page();browser.emit('targetcreated',{page:async()=>popup});
          await new Promise(resolve=>setImmediate(resolve));
          popup.emit('response',{url:()=> 'http://fixture/lytronaut_client_bg.wasm',frame:()=>popup});
        }
        if(scenario==='undumped')page.emit('framenavigated',page);
        else if(scenario==='navigation')await page.goto();
        else await browser.close();
      })().catch(e=>{console.error(e);process.exitCode=1;});
    `;
    const result=spawnSync(process.execPath,['-e',program,scenario],{env:{...process.env,LYTRONAUT_COVERAGE_DIR:dir},encoding:'utf8'});
    const files=fs.readdirSync(dir);
    const summary=JSON.parse(fs.readFileSync(path.join(dir,files.find(f=>f.startsWith('driver-')))));
    return {result,summary,profiles:files.filter(f=>f.endsWith('.profraw')).length};
  }finally{fs.rmSync(dir,{recursive:true,force:true});}
}

test('startup page profiles survive close and explicit navigation',()=>{
  for(const scenario of ['close','navigation']){
    const {result,summary,profiles}=run(scenario);
    assert.equal(result.status,0,result.stderr);
    assert.equal(summary.pagesWithClient,1);
    assert.equal(profiles,1);
    assert.deepEqual(summary.failures,[]);
  }
});
test('popup profiles are tracked and dumped before browser close',()=>{
  const {result,summary,profiles}=run('popup');
  assert.equal(result.status,0,result.stderr);
  assert.equal(summary.pagesWithClient,2);
  assert.equal(profiles,2);
  assert.deepEqual(summary.failures,[]);
});
test('missing, malformed and incompatible profiles fail the process',()=>{
  for(const scenario of ['missing','truncated','magic','version','bounds','signature']){
    const {result,summary,profiles}=run(scenario);
    assert.equal(result.status,1,scenario+result.stderr);
    assert.equal(profiles,0,scenario);
    assert.equal(summary.failures.length,1,scenario);
  }
});
test('undumped navigation and page enumeration failure cannot silently pass',()=>{
  for(const scenario of ['undumped','enumeration']){
    const {result,summary}=run(scenario);
    assert.equal(result.status,1,scenario+result.stderr);
    assert.equal(summary.failures.length,1,scenario);
  }
});
