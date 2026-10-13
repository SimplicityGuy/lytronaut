#!/usr/bin/env python3
"""Synthetic runtime proof for a prebuilt image; never opens a private library."""
import hashlib, http.client, json, os, pathlib, socket, subprocess, sys, tempfile, time, urllib.error, urllib.request

def docker(*args, capture=True):
    return subprocess.run(['docker', *args], check=True, text=True,
                          stdout=subprocess.PIPE if capture else None, timeout=180).stdout

image, platform = sys.argv[1:3]
prefix = f'lytronaut-smoke-{os.getpid()}'
volumes = [prefix+'-'+name for name in ['catalog', 'originals', 'work']]
network = prefix+'-offline'
ingress_network = prefix+'-ingress'
relay = prefix+'-relay'
relay_image = os.environ.get('LYTRONAUT_SMOKE_RELAY_IMAGE','lytronaut:smoke-relay')
container = prefix
report = {'image': image, 'platform': platform, 'checks': [], 'readinessSeconds': []}
timing = {}

def checked(name):
    report['checks'].append(name)
    print('runtime:',name,file=sys.stderr)

with tempfile.TemporaryDirectory(prefix='lytronaut-container-fixture-') as scratch:
    root = pathlib.Path(scratch)
    target = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', 'target'))
    generator = pathlib.Path(os.environ.get('LYTRONAUT_SYNTHETIC_ENCODER', target/'debug/examples/synthetic_jpeg'))
    subprocess.run([str(generator.resolve()), str(root/'synthetic.jpg')], check=True, timeout=60)
    subprocess.run([sys.executable, 'scripts/synthetic-library.py', str(root)], check=True, timeout=60)
    fixture = root/'Synthetic.lytrolibrary'
    # Independently generated opaque originals keep real hashing/archive workers
    # busy long enough to observe their running state before SIGTERM.
    for i in range(4): (fixture/f'opaque-{i}.bin').write_bytes(os.urandom(12*1024**2))
    originals = {str(p.relative_to(fixture)): p.read_bytes() for p in fixture.rglob('*') if p.is_file()}
    try:
        docker('network', 'create', '--internal', network)
        docker('network', 'create', ingress_network)
        docker('run','-d','--name',relay,'--platform',platform,'--network',ingress_network,
               '--read-only','--cap-drop=ALL','--security-opt=no-new-privileges:true',
               '--memory=64m','--pids-limit=64','-p','127.0.0.1::3000',relay_image,container+':3000')
        docker('network','connect',network,relay)
        relay_info=json.loads(docker('inspect',relay))[0]
        port=int(relay_info['NetworkSettings']['Ports']['3000/tcp'][0]['HostPort'])
        for volume in volumes: docker('volume', 'create', volume)
        def start(restored=None, listen=None):
            timing['launch']=time.monotonic()
            docker('run', '-d', '--name', container, '--platform', platform, '--network', network,
                '--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges:true', '--user', '10001:10001',
                '--cpus=2', '--memory=2g', '--pids-limit=128',
                '-e', 'LYTRONAUT_ORIGIN=http://127.0.0.1:3000', '-e', 'LYTRONAUT_FREE_FLOOR=2147483648',
                '-e', 'LYTRONAUT_UPLOAD_BYTES=67108864', '-e', 'LYTRONAUT_FILE_BYTES=16777216',
                # Archive admission reserves unique originals + 2 x 288 MiB portable catalog
                # spool/copy + framing (docs/design/007-portable-backup.md), ~625 MiB here.
                '-e', 'LYTRONAUT_ENCODED_BYTES=67108864', '-e', 'LYTRONAUT_RESERVED_BYTES=2147483648',
                '-e', 'LYTRONAUT_UPLOAD_FILES=1000',
                *(['-e','LYTRONAUT_LISTEN='+listen] if listen else []),
                *(['-e',f'LYTRONAUT_DATA=/data/work/staging.exports/{restored}.restored/store',
                   '-e',f'LYTRONAUT_ORIGINALS=/data/work/staging.exports/{restored}.restored/store/originals',
                   '-e',f'LYTRONAUT_STAGING=/data/work/staging.exports/{restored}.restored/store/staging'] if restored else []),
                *sum((['-v', f'{v}:/data/{n}'] for v,n in zip(volumes,['catalog','originals','work'])), []), image)
            info = json.loads(docker('inspect', container))[0]
            assert info['State']['Running'], (info['State'],docker('logs',container))
            return port
        port = start()
        def request(method, path, body=None, headers=None, expected=200):
            h = {'Host':'127.0.0.1:3000', **(headers or {})}
            if method not in ['GET','HEAD']:
                h.update({'Origin':'http://127.0.0.1:3000', 'X-Lytronaut-Intent':'mutation'})
            if isinstance(body, dict): body=json.dumps(body).encode();h['Content-Type']='application/json'
            req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=body, headers=h, method=method)
            try:
                with urllib.request.urlopen(req, timeout=60) as response: status=response.status; data=response.read()
            except urllib.error.HTTPError as error: status=error.code;data=error.read()
            assert status == expected, (method,path,status,data[:200])
            return data
        def ready():
            readiness_started=timing['launch']
            deadline=time.monotonic()+120
            while time.monotonic()<deadline:
                try:
                    if json.loads(request('GET','/healthz'))['ready']:
                        report['readinessSeconds'].append(time.monotonic()-readiness_started)
                        return
                except (OSError,AssertionError): pass
                time.sleep(.25)
            raise AssertionError('startup not ready in 120s')
        ready();checked('native readiness; no credentials; offline internal network')
        assert b'<html' in request('GET','/').lower() or b'<!doctype' in request('GET','/').lower()
        assert request('GET','/client/lytronaut_client_bg.wasm').startswith(b'\0asm')
        for route in ['login','setup','session','logout']: request('GET','/auth/'+route,expected=404)
        assert docker('exec',container,'/app/lytronaut-service','--healthcheck') == ''
        for env in ['LYTRONAUT_LISTEN=127.0.0.1:1','LYTRONAUT_LISTEN=invalid','LYTRONAUT_ORIGIN=https://foreign.invalid']:
            probe=subprocess.run(['docker','exec','-e',env,container,'/app/lytronaut-service','--healthcheck'],capture_output=True,timeout=10)
            assert probe.returncode==1
        checked('bounded health probe failure for unavailable/malformed listener and mismatched Host')
        info=json.loads(docker('inspect',container))[0]
        assert info['Config']['User']=='10001:10001' and info['HostConfig']['ReadonlyRootfs']
        assert info['HostConfig']['Memory']==2*1024**3 and info['HostConfig']['NanoCpus']==2_000_000_000
        assert info['HostConfig']['PidsLimit']==128 and info['HostConfig']['CapDrop']==['ALL']
        assert json.loads(docker('network','inspect',network))[0]['Internal']
        assert list(info['NetworkSettings']['Networks'])==[network]
        routes=docker('exec',container,'cat','/proc/net/route').splitlines()[1:]
        assert all(route.split()[1]!='00000000' for route in routes), routes
        assert docker('exec',container,'sh','-c','test ! -e /data/catalog/bootstrap.secret; test ! -w /app; test ! -w /etc; id -u').strip()=='10001'
        assert docker('exec',container,'cat','/sys/fs/cgroup/memory.max').strip()==str(2*1024**3)
        assert docker('exec',container,'cat','/sys/fs/cgroup/pids.max').strip()=='128'
        quota,period=map(int,docker('exec',container,'cat','/sys/fs/cgroup/cpu.max').split())
        assert quota==2*period
        docker('exec',container,'sh','-c','if touch /data/outside-volume 2>/dev/null; then exit 1; fi; touch /data/catalog/proof.tmp; rm /data/catalog/proof.tmp')
        checked('effective cgroup CPU/memory/PID caps and failed write outside mounted volumes')
        report['artifactSHA256']=docker('exec',container,'sha256sum','/app/lytronaut-service','/app/client/lytronaut_client.js','/app/client/lytronaut_client_bg.wasm','/app/notices/Cargo.lock').splitlines()
        report['packages']=docker('exec',container,'dpkg-query','-W','-f=${Package}\t${Version}\n').splitlines()
        assert not any(p.split('\t')[0] in ['python3','ffmpeg','mono-runtime','nvidia-driver'] for p in report['packages'])
        checked('nonroot read-only root; writable volume boundaries; resource caps; runtime package inventory')
        conn=http.client.HTTPConnection('127.0.0.1',port,timeout=10)
        conn.request('POST','/imports',b'{}',headers={'Host':'foreign.invalid','Origin':'http://127.0.0.1:3000','X-Lytronaut-Intent':'mutation'})
        assert conn.getresponse().status==403;conn.close()
        conn=http.client.HTTPConnection('127.0.0.1',port,timeout=10)
        conn.request('POST','/imports',b'{}',headers={'Host':'127.0.0.1:3000','Origin':'https://foreign.invalid','X-Lytronaut-Intent':'mutation'})
        assert conn.getresponse().status==403;conn.close();checked('foreign Origin and invalid Host mutations rejected')
        request('POST','/imports',{'version':1,'kind':'directory','declared_bytes':67108865,'declared_files':1},expected=413)
        checked('storage admission cap enforced')
        job=json.loads(request('POST','/imports',{'version':1,'kind':'directory','declared_bytes':sum(map(len,originals.values())),'declared_files':len(originals)},expected=201))['id']
        for path,data in originals.items():
            request('PUT',f'/imports/{job}/file',data,headers={'X-Lytronaut-Entry-Type':'file','X-Lytronaut-Path-Hex':path.encode().hex()},expected=204)
        request('POST',f'/imports/{job}/finish',b'',expected=204)
        libraries=json.loads(request('GET','/libraries'))['items'];assert len(libraries)==1
        assert libraries[0]['state']=='ready', (libraries,json.loads(request('GET','/imports/'+job)))
        library=libraries[0]['id'];captures=json.loads(request('GET',f'/libraries/{library}/captures?limit=100'))['items'];assert len(captures)==30
        focus_capture=next(c for c in captures if c['name']=='Synthetic 000')
        detail=json.loads(request('GET','/captures/'+focus_capture['id']))
        for asset in detail['originals']:
            data=request('GET',f"/assets/{asset['id']}/download")
            assert any(data==source for source in originals.values())
        focus=next(a for a in detail['originals'] if a['role']=='jpeg_focus_stack')
        assert len(json.loads(request('GET',f"/assets/{focus['id']}/focus"))['lambdas'])==3
        assert request('GET',f"/assets/{focus['id']}/focus/0/1").startswith(b'\xff\xd8')
        checked('synthetic directory import; browse; exact originals; CPU focus')
        def terminal(job):
            deadline=time.monotonic()+60
            while time.monotonic()<deadline:
                value=json.loads(request('GET','/exports/'+job))
                if value['state'] in ['succeeded','failed','cancelled']: return value
                time.sleep(.1)
            raise AssertionError('export not terminal')
        export=json.loads(request('POST','/exports',{'kind':'flatten','version':1,'asset':focus['id'],'format':'png','plane_index':0,'orientation':1,'operations':[]}))['id']
        assert terminal(export)['state']=='succeeded';assert request('GET',f'/exports/{export}/download').startswith(b'\x89PNG')
        backup=json.loads(request('POST','/exports',{'kind':'archive_originals','version':1}))['id']
        assert terminal(backup)['state']=='succeeded';bundle=request('GET',f'/exports/{backup}/download')
        restore=json.loads(request('POST','/restores',bundle))['id'];assert terminal(restore)['state']=='succeeded'
        checked('flatten export; lossless backup; isolated restore')
        # Model process interruption after exclusive creation of a private source
        # snapshot, before its normal drop cleanup. Reopen must remove scratch.
        orphan='/data/work/staging/lytronaut-desktop-00000000000000000000000000000000.db'
        docker('exec',container,'sh','-c',f'printf synthetic-interrupted-snapshot > {orphan}; chmod 400 {orphan}')
        # Switch only this disposable service to the restored store; originals must still browse.
        docker('stop','--time','30',container)
        stopped=json.loads(docker('inspect',container))[0];assert stopped['State']['ExitCode']==0
        docker('rm',container)
        port=start(restored=restore);ready();assert len(json.loads(request('GET',f'/libraries/{library}/captures?limit=100'))['items'])==30
        checked('restored store startup and browsing without credentials')
        docker('stop','--time','30',container);docker('rm',container)
        port=start();ready();assert len(json.loads(request('GET',f'/libraries/{library}/captures?limit=100'))['items'])==30
        docker('exec',container,'test','!','-e',orphan)
        checked('interrupted flat catalog snapshot removed on exclusive store recovery')
        checked('SIGTERM shutdown; same-volume restart; persistent browse')
        active_export=json.loads(request('POST','/exports',{'kind':'archive_originals','version':1}))['id']
        deadline=time.monotonic()+10
        while time.monotonic()<deadline:
            before_stop=json.loads(request('GET','/exports/'+active_export))['state']
            if before_stop=='running': break
            assert before_stop=='queued', ('archive finished before running-state observation',before_stop)
            time.sleep(.01)
        else: raise AssertionError('archive worker never ran')
        report['exportBeforeSignal']=before_stop
        shutdown_started=time.monotonic();docker('stop','--time','30',container)
        report['activeExportShutdownSeconds']=time.monotonic()-shutdown_started
        assert report['activeExportShutdownSeconds']<25
        assert json.loads(docker('inspect',container))[0]['State']['ExitCode']==0
        docker('rm',container);port=start();ready()
        report['exportAfterRestart']=json.loads(request('GET','/exports/'+active_export))['state']
        assert report['exportAfterRestart'] in ['succeeded','cancelled','failed']
        checked('observed running archive worker; graceful SIGTERM; terminal recovery after restart')
        # Stalled upload must not defer shutdown beyond the documented drain budget.
        stalled=json.loads(request('POST','/imports',{'version':1,'kind':'directory','declared_bytes':1024,'declared_files':1},expected=201))['id']
        sock=socket.create_connection(('127.0.0.1',port),timeout=10)
        sock.sendall((f'PUT /imports/{stalled}/file HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nOrigin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: mutation\r\nX-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: '+b'pending.bin'.hex()+'\r\nContent-Length: 1024\r\n\r\nx').encode())
        time.sleep(.2);shutdown_started=time.monotonic();docker('stop','--time','30',container);report['stalledShutdownSeconds']=time.monotonic()-shutdown_started;sock.close()
        assert report['stalledShutdownSeconds']<25
        assert json.loads(docker('inspect',container))[0]['State']['ExitCode']!=137
        docker('rm',container);port=start();ready()
        assert json.loads(request('GET','/imports/'+stalled))['state'] in ['cancelled','failed']
        assert len(json.loads(request('GET',f'/libraries/{library}/captures?limit=100'))['items'])==30
        checked('stalled upload bounded SIGTERM; interrupted-work recovery; existing library intact')
        # A stalled JSON extractor has no intake cancellation flag: bounded fallback
        # must exit without closing the database under in-flight handlers.
        sock=socket.create_connection(('127.0.0.1',port),timeout=10)
        sock.sendall(b'POST /exports HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nOrigin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: mutation\r\nContent-Type: application/json\r\nContent-Length: 1024\r\n\r\n{')
        time.sleep(.2);shutdown_started=time.monotonic();docker('stop','--time','30',container)
        report['boundedInterruptionShutdownSeconds']=time.monotonic()-shutdown_started;sock.close()
        assert report['boundedInterruptionShutdownSeconds']<25
        exit_code=json.loads(docker('inspect',container))[0]['State']['ExitCode']
        assert exit_code in [0,1]
        if exit_code==1: assert report['boundedInterruptionShutdownSeconds']>=19
        report['stalledJSONExitCode']=exit_code
        docker('rm',container);port=start();ready()
        assert len(json.loads(request('GET',f'/libraries/{library}/captures?limit=100'))['items'])==30
        checked('stalled JSON body bounded interruption without SIGKILL; volume recovery and browse')
        docker('stop','--time','30',container);docker('rm',container)
        port=start(listen='[::]:3000')
        deadline=time.monotonic()+120
        while time.monotonic()<deadline:
            probe=subprocess.run(['docker','exec',container,'/app/lytronaut-service','--healthcheck'],capture_output=True,timeout=10)
            if probe.returncode==0: break
            time.sleep(.25)
        else: raise AssertionError('IPv6 listener not healthy')
        checked('actual IPv6 wildcard startup and matching-family health')
        image_info=json.loads(docker('image','inspect',image))[0]
        assert image_info['Architecture']==platform.split('/')[1]
        report['imageID']=image_info['Id']
        report['engineArchitecture']=docker('info','--format','{{.Architecture}}').strip()
        report['runtimeArchitecture']=image_info['Architecture']
        report['revision']=os.environ.get('LYTRONAUT_TEST_REVISION','working-tree')
        if report['revision']!='working-tree': assert image_info['Config']['Labels']['org.opencontainers.image.revision']==report['revision']
        print(json.dumps(report,indent=2))
    finally:
        subprocess.run(['docker','rm','-f',container],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','rm','-f',relay],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for volume in volumes: subprocess.run(['docker','volume','rm',volume],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','network','rm',network],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','network','rm',ingress_network],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
