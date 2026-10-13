// MIT public synthetic integration test. Test responses expand one lawful fixture
// into two bounded sidebar pages; the actual Rust UI performs every navigation.
const assert=require('node:assert/strict');
module.exports=async({browser,origin})=>{
  const response=await fetch(origin+'/libraries?limit=24&offset=0',{signal:AbortSignal.timeout(10000)});
  assert(response.ok);const baseline=await response.json();
  const source=baseline.items.find(item=>item.state==='ready');assert(source,'public ready fixture required');
  const libraries=Array.from({length:30},(_,index)=>({...source,id:index===0?source.id:`public-sidebar-library-${index+1}`,albums:30}));
  const albums=Array.from({length:30},(_,index)=>({id:`public-sidebar-album-${index+1}`,name:`Public album ${index+1}`,name_truncated:false,sort_order:index,captures:30}));
  const page=await browser.newPage();page.setDefaultTimeout(30000);
  let held=null,holdKind=null,failKind=null,handlerFailure=false,heldReplies=0,heldCancelled=0,heldCompleted=0,captureItems=[];
  let phase='setup',observedHeld=null;
  page.on('requestfinished',request=>{if(observedHeld?.request===request){observedHeld.finished=true;observedHeld.onFinished?.();}});
  page.on('requestfailed',request=>{if(observedHeld?.request===request){observedHeld.failed=true;observedHeld.aborted=request.failure()?.errorText==='net::ERR_ABORTED';observedHeld.settle({kind:'failed'});}});
  page.on('response',response=>{if(observedHeld?.request===response.request()){observedHeld.response=true;observedHeld.settle({kind:'response',response});}});
  const reply=async(request,payload)=>{try{await request.respond({status:200,contentType:'application/json',body:JSON.stringify(payload)});}catch{handlerFailure=true;if(observedHeld?.request===request)observedHeld.replyFailed=true;}};
  const slice=(items,offset)=>({version:1,items:items.slice(offset,offset+24),offset,next_offset:offset+24<items.length?offset+24:null});
  const intercept=request=>{
    const run=async()=>{
      const url=new URL(request.url());
      if(request.method()!=='GET'){await request.abort();return;}
      if((url.protocol==='data:'&&request.url().startsWith('data:image/jpeg;base64,')&&request.url().length<=2*1024*1024)||(url.protocol==='blob:'&&url.origin===origin&&request.url().length<=1024)){await request.continue();return;}
      if(url.origin!==origin){await request.abort();return;}
      let kind=null,payload;
      if(url.pathname==='/libraries'){kind='libraries';payload=slice(libraries,Number(url.searchParams.get('offset')||0));}
      else if(/^\/libraries\/[^/]+\/albums$/.test(url.pathname)){kind='albums';payload=slice(albums,Number(url.searchParams.get('offset')||0));}
      else if(/^\/libraries\/[^/]+\/captures$/.test(url.pathname)||/^\/albums\/public-sidebar-album-\d+\/captures$/.test(url.pathname)){
        url.pathname=`/libraries/${source.id}/captures`;url.searchParams.delete('album');
        const result=await fetch(url,{signal:AbortSignal.timeout(10000)});assert(result.ok);
        const body=await result.text();assert(Buffer.byteLength(body)<=256*1024);
        captureItems=JSON.parse(body).items;
        await request.respond({status:200,contentType:'application/json',body});return;
      }
      if(kind){
        if(failKind===kind&&Number(url.searchParams.get('offset')||0)===24){failKind=null;await request.respond({status:503,contentType:'application/json',body:JSON.stringify({version:1,code:'storage_unavailable',retryable:true})});return;}
        if(holdKind===kind&&Number(url.searchParams.get('offset')||0)===24){
          // Install the exact-request outcome fence before any UI cancellation.
          let settle;const outcome=new Promise(resolve=>settle=resolve);
          observedHeld={request,finished:false,failed:false,aborted:false,response:false,replyFailed:false,settle,outcome};
          held={request,payload,lifecycle:observedHeld};holdKind=null;return;
        }
        await reply(request,payload);return;
      }
      await request.continue();
    };
    run().catch(()=>{handlerFailure=true;request.abort().catch(()=>{});});
  };
  try{
    await page.evaluateOnNewDocument(()=>{
      Object.defineProperty(navigator,'gpu',{value:undefined});
      Object.defineProperty(window,'VideoDecoder',{value:undefined,writable:true});
    });
    await page.setRequestInterception(true);page.on('request',intercept);
    const pageErrors=[];page.on('pageerror',()=>pageErrors.push(true));
    phase='initial-ready';await page.goto(origin);await page.waitForSelector('#directory');
    await page.waitForFunction(()=>document.querySelectorAll('.library-item').length===24&&document.querySelectorAll('.album-item').length===24&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
    const click=async selector=>{await page.waitForFunction(selector=>{const b=document.querySelector(selector);if(!b||b.disabled)return false;b.click();return true;},{},selector);};
    const waitPage=async(kind,number,count)=>page.waitForFunction((kind,number,count)=>document.querySelector(`#${kind}-page`)?.textContent===`Page ${number}`&&document.querySelectorAll(`.${kind==='library'?'library':'album'}-item`).length===count,{},kind,number,count);
    const selected=()=>page.evaluate(()=>({capture:document.querySelector('.viewer-strip span')?.textContent,stage:document.querySelector('.stage img')?.src,selectedAlbum:document.querySelector('.album-item[aria-pressed="true"]')?.textContent||null}));
    // Fence each real select before clicking: shared fixture pixels need not change URL.
    const selectComplete=async(interact,index,mappedPath=null)=>{
      const expected=captureItems[index];assert(expected,'bounded mapped capture required');
      // Status only: the app may cancel a response stream once it has what it needs.
      const detailResponse=page.waitForResponse(response=>new URL(response.url()).pathname===`/captures/${expected.id}`);
      const mappedResponse=mappedPath?page.waitForResponse(response=>new URL(response.url()).pathname===mappedPath):null;
      await page.evaluate(()=>{
        const original=document.querySelector('.stage img');
        const state={changed:false,observer:null};
        state.observer=new MutationObserver(()=>{
          if(document.querySelector('.status')?.textContent==='Loading capture…'||document.querySelector('.stage img')!==original)state.changed=true;
        });
        state.observer.observe(document.body,{subtree:true,childList:true,characterData:true,attributes:true});
        window.__sidebarSelectionFence=state;
      });
      let focus=false,geometry={width:0,height:0};
      try{
        await interact();
        // Never read bodies back through CDP: the app may cancel a superseded or
        // completed response stream, after which Chrome discards its data. The
        // mapped page body is the one this test served; the detail is fetched
        // independently from the service for the exact finished request's id.
        if(mappedResponse){const response=await mappedResponse;assert(response.ok());assert.equal(captureItems[0]?.id,expected.id,'mapped page selected expected first capture');}
        const response=await detailResponse;assert(response.ok());const independent=await fetch(origin+`/captures/${expected.id}`,{signal:AbortSignal.timeout(10000)});assert(independent.ok);const bytes=Buffer.from(await independent.arrayBuffer());assert(bytes.length<=256*1024);const detail=JSON.parse(bytes);assert.equal(detail.capture.id,expected.id);
        focus=detail.originals.some(file=>file.role==='jpeg_focus_stack');
        geometry=focus?{width:360,height:240}:{width:640,height:360};
        await page.waitForFunction((expected,focus,geometry)=>{
          const image=document.querySelector('.stage img'),status=document.querySelector('.status')?.textContent;
          return window.__sidebarSelectionFence?.changed===true&&document.querySelector('.viewer-strip span')?.textContent===expected.name&&document.querySelector('.capture[aria-pressed="true"] span')?.textContent===expected.name&&image?.complete&&image.naturalWidth===geometry.width&&image.naturalHeight===geometry.height&&image.alt.endsWith(': '+expected.name)&&(focus?status?.startsWith('Discrete focus: stored plane '):status==='Static JPEG preview ready. Color profile unverified');
        },{},expected,focus,geometry);
      }catch(error){
        const diagnostic=await page.evaluate((expected,focus,geometry)=>{
          const image=document.querySelector('.stage img'),status=document.querySelector('.status')?.textContent;
          const statusKind=status==='Static JPEG preview ready. Color profile unverified'?'staticready':status?.startsWith('Discrete focus: stored plane ')?'focusready':status==='Discrete focus unavailable. Static preview and originals remain usable'?'focusunavailable':status==='Loading capture…'?'loading':'other';
          return {sidebarSelectionDiagnostic:true,fenceChanged:window.__sidebarSelectionFence?.changed===true,gridMatch:document.querySelector('.capture[aria-pressed="true"] span')?.textContent===expected.name,viewerMatch:document.querySelector('.viewer-strip span')?.textContent===expected.name,altMatch:image?.alt.endsWith(': '+expected.name)===true,imageComplete:image?.complete===true,naturalWidth:image?.naturalWidth||0,naturalHeight:image?.naturalHeight||0,expectedWidth:geometry.width,expectedHeight:geometry.height,focusFlag:focus,statusKind};
        },expected,focus,geometry).catch(()=>({sidebarSelectionDiagnosticUnavailable:true}));
        console.error(JSON.stringify(diagnostic));
        throw error;
      }finally{
        await page.evaluate(()=>{window.__sidebarSelectionFence?.observer.disconnect();delete window.__sidebarSelectionFence;});
      }
    };
    const release=async(expected='aborted')=>{
      phase+='-release';const old=held;held=null;assert(old);
      let timer;
      const deadline=new Promise((_,reject)=>timer=setTimeout(()=>reject(Error('held sidebar outcome timeout')),page.getDefaultTimeout()));
      const settlement=(async()=>{await reply(old.request,old.payload);return old.lifecycle.outcome;})();
      const outcome=await Promise.race([settlement,deadline]).finally(()=>clearTimeout(timer));
      if(expected==='aborted'){
        assert.equal(outcome.kind,'failed','obsolete sidebar request must actually fail');
        assert.equal(old.lifecycle.aborted,true,'obsolete request must specifically settle net::ERR_ABORTED');
        assert.equal(old.lifecycle.response,false,'cancelled obsolete request cannot produce HTTP response');
        assert.equal(old.lifecycle.finished,false);heldCancelled++;
      }else{
        assert.equal(expected,'completed');assert.equal(outcome.kind,'response');
        // Read the exact held request's body only after it finished loading.
        let finishTimer;await new Promise((resolve,reject)=>{finishTimer=setTimeout(()=>reject(Error('held sidebar body never finished')),page.getDefaultTimeout());if(old.lifecycle.finished)resolve();else old.lifecycle.onFinished=resolve;}).finally(()=>clearTimeout(finishTimer));
        assert.equal(outcome.response.status(),200);assert(Buffer.byteLength(JSON.stringify(old.payload))<=256*1024,'bounded current response completed');
        assert.equal(old.lifecycle.failed,false);assert.equal(old.lifecycle.aborted,false);heldCompleted++;
      }
      await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      if(expected==='completed')assert.equal(old.lifecycle.finished,true,'current response body actually completed');
      heldReplies++;
    };
    // Select an album/capture, then page both sidebars without changing the viewer.
    phase='album-select';
    await selectComplete(()=>click('.album-item'),0,'/albums/public-sidebar-album-1/captures');
    assert(await page.$('.album-item[aria-pressed="true"]'));
    for(const selector of ['.library-item','.album-item','.capture']){
      const tokens=await page.$$eval(selector,buttons=>buttons.map(button=>button.getAttribute('aria-pressed')));
      assert.equal(tokens.length,24);assert(tokens.every(token=>token==='true'||token==='false'),'valid ARIA pressed tokens');
      assert.equal(tokens.filter(token=>token==='true').length,1);assert.equal(tokens.filter(token=>token==='false').length,23);
    }
    const first=await selected();
    phase='second-capture';
    await selectComplete(()=>page.evaluate(()=>document.querySelectorAll('.capture')[1].click()),1);
    assert.notEqual((await selected()).capture,first.capture);
    const before=await selected();assert(before.capture&&before.capture!=='No capture selected'&&before.stage);
    phase='album-page-next';
    await click('#album-next');await waitPage('album',2,6);
    assert.equal(await page.$('.album-item[aria-pressed="true"]'),null,'selected album may be off-page');
    assert.deepEqual(await page.evaluate(()=>({capture:document.querySelector('.viewer-strip span').textContent,stage:document.querySelector('.stage img').src})),{capture:before.capture,stage:before.stage});
    phase='album-page-prev';
    await click('#album-prev');await waitPage('album',1,24);
    assert.equal((await selected()).selectedAlbum,before.selectedAlbum);
    phase='library-page-next';
    await click('#library-next');await waitPage('library',2,6);
    assert.deepEqual(await page.$$eval('.library-item',buttons=>buttons.map(b=>b.textContent)),Array.from({length:6},(_,i)=>`Library ${25+i}`));
    assert.equal(await page.$('.library-item[aria-pressed="true"]'),null,'selected library remains active off-page');
    assert.equal((await selected()).capture,before.capture);assert.equal((await selected()).stage,before.stage);
    phase='library-page-prev';
    await click('#library-prev');await waitPage('library',1,24);
    assert(await page.$('.library-item[aria-pressed="true"]'));
    // Hold an older Next response; Previous is reachable while Next is pending.
    const delayed=async kind=>{
      phase='delayed-'+kind;holdKind=kind==='library'?'libraries':'albums';
      await click(`#${kind}-next`);
      for(let i=0;i<150&&!held;i++)await new Promise(resolve=>setTimeout(resolve,20));
      assert(held,'bounded held sidebar response');
      await click(`#${kind}-prev`);await waitPage(kind,1,24);
      await release();
      await waitPage(kind,1,24);
      assert.equal((await selected()).capture,before.capture,'sidebar races preserve selected capture');
    };
    await delayed('library');await delayed('album');
    phase='capture-during-album';
    // Capture selection has its own generation and must not cancel album paging.
    holdKind='albums';await click('#album-next');
    for(let i=0;i<150&&!held;i++)await new Promise(resolve=>setTimeout(resolve,20));assert(held);
    const prior=await selected();
    await selectComplete(async()=>{
      await page.evaluate(()=>document.querySelectorAll('.capture')[2].click());
      // The current album reserves the existing control working lease. Settle
      // it before waiting for the new capture's control response, without abort.
      await release('completed');
    },2);
    assert.notEqual((await selected()).capture,prior.capture);
    const duringCapture=await selected();assert(duringCapture.capture!=='No capture selected');
    await waitPage('album',2,6);
    assert.equal((await selected()).capture,duringCapture.capture);
    await click('#album-prev');await waitPage('album',1,24);
    phase='library-during-album';
    // A pending old-library album request must not replace the new library page.
    holdKind='albums';await click('#album-next');
    for(let i=0;i<150&&!held;i++)await new Promise(resolve=>setTimeout(resolve,20));assert(held);
    await selectComplete(()=>page.evaluate(()=>document.querySelectorAll('.library-item')[1].click()),0,'/libraries/public-sidebar-library-2/captures');
    await page.waitForFunction(()=>document.querySelectorAll('.library-item')[1]?.getAttribute('aria-pressed')==='true'&&document.querySelectorAll('.album-item').length===24&&document.querySelector('.album-item[aria-pressed="true"]')===null);
    await release();await waitPage('album',1,24);
    assert.equal(await page.$('.album-item[aria-pressed="true"]'),null,'new library keeps existing album reset semantics');
    await page.waitForFunction(()=>document.querySelectorAll('.capture').length===24&&document.querySelectorAll('.capture img').length===24&&[...document.querySelectorAll('.capture img')].every(image=>image.complete&&image.naturalWidth>0));
    const completedLibrary=await selected();assert(completedLibrary.capture&&completedLibrary.capture!=='No capture selected'&&completedLibrary.stage);
    // A failed requested page leaves the committed rows/selection reachable.
    for(const kind of ['library','album']){
      phase='failed-'+kind;failKind=kind==='library'?'libraries':'albums';
      // Track this response explicitly; previous failure status is insufficient.
      const failedResponse=page.waitForResponse(response=>{
        const url=new URL(response.url());
        return response.status()===503&&Number(url.searchParams.get('offset'))===24&&(kind==='library'?url.pathname==='/libraries':/^\/libraries\/[^/]+\/albums$/.test(url.pathname));
      });
      await click(`#${kind}-next`);await failedResponse;
      await page.waitForFunction(kind=>document.querySelector(`#${kind}-prev`)?.disabled===true&&document.querySelector('.status')?.textContent==='The service could not complete this request. Retry or inspect import status',{},kind);
      await waitPage(kind,1,24);
      const preserved=await selected();assert.equal(preserved.capture,completedLibrary.capture);assert.equal(preserved.stage,completedLibrary.stage);
      await click(`#${kind}-next`);await waitPage(kind,2,6);
      await click(`#${kind}-prev`);await waitPage(kind,1,24);
    }
    assert.equal(heldCancelled,3);assert.equal(heldCompleted,1);assert.equal(heldReplies,4);
    assert.equal(handlerFailure,false);assert.deepEqual(pageErrors,[]);
    return {publicSynthetic:true,libraryItems:30,albumItems:30,pageLimit:24,absoluteLibraryNumbers:true,selectedCapturePreserved:true,offPageSelectionPreserved:true,obsoleteSidebarRequestsCancelled:heldCancelled,currentSidebarResponseCompleted:heldCompleted,captureOperationsDoNotInvalidateSidebar:true,newLibraryResetsAlbum:true,failedPageRecovery:true};
  }catch(error){
    const state=await page.evaluate(()=>({libraryRows:document.querySelectorAll('.library-item').length,albumRows:document.querySelectorAll('.album-item').length,captureRows:document.querySelectorAll('.capture').length,stageReady:document.querySelector('.stage img')?.complete===true&&document.querySelector('.stage img')?.naturalWidth>0})).catch(()=>null);
    console.error(JSON.stringify({sidebarPhaseDiagnostic:true,phase,heldObserved:observedHeld!==null,heldFinished:observedHeld?.finished===true,heldFailed:observedHeld?.failed===true,heldAborted:observedHeld?.aborted===true,heldResponse:observedHeld?.response===true,heldReplyFailed:observedHeld?.replyFailed===true,handlerFailure,heldReplies,heldCancelled,heldCompleted,state}));
    throw error;
  }finally{
    if(held)await held.request.abort().catch(()=>{});
    await page.close();
  }
};
