// MIT, public synthetic authoring only. No WASM or production encoder dependency.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http'),os=require('node:os');
const pp=require('./browser/node_modules/puppeteer-core');
if(process.env.LYTRONAUT_VIDEO_PRIVATE_ROOT||process.env.LYTRONAUT_PRIVATE_SEALED_ROOT||process.env.LYTRONAUT_PRIVATE_TEST==='1')throw Error('public fixture writer rejects private mode');
const shapes={large:[2048,1024],padded:[1138,1842]};
const [output,shape]=process.argv.slice(2);
if(!output||!Object.hasOwn(shapes,shape)||process.argv.length!==4)throw Error('public fixture output and shape required');
const destination=path.resolve(output),parent=fs.lstatSync(path.dirname(destination));
if(!parent.isDirectory()||parent.isSymbolicLink()||(parent.mode&0o777)!==0o700||fs.existsSync(destination))throw Error('isolated fresh fixture destination required');
const scratch=fs.mkdtempSync(path.join(os.tmpdir(),'lytronaut-public-author-'));fs.chmodSync(scratch,0o700);
const bounded=async(operation,ms)=>{let timer;try{return await Promise.race([operation,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('public author timeout')),ms);timer.unref();})]);}finally{clearTimeout(timer);}};
const server=http.createServer((req,res)=>{if(req.method!=='GET'||req.url!=='/'){res.statusCode=404;res.end();return;}res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Public fixture author</title>');});
(async()=>{
  let browser;
  try {
    await bounded(new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',()=>{server.removeListener('error',reject);resolve();});}),10000);
    browser=await require('./coverage.cjs').launch(pp,{executablePath:process.env.LYTRONAUT_CHROME||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true,userDataDir:path.join(scratch,'profile'),timeout:30000});
    const page=await browser.newPage();
    await page.setRequestInterception(true);
    let interceptionFailed=false;
    page.on('request',request=>{
      try {
        const url=new URL(request.url());
        const operation=url.origin===`http://127.0.0.1:${server.address().port}`&&request.method()==='GET'?request.continue():request.abort();
        operation.catch(()=>{interceptionFailed=true;});
      } catch {interceptionFailed=true;request.abort().catch(()=>{});}
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`,{timeout:30000});
    await page.addScriptTag({content:fs.readFileSync(path.join(__dirname,'live-photo-fixture.js'),'utf8')});
    const result=await bounded(page.evaluate(async dimensions=>{
      const fixture=await window.authorLivePhotoFixture(...dimensions);
      if(fixture.authoredFrames!==5||fixture.bytes.length>8*1024*1024)throw Error('public fixture admission');
      return Array.from(fixture.bytes);
    },shapes[shape]),60000);
    if(interceptionFailed)throw Error('public author request failure');
    if(!Array.isArray(result)||result.length===0||result.length>8*1024*1024||result.some(v=>!Number.isInteger(v)||v<0||v>255))throw Error('bounded public fixture bytes required');
    fs.writeFileSync(destination,Buffer.from(result),{mode:0o600,flag:'wx'});
  } catch {console.error('public fixture author failed');process.exitCode=1;}
  finally {
    try {
      if(browser){try{await bounded(browser.close(),10000);}catch{browser.process()?.kill('SIGKILL');}}
    } catch {console.error('public author browser cleanup failed');process.exitCode=1;}
    finally {
      try {
        server.closeAllConnections();
        if(server.listening)await bounded(new Promise((resolve,reject)=>server.close(error=>error?reject(error):resolve())),10000);
      } catch {console.error('public author server cleanup failed');process.exitCode=1;}
      finally {
        try {fs.rmSync(scratch,{recursive:true,force:true});}
        catch {console.error('public author scratch cleanup failed');process.exitCode=1;}
      }
    }
  }
})();
