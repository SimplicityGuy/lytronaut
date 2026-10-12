//! Bounded logical catalog wire format. Identifiers and SQL are never archive input.
//! Opaque imported record bytes contain their own lossless source typed values.
use crate::StorageError as E;

const MAGIC: &[u8; 8] = b"LYTROR02";
// These are the explicit NEW version-2 contract, not an asserted expansion bound
// or acceptance filter for legacy SQLite archives. Legacy extraction is separate.
pub const MAX_BYTES: usize = 288 * 1024 * 1024;
pub const MAX_ROWS: usize = 32 * 1024 * 1024;
const MAX_CELLS: usize = 32 * 1024 * 1024;
const MAX_CELL: usize = 32 * 1024 * 1024;
const MAX_ROW_BYTES: usize = MAX_CELL + 256;

/// Debug deliberately omits source values.
#[derive(Clone, PartialEq, Eq)]
pub enum Cell {
    Null,
    Integer(i64),
    Text(String),
    Blob(Vec<u8>),
}
impl std::fmt::Debug for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cell(<source value redacted>)")
    }
}
#[derive(Clone, Copy)]
pub enum Kind {
    Integer,
    Text,
    SourceKey,
    SourceText,
    Blob,
}
#[derive(Clone, Copy)]
pub struct Column {
    pub name: &'static str,
    pub kind: Kind,
    pub nullable: bool,
}
pub struct Table {
    pub name: &'static str,
    pub columns: &'static [Column],
}
const fn col(name: &'static str, kind: Kind) -> Column {
    Column {
        name,
        kind,
        nullable: false,
    }
}
const fn optional(name: &'static str, kind: Kind) -> Column {
    Column {
        name,
        kind,
        nullable: true,
    }
}
use Kind::{Blob, Integer, SourceKey, SourceText, Text};

// Dependency order is part of version 2. No generated display/search columns,
// authentication tables, transient export state or inbound DDL appear here.
pub const TABLES: &[Table] = &[
    Table {
        name: "intake_jobs",
        columns: &[
            col("id", Text),
            col("kind", Text),
            col("state", Text),
            col("total", Integer),
            col("budget", Integer),
            col("max_files", Integer),
            col("encoded_limit", Integer),
            col("zip_complete", Integer),
            col("completed", Integer),
            col("files", Integer),
            col("path_bytes", Integer),
            col("reserved", Integer),
            optional("error", Text),
        ],
    },
    Table {
        name: "intake_sources",
        columns: &[
            col("job", Text),
            col("path", Text),
            col("canonical", Text),
            col("digest", Text),
            col("length", Integer),
        ],
    },
    Table {
        name: "catalog_versions",
        columns: &[
            col("id", Integer),
            col("fingerprint", Text),
            col("source_job", Text),
            optional("library_uuid", SourceKey),
            col("state", Text),
            optional("reason", Text),
            col("conflict", Integer),
        ],
    },
    Table {
        name: "catalog_sources",
        columns: &[
            col("version", Integer),
            col("job", Text),
            col("path", Text),
            col("digest", Text),
            col("length", Integer),
        ],
    },
    Table {
        name: "catalog_records",
        columns: &[
            col("version", Integer),
            col("kind", Text),
            col("ordinal", Integer),
            col("original", Blob),
        ],
    },
    Table {
        name: "catalog_pictures",
        columns: &[
            col("version", Integer),
            col("uuid", SourceKey),
            col("bin_uuid", SourceKey),
            col("event_uuid", SourceKey),
            col("name", SourceText),
            col("caption", SourceText),
            col("rating", Integer),
            col("flag_status", Integer),
        ],
    },
    Table {
        name: "catalog_albums",
        columns: &[
            col("version", Integer),
            col("uuid", SourceKey),
            col("name", SourceText),
            col("sort_order", Integer),
        ],
    },
    Table {
        name: "catalog_memberships",
        columns: &[
            col("version", Integer),
            col("album_uuid", SourceKey),
            col("picture_uuid", SourceKey),
            col("picture_index", Integer),
        ],
    },
    Table {
        name: "catalog_item_errors",
        columns: &[
            col("version", Integer),
            col("picture_uuid", SourceKey),
            col("code", Text),
        ],
    },
    Table {
        name: "api_ids",
        columns: &[
            col("id", Text),
            col("kind", Text),
            col("version", Integer),
            col("source_key", SourceKey),
        ],
    },
    Table {
        name: "api_imports",
        columns: &[col("job", Text), col("version", Integer)],
    },
    Table {
        name: "api_adaptation_errors",
        columns: &[col("job", Text), col("code", Text)],
    },
    Table {
        name: "application_job_order",
        columns: &[col("ordinal", Integer), col("kind", Text), col("job", Text)],
    },
];

/// A sequential encoder; callers stream rows from a consistent database cursor.
/// No whole-catalog vector or second encoded copy is retained.
pub struct Encoder<W> {
    output: W,
    written: usize,
    table: usize,
    pending: usize,
    rows: usize,
    cells: usize,
}
fn compatible(cell: &Cell, column: Column) -> bool {
    match (cell, column.kind) {
        (Cell::Null, _) => column.nullable,
        (Cell::Integer(_), Integer) | (Cell::Blob(_), Blob) => true,
        (Cell::Text(_), Text | SourceKey | SourceText) => true,
        _ => false,
    }
}
fn counts(rows: &mut usize, cells: &mut usize, count: usize, width: usize) -> Result<(), E> {
    *rows = rows.checked_add(count).ok_or(E::Quota)?;
    *cells = cells
        .checked_add(count.checked_mul(width).ok_or(E::Quota)?)
        .ok_or(E::Quota)?;
    if *rows > MAX_ROWS || *cells > MAX_CELLS {
        return Err(E::Quota);
    }
    Ok(())
}
fn row_size(row: &[Cell]) -> Result<usize, E> {
    let mut size = 0usize;
    for cell in row {
        let bytes = match cell {
            Cell::Null => 1,
            Cell::Integer(_) => 9,
            Cell::Text(v) => v.len().checked_add(5).ok_or(E::Quota)?,
            Cell::Blob(v) => v.len().checked_add(5).ok_or(E::Quota)?,
        };
        size = size.checked_add(bytes).ok_or(E::Quota)?;
    }
    if size > MAX_ROW_BYTES {
        return Err(E::Quota);
    }
    Ok(size)
}
impl<W: std::io::Write> Encoder<W> {
    pub fn new(output: W) -> Result<Self, E> {
        let mut value = Self {
            output,
            written: 0,
            table: 0,
            pending: 0,
            rows: 0,
            cells: 0,
        };
        value.append(MAGIC)?;
        Ok(value)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), E> {
        let size = self.written.checked_add(bytes.len()).ok_or(E::Quota)?;
        if size > MAX_BYTES {
            return Err(E::Quota);
        }
        self.output.write_all(bytes)?;
        self.written = size;
        Ok(())
    }
    pub fn table(&mut self, count: usize) -> Result<(), E> {
        if self.pending != 0 || self.table >= TABLES.len() {
            return Err(E::Invalid);
        }
        counts(
            &mut self.rows,
            &mut self.cells,
            count,
            TABLES[self.table].columns.len(),
        )?;
        self.append(&[self.table as u8])?;
        self.append(&(u32::try_from(count).map_err(|_| E::Quota)?).to_le_bytes())?;
        self.table += 1;
        self.pending = count;
        Ok(())
    }
    pub fn row(&mut self, row: &[Cell]) -> Result<(), E> {
        if self.pending == 0 || self.table == 0 {
            return Err(E::Invalid);
        }
        let columns = TABLES[self.table - 1].columns;
        if row.len() != columns.len() || !row.iter().zip(columns).all(|(v, &c)| compatible(v, c)) {
            return Err(E::Invalid);
        }
        row_size(row)?;
        for value in row {
            match value {
                Cell::Null => self.append(&[0])?,
                Cell::Integer(value) => {
                    self.append(&[1])?;
                    self.append(&value.to_le_bytes())?;
                }
                Cell::Text(value) => self.bytes(2, value.as_bytes())?,
                Cell::Blob(value) => self.bytes(3, value)?,
            }
        }
        self.pending -= 1;
        Ok(())
    }
    fn bytes(&mut self, tag: u8, bytes: &[u8]) -> Result<(), E> {
        if bytes.len() > MAX_CELL {
            return Err(E::Quota);
        }
        self.append(&[tag])?;
        self.append(&(bytes.len() as u32).to_le_bytes())?;
        self.append(bytes)
    }
    pub fn finish(self) -> Result<W, E> {
        if self.table != TABLES.len() || self.pending != 0 {
            return Err(E::Invalid);
        }
        Ok(self.output)
    }
}
/// Consumes precisely a length-delimited catalog section, keeping only one row.
/// It never allocates based on row count or retains a copy of the entire section.
/// Decoded cell payloads total at most MAX_ROW_BYTES; SQLx driver rows and
/// destination projections are additional memory owned by the database adapter.
pub struct Decoder<R> {
    input: R,
    remaining: usize,
    table: usize,
    pending: usize,
    rows: usize,
    cells: usize,
}
impl<R: std::io::Read> Decoder<R> {
    pub fn new(input: R, length: usize) -> Result<Self, E> {
        if length > MAX_BYTES {
            return Err(E::Quota);
        }
        let mut value = Self {
            input,
            remaining: length,
            table: 0,
            pending: 0,
            rows: 0,
            cells: 0,
        };
        let mut magic = [0; 8];
        value.read(&mut magic)?;
        if &magic != MAGIC {
            return Err(E::Invalid);
        }
        Ok(value)
    }
    fn read(&mut self, bytes: &mut [u8]) -> Result<(), E> {
        if bytes.len() > self.remaining {
            return Err(E::Invalid);
        }
        self.input.read_exact(bytes)?;
        self.remaining -= bytes.len();
        Ok(())
    }
    fn byte(&mut self) -> Result<u8, E> {
        let mut b = [0];
        self.read(&mut b)?;
        Ok(b[0])
    }
    fn number(&mut self) -> Result<u32, E> {
        let mut b = [0; 4];
        self.read(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    pub fn table(&mut self) -> Result<Option<(usize, usize)>, E> {
        if self.pending != 0 {
            return Err(E::Invalid);
        }
        if self.table == TABLES.len() {
            if self.remaining != 0 {
                return Err(E::Invalid);
            }
            return Ok(None);
        }
        let id = self.table;
        if self.byte()? != id as u8 {
            return Err(E::Invalid);
        }
        let count = self.number()? as usize;
        let width = TABLES[id].columns.len();
        counts(&mut self.rows, &mut self.cells, count, width)?;
        if count.checked_mul(width).is_none_or(|n| n > self.remaining) {
            return Err(E::Invalid);
        }
        self.table += 1;
        self.pending = count;
        Ok(Some((id, count)))
    }
    pub fn row(&mut self) -> Result<Vec<Cell>, E> {
        if self.pending == 0 || self.table == 0 {
            return Err(E::Invalid);
        }
        let columns = TABLES[self.table - 1].columns;
        let mut values = Vec::with_capacity(columns.len());
        let mut size = 0usize;
        for &column in columns {
            let cell = match self.byte()? {
                0 => {
                    size = size.checked_add(1).ok_or(E::Quota)?;
                    Cell::Null
                }
                1 => {
                    size = size.checked_add(9).ok_or(E::Quota)?;
                    let mut bytes = [0; 8];
                    self.read(&mut bytes)?;
                    Cell::Integer(i64::from_le_bytes(bytes))
                }
                tag @ (2 | 3) => {
                    let length = self.number()? as usize;
                    size = size
                        .checked_add(length.checked_add(5).ok_or(E::Quota)?)
                        .ok_or(E::Quota)?;
                    if length > MAX_CELL || size > MAX_ROW_BYTES {
                        return Err(E::Quota);
                    }
                    if length > self.remaining {
                        return Err(E::Invalid);
                    }
                    let mut bytes = vec![0; length];
                    self.read(&mut bytes)?;
                    if tag == 2 {
                        Cell::Text(String::from_utf8(bytes).map_err(|_| E::Invalid)?)
                    } else {
                        Cell::Blob(bytes)
                    }
                }
                _ => return Err(E::Invalid),
            };
            if !compatible(&cell, column) {
                return Err(E::Invalid);
            }
            values.push(cell);
        }
        if size > MAX_ROW_BYTES {
            return Err(E::Quota);
        }
        self.pending -= 1;
        Ok(values)
    }
    pub fn finish(self) -> Result<R, E> {
        if self.table != TABLES.len() || self.pending != 0 || self.remaining != 0 {
            return Err(E::Invalid);
        }
        Ok(self.input)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn encode(rows: &[Vec<Vec<Cell>>]) -> Result<Vec<u8>, E> {
        let mut writer = Encoder::new(Vec::new())?;
        for table in rows {
            writer.table(table.len())?;
            for row in table {
                writer.row(row)?;
            }
        }
        writer.finish()
    }
    fn decode(bytes: &[u8]) -> Result<Vec<Vec<Vec<Cell>>>, E> {
        let mut reader = Decoder::new(bytes, bytes.len())?;
        let mut tables = Vec::new();
        while let Some((_, count)) = reader.table()? {
            let mut rows = Vec::new();
            for _ in 0..count {
                rows.push(reader.row()?);
            }
            tables.push(rows);
        }
        reader.finish()?;
        Ok(tables)
    }
    #[test]
    fn signed64_nul_unicode_and_large_opaque_records_roundtrip() {
        let mut tables = vec![Vec::new(); TABLES.len()];
        for id in [i64::MIN, -1, 0, i64::MAX] {
            tables[4].push(vec![
                Cell::Integer(id),
                Cell::Text("unknown".into()),
                Cell::Integer(id),
                Cell::Blob(vec![0, 255, 128, 1]),
            ]);
        }
        tables[6].push(vec![
            Cell::Integer(0),
            Cell::Text("a\0雪".into()),
            Cell::Text("雪\0🌌".into()),
            Cell::Integer(i64::MIN),
        ]);
        let bytes = encode(&tables).unwrap();
        assert_eq!(decode(&bytes).unwrap(), tables);
        assert!(!format!("{tables:?}").contains('雪'));
        for length in 0..bytes.len() {
            assert!(decode(&bytes[..length]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        tables[4][0][3] = Cell::Blob(vec![42; 1024 * 1024 + 1]);
        assert_eq!(decode(&encode(&tables).unwrap()).unwrap(), tables);
    }
    #[test]
    fn rejects_unknown_tables_impossible_counts_wrong_types_and_missing_rows() {
        let empty = vec![Vec::new(); TABLES.len()];
        let bytes = encode(&empty).unwrap();
        let mut unknown = bytes.clone();
        unknown[8] = 99;
        assert!(decode(&unknown).is_err());
        let mut huge = bytes.clone();
        huge[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&huge), Err(E::Quota)));
        let mut wrong = empty;
        wrong[11].push(vec![Cell::Integer(0), Cell::Null]);
        assert!(matches!(encode(&wrong), Err(E::Invalid)));
        let mut writer = Encoder::new(Vec::new()).unwrap();
        writer.table(1).unwrap();
        assert!(writer.table(0).is_err());
        assert!(writer.finish().is_err());
        assert!(Decoder::new(&bytes[..], MAX_BYTES + 1).is_err());
    }
}
