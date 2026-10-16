// MIT synthetic HTTP + actual Rust/WASM import ownership regression tests.
'use strict';
const assert=require('node:assert/strict');
module.exports=async({browser,origin})=>{
  const completed=[];
  for(const mode of ['failed-finish','detach-during-file','detach-before-id']){
    const page=await browser.newPage();page.setDefaultTimeout(60000);
    let id=null,held=null,handlerError=null,resolveHeld;
    const heldReady=new Promise(resolve=>resolveHeld=resolve);
    const bounded=async promise=>{
      let timer;
      try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('import ownership outcome timeout: '+mode)),60000);})]);}
      finally{clearTimeout(timer);}
    };
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
        if(mode==='failed-finish'&&request.method()==='POST'&&url.pathname===`/imports/${id}/finish`){
          await request.respond({status:503,contentType:'application/json',body:JSON.stringify({version:1,code:'storage_unavailable',retryable:true})});return;
        }
        await request.continue();
      })().catch(error=>{handlerError=error;request.abort().catch(()=>{});});
    });
    try{
      await page.goto(origin);await page.waitForSelector('#directory');
      const cancelled=page.waitForResponse(response=>id&&new URL(response.url()).pathname===`/imports/${id}/cancel`&&response.request().method()==='POST');
      await page.evaluate(()=>{
        const input=document.querySelector('#directory'),transfer=new DataTransfer();
        const file=new File([new Uint8Array([1,2,3,4])],'independent-import-lifetime.txt');
        Object.defineProperty(file,'webkitRelativePath',{value:'Public/independent-import-lifetime.txt'});
        transfer.items.add(file);input.files=transfer.files;input.dispatchEvent(new Event('change',{bubbles:true}));
      });
      if(mode!=='failed-finish'){
        await bounded(heldReady);
        const aborted=mode==='detach-during-file'?new Promise(resolve=>page.on('requestfailed',request=>{if(request===held.request)resolve(request.failure()?.errorText);})):null;
        // Existing pagehide disposal unmounts the app's owner; do not navigate
        // away before it can send its best-effort cancellation request.
        await page.evaluate(()=>window.dispatchEvent(new Event('pagehide')));
        if(mode==='detach-before-id')await held.request.respond(held.payload);
        // The held XHR must be cancelled, not released to finish its upload.
        if(aborted)assert.equal(await bounded(aborted),'net::ERR_ABORTED','detaching aborts the held XHR');
      }
      assert((await bounded(cancelled)).ok(),'owned import cancellation reached the service');
      const result=await fetch(origin+`/imports/${id}`,{signal:AbortSignal.timeout(30000)});
      assert(result.ok());assert.equal((await result.json()).state,'cancelled');
      assert.equal(handlerError,null);
      completed.push(mode);
    }finally{
      if(held)await held.request.abort().catch(()=>{});
      await page.close();
    }
  }
  return {completed,cancellationConfirmedByService:true};
};
