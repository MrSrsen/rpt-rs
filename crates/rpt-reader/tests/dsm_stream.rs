//! The `DataSourceManager` (saved-data catalog) stream is encrypted with the `Contents` cipher but
//! carries QE-dialect records. Its logical payload is decoded at stream-decode time so inspection
//! tooling (`rpt dump --stream DataSourceManager`) can read its record tree. This verifies the
//! decoded logical bytes are exposed through the normal `streams()` surface and that the QE record
//! tree carries the DSM record types, accumulated across the committed fixture corpus.

#[path = "fixtures.rs"]
mod fixtures;

use rpt_reader::raw::RecordNode;
use rpt_reader::{Rpt, StreamId};

/// The three DSM record types: structure (`0x2d`), field header (`0x41`), batch entry (`0x6d`).
const DSM_STRUCTURE: u16 = 0x2d;
const DSM_FIELD_HEADER: u16 = 0x41;
const DSM_BATCH_ENTRY: u16 = 0x6d;

fn contains_rtype(tree: &[RecordNode], want: u16) -> bool {
    let mut found = false;
    for root in tree {
        root.walk(&mut |n| {
            if n.rtype == want {
                found = true;
            }
        });
    }
    found
}

#[test]
fn data_source_manager_logical_bytes_exposed_via_streams() {
    let mut checked = 0;
    // A DSM with saved batches carries all three record types; a schema-only DSM (no cached rows)
    // has the structure/field-header but no batch entry, so accumulate across the corpus.
    let (mut saw_structure, mut saw_field, mut saw_batch) = (false, false, false);
    for path in fixtures::public_rpts() {
        let Ok(rpt) = Rpt::open(&path) else { continue };
        let Some((_, dsm)) = rpt
            .streams()
            .find(|(id, _)| matches!(id, StreamId::DataSourceManager(_)))
        else {
            continue;
        };
        if dsm.logical_bytes().is_empty() {
            continue;
        }
        checked += 1;

        // The DSM stream is parsed with the QE dialect; its field-header record indexes the schema
        // and is present whenever the stream decodes.
        let tree = dsm.record_tree();
        assert!(
            contains_rtype(&tree, DSM_FIELD_HEADER),
            "DSM field-header record (0x41) present"
        );
        saw_structure |= contains_rtype(&tree, DSM_STRUCTURE);
        saw_field |= contains_rtype(&tree, DSM_FIELD_HEADER);
        saw_batch |= contains_rtype(&tree, DSM_BATCH_ENTRY);
    }

    // The three accumulated assertions below are all satisfiable by an empty sweep, so the number of
    // streams actually walked is asserted first: a corpus that stopped exposing decoded DSM streams
    // has to fail here rather than pass three `saw_*` flags that nothing ever set.
    eprintln!("[dsm] {checked} decoded DataSourceManager stream(s)");
    assert!(
        checked >= 100,
        "only {checked} fixture(s) expose a decoded DataSourceManager stream — the sweep is not covering the corpus"
    );
    // Every DSM record type the saved-data path decodes must be reachable through `streams()`.
    assert!(
        saw_structure,
        "a fixture exposes the DSM structure record (0x2d)"
    );
    assert!(
        saw_field,
        "a fixture exposes the DSM field-header record (0x41)"
    );
    assert!(
        saw_batch,
        "a fixture exposes the DSM batch-entry record (0x6d)"
    );
}
