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
  checkpoint='discrete-focus-corpus';
  // Aggregate-only actual native generation + Chrome decoding; never log source IDs/pixels.
  const focusProof=await page.evaluate(async()=>{
   const libraries=(await (await fetch('/libraries')).json()).items.filter(l=>l.state==='ready');
   let stacks=0,planes=0,maxSourceBytes=0;const times=[];
   for(const library of libraries){let offset=0;for(;;){const captures=await (await fetch(`/libraries/${library.id}/captures?limit=100&offset=${offset}`)).json();
    for(const capture of captures.items){const detail=await (await fetch(`/captures/${capture.id}`)).json();
     const file=detail.originals.find(f=>f.role==='jpeg_focus_stack');if(!file)continue;
     const response=await fetch(`/assets/${file.id}/focus`);if(!response.ok)throw Error('focus metadata failed');
     const stack=await response.json();if(stack.lambdas.some((v,i)=>!Number.isFinite(v)||(i&&v<=stack.lambdas[i-1])))throw Error('invalid focus order');
     stacks++;maxSourceBytes=Math.max(maxSourceBytes,file.encoded_bytes);
     for(let i=0;i<stack.lambdas.length;i++){
      const start=performance.now();const response=await fetch(`/assets/${file.id}/focus/${i}/1`);if(!response.ok)throw Error('focus derivative failed');
      const meta=JSON.parse(response.headers.get('X-Lytronaut-Derivative'));const bytes=await response.arrayBuffer();
      const digest=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(v=>v.toString(16).padStart(2,'0')).join('');
      if(meta.representation!=='jpeg_focus_plane'||meta.plane_index!==i||meta.lambda!==stack.lambdas[i]||meta.encoded_bytes!==bytes.byteLength||meta.content_sha256!==digest||meta.orientation!==1)throw Error('derivative metadata or integrity failed');
      const bitmap=await createImageBitmap(new Blob([bytes],{type:'image/jpeg'}));
      if(bitmap.width!==meta.output_dimensions.width||bitmap.height!==meta.output_dimensions.height)throw Error('derivative decoded geometry failed');bitmap.close();
      times.push(performance.now()-start);planes++;
     }
    }if(captures.next_offset==null)break;offset=captures.next_offset;
   }}
   times.sort((a,b)=>a-b);return {stacks,planes,maxSourceBytes,p95NativeHttpDecodeMs:times[Math.ceil(times.length*.95)-1]};
  });
  assert.equal(focusProof.stacks,privateMode?58:1);assert.equal(focusProof.planes,privateMode?420:3);assert(focusProof.maxSourceBytes<=8*1024*1024);
  if(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1')assert(focusProof.p95NativeHttpDecodeMs<=100);
  checkpoint='focus-ui-benchmark';
  await click('All captures');
  const focusCaptureIndex=await page.evaluate(async()=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');let offset=0;for(;;){const page=await (await fetch(`/libraries/${l.id}/captures?limit=24&offset=${offset}`)).json();for(let i=0;i<page.items.length;i++){const d=await (await fetch(`/captures/${page.items[i].id}`)).json();if(d.capabilities.discrete_focus.state==='available')return offset+i;}if(page.next_offset==null)return -1;offset=page.next_offset;}});
  assert(focusCaptureIndex>=0);await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24);
  for(let offset=0;offset<Math.floor(focusCaptureIndex/24);offset++){await click('Next page');await page.waitForFunction(p=>document.querySelector('.pagination span').textContent===`Page ${p}`,{},offset+2);}
  await page.evaluate(i=>document.querySelectorAll('.capture')[i].click(),focusCaptureIndex%24);
  await page.waitForFunction(()=>!document.querySelector('#focus').disabled&&document.querySelector('.status').textContent.includes('Discrete focus:'));
  const measureFocusUi=async()=>page.evaluate(async()=>{const times=[];const pendingGridAtStart=document.querySelectorAll('.capture').length-[...document.querySelectorAll('.capture img')].filter(i=>i.complete&&i.naturalWidth>0).length;const range=document.querySelector('#focus');for(let i=0;i<24;i++){
   const image=document.querySelector('.stage img'),old=image.src;const start=performance.now();range.value=i%2?range.min:range.max;
   const displayed=new Promise((resolve,reject)=>{const timer=setTimeout(()=>{observer.disconnect();reject(Error('focus interaction timeout'));},5000);const observer=new MutationObserver(()=>{if(image.src!==old){clearTimeout(timer);observer.disconnect();resolve();}});observer.observe(image,{attributes:true,attributeFilter:['src']});});
   range.dispatchEvent(new Event('input',{bubbles:true}));await displayed;await image.decode();await new Promise(requestAnimationFrame);times.push(performance.now()-start);
  }times.sort((a,b)=>a-b);return {samples:times.length,pendingGridAtStart,p50InputToDecodedFrameReadyMs:times[Math.ceil(times.length*.5)-1],p95InputToDecodedFrameReadyMs:times[Math.ceil(times.length*.95)-1],maxInputToDecodedFrameReadyMs:times[times.length-1]};});
  const focusUiLoadContendedTimes=await measureFocusUi();
  await page.waitForFunction(()=>document.querySelectorAll('.capture img').length===document.querySelectorAll('.capture').length&&[...document.querySelectorAll('.capture img')].every(i=>i.complete&&i.naturalWidth>0),{timeout:120000});
  const focusUiTimes=await measureFocusUi();
  if(privateMode)console.log(JSON.stringify({focusProof,focusUiLoadContendedTimes,focusUiTimes}));
  if(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1')assert(focusUiTimes.p95InputToDecodedFrameReadyMs<=100&&focusUiLoadContendedTimes.p95InputToDecodedFrameReadyMs<=100,JSON.stringify({focusUiLoadContendedTimes,focusUiTimes}));
  if(!privateMode){
   checkpoint='focus-controls';await page.waitForFunction(()=>!document.querySelector('#focus').disabled&&document.querySelector('.status').textContent.includes('Discrete focus:'));
   assert.deepEqual(await page.$eval('.stage img',e=>[e.naturalWidth,e.naturalHeight]),[360,240]);
   const corners=async()=>page.evaluate(async()=>{const img=document.querySelector('.stage img');await img.decode();const c=document.createElement('canvas');c.width=img.naturalWidth;c.height=img.naturalHeight;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);const samples=[[20,20],[c.width-20,20],[20,c.height-20],[c.width-20,c.height-20]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data].slice(0,3));c.width=c.height=0;return samples.map(rgb=>rgb.map(v=>v>180?'H':'L').join(''));});
   assert.deepEqual(await corners(),['HLL','LHL','LLH','HHL'],'baked8/sidecar6 conflict defaults to identity');
   checkpoint='focus-keyboard-end';await page.focus('#focus');await page.keyboard.press('End');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('stored plane 3 of 3'));
   checkpoint='focus-keyboard-home';await page.keyboard.press('Home');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('stored plane 1 of 3'));
   checkpoint='focus-orientation6';await page.select('#orientation','6');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===240&&document.querySelector('.status').textContent.includes('EXIF 6'));
   assert.deepEqual(await corners(),['LLH','HLL','HHL','LHL']);
   checkpoint='focus-orientation1';await page.select('#orientation','1');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===360&&document.querySelector('.status').textContent.includes('EXIF 1'));
   checkpoint='export-png-ui';await page.select('#orientation','6');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===240&&document.querySelector('.status').textContent.includes('EXIF 6'));
   await click('Export PNG');await page.waitForFunction(()=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&document.querySelector('#export-download'));
   const exportProof=await page.evaluate(async()=>{
    const recipe=await (await fetch(document.querySelector('#export-recipe').getAttribute('href'))).json();const r=await fetch(document.querySelector('#export-download').getAttribute('href'));const bytes=await r.arrayBuffer();
    const hash=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(b=>b.toString(16).padStart(2,'0')).join('');
    const image=await createImageBitmap(new Blob([bytes],{type:'image/png'}));const c=document.createElement('canvas');c.width=image.width;c.height=image.height;const ctx=c.getContext('2d');ctx.drawImage(image,0,0);const colors=[[20,20],[c.width-20,20],[20,c.height-20],[c.width-20,c.height-20]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data].slice(0,3).map(v=>v>180?'H':'L').join(''));image.close();c.width=c.height=0;
    return {recipe,bytes:bytes.byteLength,hash,colors};
   });
   assert.equal(exportProof.recipe.source_variant,'stored_jpeg_focus_plane');assert.equal(exportProof.recipe.plane_index,0);assert.equal(exportProof.recipe.orientation,6);assert.deepEqual(exportProof.recipe.output_dimensions,{width:240,height:360});assert.deepEqual(exportProof.recipe.visible_geometry,[0,0,240,360]);assert.equal(exportProof.recipe.refocus_loss,true);assert.equal(exportProof.recipe.color_space,'unknown');assert.equal(exportProof.recipe.content_sha256,exportProof.hash);assert.equal(exportProof.recipe.encoded_bytes,exportProof.bytes);assert.deepEqual(exportProof.colors,['LLH','HLL','HHL','LHL']);
   const pngHref=await page.$eval('#export-download',e=>e.getAttribute('href'));
   checkpoint='export-jpeg-ui';await click('Export JPEG');await page.waitForFunction(old=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&document.querySelector('#export-download')?.getAttribute('href')!==old,{},pngHref);
   assert.equal(await page.evaluate(async()=> (await (await fetch(document.querySelector('#export-recipe').getAttribute('href'))).json()).format),'jpeg');
   const jpegHref=await page.$eval('#export-download',e=>e.getAttribute('href'));
   checkpoint='lossless-backup-ui';await click('Back up originals and catalog');await page.waitForFunction(old=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&document.querySelector('#export-download')?.getAttribute('href')!==old,{},jpegHref);
   assert.equal(await page.$('#export-recipe'),null);
   const bundleBytes=await page.evaluate(async()=>[...new Uint8Array(await (await fetch(document.querySelector('#export-download').getAttribute('href'))).arrayBuffer())]);const bundlePath=path.join(root,'synthetic-backup.lytronaut');fs.writeFileSync(bundlePath,Buffer.from(bundleBytes),{mode:0o600});
   checkpoint='isolated-restore-ui';await (await page.$('#restore-bundle')).uploadFile(bundlePath);await page.waitForFunction(()=>document.querySelector('#export-progress')?.textContent.includes('Succeeded')&&!document.querySelector('#export-download')&&document.querySelector('.status').textContent.includes('restore complete'));
   assert.equal(await page.evaluate(async()=> (await (await fetch('/libraries')).json()).items.filter(l=>l.state==='ready').length),1,'isolated restore must not replace the live library');
   await page.select('#orientation','1');await page.waitForFunction(()=>document.querySelector('.stage img')?.naturalWidth===360&&document.querySelector('.status').textContent.includes('EXIF 1'));
   // Corrupt a real derivative while retaining its genuine typed header. Old verified frame stays usable.
   checkpoint='focus-integrity';const focusFile=await page.evaluate(async()=>{const l=(await (await fetch('/libraries')).json()).items.find(l=>l.state==='ready');const c=(await (await fetch(`/libraries/${l.id}/captures?limit=100`)).json()).items.find(c=>c.name==='Synthetic 000');return (await (await fetch(`/captures/${c.id}`)).json()).originals.find(f=>f.role==='jpeg_focus_stack').id;});
   const genuine=await page.evaluate(async id=>{const r=await fetch(`/assets/${id}/focus/2/1`);return {meta:r.headers.get('X-Lytronaut-Derivative'),size:JSON.parse(r.headers.get('X-Lytronaut-Derivative')).encoded_bytes};},focusFile);
   await page.setRequestInterception(true);let corruptFocus=true;
   const badFocus=r=>{if(corruptFocus&&r.url().endsWith('/focus/2/1'))r.respond({status:200,headers:{'Content-Type':'image/jpeg','X-Lytronaut-Derivative':genuine.meta},body:Buffer.alloc(genuine.size)}).catch(()=>{});else r.continue().catch(()=>{});};page.on('request',badFocus);
   await page.focus('#focus');await page.keyboard.press('End');await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Derivative integrity check failed'));
   assert.deepEqual(await corners(),['HLL','LHL','LLH','HHL']);corruptFocus=false;await page.setRequestInterception(false);page.off('request',badFocus);
   // Cancel held focus work by changing capture; a late response cannot replace the static frame.
   checkpoint='focus-cancellation';let heldFocus;await page.setRequestInterception(true);const staleFocus=r=>{if(r.url().includes('/focus/1/')&&!heldFocus)heldFocus=r;else r.continue().catch(()=>{});};page.on('request',staleFocus);
   await page.$eval('#focus',e=>{e.value='0';e.dispatchEvent(new Event('input',{bubbles:true}));});for(let i=0;i<100&&!heldFocus;i++)await delay(10);assert(heldFocus);
   const selectedStatic=await page.evaluate(()=>{const button=[...document.querySelectorAll('.capture')].find(b=>b.textContent!=='Synthetic 000');const name=button.textContent;button.click();return name;});await heldFocus.continue().catch(()=>{});await page.waitForFunction(name=>document.querySelector('#focus').disabled&&document.querySelector('.viewer-strip span').textContent===name,{},selectedStatic);await delay(100);
   assert(await page.$eval('#focus',e=>e.disabled));assert(await page.$eval('.stage img',e=>e.naturalWidth===640));await page.setRequestInterception(false);page.off('request',staleFocus);
  }
  const contrast=await page.evaluate(()=>{const luminance=c=>{const rgb=c.match(/[\d.]+/g).slice(0,3).map(x=>{let v=Number(x)/255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;});return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722;};const ratio=(a,b)=>{a=luminance(a);b=luminance(b);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);};const root=getComputedStyle(document.documentElement),button=getComputedStyle(document.querySelector('.search button'));return {text:ratio(root.color,root.backgroundColor),control:ratio(button.borderTopColor,button.backgroundColor)};});
  assert(contrast.text>=4.5);assert(contrast.control>=3);
  checkpoint='capture-pagination';await click('All captures');await page.waitForFunction(()=>document.querySelector('.pagination span').textContent==='Page 1'&&document.querySelectorAll('.capture img').length===24,{timeout:120000});let seen=0;
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
   await page.evaluate(()=>[...document.querySelectorAll('.capture')].find(b=>b.textContent!=='Synthetic 000').click());await page.waitForFunction(()=>document.querySelector('#focus').disabled&&document.querySelector('.stage img')?.naturalWidth===640);
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
  const result={mode:privateMode?'private-aggregate':'synthetic',revision:process.env.LYTRONAUT_TEST_REVISION||'working-tree',artifactProfiles:process.env.LYTRONAUT_ARTIFACT_PROFILES||(process.env.LYTRONAUT_RELEASE_BENCHMARK==='1'?'native release; wasm release':'native debug; wasm debug'),performanceGateEnabled:process.env.LYTRONAUT_RELEASE_BENCHMARK==='1',startupMs,focusProof,focusUiLoadContendedTimes,focusUiTimes,faultSuite:!privateMode,keyboardNavigation:true,contrast,preZoomURLsReleasedExceptCurrentStage:true,captures:seen,albums:albumCount,orderedMemberships:memberships,assets429:responses.filter(s=>s===429).length,blobURLsReleased:true,absentGPUAndCodecInjected:true,pageErrors:errors.length,reducedMotion:reduced,forcedColors:true,AXTree:true,responsive:true,zoom,browser:await browser.version(),screenReader:'AX inspection only; VoiceOver not manually certified'};
  console.log(JSON.stringify(result));
  fs.writeFileSync(process.env.LYTRONAUT_BROWSER_REPORT||path.join(root,privateMode?'browser-aggregate.json':'browser-results.json'),JSON.stringify(result,null,2),{mode:0o600});
 }finally{if(browser)await browser.close();fs.rmSync(path.join(root,'browser-profile-'+process.pid),{recursive:true,force:true});server.kill('SIGINT');}
})().catch(e=>{console.error(privateMode?'Browser validation failed at '+checkpoint+' '+(e.stack?.match(/browser-test.cjs:\d+:\d+/)?.[0]||'')+' (details redacted)':'checkpoint '+checkpoint+' '+e.stack);process.exitCode=1;});
