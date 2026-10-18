// Test-only passive readiness observer. It neither creates network traffic nor
// changes product state; a real enabled control is usable before initial focus
// rendering has settled. Kept self-contained for evaluateOnNewDocument.
module.exports=function installFocusReadiness({names,selectedName,notify},scope=globalThis){
 const doc=scope.document;
 let disposed=false,announced=false,inflight=false,dirty=false,checks=0;
 const snapshot=()=>{
  const labels=[...doc.querySelectorAll('.capture span')].map(e=>e.textContent);
  const range=doc.querySelector('#focus'),image=doc.querySelector('.stage img');
  if(labels.length!==names.length||!labels.every((name,i)=>name===names[i])||doc.querySelector('.viewer-strip span')?.textContent!==selectedName||!range||range.disabled||!image||!image.complete||image.naturalWidth<=0)return null;
  const minimum=Number(range.min),maximum=Number(range.max);
  if(!Number.isFinite(minimum)||!Number.isFinite(maximum)||maximum<=minimum||Number(range.value)!==minimum)return null;
  return {range,image,src:image.src};
 };
 let observer;
 const dispose=()=>{disposed=true;observer?.disconnect();scope.removeEventListener('pagehide',dispose);scope.removeEventListener('load',check,true);};
 const check=async()=>{
  if(disposed||announced)return;
  if(inflight){dirty=true;return;}
  checks++;
  const before=snapshot();if(!before)return;
  inflight=true;dirty=false;
  try{
   await before.image.decode();
   const after=snapshot();
   if(disposed||announced||!after||after.image!==before.image||after.src!==before.src)return;
   const state={metadataReady:true,selectedIdentityVerified:true,orderedLabelsVerified:true,existingStageDecoded:true,pageReadyMs:scope.performance.now(),pendingGrid:doc.querySelectorAll('.capture').length-[...doc.querySelectorAll('.capture img')].filter(i=>i.complete&&i.naturalWidth>0).length,rangeValue:Number(after.range.value),rangeMin:Number(after.range.min),rangeMax:Number(after.range.max),stageKind:doc.querySelector('.status')?.textContent.includes('Discrete focus:')?'focused':'static-or-initial',checks};
   announced=true;observer.disconnect();scope.removeEventListener('pagehide',dispose);scope.removeEventListener('load',check,true);
   await scope[notify](state);
  }catch{}finally{
   inflight=false;
   if(dirty&&!disposed&&!announced)scope.queueMicrotask(check);
  }
 };
 observer=new scope.MutationObserver(check);
 observer.observe(doc,{childList:true,subtree:true,attributes:true,characterData:true});
 scope.addEventListener('pagehide',dispose,{once:true});
 // Image load can make complete/naturalWidth usable without another DOM mutation.
 scope.addEventListener('load',check,true);
 const frameMatches=(state,expected)=>{
  const output=state.output?.match(/· plane (\d+) · relative lambda ([^·]+) ·/);
  const status=state.status?.match(/^Discrete focus: stored plane (\d+) of (\d+), lambda ([^·]+) ·/);
  return state.changed===true&&state.selected===expected.name&&state.alt===`Stored JPEG focus derivative: ${expected.name}`&&!!output&&!!status&&Number(output[1])===expected.plane&&Number(output[2].trim())===expected.lambda&&Number(status[1])===expected.plane&&Number(status[2])===3&&Number(status[3].trim())===expected.lambda;
 };
 scope.__focusReadinessObserver={dispose,frameMatches,state:()=>({disposed,announced,inflight,checks})};
 void check();
 return scope.__focusReadinessObserver;
};
