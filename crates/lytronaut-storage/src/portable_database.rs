//! Trusted logical-row adapters. Archive bytes never supply SQL identifiers.
use crate::{
    Backend, Catalog, SourceEntry, StorageError as E,
    portable::{Cell, Column, Decoder, Encoder, Kind, TABLES},
};
use futures_util::TryStreamExt;
use sqlx::{Any, AssertSqlSafe, Row, Transaction, any::AnyRow};
use std::{
    io::{Read, Write},
    sync::atomic::{AtomicBool, Ordering},
};

const ORDER: &[&str] = &[
    "id",
    "job,path",
    "id",
    "version,path",
    "version,kind,ordinal",
    "version,uuid",
    "version,uuid",
    "version,album_uuid,picture_uuid",
    "version,picture_uuid,code",
    "id",
    "job",
    "job",
    "ordinal",
];
fn check(cancel: &AtomicBool) -> Result<(), E> {
    if cancel.load(Ordering::Acquire) {
        Err(E::Cancelled)
    } else {
        Ok(())
    }
}
fn filter(table: usize) -> &'static str {
    match table {
        0 => "state='succeeded'",
        1 | 10 | 11 => "job IN (SELECT id FROM intake_jobs WHERE state='succeeded')",
        2 => "source_job IN (SELECT id FROM intake_jobs WHERE state='succeeded')",
        3..=9 => {
            "version IN (SELECT id FROM catalog_versions WHERE source_job IN (SELECT id FROM intake_jobs WHERE state='succeeded'))"
        }
        12 => "kind='intake' AND job IN (SELECT id FROM intake_jobs WHERE state='succeeded')",
        _ => unreachable!("fixed table descriptor"),
    }
}
fn select(table: usize) -> String {
    let descriptor = &TABLES[table];
    format!(
        "SELECT {} FROM {} WHERE {} ORDER BY {}",
        descriptor
            .columns
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
            .join(","),
        descriptor.name,
        filter(table),
        ORDER[table]
    )
}
fn read_cell(row: &AnyRow, index: usize, column: Column, backend: Backend) -> Result<Cell, E> {
    match column.kind {
        Kind::Integer => Ok(row
            .try_get::<Option<i64>, _>(index)?
            .map_or(Cell::Null, Cell::Integer)),
        Kind::Blob => Ok(row
            .try_get::<Option<Vec<u8>>, _>(index)?
            .map_or(Cell::Null, Cell::Blob)),
        Kind::SourceText if backend == Backend::Postgres => {
            match row.try_get::<Option<Vec<u8>>, _>(index)? {
                Some(bytes) => Ok(Cell::Text(
                    String::from_utf8(bytes).map_err(|_| E::Invalid)?,
                )),
                None => Ok(Cell::Null),
            }
        }
        kind => match row.try_get::<Option<String>, _>(index)? {
            Some(text) => {
                let text = if matches!(kind, Kind::SourceKey) && backend == Backend::Postgres {
                    crate::source_text::decode_key(&text).map_err(|_| E::Invalid)?
                } else {
                    text
                };
                Ok(Cell::Text(text))
            }
            None => Ok(Cell::Null),
        },
    }
}
fn logical_row(row: &AnyRow, table: usize, backend: Backend) -> Result<Vec<Cell>, E> {
    TABLES[table]
        .columns
        .iter()
        .enumerate()
        .map(|(index, &column)| read_cell(row, index, column, backend))
        .collect()
}

/// Capture rows AND inventory in one consistent snapshot, ending it before blob
/// hashing/streaming. SQLite uses a dedicated typed connection with bounded
/// worker prefetch; PostgreSQL consumes its connection stream sequentially.
pub async fn capture<W: Write>(
    catalog: &Catalog,
    output: W,
    max_files: u32,
    cancel: &AtomicBool,
) -> Result<(W, Vec<SourceEntry>), E> {
    check(cancel)?;
    if catalog.backend() == Backend::Sqlite {
        return capture_sqlite(catalog, output, max_files, cancel).await;
    }
    let mut tx = catalog.begin_snapshot().await?;
    let mut encoder = Encoder::new(output)?;
    for (index, table) in TABLES.iter().enumerate() {
        check(cancel)?;
        let count_sql = format!(
            "SELECT count(*) FROM {} WHERE {}",
            table.name,
            filter(index)
        );
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(&count_sql))
            .fetch_one(&mut *tx)
            .await?;
        encoder.table(usize::try_from(count).map_err(|_| E::Invalid)?)?;
        let statement = select(index);
        let mut rows = sqlx::query(AssertSqlSafe(&statement)).fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            encoder.row(&logical_row(&row, index, catalog.backend())?)?;
        }
    }
    let mut inventory = Vec::new();
    {
        let mut rows = sqlx::query("SELECT digest,length FROM intake_sources WHERE job IN (SELECT id FROM intake_jobs WHERE state='succeeded') GROUP BY digest,length ORDER BY digest,length").fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            if inventory.len() >= max_files as usize {
                return Err(E::Quota);
            }
            inventory.push(SourceEntry {
                logical_path: String::new(),
                digest: row.try_get(0)?,
                length: u64::try_from(row.try_get::<i64, _>(1)?).map_err(|_| E::Invalid)?,
            });
        }
    }
    tx.commit().await?;
    Ok((encoder.finish()?, inventory))
}

async fn insert(
    catalog: &Catalog,
    tx: &mut Transaction<'_, Any>,
    table: usize,
    row: &[Cell],
) -> Result<(), E> {
    let descriptor = &TABLES[table];
    if row.len() != descriptor.columns.len() {
        return Err(E::Invalid);
    }
    let mut names = descriptor
        .columns
        .iter()
        .map(|c| c.name)
        .collect::<Vec<_>>();
    let derived = catalog.backend() == Backend::Postgres && matches!(table, 5 | 6);
    if derived {
        names.extend(["name_display", "name_fold", "name_bytes"]);
        if table == 5 {
            names.extend(["caption_display", "caption_fold", "caption_bytes"]);
        }
    }
    let parameters = (1..=names.len())
        .map(|n| format!("${n}"))
        .collect::<Vec<_>>()
        .join(",");
    let statement = format!(
        "INSERT INTO {}({}) VALUES({parameters})",
        descriptor.name,
        names.join(",")
    );
    let mut query = sqlx::query(AssertSqlSafe(&statement));
    for (cell, column) in row.iter().zip(descriptor.columns) {
        query = match cell {
            Cell::Integer(value) => query.bind(*value),
            Cell::Blob(value) => query.bind(value.clone()),
            Cell::Text(value) => match column.kind {
                Kind::SourceKey => query.bind(catalog.encode_source_key(value)),
                Kind::SourceText if catalog.backend() == Backend::Postgres => {
                    query.bind(value.as_bytes().to_vec())
                }
                _ => query.bind(value.clone()),
            },
            Cell::Null if column.nullable => match column.kind {
                Kind::Integer => query.bind(Option::<i64>::None),
                Kind::Blob => query.bind(Option::<Vec<u8>>::None),
                _ => query.bind(Option::<String>::None),
            },
            Cell::Null => return Err(E::Invalid),
        };
    }
    if derived {
        let fields: &[(usize, usize)] = if table == 5 {
            &[(4, 256), (5, 1024)]
        } else {
            &[(2, 256)]
        };
        for &(index, cap) in fields {
            let Cell::Text(value) = &row[index] else {
                return Err(E::Invalid);
            };
            let projection = crate::source_text::TextProjection::new(value, cap);
            query = query
                .bind(projection.display)
                .bind(projection.folded_prefix)
                .bind(projection.original_bytes);
        }
    }
    query.execute(&mut **tx).await?;
    Ok(())
}
/// Destination schemas are freshly created using application DDL. All tables
/// publish together; constraint or cancellation failure rolls back every row.
pub async fn import<R: Read>(
    catalog: &Catalog,
    input: R,
    length: usize,
    cancel: &AtomicBool,
) -> Result<R, E> {
    let mut decoder = Decoder::new(input, length)?;
    let mut tx = catalog.begin_write().await?;
    while let Some((table, count)) = decoder.table()? {
        for _ in 0..count {
            check(cancel)?;
            insert(catalog, &mut tx, table, &decoder.row()?).await?;
        }
    }
    check(cancel)?;
    let input = decoder.finish()?;
    tx.commit().await?;
    Ok(input)
}

/// Legacy input has already passed exact read-only schema/integrity/inventory
/// validation. This bypasses NEW v2 row/wire limits: SQLite's 32 MiB FILE limit
/// remains its acceptance bound, even when tagged rows or UTF-8 expand.
pub async fn import_legacy(
    catalog: &Catalog,
    source: &mut sqlx::SqliteConnection,
    has_order: bool,
    cancel: &AtomicBool,
) -> Result<(), E> {
    let mut read = sqlx::Connection::begin(source).await?;
    let mut write = catalog.begin_write().await?;
    for table in 0..TABLES.len() {
        let statement = if table == 12 && !has_order {
            "SELECT rowid AS ordinal,'intake' AS kind,id AS job FROM intake_jobs WHERE state='succeeded' ORDER BY rowid".to_owned()
        } else {
            select(table)
        };
        let mut rows = sqlx::query(AssertSqlSafe(&statement)).fetch(&mut *read);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            insert(catalog, &mut write, table, &sqlite_row(&row, table)?).await?;
        }
    }
    check(cancel)?;
    read.commit().await?;
    write.commit().await?;
    Ok(())
}

fn sqlite_row(row: &sqlx::sqlite::SqliteRow, table: usize) -> Result<Vec<Cell>, E> {
    TABLES[table]
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| match column.kind {
            Kind::Integer => Ok(row
                .try_get::<Option<i64>, _>(index)?
                .map_or(Cell::Null, Cell::Integer)),
            Kind::Blob => Ok(row
                .try_get::<Option<Vec<u8>>, _>(index)?
                .map_or(Cell::Null, Cell::Blob)),
            _ => Ok(row
                .try_get::<Option<String>, _>(index)?
                .map_or(Cell::Null, Cell::Text)),
        })
        .collect()
}
async fn capture_sqlite<W: Write>(
    catalog: &Catalog,
    output: W,
    max_files: u32,
    cancel: &AtomicBool,
) -> Result<(W, Vec<SourceEntry>), E> {
    // This connector carries row_buffer_size(1) directly into SQLite's worker;
    // at most one queued row, one held row and one worker-produced row exist.
    // Conversion and destination bindings are additional counted allocations.
    let mut connection = catalog.sqlite_snapshot_connection().await?;
    let mut tx = sqlx::Connection::begin(&mut connection).await?;
    let mut encoder = Encoder::new(output)?;
    for (index, table) in TABLES.iter().enumerate() {
        check(cancel)?;
        let statement = format!(
            "SELECT count(*) FROM {} WHERE {}",
            table.name,
            filter(index)
        );
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(&statement))
            .fetch_one(&mut *tx)
            .await?;
        encoder.table(usize::try_from(count).map_err(|_| E::Invalid)?)?;
        let statement = select(index);
        let mut rows = sqlx::query(AssertSqlSafe(&statement)).fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            encoder.row(&sqlite_row(&row, index)?)?;
        }
    }
    let mut inventory = Vec::new();
    {
        let mut rows=sqlx::query("SELECT digest,length FROM intake_sources WHERE job IN (SELECT id FROM intake_jobs WHERE state='succeeded') GROUP BY digest,length ORDER BY digest,length").fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            if inventory.len() >= max_files as usize {
                return Err(E::Quota);
            }
            inventory.push(SourceEntry {
                logical_path: String::new(),
                digest: row.try_get(0)?,
                length: u64::try_from(row.try_get::<i64, _>(1)?).map_err(|_| E::Invalid)?,
            });
        }
    }
    tx.commit().await?;
    sqlx::Connection::close(connection).await?;
    Ok((encoder.finish()?, inventory))
}
