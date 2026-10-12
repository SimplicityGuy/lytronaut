#!/usr/bin/env python3
"""Independently authored schema-23 records; synthetic JPEG provided by Rust encoder."""
import pathlib, sqlite3, sys, zipfile
root = pathlib.Path(sys.argv[1]); root.mkdir(mode=0o700, parents=True, exist_ok=True)
source = root / 'Synthetic.lytrolibrary'; source.mkdir(exist_ok=True)
(source / 'lytrolibrary.db').unlink(missing_ok=True)
c = sqlite3.connect(source / 'lytrolibrary.db')
c.executescript('''CREATE TABLE library_metadata(version INTEGER); INSERT INTO library_metadata VALUES(23);
CREATE TABLE picture(uuid TEXT,imagebin_uuid TEXT,event_uuid TEXT,name TEXT,caption TEXT,rating INTEGER,flag_status INTEGER);
CREATE TABLE picture_metadata(uuid TEXT,lfe_major INTEGER,lfe_minor INTEGER);
CREATE TABLE album(uuid TEXT,name TEXT,sort_order INTEGER,public INTEGER);
CREATE TABLE album_to_picture(album_uuid TEXT,picture_uuid TEXT,picture_index INTEGER);
CREATE TABLE event(uuid TEXT,library_uuid TEXT); INSERT INTO event VALUES('event','synthetic-library');
CREATE TABLE image_bin(uuid TEXT); INSERT INTO image_bin VALUES('bin');
CREATE TABLE picture_list(uuid TEXT,filter INTEGER,sort_order INTEGER); INSERT INTO picture_list VALUES('list',0,0);
CREATE TABLE imported_picture_history(hash TEXT);''')
for i in range(30):
    uid = f'synthetic-{i:03}'
    c.execute('INSERT INTO picture VALUES(?,?,?,?,?,?,?)',(uid,'bin','event',f'Synthetic {i:03}',f'Independent generated scene {i}', i % 6, 0))
    c.execute('INSERT INTO picture_metadata VALUES(?,?,?)',(uid,10,0))
    folder = source / 'bin' / uid; folder.mkdir(parents=True,exist_ok=True)
    (folder / 'thumbnail.jpg').write_bytes((root / 'synthetic.jpg').read_bytes())
    (folder / 'raw.lfp').write_bytes(b'independent preservation fixture')
for a in range(3):
    c.execute('INSERT INTO album VALUES(?,?,?,?)',(f'album-{a}',f'Album {a}',a,0))
    for i in range(a*10,a*10+10): c.execute('INSERT INTO album_to_picture VALUES(?,?,?)',(f'album-{a}',f'synthetic-{i:03}',9-i%10))
c.commit(); c.close()
with zipfile.ZipFile(root / 'synthetic.zip','w',compression=zipfile.ZIP_DEFLATED) as z:
    for p in source.rglob('*'):
        if p.is_file(): z.write(p,p.relative_to(root))
preserved = root / 'Preserved.lytrolibrary'; preserved.mkdir(exist_ok=True)
(preserved / 'lytrolibrary.db').unlink(missing_ok=True)
p = sqlite3.connect(preserved / 'lytrolibrary.db')
p.executescript('CREATE TABLE library_metadata(version INTEGER); INSERT INTO library_metadata VALUES(99);')
p.commit();p.close()
(preserved / 'unknown.bin').write_bytes(b'independent future schema preservation fixture')
# ZIP64 framing with small actual extents; no multi-gigabyte allocation.
with zipfile.ZipFile(root / 'synthetic-zip64.zip','w',compression=zipfile.ZIP_STORED) as z:
    for p in source.rglob('*'):
        if p.is_file():
            info=zipfile.ZipInfo(str(p.relative_to(root)))
            with z.open(info,'w',force_zip64=True) as f: f.write(p.read_bytes())
