// MIT independently authored PUBLIC test fixture writer; never production code.
const authorPublicLivePhotoFixture = async (width,height,stressBitrate=0) => {
  if(stressBitrate!==0&&![32000000,64000000].includes(stressBitrate))throw Error('fixed public bitrate required');
  if(!((width===2048&&height===1024)||(width===1138&&height===1842)))throw Error('public fixture shape required');
  let authoredFrames=0;
  const bounded = async promise => {let timer;try{return await Promise.race([promise,new Promise((_,reject)=>timer=setTimeout(()=>reject(Error('public fixture timeout')),30000))]);}finally{clearTimeout(timer);}};

      const largeCanvas = document.createElement('canvas'); largeCanvas.width = width; largeCanvas.height = height;
      const chunks = []; let encoderFailed = false, encodedBytes = 0, encoder, outputBudget=false, keyChunks=0, deltaChunks=0;
      try {
        const context = largeCanvas.getContext('2d');
        if(!context)throw Error('public fixture canvas unavailable');
        encoder = new VideoEncoder({ output: chunk => {
          if(encoderFailed)return;
          if(chunks.length>=5||chunk.byteLength+6>8*1024*1024-encodedBytes){encoderFailed=true;outputBudget=true;return;}
          const data = new Uint8Array(chunk.byteLength); chunk.copyTo(data); chunks.push(data); encodedBytes+=data.length+6;if(chunk.type==='key')keyChunks++;else deltaChunks++;
        }, error: () => encoderFailed = true });
        encoder.configure({ codec: 'avc1.420032', width, height, framerate: 1, latencyMode: 'realtime', hardwareAcceleration: 'prefer-software', avc: { format: 'annexb' }, ...(stressBitrate?{bitrate:stressBitrate}:{}) });
        if(stressBitrate){
          const noise=context.createImageData(width,height);let state=0x13579bdf;
          for(let offset=0;offset<noise.data.length;offset+=4){
            for(let channel=0;channel<3;channel++){state^=state<<13;state^=state>>>17;state^=state<<5;noise.data[offset+channel]=state&255;}
            noise.data[offset+3]=255;
          }
          context.putImageData(noise,0,0);
        }
        for (let i = 0; i < 5; i++) {
          if(!stressBitrate){context.fillStyle = ['red', 'green', 'blue', 'yellow', 'white'][i]; context.fillRect(0, 0, width, height);}
          const frame = new VideoFrame(largeCanvas, { timestamp: i * 1000 });
          try { encoder.encode(frame, { keyFrame: !stressBitrate||i===0 }); authoredFrames++; } finally { frame.close(); }
        }
        await bounded(encoder.flush());
      } finally { try { encoder?.close(); } finally { largeCanvas.width = 0; largeCanvas.height = 0; } }
      if(stressBitrate&&outputBudget)throw Error('public fixture output budget');
      if (encoderFailed || chunks.length !== 5) throw Error('maximum-geometry synthetic fixture failed');
      const stream = new Uint8Array(chunks.reduce((n, chunk) => n + chunk.length + 6, 0)); let position = 0;
      for (const chunk of chunks) { stream.set([0, 0, 0, 1, 9, 0xf0], position); position += 6; stream.set(chunk, position); position += chunk.length; }
      chunks.length = 0;
      const sha = async data => 'sha1-' + [...new Uint8Array(await crypto.subtle.digest('SHA-1', data))].map(v => v.toString(16).padStart(2, '0')).join('');
      const directory = new TextEncoder().encode(JSON.stringify({ version: { major: 1, minor: 0 }, picture: { accelerationArray: [{ type: 'com.lytro.acceleration.refocusStack', vendorContent: { blockOfImages: { representation: 'h264', blockOfImagesRef: await sha(stream), metadataArray: [0,3,-2,1,2].map(lambda => ({ width, height, lambda })) } } }] } }));
      const parts = [new Uint8Array([137,76,70,80,13,10,26,10,0,0,0,1,0,0,0,0])]; let total = 16;
      for (const [isDirectory, data] of [[true, directory], [false, stream]]) {
        const header = new Uint8Array(96); header.set([137,76,70,isDirectory?77:67,13,10,26,10]); new DataView(header.buffer).setUint32(12, data.length); header.set(new TextEncoder().encode(await sha(data)), 16);
        parts.push(header, data); total += 96 + data.length; const padding = (16 - total % 16) % 16; parts.push(new Uint8Array(padding)); total += padding;
      }
      if(total>8*1024*1024)throw Error('public fixture container budget');
      const large = new Uint8Array(total); position = 0; for (const part of parts) { large.set(part, position); position += part.length; }
      return {bytes:large,authoredFrames,...(stressBitrate?{encodedBytes,keyChunks,deltaChunks}:{})};
      };

// Ordinary callers retain the exact five-color/all-keyframe configuration.
window.authorLivePhotoFixture=(width,height)=>authorPublicLivePhotoFixture(width,height);
window.authorLivePhotoStressFixture=bitrate=>authorPublicLivePhotoFixture(2048,1024,bitrate);
