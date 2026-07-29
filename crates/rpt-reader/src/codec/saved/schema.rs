//! What the saved-data catalog says, and the layout facts derived from it.
//!
//! The catalog itself — the `DataSourceManager` batch directory and the stored-record field
//! catalog — is a tree of records, so it is read where record tables live, one layer up
//! ([`crate::build_model`]). What arrives here is the reading: the batch directory in file order
//! and the stored fields in record-layout order. Everything below is a derivation over that, and
//! everything else in this module turns it into bytes.

/// Batch size (row capacity) of the record-index batch (`SavedRecordsStream`) — a fixed 1000-row cap.
pub(crate) const INDEX_BATCH_SIZE: u32 = 1000;

/// Fixed byte width of one memo-descriptor cell: `[u16 col][u16 flag][u32 heap_offset][u32 byte_length]`.
pub(crate) const MEMO_CELL_SIZE: u32 = 12;

/// The byte budget a memo-descriptor batch fills: its row capacity (the IV's first
/// word) is `DESC_BATCH_BYTE_BUDGET / item_size` (e.g. an item size of 72 gives a row capacity of
/// 142).
pub(crate) const DESC_BATCH_BYTE_BUDGET: u32 = 10224;

/// The widest per-item record a directory entry is taken at its word for. A saved record is one
/// fixed-width row of a database rowset, so an entry claiming more than this is a spurious match on
/// bytes that are not a directory entry rather than a very wide row.
pub(super) const MAX_ITEM_SIZE: u32 = 0x1_0000;

/// The most rows one is taken at its word for, on the same reasoning.
pub(super) const MAX_BATCH_COUNT: u32 = 0x0100_0000;

/// One saved-data batch descriptor from the `DataSourceManager` directory: row `count`, fixed
/// per-item byte width, and the batch's byte span within its physical stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct BatchDesc {
    /// Number of items (records) in the batch.
    pub count: u32,
    /// Fixed per-item byte width.
    pub item_size: u32,
    /// Byte offset of the batch's ciphertext within its physical stream.
    pub stream_off: u32,
    /// Byte length of the batch's ciphertext within its physical stream.
    pub stream_len: u32,
}

/// One batch of the directory: its descriptor, its own column table, and the record's field bytes.
#[derive(Debug, Clone, Default)]
pub(crate) struct SavedBatch {
    /// Count, item size and byte span.
    pub desc: BatchDesc,
    /// The batch's column table, as stored. A **packed** record (string columns stored inline) is
    /// compacted per batch to that batch's own per-column maxima, so the boundaries between its
    /// columns vary per batch and are stored rather than derived.
    pub columns: Vec<u32>,
    /// The batch record's own field bytes, for byte-level inspection.
    pub bytes: Vec<u8>,
}

/// A stored saved-record field descriptor from the `DataSourceManager` catalog.
#[derive(Debug, Clone)]
pub(crate) struct SavedFieldDesc {
    /// Byte offset of the field's slot within the fixed record.
    pub rec_offset: usize,
    /// The stored field name (e.g. `countries_all_iso.id`).
    pub name: String,
    /// Variable-length (memo/string) field — its value lives in `MemoValuesStream`, not inline.
    pub is_memo: bool,
}

/// The saved-data catalog: the in-memory record width, the batch directory in file order, and the
/// stored fields in record-layout order.
#[derive(Debug, Clone, Default)]
pub(crate) struct SavedCatalog {
    /// The in-memory (persistent) record width. This is the record size the batch cipher IV keys
    /// on — for a record whose string columns are stored **inline** it is larger than a batch's
    /// on-disk `item_size` (the packed record width), and equal to it when no columns are packed.
    /// The value is echoed in each decoded batch's own header
    /// (`[type u16][count u32][item_size u32][batch_size u32]`).
    pub item_size: Option<u32>,
    /// Every batch of the directory, in file order: the record-index batches (`item_size` = the
    /// fixed record width), then the memo-descriptor batches (`item_size` = `memo_cols *
    /// MEMO_CELL_SIZE`), then the memo-value batches (`item_size` = 0).
    pub batches: Vec<SavedBatch>,
    /// The stored database fields, in record-layout order.
    pub fields: Vec<SavedFieldDesc>,
}

impl SavedCatalog {
    /// The full batch directory (unguarded): every entry in file order.
    pub(crate) fn descriptors(&self) -> Vec<BatchDesc> {
        self.batches.iter().map(|b| b.desc).collect()
    }

    /// The batch directory, guarded: every entry whose `item_size` is a plausible positive width
    /// (dropping spurious matches). Returns the entries in file order.
    pub(crate) fn guarded_directory(&self) -> Vec<BatchDesc> {
        self.descriptors()
            .into_iter()
            .filter(|b| b.item_size > 0 && b.item_size < MAX_ITEM_SIZE && b.count < MAX_BATCH_COUNT)
            .collect()
    }

    /// How many memo (variable-length) columns the stored record has.
    pub(crate) fn memo_columns(&self) -> u32 {
        self.fields.iter().filter(|f| f.is_memo).count() as u32
    }

    /// The record-index batches: the `SavedRecordsStream` entries up to the first memo-descriptor
    /// batch. Within that stream the index batches (the fixed-width record index) come first, then
    /// the memo-descriptor batches (`item_size` = `memo_col_count * 12`, present only when the
    /// record has memo columns). A large saved rowset splits the index across several batches, so
    /// the record count is the **sum** of these, not the max.
    pub(crate) fn index_directory(&self) -> Vec<BatchDesc> {
        let desc_item = self.memo_columns() * MEMO_CELL_SIZE;
        srs_directory(&self.guarded_directory())
            .into_iter()
            .take_while(|b| desc_item == 0 || b.item_size != desc_item)
            .collect()
    }

    /// The report's saved record count — the total across all record-index batches. `None` when
    /// there is no saved data.
    pub(crate) fn record_count(&self) -> Option<u32> {
        let total: u32 = self.index_directory().iter().map(|b| b.count).sum();
        (total > 0).then_some(total)
    }
}

/// The widths a batch directory is read with: which class each entry belongs to, and which record
/// width its cipher IV keys on. Both the row decode and the byte-level inspection read the same
/// directory, so they resolve it once here rather than each from the raw fields.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BatchShape {
    /// On-disk item width of a record-index batch — the directory's first entry. Zero when the
    /// directory opens with a memo-value heap, the one batch class stored with no per-item width.
    pub index_item: u32,
    /// How many memo (variable-length) columns the stored record has.
    pub memo_cols: u32,
    /// Item width of a memo-descriptor batch: one [`MEMO_CELL_SIZE`]-byte cell per memo column.
    /// Zero for a record with no memo column, which has no descriptor batch.
    pub desc_item: u32,
    /// The in-memory (persistent) record width.
    pub persistent: u32,
    /// The item-size word a record-index batch's IV is built from: the persistent width for a
    /// packed record, the on-disk width otherwise. The two differ only when string columns are
    /// stored inline, so this is the on-disk width for every memo-heap report.
    pub index_iv_item: u32,
}

impl BatchShape {
    /// Resolve the shape of a catalog's batch directory.
    pub(crate) fn of(catalog: &SavedCatalog) -> BatchShape {
        let index_item = catalog
            .batches
            .first()
            .map(|b| b.desc.item_size)
            .unwrap_or(0);
        let memo_cols = catalog.memo_columns();
        let persistent = catalog.item_size.unwrap_or(index_item);
        BatchShape {
            index_item,
            memo_cols,
            desc_item: memo_cols * MEMO_CELL_SIZE,
            persistent,
            index_iv_item: if memo_cols == 0 {
                persistent
            } else {
                index_item
            },
        }
    }

    /// Whether the record is **packed**: a memo-less record stores its string columns inline,
    /// compacted per batch to that batch's own per-column maxima, so the on-disk record is narrower
    /// than the in-memory one.
    pub(crate) fn is_packed(&self) -> bool {
        self.memo_cols == 0 && self.persistent > self.index_item
    }
}

/// The directory entries physically stored in `SavedRecordsStream`: the leading run whose byte spans
/// **chain contiguously from offset 0**.
///
/// The directory describes two physical streams back to back — `SavedRecordsStream` (the record-index
/// batches, then the memo-descriptor batches) and `MemoValuesStream` (the memo-value heaps) — and
/// each stream's entries are laid out end to end from its own offset 0. So the first entry whose
/// `stream_off` does not continue the running cursor is the first entry of the *next* stream, and
/// the run before it is exactly the `SavedRecordsStream` set. The chain is self-validating: its
/// spans sum to the raw stream length.
///
/// This is the only reliable delimiter. Neither "shares the first entry's `item_size`" nor any
/// width-based rule works, because a **packed** record (string columns stored inline) is compacted
/// per batch to that batch's own per-column maxima — consecutive index batches of one report carry
/// different on-disk widths, and a later batch's width can coincide with another batch class's.
pub(crate) fn srs_directory(directory: &[BatchDesc]) -> Vec<BatchDesc> {
    let mut cursor = 0u32;
    directory
        .iter()
        .copied()
        .take_while(|b| {
            // A zero-length span would satisfy the chain test without consuming anything, so it
            // could not end the run; require real forward progress.
            let chains = b.stream_off == cursor && b.stream_len > 0;
            cursor = cursor.saturating_add(b.stream_len);
            chains
        })
        .collect()
}
