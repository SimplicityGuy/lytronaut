#!/usr/bin/env bash
set -euo pipefail
if [[ -n ${LYTRONAUT_VIDEO_PRIVATE_ROOT:-} || -n ${LYTRONAUT_PRIVATE_SEALED_ROOT:-} || ${LYTRONAUT_PRIVATE_TEST:-0} == 1 ]]; then
    echo 'Public owner suite rejects private mode' >&2; exit 1
fi
for shape in small large padded; do
    case "$shape" in small) suffix=; port=3017;; large) suffix=-large; port=3018;; padded) suffix=-padded; port=3019;; esac
    LYTRONAUT_TEST_ORIGIN="http://127.0.0.1:$port" LYTRONAUT_OWNER_SHAPE="$shape" LYTRONAUT_OWNER_SUFFIX="$suffix" bash scripts/browser-test.sh
done
node <<'JS'
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const root=process.env.LYTRONAUT_OWNER_EVIDENCE;
const reports=['runtime.json','runtime-large.json','runtime-padded.json'].map(name=>{
  const file=path.join(root,name);assert(fs.statSync(file).size<=256*1024,'bounded public UI report');return JSON.parse(fs.readFileSync(file,'utf8'));
});
const expected=[['small',32,32],['large',2048,1024],['padded',1138,1842]];
for(const [index,report] of reports.entries()){
  const trace=report.partialCapacityTrace;
  assert.equal(trace.shape,expected[index][0]);
  assert.deepEqual(trace.geometry,{width:expected[index][1],height:expected[index][2]});
  assert.equal(trace.complete,true);assert.equal(trace.dropped,0);assert.equal(trace.totalMemoryProof,false);
  assert.equal(trace.platform.complete,true);assert.equal(trace.platform.dropped,0);
  assert.equal(trace.platform.physicalHeapMeasured,false);
  assert.equal(report.downloads,3);
  const lifetime=report.browseLifetimeProof;
  if(index===0){
    assert(lifetime&&typeof lifetime==='object','small canonical owner must execute browser lifetime regressions');
    const facts=['stalledRead','stalledCancel','remountDrains','synchronousCaptureControlBurst','reentrantSelected','reentrantMain','reentrantLibrary','actionBeforePollDisposed','acceptedMutationRecovery','importLaneRemount','constructorFailure','decodeFailure','urlsReleasedBeforeClose'];
    const keys=['admission','jpeg',...facts,'rendererDestroyReentryCovered','rendererDestroyReentryPending','created','revoked','logicalJpegBytesMax','capacityCollection','totalMemoryProof'];
    assert.deepEqual(Object.keys(lifetime).sort(),keys.sort(),'fixed public lifetime schema');
    assert.deepEqual(lifetime.admission,{alreadyAborted:true,nativeAbortWake:true,deferredAbortWake:true,registrationRace:true,activeDrainRetained:true,capacityReusable:true});
    for(const key of facts)assert.equal(lifetime[key],true,`required actual synthetic lifecycle fact: ${key}`);
    assert.equal(lifetime.rendererDestroyReentryCovered,false);assert.equal(lifetime.rendererDestroyReentryPending,true,'absent-GPU fixture cannot claim device-destroy coverage');
    assert.equal(lifetime.capacityCollection,false);assert.equal(lifetime.totalMemoryProof,false);
    for(const key of ['created','revoked','logicalJpegBytesMax'])assert(Number.isSafeInteger(lifetime[key])&&lifetime[key]>0&&lifetime[key]<=64*1024*1024);
    assert.equal(lifetime.created,lifetime.revoked,'owned synthetic URLs released before closing page');
    const jpeg=lifetime.jpeg;
    assert.deepEqual(Object.keys(jpeg).sort(),['byteIdentical','shaIdentical','highBitBytes','mimeVerified','rejections','logicalJpegBytes'].sort());
    for(const key of ['byteIdentical','shaIdentical','highBitBytes','mimeVerified'])assert.equal(jpeg[key],true);
    assert.equal(jpeg.rejections,4);assert(Number.isSafeInteger(jpeg.logicalJpegBytes)&&jpeg.logicalJpegBytes>0&&jpeg.logicalJpegBytes<=1572846);
  }else assert.equal(lifetime,null,'large/padded keep their existing owner workload');

  assert(Number.isSafeInteger(report.created)&&report.created>0);assert.equal(report.created,report.closed);
  assert(Number.isSafeInteger(report.outputs)&&report.outputs>0);assert.equal(report.outputs,report.framesClosed);
  assert(/^[0-9a-f]{40}$/.test(report.revision),'exact source revision required');
  for(const key of ['applicationWasmSha256','generatedWasmSha256'])assert(/^[0-9a-f]{64}$/.test(report[key]),'nonempty program identity required');
  assert.equal(report.artifactFeature,'test-harness');assert.equal(report.productionArtifact,false);
  for(const key of ['revision','applicationWasmSha256','generatedWasmSha256'])assert.equal(report[key],reports[0][key],'same feature program/source for all UI shapes');
}
JS
