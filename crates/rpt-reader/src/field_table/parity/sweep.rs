//! The corpus sweep: read every record through its table and account for every byte.
//!
//! [`sweep_record`] is the whole of it for one record — the reading must consume the record
//! exactly and re-emit it byte for byte. The tests after it widen that to the corpus and add the
//! structural cross-checks a single record cannot make: a table's reading against the model the
//! decoder builds from it, and against the records a counted field counts.

use super::*;

/// One record: the table's reading must account for it exactly and re-emit it byte for byte.
fn sweep_record(table: &Table, content: &RecordContent, file: &str, stats: &mut Stats) {
    let reading = read_strings(table, content, StringFormat::Enhanced);
    stats.records += 1;
    if reading.exact() {
        stats.exact += 1;
    } else if stats.inexact_files.len() < 12 {
        stats.inexact_files.push(format!(
            "{file}: unread={} undeclared_children={} stop={:?}",
            reading.unread, reading.undeclared_children, reading.stop
        ));
    }
    if !reading.complete {
        stats.incomplete += 1;
    }
    // The write direction: re-emitting the row must reproduce the record byte for byte — including
    // a record that ended before its last declared field, which re-emits just as short. It is
    // re-emitted as a record of its own version, since a version can decide a field's width.
    if write_as(table, &reading.row, content.schema, StringFormat::Enhanced) == content.pieces {
        stats.roundtrip_ok += 1;
    }
}

/// Every table accounts for every corpus record of its type exactly, and re-emits it byte for byte.
///
/// This is what a table breaks against: a field that changes width, moves, or stops being declared
/// leaves bytes unread or re-emits a record that is no longer the one the file holds.
///
/// The sweep is only worth what it covered, and the failures below are derived from what the walk
/// found: a sweep that read nothing would report nothing. So the coverage itself is asserted —
/// every table is required by name to have met a record, and the total read has a floor.
#[test]
fn every_table_accounts_for_every_corpus_record_exactly() {
    // Keyed by (dialect, record type): the same number is a different record in each dialect, so
    // they are counted apart.
    let mut per_type: BTreeMap<(Dialect, u16), Stats> = BTreeMap::new();
    corpus::for_each_record(|dialect, node, logical, path| {
        let Some(table) = tables::for_record(node.rtype, node.schema, dialect) else {
            return;
        };
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        sweep_record(
            table,
            &content_of(node, logical),
            &file,
            per_type.entry((dialect, node.rtype)).or_default(),
        );
    });

    let mut summary = String::new();
    let mut failures = Vec::new();
    for (dialect, table) in registry() {
        if !per_type.contains_key(&(dialect, table.rtype))
            && !UNREACHED.contains(&(dialect, table.rtype))
        {
            failures.push(format!(
                "0x{:04x} {}: the sweep found no record of this type",
                table.rtype, table.name
            ));
        }
    }
    let swept: usize = per_type.values().map(|s| s.records).sum();
    if swept < SWEPT_FLOOR {
        failures.push(format!(
            "the sweep read {swept} records, below the {SWEPT_FLOOR} the committed fixtures alone reach"
        ));
    }
    for ((dialect, rtype), s) in &per_type {
        let schema = tables::tabled_schema(*rtype, *dialect).unwrap_or_default();
        let table =
            tables::for_record(*rtype, schema, *dialect).expect("stats keyed by a tabled rtype");
        summary.push_str(&format!(
            "\n0x{rtype:04x} {:<22} records={} exact={} incomplete={} roundtrip={}\n",
            table.name, s.records, s.exact, s.incomplete, s.roundtrip_ok
        ));
        for f in &s.inexact_files {
            summary.push_str(&format!("    INEXACT {f}\n"));
        }
        if s.exact != s.records {
            failures.push(format!(
                "0x{rtype:04x}: {} of {} records not accounted for exactly",
                s.records - s.exact,
                s.records
            ));
        }
        if s.roundtrip_ok != s.records {
            failures.push(format!(
                "0x{rtype:04x}: {} of {} records did not re-emit byte for byte",
                s.records - s.roundtrip_ok,
                s.records
            ));
        }
    }

    if !failures.is_empty() || std::env::var_os("RPT_FIELD_TABLE_REPORT").is_some() {
        eprintln!("field-table sweep{summary}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every table is registered under the dialect it declares itself to be in.
///
/// The two must agree because they answer different questions from the same fact: the registry
/// routes a record of a stream to its table, and the table's own dialect is what the version
/// ceiling is looked up by. A table filed under the wrong set would be reached for one vocabulary
/// and gated as another, and both readings would look plausible.
#[test]
fn every_table_is_registered_under_the_dialect_it_declares() {
    for (dialect, table) in registry() {
        assert_eq!(
            table.dialect, dialect,
            "0x{:04x} {} is registered under {dialect:?}",
            table.rtype, table.name
        );
    }
}

/// The table's reading of the field that lives past a nested child record agrees with the model
/// the decoder builds — so a field whose position depends on a child is checked against something
/// outside the harness, not only against its own declaration.
#[test]
fn group_area_format_matches_the_decoded_model() {
    let mut checked = 0usize;
    corpus::for_each_stream(|rpt, dialect, stream, path| {
        if dialect != Dialect::Contents {
            return;
        }
        let logical = stream.logical_bytes();
        let mut from_table: Vec<i32> = Vec::new();
        for root in stream.record_tree() {
            root.walk(&mut |node| {
                if node.rtype != 0x0088 {
                    return;
                }
                let r = read_strings(
                    &tables::GROUP_AREA_FORMAT,
                    &content_of(node, logical),
                    StringFormat::Enhanced,
                );
                from_table.push(r.row.i("visible_groups_per_page"));
            });
        }
        if from_table.is_empty() {
            return;
        }
        let mut from_model: Vec<i32> = rpt
            .report()
            .report_definition
            .areas
            .iter()
            .filter_map(|a| a.format.group.as_ref())
            .map(|g| g.visible_groups_per_page)
            .collect();
        from_model.sort_unstable();
        from_table.sort_unstable();
        // The model keeps one group-area format per group area; the records include every
        // `0x0088` in the stream. Compare the values the model kept as a sub-multiset.
        for v in &from_model {
            assert!(
                from_table.contains(v),
                "{}: model VisibleGroupNumberPerPage {v} not among the table's {from_table:?}",
                path.display()
            );
        }
        checked += 1;
    });
    assert!(checked > 0, "no fixture exercised the group-area format");
}

/// The text opener's two counted fields agree with the records they count.
///
/// Both are read past the opener's nested `ObjectName`, so their position in the sequence is settled
/// by the child rather than by an offset — and both are checked against a second, independent
/// reading of the same stream: the `0x00c0` paragraphs a text object holds, and the `0x0166` records
/// that name the field object a heading heads. A field read one step out of place would have to
/// coincide with a record count in every stream of every report to pass.
#[test]
fn the_text_openers_counts_agree_with_the_records_they_count() {
    let mut streams = 0usize;
    let mut openers = 0usize;
    corpus::for_each_stream(|_, dialect, stream, path| {
        if dialect != Dialect::Contents {
            return;
        }
        let logical = stream.logical_bytes();
        let (mut paragraphs, mut headings) = (0u32, 0usize);
        let (mut paragraph_records, mut heading_records) = (0u32, 0usize);
        for root in stream.record_tree() {
            root.walk(&mut |node| match node.rtype {
                0x00a5 => {
                    let r = read_strings(
                        &tables::TEXT_OBJECT_CONTAINER,
                        &content_of(node, logical),
                        StringFormat::Enhanced,
                    );
                    paragraphs += r.row.u("paragraph_count");
                    headings += usize::from(r.row.i("is_field_heading") != 0);
                    openers += 1;
                }
                0x00c0 => paragraph_records += 1,
                0x0166 => heading_records += 1,
                _ => {}
            });
        }
        if paragraph_records == 0 && heading_records == 0 && paragraphs == 0 {
            return;
        }
        assert_eq!(
            paragraphs,
            paragraph_records,
            "{}: the openers claim {paragraphs} paragraphs, the stream holds {paragraph_records}",
            path.display()
        );
        assert_eq!(
            headings,
            heading_records,
            "{}: {headings} openers flag a heading, the stream holds {heading_records} heading links",
            path.display()
        );
        streams += 1;
    });
    assert!(
        streams > 0 && openers > 400,
        "{openers} text openers over {streams} streams"
    );
}

/// A field name is how both directions address a value, and [`crate::field_table::table::Row::get`] answers with
/// the **last** entry of that name — so two fields sharing one within the same row is not a
/// cosmetic clash: the writer hands the second field's value to the first field's kind and stops.
/// A repeat body builds its own row, so names may repeat across scopes but never within one.
#[test]
fn every_field_name_in_a_row_is_distinct() {
    fn check(name: &str, fields: &'static [Field]) {
        let mut seen: Vec<&str> = fields.iter().map(|f| f.name).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "{name} repeats a field name");
        for f in fields {
            if let Kind::Repeat { body, .. } = f.kind {
                check(&format!("{name}.{}", f.name), body);
            }
        }
    }
    for (_, table) in registry() {
        check(
            &format!("0x{:04x} {}", table.rtype, table.name),
            table.fields,
        );
    }
}

/// A saved-data descriptor's stream id names a stream the compound file really holds.
///
/// The container writes each of its streams as `<name> <id>l`, so an id is checkable against the
/// directory rather than only against itself: the descriptor's (`0x0061`) names the analysis-grids
/// stream and the report root's saved-data handle names the data-source manager's, and a field read
/// at the wrong offset or width would name neither. The two records agree as well — a root that
/// states it was saved with data is in a stream with a descriptor, and one that does not is not.
#[test]
fn a_saved_data_stream_id_names_a_stream_the_file_holds() {
    use crate::StreamId;
    let mut checked = 0usize;
    let mut wrong = Vec::new();
    corpus::for_each_stream(|rpt, dialect, stream, path| {
        if dialect != Dialect::Contents {
            return;
        }
        let logical = stream.logical_bytes();
        let (mut grids, mut managers) = (Vec::new(), Vec::new());
        for root in stream.record_tree() {
            root.walk(&mut |node| {
                let content = content_of(node, logical);
                if node.rtype == tables::SAVED_DATA.rtype {
                    let row =
                        read_strings(&tables::SAVED_DATA, &content, StringFormat::Enhanced).row;
                    grids.push(row.u("stream_id"));
                } else if node.rtype == tables::REPORT_ROOT.rtype {
                    let row =
                        read_strings(&tables::REPORT_ROOT, &content, StringFormat::Enhanced).row;
                    if let Some(id) = row.get("saved_data_handle").and_then(Cell::u) {
                        managers.push(id);
                    }
                }
            });
        }
        if grids.len() != managers.len() {
            wrong.push(format!(
                "{}: {} saved-data descriptor(s) against {} root(s) stating saved data",
                path.display(),
                grids.len(),
                managers.len()
            ));
        }
        if grids.is_empty() && managers.is_empty() {
            return;
        }
        // The container keeps a subreport's streams under its own storage, so compare leaf names.
        let leaves: Vec<String> = rpt
            .streams()
            .filter_map(|(id, _)| match id {
                StreamId::Other(p) | StreamId::DataSourceManager(p) => {
                    Some(p.rsplit('/').next().unwrap_or(p).to_owned())
                }
                _ => None,
            })
            .collect();
        let named = grids
            .iter()
            .map(|id| ("AnalysisGridsStream", id))
            .chain(managers.iter().map(|id| ("DataSourceManager", id)));
        for (prefix, id) in named {
            checked += 1;
            let want = format!("{prefix} {id}l");
            if !leaves.contains(&want) {
                wrong.push(format!("{}: no stream named `{want}`", path.display()));
            }
        }
    });
    assert!(checked > 200, "only {checked} stream ids were checked");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
