//! Application-owned SQLite schema. These statements retain existing deployed table definitions.

pub(crate) const INTAKE: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS intake_jobs (id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL, total INTEGER NOT NULL, budget INTEGER NOT NULL, max_files INTEGER NOT NULL, encoded_limit INTEGER NOT NULL, zip_complete INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0, files INTEGER NOT NULL DEFAULT 0, path_bytes INTEGER NOT NULL DEFAULT 0, reserved INTEGER NOT NULL, error TEXT)",
    "CREATE TABLE IF NOT EXISTS intake_sources (job TEXT NOT NULL REFERENCES intake_jobs(id), path TEXT NOT NULL, canonical TEXT NOT NULL, digest TEXT NOT NULL, length INTEGER NOT NULL, PRIMARY KEY(job,path), UNIQUE(job,canonical))",
    "CREATE TABLE IF NOT EXISTS intake_scratch (owner TEXT PRIMARY KEY, root TEXT NOT NULL, job TEXT NOT NULL REFERENCES intake_jobs(id), bytes INTEGER NOT NULL CHECK(bytes>0))",
];

pub(crate) const CATALOG: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS catalog_versions(id INTEGER PRIMARY KEY, fingerprint TEXT NOT NULL UNIQUE, source_job TEXT NOT NULL REFERENCES intake_jobs(id), library_uuid TEXT, state TEXT NOT NULL, reason TEXT, conflict INTEGER NOT NULL DEFAULT 0)",
    "CREATE TABLE IF NOT EXISTS catalog_sources(version INTEGER NOT NULL REFERENCES catalog_versions(id), job TEXT NOT NULL, path TEXT NOT NULL, digest TEXT NOT NULL, length INTEGER NOT NULL, PRIMARY KEY(version,path), FOREIGN KEY(job,path) REFERENCES intake_sources(job,path))",
    "CREATE TABLE IF NOT EXISTS catalog_records(version INTEGER NOT NULL REFERENCES catalog_versions(id), kind TEXT NOT NULL, ordinal INTEGER NOT NULL, original BLOB NOT NULL, PRIMARY KEY(version,kind,ordinal))",
    "CREATE TABLE IF NOT EXISTS catalog_pictures(version INTEGER NOT NULL REFERENCES catalog_versions(id), uuid TEXT NOT NULL, bin_uuid TEXT NOT NULL, event_uuid TEXT NOT NULL, name TEXT NOT NULL, caption TEXT NOT NULL, rating INTEGER NOT NULL, flag_status INTEGER NOT NULL, PRIMARY KEY(version,uuid))",
    "CREATE TABLE IF NOT EXISTS catalog_albums(version INTEGER NOT NULL REFERENCES catalog_versions(id), uuid TEXT NOT NULL, name TEXT NOT NULL, sort_order INTEGER NOT NULL, PRIMARY KEY(version,uuid))",
    "CREATE TABLE IF NOT EXISTS catalog_memberships(version INTEGER NOT NULL, album_uuid TEXT NOT NULL, picture_uuid TEXT NOT NULL, picture_index INTEGER NOT NULL, PRIMARY KEY(version,album_uuid,picture_uuid), UNIQUE(version,album_uuid,picture_index), FOREIGN KEY(version,album_uuid) REFERENCES catalog_albums(version,uuid), FOREIGN KEY(version,picture_uuid) REFERENCES catalog_pictures(version,uuid))",
    "CREATE TABLE IF NOT EXISTS catalog_item_errors(version INTEGER NOT NULL REFERENCES catalog_versions(id), picture_uuid TEXT NOT NULL, code TEXT NOT NULL, PRIMARY KEY(version,picture_uuid,code))",
];

pub(crate) const API: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS api_ids(id TEXT PRIMARY KEY,kind TEXT NOT NULL,version INTEGER NOT NULL REFERENCES catalog_versions(id),source_key TEXT NOT NULL,UNIQUE(kind,version,source_key))",
    "CREATE TABLE IF NOT EXISTS api_imports(job TEXT PRIMARY KEY REFERENCES intake_jobs(id),version INTEGER NOT NULL REFERENCES catalog_versions(id))",
    "CREATE INDEX IF NOT EXISTS api_ids_lookup ON api_ids(kind,version,source_key)",
    "CREATE TABLE IF NOT EXISTS api_adaptation_errors(job TEXT PRIMARY KEY REFERENCES intake_jobs(id),code TEXT NOT NULL)",
];

pub(crate) const EXPORT: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS export_jobs(id TEXT PRIMARY KEY,kind TEXT NOT NULL,state TEXT NOT NULL,completed INTEGER NOT NULL DEFAULT 0,total INTEGER NOT NULL DEFAULT 0,reserved INTEGER NOT NULL DEFAULT 0,retained INTEGER NOT NULL DEFAULT 0,error TEXT,recipe TEXT)",
];
