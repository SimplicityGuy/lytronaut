// MIT. Real reference-host WebGPU promises; public synthetic fixture only.
const assert=require('node:assert/strict');
module.exports=async({browser,origin})=>{
  assert(!process.env.LYTRONAUT_VIDEO_PRIVATE_ROOT&&!process.env.LYTRONAUT_PRIVATE_SEALED_ROOT&&process.env.LYTRONAUT_PRIVATE_TEST!=='1');
  const reports=[];
  for(const held of ['adapter','device']){
    const page=await browser.newPage();page.setDefaultTimeout(30000);
    let errors=0;page.on('pageerror',()=>errors++);
    let result;
    try{
      await page.setRequestInterception(true);
      page.on('request',request=>{const u=new URL(request.url());const allowed=u.origin===origin||u.protocol==='blob:'||request.url().startsWith('data:image/jpeg;');(allowed?request.continue():request.abort()).catch(()=>{});});
      await page.evaluateOnNewDocument(held=>{
        if(!navigator.gpu)throw Error('actual reference GPU required');
        const p=window.__gpuLifetime={held,adapterCalls:0,deviceCalls:0,created:0,destroyed:0,release:null,holding:false,lateDestroyPending:false,lateDestroyCalls:0,lateSnapshot:null,armed:false};
        const nativeRequestAdapter=navigator.gpu.requestAdapter;const requestAdapter=nativeRequestAdapter.bind(navigator.gpu);p.restore=()=>navigator.gpu.requestAdapter=nativeRequestAdapter;
        const hold=async()=>{p.holding=true;await new Promise(resolve=>p.release=resolve);p.holding=false;p.release=null;};
        navigator.gpu.requestAdapter=async(...args)=>{
          p.adapterCalls++;const adapter=await requestAdapter(...args);if(!adapter)throw Error('actual adapter required');
          if(p.held==='adapter'){p.held='';await hold();}
          const requestDevice=adapter.requestDevice.bind(adapter);
          adapter.requestDevice=async(...args)=>{
            p.deviceCalls++;const device=await requestDevice(...args);p.created++;
            const destroy=device.destroy.bind(device);let destroyed=false;
            device.destroy=()=>{
              if(!destroyed){destroyed=true;p.destroyed++;
                if(p.armed){p.armed=false;p.lateDestroyCalls++;p.lateDestroyPending=window.__gpuModule.harness_gpu_acquisition_pending();p.lateSnapshot={pending:p.lateDestroyPending,calls:p.lateDestroyCalls,destroyed:p.destroyed,created:p.created};const calls=p.adapterCalls;window.__gpuModule.harness_remount();window.__gpuModule.harness_dispose();if(p.adapterCalls!==calls)throw Error('destroy reentry issued acquisition');}
              }
              return destroy();
            };
            if(p.held==='device'){p.held='';await hold();}
            return device;
          };return adapter;
        };
      },held);
      await page.goto(origin);await page.waitForSelector('#directory');
      await page.evaluate(async()=>window.__gpuModule=await import('/client/lytronaut_client.js'));
      const select=async()=>{
        await page.waitForFunction(()=>document.querySelector('#sort')&&document.querySelectorAll('.capture').length>0);
        if(await page.$eval('#sort',element=>element.value)!=='name')await page.select('#sort','name');
        await page.waitForFunction(()=>document.querySelector('.capture span')?.textContent==='Synthetic 000');
        await page.evaluate(()=>{if(document.querySelector('.viewer-strip span')?.textContent!=='Synthetic 000')[...document.querySelectorAll('.capture')].find(button=>button.querySelector('span')?.textContent==='Synthetic 000').click();});
      };
      await select();await page.waitForFunction(()=>window.__gpuLifetime.holding);
      // Wait for the real 700 ms fallback, not an injected acquisition result.
      await page.evaluate(()=>new Promise(resolve=>setTimeout(resolve,800)));
      await page.waitForFunction(()=>/GPU acquisition (timed out|pending)/.test(document.querySelector('#gpu-status')?.textContent||''));
      assert(await page.evaluate(()=>window.__gpuModule.harness_gpu_acquisition_pending()));
      for(let i=0;i<3;i++){
        await page.evaluate(()=>window.__gpuModule.harness_remount());await select();
        await page.waitForFunction(()=>document.querySelector('#gpu-status')?.textContent.includes('GPU acquisition pending'));
        // Keep the native bridge unresolved across multiple timeout intervals.
        await page.evaluate(()=>new Promise(resolve=>setTimeout(resolve,800)));
        assert.deepEqual(await page.evaluate(()=>({adapter:window.__gpuLifetime.adapterCalls,device:window.__gpuLifetime.deviceCalls,pending:window.__gpuModule.harness_gpu_acquisition_pending()})),{adapter:1,device:held==='adapter'?0:1,pending:true});
      }
      await page.evaluate(()=>{const p=window.__gpuLifetime;p.armed=true;p.release();});
      await page.waitForFunction(()=>window.__gpuLifetime.lateDestroyCalls===1&&!window.__gpuModule.harness_gpu_acquisition_pending());
      const late=await page.evaluate(()=>window.__gpuLifetime.lateSnapshot);
      assert.deepEqual(late,{pending:true,calls:1,destroyed:1,created:1});
      await page.evaluate(()=>window.__gpuModule.harness_remount());await select();await page.waitForFunction(()=>document.querySelector('#gpu-status')?.textContent.startsWith('GPU verified derivative')&&window.__gpuLifetime.adapterCalls>1);
      await page.evaluate(()=>window.__gpuModule.harness_dispose());
      await page.waitForFunction(()=>window.__gpuLifetime.created>0&&window.__gpuLifetime.created===window.__gpuLifetime.destroyed&&!window.__gpuModule.harness_gpu_acquisition_pending());
      result=await page.evaluate(()=>{const p=window.__gpuLifetime;return {heldPromise:p.held=== ''?'settled':'invalid',actualAdapterCalls:p.adapterCalls,actualDeviceCalls:p.deviceCalls,actualDevicesCreated:p.created,actualDevicesDestroyed:p.destroyed,lateDestroyCallbackCalls:p.lateDestroyCalls,lateDestroyHeldAdmission:p.lateDestroyPending};});
      assert.equal(errors,0);assert.equal(result.heldPromise,'settled');
      for(const key of ['actualAdapterCalls','actualDeviceCalls','actualDevicesCreated','actualDevicesDestroyed'])assert(Number.isSafeInteger(result[key])&&result[key]>1&&result[key]<=8,'bounded actual native GPU counts');
      assert.equal(result.actualDevicesCreated,result.actualDevicesDestroyed);assert.equal(result.lateDestroyCallbackCalls,1);assert.equal(result.lateDestroyHeldAdmission,true);
      reports.push({held,...result,threeRemountsBounded:true,newAcquisitionAfterSettlement:true});
    }finally{
      try{
        await page.evaluate(()=>{const p=window.__gpuLifetime;if(p){p.armed=false;p.release?.();}window.__gpuModule?.harness_dispose();});
        await page.waitForFunction(()=>!window.__gpuModule||(!window.__gpuModule.harness_gpu_acquisition_pending()&&window.__gpuLifetime.created===window.__gpuLifetime.destroyed));
      }finally{
        try{await page.evaluate(()=>window.__gpuLifetime?.restore());}
        finally{await page.close();}
      }
    }
  }
  assert.equal(reports.length,2);
  return {actualGpu:true,adapterAndDevicePromises:true,reports,physicalMemoryMeasured:false,totalMemoryProof:false};
};
