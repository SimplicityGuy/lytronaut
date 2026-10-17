// Public browser harness: correlate completed responses with request objects
// first observed after arming, never an older response with the same URL.
module.exports=(page,url,status=200,headersOnly=false)=>{
  if(headersOnly&&status<400)throw Error('header-only fence is limited to rejected responses');
  const candidates=new Set();
  const observed=new Map();let seen=0;
  const snapshot=()=>({seen,active:candidates.size,requests:[...observed.values()].map(item=>({...item}))});
  let timer,resolve,reject;
  const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});
  // A click can fail before this fence is awaited; cleanup still rejects it.
  promise.catch(()=>{});
  const cleanup=()=>{
    clearTimeout(timer);page.off('request',started);page.off('requestfinished',finished);page.off('requestfailed',failed);page.off('response',headers);
  };
  const started=request=>{if(request.method()==='GET'&&request.url()===url){
    candidates.add(request);observed.set(request,{sequence:++seen,headers:false,finished:false,failed:false});
    if(observed.size>8)observed.delete(observed.keys().next().value);
  }};
  const failed=request=>{const item=observed.get(request);if(item){item.failed=true;item.error=(request.failure?.()?.errorText||'unknown').slice(0,128);}candidates.delete(request);};
  const headers=response=>{const item=observed.get(response.request());if(item){item.headers=true;item.status=response.status();item.fromCache=response.fromCache?.()||false;}if(headersOnly)complete(response.request(),response);};
  const finished=request=>{const item=observed.get(request);if(item)item.finished=true;complete(request,request.response());};
  const complete=(request,response)=>{
    if(!candidates.delete(request))return;
    if(!response||response.request()!==request||response.status()!==status){
      cleanup();reject(Error('current sidebar request did not complete successfully'));return;
    }
    cleanup();resolve(response);
  };
  page.on('request',started);page.on('requestfinished',finished);page.on('requestfailed',failed);page.on('response',headers);
  timer=setTimeout(()=>{cleanup();const error=Error('current sidebar request did not finish before deadline');error.requestLifecycle=snapshot();reject(error);},page.getDefaultTimeout());
  return {promise,snapshot,cancel(){cleanup();reject(Error('sidebar interaction ended before request completion'));}};
};
