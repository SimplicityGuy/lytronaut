// MIT. Public synthetic app-owner regression; no private mode or content reports.
const assert=require('node:assert/strict');
module.exports=async({browser,origin})=>{
  assert(!process.env.LYTRONAUT_VIDEO_PRIVATE_ROOT&&!process.env.LYTRONAUT_PRIVATE_SEALED_ROOT&&process.env.LYTRONAUT_PRIVATE_TEST!=='1');
  const fixture=await fetch(origin+'/libraries',{signal:AbortSignal.timeout(10000)});assert(fixture.ok);
  const source=(await fixture.json()).items.find(library=>library.state==='ready');assert(source);
  const page=await browser.newPage();page.setDefaultTimeout(30000);
  const errors=[];page.on('pageerror',()=>errors.push(true));
  let phase='setup';
  try {
    await page.evaluateOnNewDocument(source=>{
      Object.defineProperty(navigator,'gpu',{value:undefined});
      const native={fetch:window.fetch.bind(window),create:URL.createObjectURL,revoke:URL.revokeObjectURL,blob:Blob,image:Image,decode:HTMLImageElement.prototype.decode};
      const probe=window.__lifetime={native,source,mode:'',lastMainSort:null,lastMainAlias:null,lastAlbumAlias:null,libraryCalls:0,captureCalls:0,posts:0,hold:null,releasePost:null,postAccepted:false,urls:new Map(),created:0,revoked:0,blobMax:0,abortCreate:null,createFailure:false,imageFailure:false,decodeFailure:false,imageFailures:0,decodeFailures:0,reenter:null};
      URL.createObjectURL=function(blob){if(probe.createFailure)throw Error('public URL creation failure');const url=native.create.call(this,blob);probe.created++;probe.urls.set(url,blob.size);probe.blobMax=Math.max(probe.blobMax,blob.size);if(probe.abortCreate){const abort=probe.abortCreate;probe.abortCreate=null;abort.abort();}return url;};
      URL.revokeObjectURL=function(url){if(probe.urls.delete(url))probe.revoked++;return native.revoke.call(this,url);};
      window.Image=function(...args){if(probe.imageFailure){probe.imageFailures++;throw Error('public image construction failure');}return new native.image(...args);};
      HTMLImageElement.prototype.decode=function(){if(probe.decodeFailure){probe.decodeFailures++;return Promise.reject(Error('public image decode failure'));}return native.decode.call(this);};
      window.fetch=async(input,init)=>{
        let request=input instanceof Request?input:new Request(input,init);const url=new URL(request.url);
        if(url.origin!==location.origin&&url.protocol!=='blob:')throw Error('public probe rejects external requests');
        const alias=url.pathname.match(/^\/libraries\/public-lifetime-library-([01])\//);
        if(/\/captures$/.test(url.pathname)){probe.lastMainSort=url.searchParams.get('sort')||'order';probe.lastMainAlias=alias?Number(alias[1]):null;}
        if(/\/albums$/.test(url.pathname))probe.lastAlbumAlias=alias?Number(alias[1]):null;
        const libraries=url.pathname==='/libraries';const capture=/^\/captures\/[^/]+$/.test(url.pathname);
        if(libraries)probe.libraryCalls++;if(capture)probe.captureCalls++;
        if((libraries&&probe.mode==='hold-libraries')||(capture&&probe.mode==='hold-capture')||(/\/captures$/.test(url.pathname)&&probe.mode==='hold-main')||(/\/albums$/.test(url.pathname)&&probe.mode==='hold-albums')){
          probe.mode='';let releaseRead,releaseCancel;
          const read=new Promise(resolve=>releaseRead=()=>resolve({done:true}));const cancellation=new Promise(resolve=>releaseCancel=resolve);
          const stream=new ReadableStream();const getReader=stream.getReader.bind(stream);
          stream.getReader=function(...args){const reader=getReader(...args),cancel=reader.cancel.bind(reader);reader.read=()=>read;reader.cancel=()=>{probe.hold.cancelled++;cancel().catch(()=>{});return cancellation;};return reader;};
          probe.hold={cancelled:0,releaseRead,releaseCancel};
          if(probe.reenter){request.signal.addEventListener('abort',()=>{const callback=probe.reenter;probe.reenter=null;callback();},{once:true});}
          return new Response(stream,{headers:{'Content-Type':'application/json'}});
        }
        if(/^\/libraries\/public-lifetime-library-[01]\//.test(url.pathname)){url.pathname=url.pathname.replace(/public-lifetime-library-[01]/,source.id);request=new Request(url,request);}
        if(request.method==='POST'&&url.pathname==='/exports')probe.posts++;
        const response=await native.fetch(request);
        if(libraries&&response.ok){const body=await response.json();return new Response(JSON.stringify({...body,items:[0,1].map(index=>({...source,id:'public-lifetime-library-'+index}))}),{headers:{'Content-Type':'application/json'}});}
        if(request.method==='POST'&&url.pathname==='/exports'){
          if(probe.mode==='hold-post'){assertOk(response);probe.mode='';probe.postAccepted=true;return new Promise(resolve=>probe.releasePost=()=>resolve(response));}
        }
        return response;
      };
      function assertOk(response){if(!response.ok)throw Error('public accepted mutation required');}
    },source);
    await page.goto(origin);await page.waitForSelector('#directory');
    await page.evaluate(async()=>window.__lifetimeModule=await import('/client/lytronaut_client.js'));
    const ready=()=>page.waitForFunction(()=>document.querySelectorAll('.capture').length===24&&document.querySelectorAll('.capture img').length===24&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
    const settle=()=>page.evaluate(async()=>{await new Promise(resolve=>setTimeout(resolve,0));await new Promise(resolve=>setTimeout(resolve,0));});
    await ready();
    phase='jpeg-byte-identity';
    const jpeg=await page.evaluate(async()=>{
      const m=window.__lifetimeModule,p=window.__lifetime;const canvas=document.createElement('canvas');canvas.width=canvas.height=16;
      const context=canvas.getContext('2d');const image=context.createImageData(16,16);for(let i=0;i<image.data.length;i++)image.data[i]=(i*73+29)&255;context.putImageData(image,0,0);
      let url='';try{
        const data=canvas.toDataURL('image/jpeg');const binary=atob(data.split(',')[1]);const expected=Uint8Array.from(binary,character=>character.charCodeAt(0));
        if(!expected.some(value=>value>127))throw Error('high-bit JPEG fixture required');
        url=m.harness_display_jpeg(data,2*1024*1024,new AbortController());const response=await p.native.fetch(url);const actual=new Uint8Array(await response.arrayBuffer());
        if(response.headers.get('Content-Type')!=='image/jpeg'||actual.length!==expected.length||actual.some((value,index)=>value!==expected[index]))throw Error('display bytes or MIME mismatch');
        const digest=async bytes=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))).join(':');if(await digest(actual)!==await digest(expected))throw Error('display digest mismatch');
        let rejected=0;const reject=(fn,error)=>{try{fn();}catch(actual){if(actual!==error)throw Error('display rejection mismatch');rejected++;return;}throw Error('display rejection missing');};
        reject(()=>m.harness_display_jpeg(data,data.length-1,new AbortController()),'Invalid display JPEG');
        reject(()=>m.harness_display_jpeg('data:image/jpeg;base64,!',2*1024*1024,new AbortController()),'Invalid display JPEG');
        p.createFailure=true;try{reject(()=>m.harness_display_jpeg(data,2*1024*1024,new AbortController()),'Display unavailable');}finally{p.createFailure=false;}
        const abort=new AbortController();p.abortCreate=abort;const before=p.urls.size;
        reject(()=>m.harness_display_jpeg(data,2*1024*1024,abort),'Cancelled');if(!abort.signal.aborted||p.urls.size!==before)throw Error('abort-create URL leaked');
        return {byteIdentical:true,shaIdentical:true,highBitBytes:true,mimeVerified:true,rejections:rejected,logicalJpegBytes:actual.length};
      }finally{if(url)URL.revokeObjectURL(url);canvas.width=canvas.height=0;p.abortCreate=null;p.createFailure=false;}
    });
    assert.equal(jpeg.rejections,4);assert(jpeg.logicalJpegBytes>0);
    for(const first of ['read','cancel']){
      phase='stalled-'+first;
      await page.evaluate(()=>{window.__lifetimeModule.harness_dispose();window.__lifetime.mode='hold-libraries';window.__lifetime.hold=null;window.__lifetimeModule.harness_remount();});
      await page.waitForFunction(()=>window.__lifetime.hold!==null);await settle();
      const calls=await page.evaluate(()=>{const p=window.__lifetime;const calls=p.libraryCalls;for(let i=0;i<100;i++)window.__lifetimeModule.harness_remount();return calls;});
      await page.waitForFunction(()=>window.__lifetime.hold.cancelled===1);
      await settle();assert.equal(await page.evaluate(()=>window.__lifetime.libraryCalls),calls,'remounts reuse draining lane');
      await page.evaluate(first=>window.__lifetime.hold[first==='read'?'releaseRead':'releaseCancel'](),first);
      await settle();assert.equal(await page.evaluate(()=>window.__lifetime.libraryCalls),calls,'remaining native bridge still owns lane');
      await page.evaluate(first=>window.__lifetime.hold[first==='read'?'releaseCancel':'releaseRead'](),first);
      await ready();
    }
    phase='selected-burst';
    const selectedName=await page.evaluate(()=>{
      const captures=[...document.querySelectorAll('.capture')];const expected=captures[2].querySelector('span').textContent;
      for(let i=0;i<100;i++){captures[i%3].click();document.querySelector('#focus').dispatchEvent(new Event('input',{bubbles:true}));[...document.querySelectorAll('button')].find(button=>button.textContent==='Try browser H.264 refocus').dispatchEvent(new MouseEvent('click',{bubbles:true}));}
      captures[2].click();return expected;
    });
    await page.waitForFunction(expected=>document.querySelector('.viewer-strip span')?.textContent===expected&&document.querySelector('.capture[aria-pressed="true"] span')?.textContent===expected&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0,{},selectedName);
    phase='selected-reentrant-abort';
    await page.evaluate(()=>{const p=window.__lifetime;p.mode='hold-capture';p.hold=null;p.reenter=()=>document.querySelectorAll('.capture')[2].click();document.querySelectorAll('.capture')[0].click();});
    await page.waitForFunction(()=>window.__lifetime.hold!==null);await settle();
    await page.evaluate(()=>document.querySelectorAll('.capture')[1].click());
    await page.waitForFunction(()=>window.__lifetime.hold.cancelled===1);
    await page.evaluate(()=>{window.__lifetime.hold.releaseRead();window.__lifetime.hold.releaseCancel();});
    await page.waitForFunction(()=>document.querySelector('.capture[aria-pressed="true"]')===document.querySelectorAll('.capture')[2]&&document.querySelector('.stage img')?.complete&&document.querySelector('.stage img')?.naturalWidth>0);
    phase='main-reentrant-abort';
    await page.evaluate(()=>{const p=window.__lifetime;p.mode='hold-main';p.hold=null;p.reenter=()=>{const sort=document.querySelector('#sort');sort.value='rating';sort.dispatchEvent(new Event('change',{bubbles:true}));};const sort=document.querySelector('#sort');sort.value='name';sort.dispatchEvent(new Event('change',{bubbles:true}));});
    await page.waitForFunction(()=>window.__lifetime.hold!==null);await settle();
    await page.evaluate(()=>{const sort=document.querySelector('#sort');sort.value='order';sort.dispatchEvent(new Event('change',{bubbles:true}));});
    await page.waitForFunction(()=>window.__lifetime.hold.cancelled===1);await page.evaluate(()=>{window.__lifetime.hold.releaseRead();window.__lifetime.hold.releaseCancel();});await ready();
    assert.equal(await page.$eval('#sort',element=>element.value),'rating');
    assert.equal(await page.evaluate(()=>window.__lifetime.lastMainSort),'rating','newer Main descriptor, not only control value');
    const rated=await fetch(origin+`/libraries/${source.id}/captures?limit=24&offset=0&sort=rating`,{signal:AbortSignal.timeout(10000)});assert(rated.ok);
    const expectedRated=(await rated.json()).items.map(capture=>capture.name);assert.equal(expectedRated.length,24);
    await page.waitForFunction(expected=>JSON.stringify([...document.querySelectorAll('.capture span')].map(span=>span.textContent))===JSON.stringify(expected),{},expectedRated);
    phase='library-reentrant-abort';
    await page.evaluate(()=>{const p=window.__lifetime;p.mode='hold-albums';p.hold=null;p.reenter=()=>document.querySelectorAll('.library-item')[1].click();document.querySelectorAll('.library-item')[0].click();});
    await page.waitForFunction(()=>window.__lifetime.hold!==null);await settle();
    await page.evaluate(()=>document.querySelectorAll('.library-item')[0].click());await page.waitForFunction(()=>window.__lifetime.hold.cancelled===1);
    await page.evaluate(()=>{window.__lifetime.hold.releaseRead();window.__lifetime.hold.releaseCancel();});await ready();
    assert.equal(await page.$eval('.library-item[aria-pressed="true"]',element=>element.textContent),'Library 2');
    assert.deepEqual(await page.evaluate(()=>[window.__lifetime.lastMainAlias,window.__lifetime.lastAlbumAlias]),[1,1],'newer library owns both Main and Albums continuations');
    phase='action-before-first-poll';
    const before=await page.evaluate(()=>{const before=window.__lifetime.posts;document.querySelector('#backup-library').click();window.__lifetimeModule.harness_dispose();return before;});await settle();
    assert.equal(await page.evaluate(()=>window.__lifetime.posts),before,'unpolled detached action issued no mutation');
    await page.evaluate(()=>window.__lifetimeModule.harness_remount());await ready();
    phase='accepted-mutation-recovery';
    await page.evaluate(()=>{window.__lifetime.mode='hold-post';sessionStorage.removeItem('lytronaut-export');for(let i=0;i<100;i++)document.querySelector('#backup-library').click();});
    await page.waitForFunction(()=>window.__lifetime.postAccepted);assert.equal(await page.evaluate(()=>window.__lifetime.posts),before+1,'burst admits one durable mutation');
    await page.evaluate(()=>{window.__lifetimeModule.harness_dispose();window.__lifetimeModule.harness_remount();window.__lifetime.releasePost();});
    await page.waitForFunction(()=>sessionStorage.getItem('lytronaut-export')!==null);await ready();await settle();
    await page.click('#recover-export');await page.waitForFunction(()=>document.querySelector('#export-progress')?.textContent.startsWith('Succeeded'));
    phase='constructor-decode-cleanup';
    for(const fault of ['imageFailure','decodeFailure']){
      await page.evaluate(fault=>{window.__lifetimeModule.harness_dispose();window.__lifetime[fault]=true;window.__lifetimeModule.harness_remount();},fault);
      await page.waitForFunction(fault=>window.__lifetime[fault==='imageFailure'?'imageFailures':'decodeFailures']>0&&document.querySelectorAll('.capture').length===24&&document.querySelectorAll('.capture .thumb-empty').length>0,{},fault);
      await page.evaluate(fault=>{window.__lifetimeModule.harness_dispose();window.__lifetime[fault]=false;window.__lifetimeModule.harness_remount();},fault);await ready();
    }
    phase='url-cleanup';await page.evaluate(()=>window.__lifetimeModule.harness_dispose());
    await page.waitForFunction(()=>window.__lifetime.urls.size===0);const urls=await page.evaluate(()=>({created:window.__lifetime.created,revoked:window.__lifetime.revoked,logicalJpegBytesMax:window.__lifetime.blobMax}));
    assert(urls.created>0);assert.equal(urls.created,urls.revoked);assert(urls.logicalJpegBytesMax>0);assert.deepEqual(errors,[]);
    return {jpeg,stalledRead:true,stalledCancel:true,remountDrains:true,synchronousCaptureControlBurst:true,reentrantSelected:true,reentrantMain:true,reentrantLibrary:true,actionBeforePollDisposed:true,acceptedMutationRecovery:true,constructorFailure:true,decodeFailure:true,rendererDestroyReentryCovered:false,rendererDestroyReentryPending:true,urlsReleasedBeforeClose:true,...urls,capacityCollection:false,totalMemoryProof:false};
  }catch{throw Error('public browser lifetime regression failed: '+phase);}
  finally{
    try{if(!page.isClosed())await page.evaluate(()=>{const p=window.__lifetime;if(!p)return;if(p.hold){p.hold.releaseRead();p.hold.releaseCancel();}if(p.releasePost)p.releasePost();window.__lifetimeModule?.harness_dispose();window.fetch=p.native.fetch;URL.createObjectURL=p.native.create;URL.revokeObjectURL=p.native.revoke;window.Image=p.native.image;HTMLImageElement.prototype.decode=p.native.decode;});}
    finally{await page.close();}
  }
};
