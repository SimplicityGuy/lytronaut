// MIT public-only experiment. No private intake, pixels, identifiers or URLs in reports.
const assert=require('node:assert/strict');
module.exports=async({page,click,importCatalog,stressCatalog,retryCatalog,retryBytes})=>{
  await page.evaluate(async()=>{
    window.__stressModule=await import('/client/lytronaut_client.js');
    window.__stressInvoke=async(bytes,controller=new AbortController())=>{
      const timer=setTimeout(()=>controller.abort(),60000);
      try{return await window.__stressModule.export_archived_live_photo(bytes,0,'A8A8A8A8-1111-4444-8888-121212121212',controller.signal);}
      finally{clearTimeout(timer);}
    };
    const native={decoder:VideoDecoder,close:VideoFrame.prototype.close,canvas:HTMLCanvasElement.prototype.toDataURL,anchor:HTMLAnchorElement.prototype.click};
    const counts={created:0,closed:0,outputs:0,framesClosed:0,downloads:0};const seen=new WeakSet();
    let canvasRecords=[],dropped=0,cancel=null;
    // Restoration exists before mutations; no payload objects enter observations.
    window.__stressRestore=()=>{window.VideoDecoder=native.decoder;VideoFrame.prototype.close=native.close;HTMLCanvasElement.prototype.toDataURL=native.canvas;HTMLAnchorElement.prototype.click=native.anchor;};
    try{
      VideoFrame.prototype.close=function(){if(!seen.has(this)){seen.add(this);counts.framesClosed++;}return Reflect.apply(native.close,this,[]);};
      window.VideoDecoder=class extends native.decoder{
        constructor(init){super({...init,output:frame=>{counts.outputs++;init.output(frame);}});this.closedByTest=false;counts.created++;}
        close(){if(!this.closedByTest){this.closedByTest=true;counts.closed++;}return super.close();}
        decode(chunk){super.decode(chunk);if(cancel){const controller=cancel;cancel=null;controller.abort();}}
      };
      HTMLCanvasElement.prototype.toDataURL=function(...args){const result=Reflect.apply(native.canvas,this,args);if(canvasRecords.length===32)dropped++;else canvasRecords.push([this.width,this.height,result.length]);return result;};
      HTMLAnchorElement.prototype.click=function(...args){if(this.download==='lytronaut-live-photo.zip'){counts.downloads++;return;}return Reflect.apply(native.anchor,this,args);};
      window.__stressStart=()=>{canvasRecords=[];dropped=0;window.__stressModule.harness_capacity_start();};
      window.__stressTake=()=>{const text=window.__stressModule.harness_capacity_take();if(text.length>192*1024)throw Error('bounded stress capacity required');return {rust:JSON.parse(text),canvas:canvasRecords,dropped};};
      window.__stressCounts=()=>({...counts});window.__stressCancel=controller=>{cancel=controller;};
    }catch(error){window.__stressRestore();throw error;}
  });
  const report={viewerFallback:false,directRejected:false,jsJpegCharacters:0,jsUtf16LogicalBytes:0,rustJpegLength:0,rustJpegCapacity:0,crossed:false,cancelled:false,retryViewer:false,retryExport:false,freshSourceVerified:false,cleanupFailed:false,complete:false,dropped:0};
  const take=async()=>{
    const trace=await page.evaluate(()=>window.__stressTake());
    assert.equal(trace.rust.complete,true);assert.equal(trace.rust.dropped,0);assert.equal(trace.dropped,0);
    assert(Array.isArray(trace.canvas)&&trace.canvas.length<=32);
    const available=new Map();
    for(const record of trace.canvas){assert.equal(record.length,3);for(const value of record)assert(Number.isSafeInteger(value)&&value>=0&&value<=128*1024*1024);const key=record.join(':');available.set(key,(available.get(key)||0)+1);}
    for(const observation of trace.rust.observations){
      assert.equal(observation.bytes.length,4);for(const value of observation.bytes)assert(Number.isSafeInteger(value)&&value>=0&&value<=128*1024*1024);
      if(observation.stage!=='jpeg_platform_result')continue;
      const [capacity,length,width,height]=observation.bytes;const key=[width,height,length].join(':');
      assert((available.get(key)||0)>0,'native ASCII JPEG must match actual Rust precheck');available.set(key,available.get(key)-1);
      if(width===2048&&height===1024){report.jsJpegCharacters=Math.max(report.jsJpegCharacters,length);report.jsUtf16LogicalBytes=2*report.jsJpegCharacters;report.rustJpegLength=Math.max(report.rustJpegLength,length);report.rustJpegCapacity=Math.max(report.rustJpegCapacity,capacity);}
    }
    return trace.rust.observations.some(o=>o.stage==='jpeg_platform_result'&&o.bytes[2]===2048&&o.bytes[3]===1024&&o.bytes[1]>2*1024*1024);
  };
  const settled=()=>page.waitForFunction(()=>{const counts=window.__stressCounts();return counts.created===counts.closed&&counts.outputs===counts.framesClosed;});
  let failure;
  try{
    assert.equal(await importCatalog(stressCatalog),true);
    const retained=await page.$eval('.stage img',image=>image.src);
    await page.evaluate(()=>window.__stressStart());
    let viewerCrossed;
    try{
      await click('Try browser H.264 refocus');
      await page.waitForFunction(()=>{const text=document.querySelector('.status')?.textContent||'';return text.includes('Browser video decode unavailable. Verified JPEG/static preview retained')||text.includes('Archived discrete browser focus decoded');});
    }finally{viewerCrossed=await take();}
    report.viewerFallback=await page.evaluate(previous=>document.querySelector('.status').textContent.includes('Browser video decode unavailable. Verified JPEG/static preview retained')&&document.querySelector('.stage img').src===previous&&document.querySelector('#export-live-photo').disabled,retained);
    assert.equal((await page.evaluate(()=>window.__stressCounts())).downloads,0);
    await settled();
    await page.evaluate(()=>window.__stressStart());
    let directCrossed;
    try{
      report.directRejected=await page.evaluate(async()=>{
        try{const zip=await window.__stressInvoke(window.__stressBytes);return false;}
        catch(error){if(error instanceof TypeError||String(error)!=='actual browser codec or movie encode failed; retain ordinary still export')throw Error('unexpected direct rejection');return true;}
      });
    }finally{directCrossed=await take();}
    await settled();
    report.crossed=viewerCrossed&&directCrossed;
    // In-flight real decode cancellation, followed by fresh admitted color input.
    report.cancelled=await page.evaluate(async()=>{
      const controller=new AbortController();window.__stressCancel(controller);
      try{await window.__stressInvoke(window.__stressBytes,controller);return false;}
      catch(error){if(error instanceof TypeError||String(error)!=='cancelled'||!controller.signal.aborted)throw Error('real Rust cancellation required');return true;}
    });
    assert.equal(report.cancelled,true);await settled();
    assert.equal(await importCatalog(retryCatalog),true);report.freshSourceVerified=true;
    await click('Try browser H.264 refocus');
    await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Archived discrete browser focus decoded')&&!document.querySelector('#export-live-photo').disabled);
    await page.waitForFunction(()=>{const image=document.querySelector('.stage img');return image?.complete&&image.naturalWidth===32&&image.naturalHeight===32;});
    report.retryViewer=true;
    report.retryExport=await page.evaluate(async bytes=>{const zip=await window.__stressInvoke(new Uint8Array(bytes));return zip.length>0&&zip.length<=18*1024*1024;},retryBytes);
    assert.equal(report.retryExport,true);
    await click('Export Live Photo pair (.zip)');
    await page.waitForFunction(()=>document.querySelector('.status').textContent.includes('Complete Live Photo ZIP downloaded'));
    await page.evaluate(()=>window.__stressModule.harness_dispose());await settled();
    Object.assign(report,await page.evaluate(()=>window.__stressCounts()));assert.equal(report.downloads,1);
    assert(report.created>0&&report.created===report.closed&&report.outputs>0&&report.outputs===report.framesClosed);
    report.complete=true;
  }catch(error){failure=error;}
  finally{
    // Await restoration before the driver closes resources; never replace partial proof.
    try{await page.evaluate(()=>{try{window.__stressModule.harness_dispose();}finally{window.__stressRestore();window.__stressBytes=null;}});}
    catch{report.cleanupFailed=true;report.complete=false;failure ||= Error('stress restoration failed');}
  }
  if(failure){failure.experiment=report;throw failure;}
  return report;
};
