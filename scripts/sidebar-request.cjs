// Public browser harness: correlate completed responses with request objects
// first observed after arming, never an older response with the same URL.
module.exports=(page,url,status=200,headersOnly=false)=>{
  if(headersOnly&&status<400)throw Error('header-only fence is limited to rejected responses');
  const candidates=new Set();
  let timer,resolve,reject;
  const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});
  // A click can fail before this fence is awaited; cleanup still rejects it.
  promise.catch(()=>{});
  const cleanup=()=>{
    clearTimeout(timer);page.off('request',started);page.off('requestfinished',finished);page.off('requestfailed',failed);page.off('response',headers);
  };
  const started=request=>{if(request.method()==='GET'&&request.url()===url)candidates.add(request);};
  const failed=request=>{candidates.delete(request);};
  const headers=response=>{if(headersOnly)complete(response.request(),response);};
  const finished=request=>complete(request,request.response());
  const complete=(request,response)=>{
    if(!candidates.delete(request))return;
    if(!response||response.request()!==request||response.status()!==status){
      cleanup();reject(Error('current sidebar request did not complete successfully'));return;
    }
    cleanup();resolve(response);
  };
  page.on('request',started);page.on('requestfinished',finished);page.on('requestfailed',failed);page.on('response',headers);
  timer=setTimeout(()=>{cleanup();reject(Error('current sidebar request did not finish before deadline'));},page.getDefaultTimeout());
  return {promise,cancel(){cleanup();reject(Error('sidebar interaction ended before request completion'));}};
};
