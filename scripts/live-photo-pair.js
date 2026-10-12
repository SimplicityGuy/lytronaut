// MIT independent TEST-only ZIP/JPEG/QuickTime reader and platform movie decoder.
// No production writer/parser is called. Private resource bytes remain in-browser.
(() => {
  let phase='zip',assertion=0;
  const enter = value => {phase=value;assertion=0;};
  // Only fixed phase names and bounded assertion counters escape this helper.
  // The private harness catches them and emits its own aggregate phase counts.
  const fail = () => { throw Error(`resource pair verification failed:${phase}:${assertion}`); };
  const check = value => {assertion++;if (!value) fail();};
  const bounded = async promise => {let timer;try{return await Promise.race([promise,new Promise((_,reject)=>timer=setTimeout(()=>reject(Error('resource verification timeout')),30000))]);}finally{clearTimeout(timer);}};
  const text = bytes => new TextDecoder('utf-8', {fatal:true}).decode(bytes);
  const view = bytes => new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const range = (bytes, start, size) => { check(Number.isSafeInteger(start)&&Number.isSafeInteger(size)&&start>=0&&size>=0&&start+size<=bytes.length);return bytes.subarray(start,start+size); };
  const crc = bytes => {
    let value=0xffffffff;
    for(const byte of bytes) {value^=byte;for(let bit=0;bit<8;bit++)value=(value>>>1)^((value&1)?0xedb88320:0);}
    return (value^0xffffffff)>>>0;
  };
  const unzip = bytes => {enter('zip');
    check(bytes.length<=18*1024*1024);const v=view(bytes),entries=[];let pos=0;
    for(let i=0;i<3;i++) {
      range(bytes,pos,30);check(v.getUint32(pos,true)===0x04034b50&&v.getUint16(pos+6,true)===0&&v.getUint16(pos+8,true)===0);
      const size=v.getUint32(pos+18,true),names=v.getUint16(pos+26,true);check(size===v.getUint32(pos+22,true)&&v.getUint16(pos+28,true)===0);
      const name=text(range(bytes,pos+30,names)),data=range(bytes,pos+30+names,size);check(crc(data)===v.getUint32(pos+14,true));
      entries.push({name,data,start:pos,crc:v.getUint32(pos+14,true)});pos+=30+names+size;
    }
    check(entries.map(e=>e.name).join(',')==='focus.MOV,focus.JPG,provenance.json');const central=pos;
    for(const e of entries) {
      range(bytes,pos,46);check(v.getUint32(pos,true)===0x02014b50&&v.getUint16(pos+8,true)===0&&v.getUint16(pos+10,true)===0&&v.getUint32(pos+16,true)===e.crc);
      check(v.getUint32(pos+20,true)===e.data.length&&v.getUint32(pos+24,true)===e.data.length&&v.getUint32(pos+42,true)===e.start);
      const n=v.getUint16(pos+28,true);check(v.getUint16(pos+30,true)===0&&v.getUint16(pos+32,true)===0&&text(range(bytes,pos+46,n))===e.name);pos+=46+n;
    }
    range(bytes,pos,22);check(pos+22===bytes.length&&v.getUint32(pos,true)===0x06054b50&&v.getUint16(pos+8,true)===3&&v.getUint16(pos+10,true)===3&&v.getUint32(pos+12,true)===pos-central&&v.getUint32(pos+16,true)===central&&v.getUint16(pos+20,true)===0);
    check(entries[0].data.length<=16*1024*1024&&entries[2].data.length<=16*1024);return entries.map(e=>e.data);
  };
  const jpegIdentity = jpeg => {enter('jpeg');
    check(jpeg.length<=2*1024*1024+256&&jpeg[0]===255&&jpeg[1]===216);let pos=2,identity=null;
    while(pos<jpeg.length) {
      check(jpeg[pos]===255);const marker=jpeg[pos+1];if(marker===218)break;
      const size=view(range(jpeg,pos+2,2)).getUint16(0),data=range(jpeg,pos+4,size-2);check(size>=2);
      if(marker===225) {
        check(identity===null&&text(range(data,0,6))==='Exif\0\0');const t=range(data,6,data.length-6),v=view(t);
        check(text(range(t,0,2))==='MM'&&v.getUint16(2)===42&&v.getUint32(4)===8&&v.getUint16(8)===2);
        check(v.getUint16(10)===0x112&&v.getUint16(12)===3&&v.getUint32(14)===1&&v.getUint16(18)===1);
        check(v.getUint16(22)===0x8769&&v.getUint16(24)===4&&v.getUint32(26)===1&&v.getUint32(34)===0);
        const ifd=v.getUint32(30);range(t,ifd,18);check(v.getUint16(ifd)===1&&v.getUint16(ifd+2)===0x927c&&v.getUint16(ifd+4)===7&&v.getUint32(ifd+14)===0);
        const note=range(t,v.getUint32(ifd+10),v.getUint32(ifd+6)),n=view(note);
        check(text(range(note,0,14))==='Apple iOS\0\0\x01MM'&&n.getUint16(14)===1&&n.getUint16(16)===17&&n.getUint16(18)===2&&n.getUint32(20)===37&&n.getUint32(28)===0);
        const id=range(note,n.getUint32(24),37);check(id[36]===0);identity=text(id.subarray(0,36));
      }
      if(marker>=224&&marker<=239)check(marker===225||(marker===224&&text(range(data,0,5))==='JFIF\0')||(marker===238&&text(range(data,0,5))==='Adobe'));
      check(marker!==254); // no comments, legacy IPTC or other private metadata.
      pos+=2+size;
    }
    check(identity&&/^[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}$/.test(identity));return identity;
  };
  const movieInfo = bytes => {enter('movie');
    const v=view(bytes);
    const boxes=(start,end)=>{const out=[];while(start<end){range(bytes,start,8);const size=v.getUint32(start);check(size>=8&&start+size<=end&&out.length<128);out.push({start,data:start+8,end:start+size,type:text(range(bytes,start+4,4))});start+=size;}check(start===end);return out;};
    const children=(b,skip=0)=>boxes(b.data+skip,b.end);
    const one=(bs,type)=>{const found=bs.filter(b=>b.type===type);check(found.length===1);return found[0];};
    const at=(b,n)=>{check(b.data+n+4<=b.end);return v.getUint32(b.data+n);};
    const root=boxes(0,bytes.length);check(root.length===3&&root.map(b=>b.type).join(',')==='ftyp,mdat,moov');const mdat=root[1],moov=children(root[2]);
    const mvhd=one(moov,'mvhd');check(at(mvhd,0)===0&&at(mvhd,16)===3*at(mvhd,12));
    const tracks=moov.filter(b=>b.type==='trak');check(tracks.length===2);const parsed=tracks.map(trak=>{
      const t=children(trak),tkhd=one(t,'tkhd'),mdia=children(one(t,'mdia')),mdhd=one(mdia,'mdhd'),hdlr=one(mdia,'hdlr');
      check(at(mdhd,0)===0);const kind=text(range(bytes,hdlr.data+8,4));check(kind==='vide'||kind==='meta');
      const stbl=children(one(children(one(mdia,'minf')),'stbl')),stco=one(stbl,'stco'),stsz=one(stbl,'stsz'),stts=one(stbl,'stts'),stsc=one(stbl,'stsc');
      const count=at(stco,4);check(count>=1&&count<=64&&at(stsz,4)===0&&at(stsz,8)===count&&at(stts,4)===1&&at(stts,8)===count&&at(stts,12)===3000);
      check(at(stsc,4)===1&&at(stsc,8)===1&&at(stsc,12)===1&&at(stsc,16)===1);
      const samples=Array.from({length:count},(_,i)=>{const offset=at(stco,8+i*4),size=at(stsz,12+i*4);check(offset>=mdat.data&&offset+size<=mdat.end);return range(bytes,offset,size);});
      const stsd=one(stbl,'stsd');check(at(stsd,4)===1);const desc=children(stsd,8);check(desc.length===1);
      return {kind,t,tkhd,desc:desc[0],samples,scale:at(mdhd,12),duration:at(mdhd,16)};
    });
    const video=parsed.find(t=>t.kind==='vide'),meta=parsed.find(t=>t.kind==='meta');check(video&&meta&&video.duration/video.scale===3&&video.scale===meta.scale);
    const width=at(video.tkhd,76)>>>16,height=at(video.tkhd,80)>>>16;check(width>0&&height>0&&width<=2048&&height<=2048&&width*height<=2*1024*1024);
    check(Array.from({length:9},(_,i)=>at(video.tkhd,40+i*4)).join(',')===[65536,0,0,0,65536,0,0,0,1073741824].join(','));
    check(video.desc.type==='avc1');const avc=children(video.desc,78),avcc=one(avc,'avcC'),colr=one(avc,'colr');
    check(text(range(bytes,colr.data,4))==='nclc'&&view(range(bytes,colr.data+4,6)).getUint16(0)===1&&v.getUint16(colr.data+6)===13&&v.getUint16(colr.data+8)===6);
    const description=range(bytes,avcc.data,avcc.end-avcc.data);check(description.length>=7&&description[0]===1&&description[1]===66&&description[3]===50&&(description[4]&3)===3);
    check(meta.desc.type==='mebx'&&meta.samples.length===1&&meta.duration===3000&&meta.samples[0].length===9&&view(meta.samples[0]).getUint32(0)===9&&view(meta.samples[0]).getUint32(4)===1&&meta.samples[0][8]===255);
    const keyEntry=children(one(children(meta.desc,8),'keys'));check(keyEntry.length===1&&v.getUint32(keyEntry[0].start+4)===1);
    const keyItems=children(keyEntry[0]),keyd=one(keyItems,'keyd'),dtyp=one(keyItems,'dtyp');check(text(range(bytes,keyd.data,keyd.end-keyd.data))==='mdtacom.apple.quicktime.still-image-time'&&at(dtyp,4)===65);
    const edits=meta.t.filter(b=>b.type==='edts');check(edits.length<=1);let keySeconds=0;
    if(edits.length){const elst=one(children(edits[0]),'elst');check(at(elst,4)===2&&at(elst,12)===0xffffffff&&at(elst,16)===65536&&at(elst,20)===3000&&at(elst,24)===0&&at(elst,28)===65536);keySeconds=at(elst,8)/meta.scale;}
    const movieMeta=children(one(moov,'meta')),keys=one(movieMeta,'keys');check(at(keys,4)===1);const names=children(keys,8);check(names.length===1&&names[0].type==='mdta'&&text(range(bytes,names[0].data,names[0].end-names[0].data))==='com.apple.quicktime.content.identifier');
    const items=children(one(movieMeta,'ilst'));check(items.length===1&&v.getUint32(items[0].start+4)===1);const data=one(children(items[0]),'data');check(at(data,0)===1&&at(data,4)===0);const identity=text(range(bytes,data.data+8,data.end-data.data-8));
    return {width,height,keySeconds,identity,samples:video.samples,description};
  };
  const canvas = (width,height) => {const c=document.createElement('canvas');c.width=width;c.height=height;return c;};
  const image = async source => {const i=new Image();i.src=source;try{await bounded(i.decode());return i;}catch(error){i.src='';throw error;}};
  const difference = (a,b,width,height) => {
    let sum=0,squared=0,max=0,count=0;
    for(let row=0;row<height;row+=16){const h=Math.min(16,height-row),x=a.getImageData(0,row,width,h).data,y=b.getImageData(0,row,width,h).data;
      for(let i=0;i<x.length;i++)if(i%4!==3){const d=Math.abs(x[i]-y[i]);sum+=d;squared+=d*d;max=Math.max(max,d);count++;}}
    return {absoluteSum:sum,squaredSum:squared,samples:count,maxDifference:max};
  };
  window.verifyLivePhotoPair = async (zip,{identifier,referenceImage,expectedBlue=false}={}) => {
    const [movie,jpeg,json]=unzip(zip),p=JSON.parse(text(json)),info=movieInfo(movie);const jpegId=jpegIdentity(jpeg);enter('pair');check(info.identity===identifier&&jpegId===identifier&&p.width===info.width&&p.height===info.height&&p.orientation===1&&p.duration_seconds===3&&p.audio==='none'&&p.motion==='generated_focus_animation'&&p.camera_recorded_temporal_motion===false&&p.calibrated_synthesis===false&&p.originals==='unchanged');
    check(Object.keys(p).sort().join(',')===['version','motion','duration_seconds','key_seconds','selected_stored_plane','sweep_stored_planes','source_variant','width','height','orientation','audio','originals','camera_recorded_temporal_motion','calibrated_synthesis','imported_adjustments','color'].sort().join(',')&&p.version===1&&p.source_variant==='verified_archived_avc_planes'&&p.imported_adjustments==='preserved_only_not_replayed'&&p.color==='source profile unknown; platform-decoded RGB; sRGB transfer, BT.709 primaries, BT.601 limited-range YCbCr; JPEG/chroma quantization are lossy');
    check(p.sweep_stored_planes.length===info.samples.length&&new Set(p.sweep_stored_planes).size===info.samples.length&&Math.abs(info.keySeconds-p.key_seconds)<1e-6);
    const key=p.sweep_stored_planes.indexOf(p.selected_stored_plane);check(key>=0&&Math.abs(info.keySeconds-key*3/info.samples.length)<1e-6);
    enter('image');const url=URL.createObjectURL(new Blob([jpeg],{type:'image/jpeg'})),still=canvas(info.width,info.height),motion=canvas(info.width,info.height),baseline=referenceImage?canvas(info.width,info.height):null;
    let decoder,stillImage,referenceDecoded,failed=false,outputs=0,keySeen=false,stillLoss=null,motionLoss=null;
    try {
      const i=stillImage=await image(url);check(i.naturalWidth===info.width&&i.naturalHeight===info.height);still.getContext('2d').drawImage(i,0,0);i.src='';
      if(baseline){const ref=referenceDecoded=await image(referenceImage);check(ref.naturalWidth===info.width&&ref.naturalHeight===info.height);baseline.getContext('2d').drawImage(ref,0,0);ref.src='';stillLoss=difference(still.getContext('2d'),baseline.getContext('2d'),info.width,info.height);}
      enter('motion');decoder=new VideoDecoder({error:()=>failed=true,output:frame=>{try{
        const index=Math.round(frame.timestamp*info.samples.length/3000000);check(index===outputs&&frame.displayWidth===info.width&&frame.displayHeight===info.height);outputs++;
        if(index===key){keySeen=true;motion.getContext('2d').drawImage(frame,0,0);if(baseline)motionLoss=difference(motion.getContext('2d'),baseline.getContext('2d'),info.width,info.height);if(expectedBlue){const rgb=motion.getContext('2d').getImageData(Math.floor(info.width/2),Math.floor(info.height/2),1,1).data;check(rgb[2]>200&&rgb[0]<40&&rgb[1]<40);}}
      }catch{failed=true;}finally{frame.close();}}});
      // avcC/SPS supplies coded dimensions, including padding. tkhd dimensions
      // describe visible output and must not be supplied as codedWidth/Height.
      decoder.configure({codec:'avc1.42c032',description:info.description,hardwareAcceleration:'prefer-software'});
      const deadline=performance.now()+30000;
      for(let i=0;i<info.samples.length;i++) {check(!failed&&performance.now()<deadline);decoder.decode(new EncodedVideoChunk({type:'key',timestamp:Math.round(i*3000000/info.samples.length),data:info.samples[i]}));while(decoder.decodeQueueSize>0&&!failed){check(performance.now()<deadline);await new Promise(resolve=>setTimeout(resolve,0));}}
      await bounded(decoder.flush());check(!failed&&outputs===info.samples.length&&keySeen);
      return {width:info.width,height:info.height,frames:outputs,keySeconds:info.keySeconds,selected:p.selected_stored_plane,sweep:p.sweep_stored_planes,stillLoss,motionLoss};
    } finally {try{if(decoder)decoder.close();}finally{if(stillImage)stillImage.src='';if(referenceDecoded)referenceDecoded.src='';URL.revokeObjectURL(url);for(const c of [still,motion,baseline])if(c){c.width=c.height=0;}}}
  };
})();
