const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {requireMode,save,savePartialCpu,saveUnavailableGrid}=require('./focus-benchmark.cjs');

function fixture(t){
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'lytronaut-focus-evidence-'));
 t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const revision='a'.repeat(40),binary=path.join(root,'service'),client=path.join(root,'client');fs.mkdirSync(client);
 fs.writeFileSync(binary,'public synthetic test program');fs.writeFileSync(binary+'.source-revision',revision+'\n');fs.writeFileSync(path.join(client,'source-revision'),revision+'\n');
 for(const name of ['application.wasm','lytronaut_client_bg.wasm','lytronaut_client.js'])fs.writeFileSync(path.join(client,name),'public synthetic test artifact');
 const env={LYTRONAUT_FOCUS_BENCHMARK_ONLY:'1',LYTRONAUT_RELEASE_BENCHMARK:'1',LYTRONAUT_GRID_LOAD_TEST:'1',LYTRONAUT_TEST_REVISION:revision,LYTRONAUT_ARTIFACT_PROFILES:'native release; wasm release',LYTRONAUT_FOCUS_BENCHMARK_REPORT:path.join(root,'result.json'),LYTRONAUT_BINARY:binary,LYTRONAUT_CLIENT:client};
 const batch={samples:24,pendingGridAtStart:0,p50InputToDecodedFrameReadyMs:70,p95InputToDecodedFrameReadyMs:90,maxInputToDecodedFrameReadyMs:95};
 const rows=Array.from({length:24},(_,sample)=>({sample,start:10,end:100,pendingGridAtStart:1,bodyBytes:1000,bodyFinished:30,gridDecoded:50,requestedFrameVerified:true,requestedPlane:3,requestedLambda:1}));
 return {env,focusProof:{stacks:1,planes:3,p95NativeHttpDecodeMs:10},focusUiTimes:{...batch,inputToDecodedFrameReadySamplesMs:[...Array(12).fill(70),...Array(11).fill(90),95]},focusUiLoadContendedTimes:{...batch,p50InputToDecodedFrameReadyMs:90,maxInputToDecodedFrameReadyMs:90,pendingGridAtStart:1,actualGridResponseAndDecodeOverlap:true,intervals:rows},browser:'synthetic unit fixture',errors:[]};
}

test('default mode stays inactive; focus-only rejects private, alternate suites and unbound artifacts',t=>{
 assert.equal(requireMode({}),false);const f=fixture(t);assert.equal(requireMode(f.env),true);
 for(const key of ['LYTRONAUT_PRIVATE_TEST','LYTRONAUT_GPU_TEST','LYTRONAUT_GPU_LIFETIME_TEST','LYTRONAUT_OWNER_TEST','LYTRONAUT_VIDEO_PRIVATE_ROOT','LYTRONAUT_PRIVATE_SEALED_ROOT','LYTRONAUT_COVERAGE_DIR','LYTRONAUT_SIDEBAR_REPETITIONS','LYTRONAUT_EXTERNAL_SERVER'])assert.throws(()=>requireMode({...f.env,[key]:'1'}));
 for(const key of ['LYTRONAUT_RELEASE_BENCHMARK','LYTRONAUT_GRID_LOAD_TEST','LYTRONAUT_TEST_REVISION','LYTRONAUT_ARTIFACT_PROFILES'])assert.throws(()=>requireMode({...f.env,[key]:''}));
 fs.writeFileSync(f.env.LYTRONAUT_BINARY+'.source-revision','b'.repeat(40));assert.throws(()=>requireMode(f.env),/source sidecar/);
});
test('latency failure preserves actual measurements with passed=false before rejecting',t=>{
 const f=fixture(t);f.focusUiTimes.p95InputToDecodedFrameReadyMs=137.9;f.focusUiTimes.maxInputToDecodedFrameReadyMs=150;
 f.focusUiTimes.inputToDecodedFrameReadySamplesMs.splice(22,2,137.9,150);
 assert.throws(()=>save(f),/unchanged 100ms/);
 const report=JSON.parse(fs.readFileSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT));assert.equal(report.passed,false);assert.equal(report.focusUiTimes.p95InputToDecodedFrameReadyMs,137.9);assert.equal(report.thresholdMs,100);
 assert.match(report.programs.nativeSha256,/^[0-9a-f]{64}$/);assert.match(report.harness.driverSha256,/^[0-9a-f]{64}$/);assert.match(report.harness.readinessSha256,/^[0-9a-f]{64}$/);
 assert.throws(()=>save(f),/EEXIST/,'rerun must not overwrite failed evidence');
});
test('native decode miss also retains failed measurements',t=>{
 const f=fixture(t);f.focusProof.p95NativeHttpDecodeMs=101;assert.throws(()=>save(f),/unchanged 100ms/);assert.equal(JSON.parse(fs.readFileSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT)).passed,false);
});
test('passing benchmark is explicitly scoped; incomplete overlap and NaN cannot become passes',t=>{
 const f=fixture(t);const report=save(f);assert.equal(report.passed,true);assert.match(report.scope,/full browser suite not run/);
 fs.unlinkSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT);f.focusUiLoadContendedTimes.intervals[0].bodyBytes=0;assert.throws(()=>save(f));assert(!fs.existsSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT));
 f.focusUiLoadContendedTimes.intervals[0].bodyBytes=1000;f.focusUiTimes.p95InputToDecodedFrameReadyMs=NaN;assert.throws(()=>save(f),/finite/);
});
test('summary labels cannot hide a slower raw sample distribution',t=>{
 const f=fixture(t);f.focusUiTimes.inputToDecodedFrameReadySamplesMs.fill(150);assert.throws(()=>save(f),/summaries must match/);assert(!fs.existsSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT));
});
test('partial CPU evidence retains misses and raw samples without a complete or grid pass',t=>{
 const f=fixture(t);f.focusUiTimes.inputToDecodedFrameReadySamplesMs.splice(22,2,137.9,150);f.focusUiTimes.p95InputToDecodedFrameReadyMs=137.9;f.focusUiTimes.maxInputToDecodedFrameReadyMs=150;
 const cpu=savePartialCpu(f);assert.equal(cpu.passed,false);assert.equal(cpu.complete,false);assert.equal(cpu.gridAcceptanceVerified,false);assert.equal(cpu.cpuThresholdsMet,false);assert.equal(cpu.focusUiTimes.inputToDecodedFrameReadySamplesMs.length,24);assert.match(cpu.programs.nativeSha256,/^[0-9a-f]{64}$/);assert.match(cpu.harness.gridSha256,/^[0-9a-f]{64}$/);
 assert(!fs.existsSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT));
 const unavailable=saveUnavailableGrid({env:f.env,evidence:{available:false,reason:'no naturally pending grid',passed:true,complete:true,gridAcceptanceVerified:true}});
 assert.equal(unavailable.passed,false);assert.equal(unavailable.complete,false);assert.equal(unavailable.gridAcceptanceVerified,false);assert.match(unavailable.cpuReceiptSha256,/^[0-9a-f]{64}$/);
 assert(!fs.existsSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT));assert.throws(()=>savePartialCpu(f),/EEXIST/);assert.throws(()=>saveUnavailableGrid({env:f.env,evidence:{available:true}}));
});
test('even passing CPU-only values cannot become complete acceptance or substitute for grid intervals',t=>{
 const f=fixture(t);const cpu=savePartialCpu(f);assert.equal(cpu.cpuThresholdsMet,true);assert.equal(cpu.passed,false);
 assert.throws(()=>save({...f,focusUiLoadContendedTimes:{available:false,samples:0,intervals:[]}}));
 assert(!fs.existsSync(f.env.LYTRONAUT_FOCUS_BENCHMARK_REPORT));
 f.focusUiTimes.inputToDecodedFrameReadySamplesMs.fill(150);assert.throws(()=>savePartialCpu(f),/summaries must match/);
});
