#!/usr/bin/env python3
"""Verify and retain synthetic CI evidence; never copies source libraries or caches."""
import hashlib
import gzip
import json
import os
import pathlib
import platform
import stat
import subprocess
import sys
import tarfile


def command(*args):
    return subprocess.check_output(args, text=True, timeout=30).strip()


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def verify_saved_image(path, runtime, revision, architecture):
    # Inspect only Docker's saved image metadata; never extract archive paths.
    with tarfile.open(path, 'r:gz') as archive:
        manifest_file = archive.extractfile('manifest.json')
        assert manifest_file is not None
        manifest = json.load(manifest_file)
        assert len(manifest) == 1 and runtime['image'] in manifest[0]['RepoTags']
        config_member = archive.getmember(manifest[0]['Config'])
        assert config_member.isfile() and config_member.size <= 2 * 1024**2
        config_file = archive.extractfile(config_member)
        assert config_file is not None
        config_bytes = config_file.read()
        config_digest = 'sha256:' + hashlib.sha256(config_bytes).hexdigest()
        if config_digest != runtime['imageID']:
            # Docker's containerd image store reports the OCI manifest digest;
            # its classic image store reports the configuration digest.
            index_file = archive.extractfile('index.json')
            assert index_file is not None
            index = json.load(index_file)
            descriptors = [entry for entry in index['manifests'] if entry['digest'] == runtime['imageID']]
            assert len(descriptors) == 1, 'saved image differs from exercised image'
            digest_name = runtime['imageID'].removeprefix('sha256:')
            assert len(digest_name) == 64 and all(c in '0123456789abcdef' for c in digest_name)
            image_member = archive.getmember('blobs/sha256/' + digest_name)
            assert image_member.isfile() and image_member.size <= 2 * 1024**2
            image_file = archive.extractfile(image_member)
            assert image_file is not None
            image_bytes = image_file.read()
            assert 'sha256:' + hashlib.sha256(image_bytes).hexdigest() == runtime['imageID']
            assert json.loads(image_bytes)['config']['digest'] == config_digest
        config = json.loads(config_bytes)
        assert config['architecture'] == architecture and config['os'] == 'linux'
        assert config['config']['Labels']['org.opencontainers.image.revision'] == revision
        # Bind the actual saved filesystem content to that configuration too.
        layers = manifest[0]['Layers']
        assert len(layers) == len(config['rootfs']['diff_ids'])
        for name, expected in zip(layers, config['rootfs']['diff_ids']):
            member = archive.getmember(name)
            assert member.isfile()
            source = archive.extractfile(member)
            assert source is not None
            magic = source.read(2)
            source.seek(0)
            reader = gzip.GzipFile(fileobj=source) if magic == b'\x1f\x8b' else source
            h = hashlib.sha256()
            size = 0
            for block in iter(lambda: reader.read(1024 * 1024), b''):
                size += len(block)
                assert size <= 1024**3, 'unexpected saved layer size'
                h.update(block)
            assert 'sha256:' + h.hexdigest() == expected, 'saved filesystem layer differs from exercised image'


directory = pathlib.Path(sys.argv[1]).resolve()
architecture = sys.argv[2]
machine_names = {'amd64': {'x86_64', 'amd64'}, 'arm64': {'aarch64', 'arm64'}}
assert architecture in machine_names
assert platform.system() == 'Linux' and platform.machine() in machine_names[architecture], 'native architecture required'
revision = command('git', 'rev-parse', 'HEAD')
assert len(revision) == 40 and revision == os.environ['LYTRONAUT_TEST_REVISION'], 'checkout revision mismatch'
assert not command('git', 'status', '--porcelain', '--untracked-files=no'), 'tracked source changed'
runtime = json.loads((directory / 'runtime.json').read_text())
assert runtime['revision'] == revision and runtime['platform'] == 'linux/' + architecture
assert runtime['runtimeArchitecture'] == architecture
assert runtime['engineArchitecture'] in machine_names[architecture], 'native engine required'
assert len(runtime['checks']) == 15, 'runtime checks incomplete'
program_paths = {
    '/app/lytronaut-service': 'native/lytronaut-service',
    '/app/client/lytronaut_client.js': 'client/lytronaut_client.js',
    '/app/client/lytronaut_client_bg.wasm': 'client/lytronaut_client_bg.wasm',
    '/app/notices/Cargo.lock': 'Cargo.lock',
}
observed = {}
for entry in runtime['artifactSHA256']:
    expected, name = entry.split('  ', 1)
    assert name in program_paths and name not in observed
    actual = digest(directory / 'programs' / program_paths[name])
    assert actual == expected, 'exercised program mismatch: ' + name
    observed[name] = actual
assert observed.keys() == program_paths.keys(), 'missing exercised program'
assert (directory / 'programs/raw/lytronaut_client.wasm').is_file(), 'missing raw WASM'
assert (directory / 'image.tar.gz').stat().st_size > 0, 'missing saved image'
verify_saved_image(directory / 'image.tar.gz', runtime, revision, architecture)
tracked = subprocess.check_output(['git', 'ls-files', '-z'], timeout=30).decode().split('\0')
provenance = {
    'revision': revision,
    'sourceTree': command('git', 'rev-parse', 'HEAD^{tree}'),
    'trackedSourceSHA256': {name: digest(pathlib.Path(name)) for name in sorted(tracked) if name},
    'architecture': architecture,
    'nativeExecution': True,
    'runnerMachine': platform.machine(),
    'profiles': 'Linux native release; Rust/WASM release',
    'rustVersion': '1.99.0',
    'wasmBindgenVersion': '0.2.129',
    'github': {name: os.environ.get(name) for name in ['GITHUB_REPOSITORY', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT', 'GITHUB_WORKFLOW', 'GITHUB_REF']},
    'dockerServer': json.loads(command('docker', 'version', '--format', '{{json .Server}}')),
    'runtimeVerified': True,
}
(directory / 'source-provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
files = {}
for path in sorted(directory.rglob('*')):
    assert not path.is_symlink(), 'unexpected evidence link'
    if path.is_file() and path.name != 'artifact-manifest.json':
        files[str(path.relative_to(directory))] = {'sha256': digest(path), 'bytes': path.stat().st_size, 'mode': stat.S_IMODE(path.stat().st_mode)}
(directory / 'artifact-manifest.json').write_text(json.dumps({'revision': revision, 'architecture': architecture, 'runtimeVerified': True, 'files': files}, indent=2) + '\n')
print('Verified exact native runtime, source and saved program/image hashes:', revision, architecture)
