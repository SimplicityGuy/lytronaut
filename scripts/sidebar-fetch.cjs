// Public test observer only. A completed Fetch body can outlive a late CDP
// loadingFailed event; witness the exact post-arm invocation instead of reading
// a response body back through DevTools. The Rust application consumes its own
// branch of the response unchanged.
module.exports=function installCaptureProbe(origin,scope=globalThis){
  const original=scope.fetch;
  const limit=256*1024,maxRequests=8;
  let sequence=0,active=null;
  const fail=(row,reason)=>{row.failed=true;row.error=reason;};
  const cancel=token=>{
    if(!active||active.token!==token)return;
    const group=active;active=null;group.cancelled=true;
    for(const row of group.requests){
      if(row.reader&&!row.done){row.reader.cancel().catch(()=>{});}
    }
  };
  const observe=async(response,row,group,signal)=>{
    let reason='body';
    try{
      const reader=response.clone().body?.getReader();
      if(!reader)throw Error('missing body');
      row.reader=reader;
      // Fixed capacity: neither untrusted Content-Length nor tiny chunks can
      // grow a body accumulator or an unbounded array of chunk objects.
      const bytes=new Uint8Array(limit);let used=0;
      for(;;){
        const part=await reader.read();
        if(part.done)break;
        if(part.value.byteLength>limit-used){reason='budget';throw Error('budget');}
        bytes.set(part.value,used);used+=part.value.byteLength;
      }
      if(group.cancelled||signal?.aborted){reason='cancelled';throw Error('cancelled');}
      reason='json';const detail=JSON.parse(new TextDecoder().decode(bytes.subarray(0,used)));
      reason='identity';
      if(detail.capture?.id!==group.captureId||!Array.isArray(detail.originals))throw Error('identity');
      row.bytes=used;row.focus=detail.originals.some(file=>file.role==='jpeg_focus_stack');
      row.captureId=detail.capture.id;row.complete=true;
    }catch{
      fail(row,group.cancelled||signal?.aborted?'cancelled':reason);
      if(row.reader)row.reader.cancel().catch(()=>{});
    }finally{
      row.done=true;try{row.reader?.releaseLock();}catch{fail(row,'reader');}row.reader=null;
    }
  };
  const wrapped=function(input,init){
    const group=active;
    const request=input instanceof Request?input:null;
    const method=String(init?.method||request?.method||'GET').toUpperCase();
    let row=null;
    let matches=false;
    if(group&&method==='GET'){try{matches=new URL(request?.url||String(input),origin).href===group.url;}catch{}}
    if(matches){
      group.seen++;
      if(group.requests.length===maxRequests)group.overflow=true;
      else{row={invocation:group.seen,headers:false,complete:false,failed:false,done:false};group.requests.push(row);}
    }
    const signal=init&&'signal' in init?init.signal:request?.signal;
    let pending;
    try{pending=original.call(scope,input,init);}catch(error){if(row){row.done=true;fail(row,'fetch');}throw error;}
    if(!row)return pending;
    return Promise.resolve(pending).then(response=>{
      row.headers=true;row.status=response.status;
      if(group.cancelled||signal?.aborted){row.done=true;fail(row,'cancelled');}
      else if(response.status!==200){row.done=true;fail(row,'http');}
      else void observe(response,row,group,signal);
      return response;
    },error=>{row.done=true;fail(row,signal?.aborted?'cancelled':'fetch');throw error;});
  };
  scope.fetch=wrapped;
  const api={
    arm(captureId){
      if(active)cancel(active.token);
      active={token:++sequence,captureId,url:new URL(`/captures/${captureId}`,origin).href,seen:0,overflow:false,cancelled:false,requests:[]};
      return active.token;
    },
    ready(token){
      if(!active||active.token!==token||active.overflow)return null;
      const row=active.requests.find(row=>row.complete&&!row.failed);
      return row?{invocation:row.invocation,captureId:row.captureId,focus:row.focus,bytes:row.bytes}:null;
    },
    snapshot(token){
      if(!active||active.token!==token)return null;
      return {seen:active.seen,overflow:active.overflow,requests:active.requests.map(({invocation,headers,status,complete,failed,done,error,bytes})=>({invocation,headers,status,complete,failed,done,error,bytes}))};
    },
    cancel,
    dispose(){if(active)cancel(active.token);if(scope.fetch===wrapped)scope.fetch=original;},
  };
  scope.__sidebarCaptureProbe=api;
  return api;
};
