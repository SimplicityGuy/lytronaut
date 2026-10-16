// MIT public test tooling: exercise the SAME shared Rust response reader.
// Run before owner capacity collection; no pixels, identifiers or private input.
module.exports=async(page)=>page.evaluate(async()=>{
  const module=window.__ownerModule;
  if(typeof module.harness_read_response!=='function')throw Error('response harness artifact required');
  const bounded=async(operation)=>{let timer;try{return await Promise.race([operation,new Promise((_,reject)=>timer=setTimeout(()=>reject(Error('response test timeout')),30000))]);}finally{clearTimeout(timer);}};
  const invoke=(response,limit=64,abort=new AbortController())=>bounded(module.harness_read_response(response,limit,abort));
  const response=(bytes,headers={})=>new Response(new ReadableStream({start(controller){controller.enqueue(bytes);controller.close();}}),{headers});
  const equal=(actual,expected)=>{if(actual.length!==expected.length||actual.some((value,index)=>value!==expected[index]))throw Error('response byte mismatch');};
  const rejected=async(operation,expected)=>{let caught=false;try{await operation();}catch(error){if(error!==expected)throw Error('response rejection mismatch');caught=true;}if(!caught)throw Error('response failure accepted');};
  const backing=new Uint8Array([99,98,1,2,3,4,97]);
  equal(await invoke(response(backing.subarray(2,6))),[1,2,3,4]);
  equal(await invoke(response(new Uint8Array([5,6]),{'Content-Length':'1'})),[5,6]);
  equal(await invoke(response([7,8,9])),[7,8,9]); // Existing constructor conversion.
  let headerCancelled=0,overflowCancelled=0;
  await rejected(()=>invoke(new Response(new ReadableStream({cancel(){headerCancelled++;}}),{headers:{'Content-Length':'65'}})), 'Response exceeds resource budget');
  await rejected(()=>invoke(new Response(new ReadableStream({start(controller){controller.enqueue(new Uint8Array(65));},cancel(){overflowCancelled++;}}))), 'Response exceeds resource budget');
  if(headerCancelled!==1||overflowCancelled!==1)throw Error('response overflow cleanup mismatch');
  await rejected(()=>invoke(new Response(new ReadableStream({start(controller){controller.error(Error('public interruption'));}}))), 'Response interrupted');
  const abort=new AbortController();let abortController;
  const interrupted=new Response(new ReadableStream({start(controller){abortController=controller;}}));
  abort.signal.addEventListener('abort',()=>abortController.error(new DOMException('public abort','AbortError')),{once:true});
  let abortTimer;
  try {abortTimer=setTimeout(()=>abort.abort(),20);await rejected(()=>invoke(interrupted,64,abort),'Cancelled');}
  finally {clearTimeout(abortTimer);}
  if(!abort.signal.aborted)throw Error('response cancellation signal required');
  const iframe=document.createElement('iframe');iframe.srcdoc='<!doctype html><title>public byte realm</title>';
  try {
    await bounded(new Promise((resolve,reject)=>{iframe.onload=resolve;iframe.onerror=()=>reject(Error('public byte realm failed'));document.body.append(iframe);}));
    const other=new iframe.contentWindow.Uint8Array([77,10,11,12,76]);
    if(other instanceof Uint8Array)throw Error('distinct public byte realm required');
    equal(await invoke(response(other.subarray(1,4))),[10,11,12]);
  } finally {iframe.onload=iframe.onerror=null;iframe.remove();}
  return {sameRealmOffsetView:true,crossRealmOffsetView:true,customArrayConversion:true,absentContentLength:true,incorrectContentLength:true,oversizeContentLength:true,chunkOverflow:true,interruptedStream:true,cancellation:true,capacityCollection:false};
});
