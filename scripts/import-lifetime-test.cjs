// MIT synthetic HTTP + actual Rust/WASM import ownership regression tests.
'use strict';
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
module.exports=async({browser,origin})=>{
  const completed=[];
  const packageBytes=Buffer.from(fs.readFileSync(path.join(__dirname,'../fixtures/synthetic/chrome-macos-library-package.hex'),'utf8').replace(/\s/g,''),'hex');
  for(const mode of ['failed-finish','detach-during-file','detach-before-id','detach-during-package-poll']){
    const page=await browser.newPage();page.setDefaultTimeout(60000);
    let id=null,held=null,heldPoll=null,handlerError=null,resolveHeld,resolvePoll;
    const heldReady=new Promise(resolve=>resolveHeld=resolve);
    const pollReady=new Promise(resolve=>resolvePoll=resolve);
    const bounded=async promise=>{
      let timer;
      try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('import ownership outcome timeout: '+mode)),60000);})]);}
      finally{clearTimeout(timer);}
    };
    const packageCdp=mode==='detach-during-package-poll'?await page.target().createCDPSession():null;
    if(packageCdp){
      await packageCdp.send('Fetch.enable',{patterns:[{urlPattern:'*/imports/*/zip',requestStage:'Response'}]});
      packageCdp.on('Fetch.requestPaused',event=>{held={requestId:event.requestId,responseStatusCode:event.responseStatusCode};resolveHeld();});
    }
    await page.setRequestInterception(true);
    page.on('request',request=>{
      (async()=>{
        const url=new URL(request.url());
        if(url.origin!==origin){await request.continue();return;}
        if(request.method()==='POST'&&url.pathname==='/imports'){
          // Forward the real create once, keeping the exact server ID available
          // even when the app's create response is held across page detach.
          const headers=request.headers();
          const response=await fetch(request.url(),{method:'POST',headers:{
            'content-type':'application/json','x-lytronaut-intent':headers['x-lytronaut-intent'],
            origin:headers.origin,
          },body:request.postData(),signal:AbortSignal.timeout(30000)});
          assert.equal(response.status,201);
          const body=await response.text();id=JSON.parse(body).id;
          assert.match(id,/^[0-9a-f]{32}$/);
          const payload={status:201,contentType:'application/json',body};
          if(mode==='detach-before-id'){held={request,payload};resolveHeld();return;}
          await request.respond(payload);return;
        }
        if(mode==='detach-during-file'&&request.method()==='PUT'&&url.pathname===`/imports/${id}/file`){
          held={request};resolveHeld();return;
        }
        if(mode==='detach-during-package-poll'&&request.method()==='GET'&&url.pathname===`/imports/${id}`){
          heldPoll=request;resolvePoll();return;
        }
        if(mode==='failed-finish'&&request.method()==='POST'&&url.pathname===`/imports/${id}/finish`){
          await request.respond({status:503,contentType:'application/json',body:JSON.stringify({version:1,code:'storage_unavailable',retryable:true})});return;
        }
        await request.continue();
      })().catch(error=>{handlerError=error;request.abort().catch(()=>{});});
    });
    try{
      await page.goto(origin);await page.waitForSelector('#directory');
      const cancelled=page.waitForResponse(response=>id&&new URL(response.url()).pathname===`/imports/${id}/cancel`&&response.request().method()==='POST');
      await page.evaluate(({packageMode,bytes})=>{
        const input=document.querySelector(packageMode?'#package':'#directory'),transfer=new DataTransfer();
        const file=packageMode?new File([new Uint8Array(bytes)],'Synthetic.lytrolibrary.zip',{type:'application/zip'}):new File([new Uint8Array([1,2,3,4])],'independent-import-lifetime.txt');
        if(!packageMode)Object.defineProperty(file,'webkitRelativePath',{value:'Public/independent-import-lifetime.txt'});
        transfer.items.add(file);input.files=transfer.files;input.dispatchEvent(new Event('change',{bubbles:true}));
      },{packageMode:mode==='detach-during-package-poll',bytes:[...packageBytes]});
      if(mode!=='failed-finish'){
        await bounded(heldReady);
        if(mode==='detach-during-package-poll')await bounded(pollReady);
        const aborted=mode==='detach-during-file'?new Promise(resolve=>page.on('requestfailed',request=>{if(request===held.request)resolve(request.failure()?.errorText);})):null;
        const pollAborted=mode==='detach-during-package-poll'?new Promise(resolve=>page.on('requestfailed',request=>{if(request===heldPoll)resolve(request.failure()?.errorText);})):null;
        // Existing pagehide disposal unmounts the app's owner; do not navigate
        // away before it can send its best-effort cancellation request.
        await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));
        if(mode==='detach-before-id')await held.request.respond(held.payload);
        // The held XHR must be cancelled, not released to finish its upload.
        if(aborted)assert.equal(await bounded(aborted),'net::ERR_ABORTED','detaching aborts the held XHR');
        if(pollAborted)assert.equal(await bounded(pollAborted),'net::ERR_ABORTED','detaching aborts the in-flight import status poll');
      }
      const cancelResponse=await bounded(cancelled);
      const result=await fetch(origin+`/imports/${id}`,{signal:AbortSignal.timeout(30000)});
      assert(result.ok);
      const progress=await result.json();
      if(mode==='detach-during-package-poll'){
        assert(held.responseStatusCode>=200&&held.responseStatusCode<300,'package bytes reached the service before response-stage hold');
        assert.equal(progress.phase,'catalog_complete','disposing a completed durable package import does not erase it');
      }else{
        assert(cancelResponse.ok(),'owned import cancellation reached the service');
        assert.equal(progress.state,'cancelled');
      }
      assert.equal(handlerError,null);
      completed.push(mode);
    }finally{
      if(held?.request)await held.request.abort().catch(()=>{});
      if(heldPoll)await heldPoll.abort().catch(()=>{});
      if(packageCdp){if(held?.requestId)await packageCdp.send('Fetch.continueResponse',{requestId:held.requestId}).catch(()=>{});await packageCdp.send('Fetch.disable').catch(()=>{});await packageCdp.detach().catch(()=>{});}
      await page.close();
    }
  }
  return {completed,cancellationConfirmedByService:true};
};
