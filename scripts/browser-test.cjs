// Browser test tooling only. Product behavior is entirely Rust/WASM.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {spawn}=require('node:child_process');
const pp=require(process.env.LYTRONAUT_PUPPETEER||'./browser/node_modules/puppeteer-core');
const root=process.env.LYTRONAUT_TEST_ROOT;
assert(root,'LYTRONAUT_TEST_ROOT must point at synthetic fixture/scratch');
const privateMode=process.env.LYTRONAUT_PRIVATE_TEST==='1';
const binary=process.env.LYTRONAUT_BINARY;
const origin=process.env.LYTRONAUT_TEST_ORIGIN||'http://127.0.0.1:3017';
const data=privateMode?root:path.join(root,'data');
fs.mkdirSync(data,{recursive:true,mode:0o700});
const server=process.env.LYTRONAUT_EXTERNAL_SERVER?{on(){},stderr:{on(){}},kill(){}}:spawn(binary,[],{cwd:process.cwd(),env:{...process.env,LYTRONAUT_DATA:data,LYTRONAUT_DATABASE:privateMode?'app.db':'catalog.db',LYTRONAUT_ORIGIN:origin,LYTRONAUT_LISTEN:new URL(origin).host},stdio:['ignore','ignore','pipe']});
let serverFailure=false;server.stderr.on("data",d=>{if(!privateMode)process.stderr.write(d);});server.on('exit',code=>{if(code)serverFailure=true;});
let checkpoint='startup';
const delay=ms=>new Promise(r=>setTimeout(r,ms));
(async()=>{
 let browser;
 try{
  const startupStart=Date.now();let ready=false;for(let i=0;i<1200;i++){try{if((await fetch(origin+'/healthz')).ok){ready=true;break;}}catch{}if(serverFailure)throw Error('test service startup failed');await delay(250);}assert(ready,'service must become healthy within 300s');const startupMs=Date.now()-startupStart;
  if(!privateMode)console.error("starting isolated Chrome");
  browser=await pp.launch({executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,userDataDir:path.join(root,'browser-profile-'+process.pid),args:[]});
  let page=await browser.newPage();
  const click=async text=>{await page.waitForFunction(text=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===text&&!b.disabled);if(!b)return false;b.click();return true;},{polling:50},text);};await page.setViewport({width:1440,height:1000});
  const instrument=async p=>p.evaluateOnNewDocument(()=>{
   Object.defineProperty(navigator,'gpu',{value:undefined});Object.defineProperty(window,'VideoDecoder',{value:undefined});
   window.__urls={created:0,revoked:0};const create=URL.createObjectURL,revoke=URL.revokeObjectURL;
   URL.createObjectURL=function(...args){window.__urls.created++;return create.apply(this,args);};
   URL.revokeObjectURL=function(...args){window.__urls.revoked++;return revoke.apply(this,args);};
  });await instrument(page);
  const errors=[],responses=[];const attach=p=>{p.on('pageerror',e=>{errors.push(e.message);if(!privateMode)console.error(e.message);});p.on('response',r=>{if(/\/assets\//.test(r.url()))responses.push(r.status());});};attach(page);
  if(!privateMode)console.error('opening app');await page.goto(origin);checkpoint='authentication';await page.waitForSelector('#username');if(!privateMode)console.error(await page.$$eval('button',b=>b.map(x=>x.textContent)));
  if(!privateMode && !fs.existsSync(path.join(root,'setup-complete'))){
   await page.evaluate(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.includes('First run')).click());
   await page.type('#username','browser');await page.type('#password','synthetic browser password');
   await page.type('#bootstrap',fs.readFileSync(path.join(data,'bootstrap.secret'),'utf8').trim());
   await click('Create administrator');
   await page.waitForFunction(()=>!document.querySelector('#bootstrap'));fs.writeFileSync(path.join(root,'setup-complete'),'synthetic');
  }
  const username=privateMode?'private_validation': 'browser';
  const password=privateMode?fs.readFileSync(path.join(root,'private-api-password'),'utf8').trim():'synthetic browser password';
  await page.$eval('#username',e=>e.value='');await page.type('#username',username);await page.type('#password',password);
  await click('Sign in');await page.waitForSelector('#directory');
  if(!privateMode){
   const directory=await page.$('#directory');await directory.uploadFile(path.join(root,'Synthetic.lytrolibrary'));
   await page.waitForFunction(async()=>{const id=sessionStorage.getItem('lytronaut-import');if(!id||document.querySelector('#directory').disabled)return false;return (await (await fetch('/imports/'+id)).json()).phase==='catalog_complete';},{timeout:120000});
   await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24,{timeout:120000});
  }
  await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length&&document.querySelectorAll('.capture').length>0,{timeout:120000});
  assert(await page.$eval('.stage img',e=>e.complete&&e.naturalWidth>0));
  checkpoint='keyboard-navigation';const firstCapture=await page.$eval('.viewer-strip span',e=>e.textContent);
  await page.focus('.stage');await page.keyboard.press('ArrowRight');
  await page.waitForFunction(first=>document.querySelector('.viewer-strip span').textContent!==first,{},firstCapture);
  await page.keyboard.press('ArrowLeft');await page.waitForFunction(first=>document.querySelector('.viewer-strip span').textContent===first,{},firstCapture);
  const contrast=await page.evaluate(()=>{const luminance=c=>{const rgb=c.match(/[\d.]+/g).slice(0,3).map(x=>{let v=Number(x)/255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;});return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722;};const ratio=(a,b)=>{a=luminance(a);b=luminance(b);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);};const root=getComputedStyle(document.documentElement),button=getComputedStyle(document.querySelector('.search button'));return {text:ratio(root.color,root.backgroundColor),control:ratio(button.borderTopColor,button.backgroundColor)};});
  assert(contrast.text>=4.5);assert(contrast.control>=3);
  checkpoint='capture-pagination';let seen=0;
  for(;;){
   const count=await page.$$eval('.capture img',imgs=>imgs.filter(i=>i.complete&&i.naturalWidth>0).length);seen+=count;
   const next=await page.evaluateHandle(()=>[...document.querySelectorAll('button')].find(b=>b.textContent==='Next page'));
   if(await next.evaluate(e=>e.disabled)){await next.dispose();break;}
   const old=await page.$eval('.pagination span',e=>e.textContent);await next.asElement().click();await next.dispose();
   await page.waitForFunction(old=>document.querySelector('.pagination span').textContent!==old,{},old);
   await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length,{timeout:120000});
  }
  assert.equal(seen,privateMode?109:30);
  checkpoint='ordered-albums';const libraryCount=await page.evaluate(async()=> (await (await fetch('/libraries')).json()).items.length);const albumCount=await page.$$eval('.rail-item',(buttons,libraryCount)=>buttons.length-libraryCount-1,libraryCount);assert.equal(albumCount,3);
  const albums=await page.evaluate(async()=>{const libraries=await (await fetch('/libraries')).json();const library=libraries.items.find(l=>l.state==='ready');return (await (await fetch(`/libraries/${library.id}/albums`)).json()).items;});
  const memberships=[];
  for(let index=0;index<albumCount;index++){
   const expected=await page.evaluate(async id=>{let offset=0,all=[];for(;;){let p=await (await fetch(`/albums/${id}/captures?limit=24&offset=${offset}`)).json();all.push(...p.items);if(p.next_offset==null)break;offset=p.next_offset;}return all.map(c=>c.name+(c.name_truncated?' [truncated]':''));},albums[index].id);
   const buttons=await page.$$('.rail-item');await buttons[index+libraryCount+1].click();await Promise.all(buttons.map(b=>b.dispose()));
   let actual=[];
   for(let offset=0;;offset+=24){
    const names=expected.slice(offset,offset+24);
    await page.waitForFunction(names=>JSON.stringify([...document.querySelectorAll('.capture span')].map(e=>e.textContent))===JSON.stringify(names),{timeout:120000},names);
    await page.waitForFunction(()=>document.querySelectorAll('.capture img').length>0&&document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length,{timeout:120000});
    actual.push(...await page.$$eval('.capture span',nodes=>nodes.map(e=>e.textContent)));
    if(offset+24>=expected.length)break;
    await click('Next page');
   }
   assert.deepEqual(actual,expected);memberships.push(actual.length);
  }
  assert.deepEqual([...memberships].sort((a,b)=>a-b),privateMode?[12,39,58]:[10,10,10]);
  if(!privateMode){
   await click('All captures');await delay(300);
   await page.type('#search','Synthetic 029');await click('Search');
   await page.waitForFunction(()=>document.querySelectorAll('.capture').length===1);assert((await page.$eval('.capture',e=>e.textContent)).includes('029'));
   await page.$eval('#search',e=>{e.value='';e.dispatchEvent(new Event('input',{bubbles:true}));});await click('Search');
   await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
   checkpoint='sorting';for(const sort of ['name','rating','order']){
    const expected=await page.evaluate(async sort=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');return (await (await fetch(`/libraries/${l.id}/captures?limit=24&offset=0&sort=${sort}`)).json()).items.map(c=>c.name);},sort);
    await page.$eval('#sort',(e,sort)=>{e.value=sort;e.dispatchEvent(new Event('change',{bubbles:true}));},sort);
    await page.waitForFunction(expected=>JSON.stringify([...document.querySelectorAll('.capture span')].map(e=>e.textContent))===JSON.stringify(expected),{},expected);
    await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
   }
   await page.screenshot({path:path.join(root,'synthetic-browser.png'),fullPage:true});
   for(const zipName of ['synthetic.zip','synthetic-zip64.zip']){const zip=await page.$('#zip');await zip.uploadFile(path.join(root,zipName));
   await page.waitForFunction(async()=>{const id=sessionStorage.getItem('lytronaut-import');if(!id||document.querySelector('#zip').disabled)return false;return (await (await fetch('/imports/'+id)).json()).phase==='catalog_complete';},{timeout:120000});}
  }
  if(!privateMode){
   checkpoint='upload-cancellation';
   // Abort a real in-progress XHR and recover its durable typed state.
   await page.setRequestInterception(true);let holdUploads=true;const heldUploads=[];
   const intercept=request=>{if(holdUploads&&request.method()==='PUT'&&request.url().includes('/file')){heldUploads.push(request);return;}request.continue().catch(()=>{});};page.on('request',intercept);
   await (await page.$('#directory')).uploadFile(path.join(root,'Synthetic.lytrolibrary'));
   await page.waitForFunction(()=>document.querySelector('#directory').disabled&&[...document.querySelectorAll('button')].some(b=>b.textContent==='Cancel import'&&!b.disabled));
   for(let i=0;i<500&&!heldUploads.length;i++)await delay(20);assert(heldUploads.length>0,'real upload has started');
   await click('Cancel import');holdUploads=false;await Promise.all(heldUploads.map(r=>r.abort().catch(()=>{})));
   await page.waitForFunction(async()=>{const id=sessionStorage.getItem('lytronaut-import');return (await (await fetch('/imports/'+id)).json()).phase==='cancelled';});
   await page.setRequestInterception(false);page.off('request',intercept);
   await click('Recover import status');
   await page.waitForFunction(()=>document.querySelector('.status').textContent.toLowerCase().includes('cancelled'));
   checkpoint='preserved-only';
   // Unknown schema stays useful through preserved-only original links.
   await (await page.$('#directory')).uploadFile(path.join(root,'Preserved.lytrolibrary'));
   await page.waitForFunction(async()=>['catalog_complete','catalog_failed'].includes((await (await fetch('/imports/'+sessionStorage.getItem('lytronaut-import'))).json()).phase)&&!document.querySelector('#directory').disabled,{timeout:120000});
   const preservedPhase=await page.evaluate(async()=> (await (await fetch('/imports/'+sessionStorage.getItem('lytronaut-import'))).json()).phase);
   if(preservedPhase==='catalog_failed')await click('Retry catalog adaptation');
   await page.waitForFunction(()=>document.querySelectorAll('.browser-panel ul a').length===2,{timeout:120000});
   const original=await page.$eval('.browser-panel ul a',e=>e.getAttribute('href'));
   assert.equal((await page.evaluate(async url=>(await fetch(url)).status,original)),200);
   checkpoint='stale-session';
   // Delayed library/albums responses after logout cannot repopulate the old session.
   const allLibraries=await page.evaluate(async()=> (await (await fetch('/libraries')).json()).items);
   const readyIndex=allLibraries.findIndex(l=>l.state==='ready');
   let held;await page.setRequestInterception(true);
   const delayed=request=>{if(request.url().includes('/albums')&&!held){held=request;}else{request.continue().catch(()=>{});}};page.on('request',delayed);
   await click('Library '+(readyIndex+1));
   for(let i=0;i<50&&!held;i++)await delay(20);assert(held);
   await click('Sign out');await held.continue().catch(()=>{});await delay(200);
   assert.equal(await page.$('#directory'),null);
   await page.setRequestInterception(false);page.off('request',delayed);
   await page.type('#username','browser');await page.type('#password','synthetic browser password');await click('Sign in');await page.waitForSelector('#directory');
   await click('Library '+(readyIndex+1));
   await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
   checkpoint='corrupt-preview';
   // Corrupt transfer bytes reject preview integrity while original downloads work.
   let corrupt=true;await page.setRequestInterception(true);
   const corruptResponse=request=>{if(corrupt&&request.url().includes('/chunks/'))request.respond({status:200,body:Buffer.alloc(10154)}).catch(()=>{});else request.continue().catch(()=>{});};page.on('request',corruptResponse);
   await click('Retry preview');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('integrity check failed'));
   const intact=await page.$$eval('.inspector a',links=>links.find(e=>e.textContent.includes('Thumbnail')).getAttribute('href'));const originalBytes=await page.evaluate(async url=>{const r=await fetch(url);if(r.status!==200)throw Error('original unavailable');return [...new Uint8Array(await r.arrayBuffer())];},intact);
   assert(Buffer.from(originalBytes).equals(fs.readFileSync(path.join(root,'synthetic.jpg'))),'preserved thumbnail bytes match independent fixture');
   corrupt=false;await page.setRequestInterception(false);page.off('request',corruptResponse);await click('Retry preview');
   await page.waitForFunction(()=>document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
   checkpoint='canvas-cleanup';
   // Canvas creation failure releases the original URL; revisit recovers thumbnails.
   await page.evaluate(()=>{window.__canvasGet=HTMLCanvasElement.prototype.getContext;HTMLCanvasElement.prototype.getContext=()=>null;});
   await click('Next page');await page.waitForFunction(()=>document.querySelector('.pagination span').textContent==='Page 2');await delay(300);
   await page.evaluate(()=>{HTMLCanvasElement.prototype.getContext=window.__canvasGet;});
   await click('Previous page');await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===24,{timeout:120000});
  }
  await page.waitForFunction(()=>document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
  // Only the currently displayed stage keeps a Blob URL; downsampled grid uses data URLs.
  const preZoomURLs=await page.evaluate(()=>window.__urls);assert(preZoomURLs.created>0);assert.equal(preZoomURLs.created-preZoomURLs.revoked,1);
  await click('Sign out');await page.waitForSelector('#username');await page.waitForFunction(()=>window.__urls.created===window.__urls.revoked);
  await page.type('#username',username);await page.type('#password',password);await click('Sign in');await page.waitForSelector('#directory');
  // Chromium stores default zoom as log(factor)/log(1.2), partition key x.
  // Restart the isolated profile with actual browser zoom; do not use page scaling.
  checkpoint='zoom200';const baseline=await page.evaluate(()=>({inner:innerWidth,dpr:devicePixelRatio}));
  await browser.close();
  const preferences=path.join(root,'browser-profile-'+process.pid,'Default','Preferences');
  const prefs=JSON.parse(fs.readFileSync(preferences,'utf8'));prefs.partition??={};
  prefs.partition.default_zoom_level={x:Math.log(2)/Math.log(1.2)};
  prefs.partition.per_host_zoom_levels={};fs.writeFileSync(preferences,JSON.stringify(prefs));
  browser=await pp.launch({executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,userDataDir:path.join(root,'browser-profile-'+process.pid),args:[]});
  page=await browser.newPage();attach(page);await instrument(page);await page.setViewport({width:1440,height:1000});await page.goto(origin);await page.waitForSelector('#directory');
  const zoom=await page.evaluate(()=>({inner:innerWidth,outer:outerWidth,dpr:devicePixelRatio}));
  assert(Math.abs(zoom.dpr/baseline.dpr-2)<0.01,'actual browser200% DPR');
  assert(Math.abs(baseline.inner/zoom.inner-2)<0.01,'actual browser200% CSS viewport');
  if(!privateMode)console.error(await page.evaluate(()=>({viewport:innerWidth,scroll:document.documentElement.scrollWidth,overflow:[...document.querySelectorAll('*')].filter(e=>e.getBoundingClientRect().right>innerWidth+1).map(e=>e.tagName+'.'+e.className).slice(0,8)})));
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  const cdp=await page.createCDPSession();
  await page.emulateMediaFeatures([{name:'prefers-reduced-motion',value:'reduce'}]);
  const reduced=await page.evaluate(()=>matchMedia('(prefers-reduced-motion: reduce)').matches);assert(reduced);
  await cdp.send('Emulation.setEmulatedMedia',{features:[{name:'forced-colors',value:'active'}]});
  assert(await page.evaluate(()=>matchMedia('(forced-colors: active)').matches));
  const ax=await cdp.send('Accessibility.getFullAXTree');
  assert(ax.nodes.some(n=>n.role?.value==='navigation'&&n.name?.value==='Library and albums'));
  assert(ax.nodes.some(n=>n.role?.value==='status'));assert(ax.nodes.some(n=>n.role?.value==='button'&&n.name?.value==='Sign out'));assert(ax.nodes.some(n=>n.role?.value==='searchbox'&&n.name?.value==='Search captures'));
  checkpoint='responsive';await page.setViewport({width:390,height:844});
  if(!privateMode)console.error(await page.evaluate(()=>({viewport:innerWidth,scroll:document.documentElement.scrollWidth,overflow:[...document.querySelectorAll('*')].filter(e=>e.getBoundingClientRect().right>innerWidth+1).map(e=>e.tagName+'.'+e.className).slice(0,8)})));
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.keyboard.press('Tab');const focus=await page.evaluate(()=>({name:document.activeElement.textContent||document.activeElement.getAttribute('aria-label'),outline:getComputedStyle(document.activeElement).outlineWidth}));
  assert.notEqual(focus.outline,'0px');
  checkpoint='final-logout';await click('Sign out');await page.waitForSelector('#username');
  const unauthorized=await page.evaluate(async()=> (await fetch('/libraries')).status);assert.equal(unauthorized,401);
  assert.equal(errors.length,0);assert(!responses.includes(429));const urls=await page.evaluate(()=>window.__urls);assert.equal(urls.created,urls.revoked);
  const result={mode:privateMode?'private-aggregate':'synthetic',revision:process.env.LYTRONAUT_TEST_REVISION||'working-tree',startupMs,faultSuite:!privateMode,keyboardNavigation:true,contrast,preZoomURLsReleasedExceptCurrentStage:true,captures:seen,albums:albumCount,orderedMemberships:memberships,assets429:responses.filter(s=>s===429).length,blobURLsReleased:true,absentGPUAndCodecInjected:true,pageErrors:errors.length,reducedMotion:reduced,forcedColors:true,AXTree:true,responsive:true,zoom,browser:await browser.version(),screenReader:'AX inspection only; VoiceOver not manually certified'};
  console.log(JSON.stringify(result));
  fs.writeFileSync(path.join(root,privateMode?'browser-aggregate.json':'browser-results.json'),JSON.stringify(result,null,2),{mode:0o600});
 }finally{if(browser)await browser.close();fs.rmSync(path.join(root,'browser-profile-'+process.pid),{recursive:true,force:true});server.kill('SIGINT');}
})().catch(e=>{console.error(privateMode?'Browser validation failed at '+checkpoint+' '+(e.stack?.match(/browser-test.cjs:\d+:\d+/)?.[0]||'')+' (details redacted)':'checkpoint '+checkpoint+' '+e);process.exitCode=1;});
