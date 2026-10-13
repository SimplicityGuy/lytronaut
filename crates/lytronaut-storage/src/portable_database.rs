//! Trusted logical-row adapters. Archive bytes never supply SQL identifiers.
//! SQL audit: every dynamic statement interpolates only static `TABLES`
//! names/columns, the static `filter`/`ORDER` fragments, fixed derived column
//! names and `$n` placeholders. Table indices come from the decoder's own
//! sequential counter (the archive byte must equal it), never from input.
use crate::{
    Backend, Catalog, StorageError as E,
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
///
/// The unique-original inventory is streamed to `inventory` as strictly
/// ascending 40-byte records (SHA-256, little-endian u64 length), so a library
/// with any number of originals is captured at constant memory.
pub async fn capture<W: Write, I: Write>(
    catalog: &Catalog,
    output: W,
    inventory: I,
    cancel: &AtomicBool,
) -> Result<(W, I, Inventory), E> {
    check(cancel)?;
    if catalog.backend() == Backend::Sqlite {
        return capture_sqlite(catalog, output, inventory, cancel).await;
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
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(count_sql.as_str()))
            .fetch_one(&mut *tx)
            .await?;
        encoder.table(usize::try_from(count).map_err(|_| E::Invalid)?)?;
        let statement = select(index);
        let mut rows = sqlx::query(AssertSqlSafe(statement.as_str())).fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            encoder.row(&logical_row(&row, index, catalog.backend())?)?;
        }
    }
    let mut inventory = inventory;
    let mut order = Ascending::default();
    {
        let mut rows = sqlx::query(inventory_sql(Backend::Postgres, true)).fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            order.write(
                &mut inventory,
                &row.try_get::<String, _>(0)?,
                row.try_get(1)?,
            )?;
        }
    }
    tx.commit().await?;
    Ok((encoder.finish()?, inventory, order.inventory()))
}

/// Bytes of one inventory record: SHA-256 and little-endian u64 length.
pub const INVENTORY_RECORD: usize = 40;
/// Count and total length of the unique originals in an inventory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub count: u64,
    pub bytes: u64,
}
/// A strictly ascending (SHA-256, length) sequence checked while it streams:
/// only the previous record is remembered, so duplicates and disorder are
/// rejected at constant memory however many originals a library holds.
#[derive(Default)]
pub struct Ascending {
    previous: Option<([u8; 32], u64)>,
    inventory: Inventory,
}
impl Ascending {
    /// Accept the next record; a repeat is `Quota`, a smaller one `Invalid`.
    pub fn push(&mut self, hash: [u8; 32], length: u64) -> Result<(), E> {
        if let Some(previous) = self.previous {
            if (hash, length) == previous {
                return Err(E::Quota);
            }
            if (hash, length) < previous {
                return Err(E::Invalid);
            }
        }
        self.previous = Some((hash, length));
        self.inventory.count = self.inventory.count.checked_add(1).ok_or(E::Quota)?;
        self.inventory.bytes = self.inventory.bytes.checked_add(length).ok_or(E::Quota)?;
        Ok(())
    }
    pub fn inventory(&self) -> Inventory {
        self.inventory
    }
    fn write<I: Write>(&mut self, out: &mut I, digest: &str, length: i64) -> Result<(), E> {
        let hash = digest_bytes(digest)?;
        let length = u64::try_from(length).map_err(|_| E::Invalid)?;
        self.push(hash, length)?;
        out.write_all(&record(&hash, length))?;
        Ok(())
    }
}
pub fn record(hash: &[u8; 32], length: u64) -> [u8; INVENTORY_RECORD] {
    let mut record = [0; INVENTORY_RECORD];
    record[..32].copy_from_slice(hash);
    record[32..].copy_from_slice(&length.to_le_bytes());
    record
}
pub fn parse_record(record: &[u8; INVENTORY_RECORD]) -> ([u8; 32], u64) {
    let mut hash = [0; 32];
    hash.copy_from_slice(&record[..32]);
    let mut length = [0; 8];
    length.copy_from_slice(&record[32..]);
    (hash, u64::from_le_bytes(length))
}
/// Lowercase-hex SHA-256 as stored by the application; anything else is invalid.
pub fn digest_bytes(digest: &str) -> Result<[u8; 32], E> {
    let text = digest.as_bytes();
    if text.len() != 64 {
        return Err(E::Invalid);
    }
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(E::Invalid),
    };
    let mut hash = [0; 32];
    for (index, byte) in hash.iter_mut().enumerate() {
        *byte = (nibble(text[index * 2])? << 4) | nibble(text[index * 2 + 1])?;
    }
    Ok(hash)
}
pub fn digest_hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
/// Unique originals in bytewise (digest, length) order on both engines; the
/// PostgreSQL "C" collation keeps that order independent of database locale.
fn inventory_sql(backend: Backend, succeeded_only: bool) -> &'static str {
    match (backend, succeeded_only) {
        (Backend::Sqlite, true) => {
            "SELECT digest,length FROM intake_sources WHERE job IN (SELECT id FROM intake_jobs WHERE state='succeeded') GROUP BY digest,length ORDER BY digest,length"
        }
        (Backend::Postgres, true) => {
            "SELECT digest,length FROM intake_sources WHERE job IN (SELECT id FROM intake_jobs WHERE state='succeeded') GROUP BY digest,length ORDER BY digest COLLATE \"C\",length"
        }
        (Backend::Sqlite, false) => {
            "SELECT digest,length FROM intake_sources GROUP BY digest,length ORDER BY digest,length"
        }
        (Backend::Postgres, false) => {
            "SELECT digest,length FROM intake_sources GROUP BY digest,length ORDER BY digest COLLATE \"C\",length"
        }
    }
}
/// Stream-compare a restored catalog's unique originals with the bundle's
/// ascending inventory records at constant memory. Any difference in count,
/// digest or length is `Conflict`.
pub async fn verify_inventory<R: Read>(
    catalog: &Catalog,
    mut records: R,
    cancel: &AtomicBool,
) -> Result<(), E> {
    let mut rows = sqlx::query(inventory_sql(catalog.backend(), false)).fetch(catalog.pool());
    while let Some(row) = rows.try_next().await? {
        check(cancel)?;
        let digest: String = row.try_get(0)?;
        let length = u64::try_from(row.try_get::<i64, _>(1)?).map_err(|_| E::Invalid)?;
        let mut next = [0; INVENTORY_RECORD];
        records.read_exact(&mut next).map_err(|_| E::Conflict)?;
        if parse_record(&next) != (digest_bytes(&digest)?, length) {
            return Err(E::Conflict);
        }
    }
    drop(rows);
    let mut tail = [0];
    if records.read(&mut tail)? != 0 {
        return Err(E::Conflict);
    }
    Ok(())
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
    let mut query = sqlx::query(AssertSqlSafe(statement.as_str()));
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
        let mut rows = sqlx::query(AssertSqlSafe(statement.as_str())).fetch(&mut *read);
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
async fn capture_sqlite<W: Write, I: Write>(
    catalog: &Catalog,
    output: W,
    inventory: I,
    cancel: &AtomicBool,
) -> Result<(W, I, Inventory), E> {
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
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(statement.as_str()))
            .fetch_one(&mut *tx)
            .await?;
        encoder.table(usize::try_from(count).map_err(|_| E::Invalid)?)?;
        let statement = select(index);
        let mut rows = sqlx::query(AssertSqlSafe(statement.as_str())).fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            encoder.row(&sqlite_row(&row, index)?)?;
        }
    }
    let mut inventory = inventory;
    let mut order = Ascending::default();
    {
        let mut rows = sqlx::query(inventory_sql(Backend::Sqlite, true)).fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            check(cancel)?;
            order.write(
                &mut inventory,
                &row.try_get::<String, _>(0)?,
                row.try_get(1)?,
            )?;
        }
    }
    tx.commit().await?;
    sqlx::Connection::close(connection).await?;
    Ok((encoder.finish()?, inventory, order.inventory()))
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    #[test]
    fn ascending_records_are_checked_at_constant_memory() {
        let low = [0u8; 32];
        let mut high = [0u8; 32];
        high[0] = 0xff;
        let mut order = Ascending::default();
        order.push(low, 7).unwrap();
        order.push(low, 8).unwrap();
        order.push(high, 1).unwrap();
        assert_eq!(
            order.inventory(),
            Inventory {
                count: 3,
                bytes: 16
            }
        );
        assert!(matches!(order.push(high, 1), Err(E::Quota)));
        assert!(matches!(order.push(low, 9), Err(E::Invalid)));
        let mut overflow = Ascending::default();
        overflow.push(low, u64::MAX).unwrap();
        assert!(matches!(overflow.push(high, 1), Err(E::Quota)));
        let mut out = Vec::new();
        let digest = digest_hex(&high);
        let mut order = Ascending::default();
        order.write(&mut out, &digest, 5).unwrap();
        assert!(matches!(
            order.write(&mut out, &digest_hex(&low), 5),
            Err(E::Invalid)
        ));
        assert!(matches!(
            order.write(&mut out, &digest, -1),
            Err(E::Invalid)
        ));
        let first: [u8; INVENTORY_RECORD] = out[..INVENTORY_RECORD].try_into().unwrap();
        assert_eq!(parse_record(&first), (high, 5));
        assert_eq!(record(&high, 5), first);
    }
    #[test]
    fn digests_are_exact_lowercase_hex() {
        let hash: [u8; 32] = std::array::from_fn(|i| (i * 9) as u8);
        assert_eq!(digest_bytes(&digest_hex(&hash)).unwrap(), hash);
        for bad in [
            String::new(),
            "a".repeat(63),
            "A".repeat(64),
            "g".repeat(64),
            "é".repeat(32),
        ] {
            assert!(matches!(digest_bytes(&bad), Err(E::Invalid)));
        }
    }
}
