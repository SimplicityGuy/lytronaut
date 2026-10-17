// Public synthetic acceptance tooling. This never changes a product setting.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {requireOverlap}=require('./focus-grid-load.cjs');

function requireMode(env){
 if(env.LYTRONAUT_FOCUS_BENCHMARK_ONLY!=='1')return false;
 assert(env.LYTRONAUT_RELEASE_BENCHMARK==='1'&&env.LYTRONAUT_GRID_LOAD_TEST==='1','focus-only evidence requires release CPU mode and real grid overlap');
 for(const key of ['LYTRONAUT_PRIVATE_TEST','LYTRONAUT_GPU_TEST','LYTRONAUT_GPU_LIFETIME_TEST','LYTRONAUT_OWNER_TEST'])assert(env[key]!=='1',`focus-only evidence rejects ${key}`);
 for(const key of ['LYTRONAUT_VIDEO_PRIVATE_ROOT','LYTRONAUT_PRIVATE_SEALED_ROOT','LYTRONAUT_COVERAGE_DIR','LYTRONAUT_SIDEBAR_REPETITIONS','LYTRONAUT_EXTERNAL_SERVER'])assert(!env[key],`focus-only evidence rejects ${key}`);
 assert(/^[0-9a-f]{40}$/.test(env.LYTRONAUT_TEST_REVISION||''),'exact application revision required');
 assert(env.LYTRONAUT_ARTIFACT_PROFILES==='native release; wasm release','explicit release build provenance required');
 assert(env.LYTRONAUT_FOCUS_BENCHMARK_REPORT&&env.LYTRONAUT_BINARY&&env.LYTRONAUT_CLIENT,'explicit report and artifact paths required');
 for(const file of [env.LYTRONAUT_BINARY+'.source-revision',path.join(env.LYTRONAUT_CLIENT,'source-revision')])assert.equal(fs.readFileSync(file,'utf8').trim(),env.LYTRONAUT_TEST_REVISION,'artifact source sidecar must match measured application');
 return true;
}

function save({env,focusProof,focusUiLoadContendedTimes,focusUiTimes,browser,errors}){
 assert.equal(requireMode(env),true,'explicit focus-only mode required');
 const finite=value=>Number.isFinite(value)&&value>=0;
 assert.equal(focusProof.stacks,1);assert.equal(focusProof.planes,3);
 assert(finite(focusProof.p95NativeHttpDecodeMs));
 assert.equal(focusUiLoadContendedTimes.samples,24);
 assert.equal(focusUiLoadContendedTimes.actualGridResponseAndDecodeOverlap,true);
 assert.equal(focusUiLoadContendedTimes.intervals.length,24);
 for(const row of focusUiLoadContendedTimes.intervals)requireOverlap(row);
 assert.equal(focusUiTimes.samples,24);assert.equal(focusUiTimes.pendingGridAtStart,0,'warm batch must have no pending grid image');
 for(const batch of [focusUiLoadContendedTimes,focusUiTimes])for(const key of ['p50InputToDecodedFrameReadyMs','p95InputToDecodedFrameReadyMs','maxInputToDecodedFrameReadyMs'])assert(finite(batch[key]),`finite ${key} required`);
 assert.deepEqual(errors,[],'benchmark page must not have application errors');
 const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
 const passed=focusProof.p95NativeHttpDecodeMs<=100&&focusUiLoadContendedTimes.p95InputToDecodedFrameReadyMs<=100&&focusUiTimes.p95InputToDecodedFrameReadyMs<=100;
 const report={scope:'public synthetic focus benchmark only; full browser suite not run',revision:env.LYTRONAUT_TEST_REVISION,artifactProfiles:env.LYTRONAUT_ARTIFACT_PROFILES,browser,performanceGateEnabled:true,passed,thresholdMs:100,focusProof,focusUiLoadContendedTimes,focusUiTimes,programs:{nativeSha256:hash(env.LYTRONAUT_BINARY),rawWasmSha256:hash(path.join(env.LYTRONAUT_CLIENT,'application.wasm')),generatedWasmSha256:hash(path.join(env.LYTRONAUT_CLIENT,'lytronaut_client_bg.wasm')),glueSha256:hash(path.join(env.LYTRONAUT_CLIENT,'lytronaut_client.js'))},harness:{driverSha256:hash(path.join(__dirname,'browser-test.cjs')),gridSha256:hash(path.join(__dirname,'focus-grid-load.cjs')),evidenceSha256:hash(__filename)}};
 // Write actual failed latency values before asserting the unchanged targets.
 // Refuse overwriting a previous experiment, including an earlier failure.
 fs.writeFileSync(env.LYTRONAUT_FOCUS_BENCHMARK_REPORT,JSON.stringify(report,null,2)+'\n',{mode:0o600,flag:'wx'});
 assert(passed,'focus benchmark exceeds unchanged 100ms targets; measurements retained in report');
 return report;
}
module.exports={requireMode,save};
