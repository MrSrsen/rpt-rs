//! Negative controls: what each declaration is load-bearing for.
//!
//! A comparison that cannot fail proves nothing. Each control perturbs one declaration — pinning a
//! width, fixing a run, dropping a gate — and shows which records move, so the agreement the sweep
//! reports is a measurement rather than a tautology. Several also build synthetic records for a
//! form no corpus report writes.

use super::*;

/// Walk the whole corpus with `table` in `dialect`, returning (records, exactly-accounted, and the
/// number whose record satisfies `pick`).
fn sweep(
    table: &Table,
    dialect: Dialect,
    pick: fn(schema: u16, &crate::field_table::table::Row) -> bool,
) -> (usize, usize, usize) {
    let (mut n, mut exact, mut picked) = (0, 0, 0);
    corpus::for_each_record_of(dialect, table.rtype, |node, logical, _| {
        let r = read_strings(table, &content_of(node, logical), StringFormat::Enhanced);
        n += 1;
        if r.exact() {
            exact += 1;
        }
        if pick(node.schema, &r.row) {
            picked += 1;
        }
    });
    (n, exact, picked)
}

/// A record's schema selects a field's **width**, and the corpus contains both widths — so one
/// fixed width cannot account for all of them.
#[test]
fn a_schema_widened_field_is_read_at_the_width_its_record_declares() {
    let (n, exact, narrow) = sweep(
        &tables::SAVED_FIELD_HEADER,
        Dialect::Catalog,
        |schema, _| schema < 0x0702,
    );
    assert!(
        n > 1000,
        "expected the corpus to hold many of these, got {n}"
    );
    assert!(
        narrow > 100 && n - narrow > 100,
        "both schema forms must be represented: {narrow} narrow, {} wide",
        n - narrow
    );
    assert_eq!(
        exact,
        n,
        "{} of {n} records were not accounted for",
        n - exact
    );

    // The control: pin the offset to the narrow width and every wide record breaks, and only those.
    const PINNED: Table = Table {
        fields: &[
            Field::new("entry", Kind::Child(0x0040)),
            Field::new("_u0", Kind::U16Be),
            Field::new("offset", Kind::U16Be),
            Field::new("_u1", Kind::I16Be),
        ],
        ..tables::SAVED_FIELD_HEADER
    };
    let (n2, exact2, _) = sweep(&PINNED, Dialect::Catalog, |schema, _| schema < 0x0702);
    assert_eq!(n2, n);
    assert_eq!(
        exact2, narrow,
        "pinning the width should leave exactly the narrow records accounted for"
    );
}

/// A format record's wrapper is a run of **field references**, one per property of the record it
/// wraps — not a fixed run of bytes.
///
/// Every slot is eight bytes while it names no formula, so a wrapper whose slots are all empty is
/// indistinguishable from a fixed skip; a wrapper with a bound slot is longer, and every slot after
/// it moves. The corpus has both, so the two readings are adjudicated against each other here: the
/// table's occupied slots must be exactly the ones the byte scan finds, at the same formula
/// indices, over every wrapper in every report.
///
/// The scan keeps only the reserved condition names, so the comparison is over those.
#[test]
fn a_format_wrapper_is_a_run_of_field_references() {
    /// The second reading: find a slot by scanning the wrapper's field bytes for a length-prefixed
    /// `@`-name, rather than by position.
    fn pos_condition_refs(bytes: &[u8]) -> Vec<(String, usize)> {
        let mut refs = Vec::new();
        let mut i = 0;
        while i + 4 <= bytes.len() {
            if let Some((s, consumed)) = crate::bytes::read_lp_string(&bytes[i..]) {
                if let Some(name) = s.strip_prefix('@') {
                    if crate::build_model::is_modeled_condition(name) {
                        let lo = bytes.get(i + consumed + 2).copied().unwrap_or(0);
                        let hi = bytes.get(i + consumed + 3).copied().unwrap_or(0);
                        refs.push((name.to_string(), usize::from(u16::from_le_bytes([lo, hi]))));
                        i += consumed;
                        continue;
                    }
                }
            }
            i += 1;
        }
        refs
    }

    let wrappers: &[&Table] = &[
        &tables::BORDER_WRAPPER,
        &tables::OBJECT_FORMAT_WRAPPER,
        &tables::SECTION_FORMAT_WRAPPER,
        &tables::FONT_CONDITION_FORMAT,
    ];
    let (mut records, mut bound) = (0usize, 0usize);
    corpus::for_each_record(|dialect, node, logical, path| {
        if dialect != Dialect::Contents {
            return;
        }
        {
            {
                {
                    let Some(table) = wrappers.iter().find(|t| t.rtype == node.rtype) else {
                        return;
                    };
                    let r = read_strings(table, &content_of(node, logical), StringFormat::Enhanced);
                    let from_table = crate::build_model::condition_slots(&r.row);
                    let from_scan = pos_condition_refs(&node.joined_runs(logical));
                    assert_eq!(
                        from_table,
                        from_scan,
                        "0x{:04x} in {}",
                        node.rtype,
                        path.display()
                    );
                    records += 1;
                    bound += from_table.len();
                }
            }
        }
    });
    assert!(records > 5_000, "the sweep saw {records} wrappers");
    // A bound slot is rare — a conditional format is an authored exception — so this is what the
    // whole comparison rests on: without one, every wrapper is a run of identical empty slots and
    // a fixed skip would agree too.
    assert!(
        bound >= 20,
        "only {bound} slots name a formula, too few to separate the two readings"
    );
}

/// An `XmlDefinition` states its absence by carrying no content at all.
///
/// Every field of the record is behind that one gate, so the empty form is not a truncated record —
/// it is the definition being absent. The populated form is exercised here on bytes of this test's
/// own making: its opening names the definition and counts the XSLT definitions that follow, and a
/// reader that took those four fields as unconditional would report every empty record as truncated
/// instead.
#[test]
fn an_xml_definition_is_absent_rather_than_truncated() {
    let empty = RecordContent {
        rtype: 0x0151,
        schema: 0x0700,
        pieces: Vec::new(),
    };
    let r = read_strings(&tables::XML_DEFINITION, &empty, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert!(r.row.get("name").is_none());
    assert_eq!(
        write_as(
            &tables::XML_DEFINITION,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        empty.pieces
    );

    // The populated opening: a name, the enum, the short, and the count of what follows.
    let populated = RecordContent {
        rtype: 0x0151,
        schema: 0x0700,
        pieces: vec![crate::field_table::cursor::Piece::Run(
            b"\x00\x00\x00\x04abc\x00\x01\x00\x07\x00\x02".to_vec(),
        )],
    };
    let r = read_strings(&tables::XML_DEFINITION, &populated, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("name"), "abc");
    assert_eq!((r.row.u("_u0"), r.row.i("_u1")), (1, 7));
    assert_eq!(r.row.u("xslt_count"), 2);
    assert_eq!(
        write_as(
            &tables::XML_DEFINITION,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        populated.pieces
    );

    // Declared unconditionally instead, the empty form reads as a record that stopped early.
    const REQUIRED: Table = Table {
        fields: &[Field::new("name", Kind::Str)],
        ..tables::XML_DEFINITION
    };
    let r = read_strings(&REQUIRED, &empty, StringFormat::Enhanced);
    assert!(!r.complete && r.stop.ended);

    // And the corpus has nothing but the empty form.
    let (n, exact, populated) = sweep(&tables::XML_DEFINITION, Dialect::Contents, |_, row| {
        row.get("name").is_some()
    });
    assert!(n > 4_000, "the type is the corpus's most numerous: {n}");
    assert_eq!(exact, n, "{} records were not accounted for", n - exact);
    assert_eq!(populated, 0, "{populated} records carry a definition");
}

/// One byte off in a trailing skip moves every record of the type at once — the property that makes
/// a wrong length impossible to miss.
#[test]
fn control_a_skip_off_by_one_moves_every_record() {
    const PERTURBED: Table = Table {
        fields: &[
            Field::new("width", Kind::U32Be),
            Field::new("height", Kind::U32Be),
            Field::new("_u0", Kind::Skip(8)),
            Field::new("name", Kind::Str),
            Field::new("_u1", Kind::Skip(4)),
            Field::new("xml_definition", Kind::Child(0x0151)),
            Field::new("object_marker", Kind::Child(0x0165)),
            Field::new("repository_uri", Kind::Str),
            Field::new("_u2", Kind::Skip(20)),
        ],
        ..tables::OBJECT_NAME
    };
    let (n, exact, _) = sweep(&PERTURBED, Dialect::Contents, |_, _| false);
    assert!(n > 0);
    assert_eq!(
        exact, 0,
        "{n} records, none should be accounted for exactly"
    );
}

/// Omitting a nested record stops the read at it instead of silently continuing on the far side —
/// so the field beyond it is lost rather than quietly wrong.
#[test]
fn control_an_undeclared_child_blocks_the_read() {
    const PERTURBED: Table = Table {
        fields: &[
            Field::new("repeat_group_header", Kind::I16Be),
            Field::new("keep_group_together", Kind::I16Be),
            Field::new("group_indent", Kind::I32Be),
            Field::new("visible_groups_per_page", Kind::I32Be),
        ],
        ..tables::GROUP_AREA_FORMAT
    };
    let (n, exact, with_value) = sweep(&PERTURBED, Dialect::Contents, |_, r| {
        r.get("visible_groups_per_page").is_some()
    });
    assert!(n > 0);
    assert_eq!(exact, 0);
    assert_eq!(with_value, 0, "the field past the child is unreachable");
    // The real table reaches it.
    let (_, _, ok) = sweep(&tables::GROUP_AREA_FORMAT, Dialect::Contents, |_, r| {
        r.get("visible_groups_per_page").is_some()
    });
    assert_eq!(ok, n);
}

/// A picture opener is its nested name record and one word, on every report that has one — a
/// static picture, a chart placeholder and a blob field alike. The word is carried by all of them
/// and stored zero by all of them, so the corpus can say it is there and how wide it is, and
/// nothing beyond that.
///
/// The control is its position: the name record comes first, so dropping it from the declaration
/// puts the word out of reach on every record rather than four bytes to the left.
#[test]
fn a_picture_opener_is_its_name_record_and_one_word() {
    let (n, exact, zero) = sweep(&tables::PICTURE_OBJECT, Dialect::Contents, |_, r| {
        r.get("_u0") == Some(&Cell::U(0))
    });
    assert!(
        n > 100,
        "expected the corpus to hold many of these, got {n}"
    );
    assert_eq!(
        exact,
        n,
        "{} of {n} records were not accounted for",
        n - exact
    );
    assert_eq!(zero, n, "{} of {n} records store the word set", n - zero);

    const PERTURBED: Table = Table {
        fields: &[Field::new("_u0", Kind::U32Be)],
        ..tables::PICTURE_OBJECT
    };
    let (m, exact, with_value) =
        sweep(&PERTURBED, Dialect::Contents, |_, r| r.get("_u0").is_some());
    assert_eq!(m, n);
    assert_eq!(exact, 0, "the undeclared name record blocks every read");
    assert_eq!(with_value, 0, "the word past the child is unreachable");
}

/// The seven band markers are one shape: each is the `0x008c` section it brackets and nothing else,
/// with the section at content offset zero.
///
/// The uniformity is the first claim, and it is measured rather than assumed — every band record of
/// every type carries exactly one child, of that type, and no field bytes at all. That last fact is
/// also the limit of what the corpus can say about **order**: with no field bytes there is nothing
/// for a mis-ordered declaration to move, so the two perturbations below are what make the order a
/// reading rather than a preference. Dropping the section leaves it undeclared on every record;
/// declaring so much as a byte ahead of it is blocked by it on every record, and yields a value on
/// none — which is what "the section comes first" means in bytes.
#[test]
fn a_band_marker_is_its_section_and_nothing_else() {
    const BANDS: &[&Table] = &[
        &tables::REPORT_HEADER_BAND,
        &tables::REPORT_FOOTER_BAND,
        &tables::PAGE_HEADER_BAND,
        &tables::PAGE_FOOTER_BAND,
        &tables::DETAIL_BAND,
        &tables::GROUP_HEADER_BAND,
        &tables::GROUP_FOOTER_BAND,
    ];

    // The two perturbations, applied to every band type in turn: the section undeclared, and a
    // single byte declared ahead of it.
    const UNDECLARED: &[Field] = &[];
    const SECTION_LAST: &[Field] = &[
        Field::new("_ahead", Kind::U8),
        Field::new("section", Kind::Child(0x008c)),
    ];

    for table in BANDS {
        // The declaration itself: one field, and it is the section.
        assert_eq!(table.fields.len(), 1, "{}", table.name);
        assert_eq!(table.fields[0].name, "section", "{}", table.name);
        assert!(
            matches!(table.fields[0].kind, Kind::Child(0x008c)),
            "{} declares something other than the section",
            table.name
        );
    }

    // rtype -> (records, section alone, accounted for with the section undeclared, a byte read
    // ahead of the section).
    let mut seen: BTreeMap<u16, (usize, usize, usize, usize)> = BTreeMap::new();
    corpus::for_each_record(|dialect, node, logical, _| {
        if dialect != Dialect::Contents {
            return;
        }
        let Some(table) = BANDS.iter().find(|t| t.rtype == node.rtype) else {
            return;
        };
        let content = content_of(node, logical);
        let e = seen.entry(node.rtype).or_default();
        e.0 += 1;

        let r = read_strings(table, &content, StringFormat::Enhanced);
        let child = matches!(r.row.get("section"), Some(Cell::Child(c)) if c.rtype == 0x008c);
        if r.exact() && r.complete && child && content.field_byte_len() == 0 {
            e.1 += 1;
        }
        if read_strings(
            &Table {
                fields: UNDECLARED,
                ..**table
            },
            &content,
            StringFormat::Enhanced,
        )
        .exact()
        {
            e.2 += 1;
        }
        let last = read_strings(
            &Table {
                fields: SECTION_LAST,
                ..**table
            },
            &content,
            StringFormat::Enhanced,
        );
        if last.exact() || last.row.get("_ahead").is_some() {
            e.3 += 1;
        }
    });

    let mut total = 0usize;
    for table in BANDS {
        let (n, section_only, undeclared_exact, ahead) =
            *seen.get(&table.rtype).unwrap_or(&(0, 0, 0, 0));
        assert!(n > 0, "{} met no record", table.name);
        assert_eq!(
            section_only,
            n,
            "{}: {} of {n} records are not the section alone",
            table.name,
            n - section_only
        );
        assert_eq!(
            undeclared_exact, 0,
            "{}: {undeclared_exact} of {n} records were accounted for without the section declared",
            table.name
        );
        assert_eq!(
            ahead, 0,
            "{}: a byte was read ahead of the section on {ahead} of {n} records",
            table.name
        );
        total += n;
    }
    assert!(
        total > 1000,
        "expected the corpus to hold many band markers, got {total}"
    );
}

/// Reading a narrowing twip as a fixed-width `u16` agrees on every record whose value fits below
/// the top bit, and diverges on the one that does not — which is exactly how much of this branch
/// the corpus can check.
#[test]
fn control_a_narrowing_field_read_as_fixed_width() {
    const PERTURBED: Table = Table {
        fields: &[
            Field::new("left", Kind::VarU32),
            Field::new("top", Kind::U16Be),
        ],
        ..tables::OBJECT_POSITION
    };
    let (n, exact, _) = sweep(&PERTURBED, Dialect::Contents, |_, _| false);
    assert!(n > 0);
    assert_eq!(
        n - exact,
        1,
        "exactly one record in the corpus stores a coordinate in the wide form"
    );
}

/// The area name is read at a stated position, not found by scanning the field bytes for something
/// that parses as a length-prefixed string. Move that position by one byte and no record yields a name
/// at all — so the position carries the reading rather than restating what a scan would have found.
#[test]
fn control_the_area_name_position_is_load_bearing() {
    const PERTURBED: Table = Table {
        fields: &[
            Field::new("_u0", Kind::Skip(5)),
            Field::new("name", Kind::Str),
            Field::new("_u1", Kind::Skip(4)),
            Field::new("xml_definition", Kind::Child(0x0151)),
            Field::new("_u2", Kind::Skip(2)),
        ],
        ..tables::AREA
    };
    let (n, exact, named) = sweep(&PERTURBED, Dialect::Contents, |_, r| {
        !r.text("name").is_empty()
    });
    assert!(n > 0);
    assert_eq!(exact, 0, "{n} records, none accounted for exactly");
    assert_eq!(named, 0, "and none yields a name");
    // The real table names every one of them.
    let (_, _, ok) = sweep(&tables::AREA, Dialect::Contents, |_, r| {
        !r.text("name").is_empty()
    });
    assert_eq!(ok, n);
}

/// A text run's text is its first field, read at the length the record states — not the first span
/// of its bytes that happens to frame as a length-prefixed string.
///
/// The scan cannot report two things a run may store, because both fail its plausibility test and
/// send it looking further along: a run whose text is **empty**, which is a stored value and not an
/// absent one, and a run whose text is not valid UTF-8, which a report authored in a legacy code
/// page has. The corpus stores one empty run and no ill-formed one, and the empty one's spacing
/// tail is zero — so the scan, finding nothing at all after it, lands on the empty string by
/// failing rather than by reading it. Both forms are built here, since the corpus separates
/// neither.
#[test]
fn control_a_runs_text_is_read_at_its_stated_length() {
    let empty = synthetic(0x00c2, 0x0700, &[0, 0, 0, 1, 0, 0, 0, 0, 0]);
    let r = read_strings(&tables::TEXT_OBJECT, &empty, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("text"), "");
    assert_eq!(r.row.i("character_spacing"), 0);

    // A one-character text in a code page that is not UTF-8, and a spacing that is set.
    let ill_formed = synthetic(0x00c2, 0x0700, &[0, 0, 0, 2, 0xe9, 0, 0, 0, 0, 10]);
    let r = read_strings(&tables::TEXT_OBJECT, &ill_formed, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("text"), "\u{fffd}", "the byte, decoded lossily");
    assert_eq!(r.row.i("character_spacing"), 10);

    // Neither run's text is anything the scan reports.
    for content in [&empty, &ill_formed] {
        let Some(crate::field_table::cursor::Piece::Run(run)) = content.pieces.first() else {
            unreachable!()
        };
        assert_eq!(crate::bytes::first_lp(run), None);
    }
}

/// A `0x009f` field object ends with the handle of the field it shows, stored **index first and the
/// pool after it** — the reverse of the order the reference's own composite puts them in.
///
/// Both orders consume the same three bytes, so byte accounting cannot separate them and a record
/// is read exactly either way. What settles the order is the reference beside it: read index-first,
/// the trailing handle is the reference's own on every record in the corpus; read the composite's
/// way round, it is on almost none — and the few left are the records whose two bytes read the same
/// either way.
#[test]
fn control_a_field_objects_trailing_handle_is_stored_index_first() {
    fn repeats_the_reference(_schema: u16, r: &Row) -> bool {
        let Some((pool, index)) = r.get("data_source").and_then(Cell::handle) else {
            return false;
        };
        r.u("field_index") as u16 == index.unwrap_or(UNSET_FIELD_INDEX) && r.u("field_kind") == pool
    }
    let (n, exact, matching) = sweep(
        &tables::FIELD_OBJECT,
        Dialect::Contents,
        repeats_the_reference,
    );
    assert!(
        n > 1000,
        "expected the corpus to hold many of these, got {n}"
    );
    assert_eq!(
        exact,
        n,
        "{} of {n} records were not accounted for",
        n - exact
    );
    assert_eq!(
        matching, n,
        "the handle repeats the reference on every record"
    );

    const REVERSED: Table = Table {
        fields: &[
            Field::new("object_name", Kind::Child(0x009e)),
            Field::new("data_source", Kind::FieldRef),
            Field::new("old_highlight_count", Kind::U16Be),
            Field::optional("highlight_count", Kind::U16Be),
            Field::optional("field_definition_is_stored", Kind::I16Be),
            Field::optional("field_kind", Kind::VarU16),
            Field::optional("field_index", Kind::U16Be),
        ],
        ..tables::FIELD_OBJECT
    };
    let (n2, exact2, matching2) = sweep(&REVERSED, Dialect::Contents, repeats_the_reference);
    assert_eq!(
        (n2, exact2),
        (n, exact),
        "the reversed order consumes the same bytes, so accounting cannot tell them apart"
    );
    // The residue is not slack: the two orders read the same three bytes the same way exactly when
    // the index's two halves and the pool are all one value, and those are the only records the
    // reversed order still gets right.
    let (_, _, coincide) = sweep(&tables::FIELD_OBJECT, Dialect::Contents, |_, r| {
        match r.get("data_source").and_then(Cell::handle) {
            Some((pool, index)) => u32::from(index.unwrap_or(UNSET_FIELD_INDEX)) == pool * 0x0101,
            None => false,
        }
    });
    assert_eq!(
        matching2, coincide,
        "the reversed order agrees only where the bytes read the same either way"
    );
    assert!(
        coincide * 4 < n,
        "and that is a small minority of the {n} records, not {coincide}"
    );
}

/// An embedded field run's reference is one composite read in sequence, so the special-field kind
/// behind it is the reference's **whole** index and the fields after it start where the reference
/// ends — not two bytes reached from the low byte of its length prefix.
///
/// The corpus cannot separate the two readings: every embedded reference is far shorter than the
/// 256 bytes it takes for a length prefix's low byte to stop being its length, and every index fits
/// in the byte the reading it replaces took. Both forms are built here, and the bytes the low-byte
/// addressing lands on are named beside the table's reading to show that it answers differently
/// rather than merely being expressed differently.
#[test]
fn control_an_embedded_references_index_is_a_whole_word() {
    /// The pool and the index as the reading this replaces addressed them: from the **low byte** of
    /// the reference's four-byte length prefix rather than from where the reference ends.
    fn from_the_prefixs_low_byte(b: &[u8]) -> (Option<u8>, Option<u8>) {
        let p = b.get(3).map_or(0, |len| 4 + *len as usize);
        (b.get(p).copied(), b.get(p + 2).copied())
    }

    // An index past a byte, on an otherwise ordinary run.
    let mut wide = vec![0, 0, 0, 2, b'f', 0, 0x03, 0x01, 0x02];
    wide.extend_from_slice(&[0, 0, 0, 0]);
    let content = synthetic(0x00c4, 0x0700, &wide);
    let r = read_strings(
        &tables::TEXT_EMBEDDED_FIELD,
        &content,
        StringFormat::Enhanced,
    );
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(
        r.row.get("data_source").and_then(Cell::handle),
        Some((3, Some(0x0102)))
    );
    // The reading it replaces takes the index's low half and calls that the special field's kind.
    assert_eq!(from_the_prefixs_low_byte(&wide).1, Some(0x02));

    // A reference of 256 bytes or more: its length no longer fits the byte the old reading took it
    // from, so everything the old reading addressed behind it lands inside the text.
    let name = vec![b'a'; 259];
    let mut long = vec![0, 0, 0x01, 0x04];
    long.extend_from_slice(&name);
    long.push(0);
    long.extend_from_slice(&[0x03, 0x00, 0x07, 0, 0, 0, 0]);
    let content = synthetic(0x00c4, 0x0700, &long);
    let r = read_strings(
        &tables::TEXT_EMBEDDED_FIELD,
        &content,
        StringFormat::Enhanced,
    );
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("data_source").len(), 259);
    assert_eq!(
        r.row.get("data_source").and_then(Cell::handle),
        Some((3, Some(7)))
    );
    assert_eq!(
        from_the_prefixs_low_byte(&long),
        (Some(b'a'), Some(b'a')),
        "the old reading takes both from the middle of the reference's own text"
    );

    // An empty reference is a stored value: the scan the old reading used rejects it and reports
    // nothing, while the record still carries its handle and its tail in the usual places.
    let empty = [0, 0, 0, 1, 0, 0x03, 0x00, 0x07, 0, 0, 0, 0];
    let content = synthetic(0x00c4, 0x0700, &empty);
    let r = read_strings(
        &tables::TEXT_EMBEDDED_FIELD,
        &content,
        StringFormat::Enhanced,
    );
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("data_source"), "");
    assert_eq!(
        r.row.get("data_source").and_then(Cell::handle),
        Some((3, Some(7)))
    );
    assert_eq!(crate::bytes::first_lp(&empty), None);
}

/// A `0x0029` sort's direction is the byte after its field handle, not "the record's last byte".
/// Narrow the handle's index by two bytes — the reading that would still find the direction at the
/// end — and every sort record is left with bytes unaccounted for.
#[test]
fn control_the_sort_direction_follows_the_whole_field_handle() {
    const PERTURBED: Table = Table {
        fields: &[
            Field::new("field", Kind::Str),
            Field::new("field_kind", Kind::U8),
            Field::new("direction", Kind::U8),
        ],
        ..tables::RECORD_SORT_FIELD
    };
    let (n, exact, _) = sweep(&PERTURBED, Dialect::Contents, |_, _| false);
    assert!(n > 0);
    assert_eq!(exact, 0, "{n} records, none accounted for exactly");
}

/// The three bytes between a `0x00e5` group's condition field and its grouping ordinal are a field
/// of the record, not padding a reader may fold into the distance it counts from the field
/// reference. Drop them and every group of every kind moves: none is accounted for, and none yields
/// the order marker that says which of the three group kinds it is.
#[test]
fn control_the_group_field_handle_is_load_bearing() {
    const PERTURBED: Table = Table {
        fields: &[
            Field::new("condition_field", Kind::Str),
            Field::new("condition_ordinal", Kind::U8),
            Field::new("_u0", Kind::U8),
            Field::new("direction", Kind::U8),
            Field::new("not_in_topn_name", Kind::Str),
            Field::new("topn_limit", Kind::U16Be),
            Field::new("discard_others", Kind::U16Be),
            Field::new("_others_name", Kind::Str),
            Field::new("group_name_field", Kind::Str),
            Field::new("_u1", Kind::Skip(3)),
            Field::new("order_marker", Kind::Str),
        ],
        ..tables::GROUP
    };
    let (n, exact, marked) = sweep(&PERTURBED, Dialect::Contents, |_, r| {
        !r.text("order_marker").is_empty()
    });
    assert!(n > 0);
    assert_eq!(exact, 0, "{n} records, none accounted for exactly");
    assert_eq!(marked, 0, "and none yields its order marker");
    // The real table names every one of them that stores a marker.
    let (_, _, ok) = sweep(&tables::GROUP, Dialect::Contents, |_, r| {
        !r.text("order_marker").is_empty()
    });
    assert!(
        ok > 0 && ok < n,
        "some groups store an empty marker: {ok}/{n}"
    );
}

/// A `0x00e5`'s trailer is walked **through** its Hierarchical-Grouping block, whose two field
/// names are length-prefixed and therefore variable — everything after them moves with them.
///
/// Every group in the committed fixtures stores that block flag-clear with both names empty, which
/// is the only reason a fixed skip from the suppress flags to the trailer's copy of the Top-N limit
/// ever agreed: the two coincide precisely when the names are empty. This control supplies the
/// record those fixtures lack — a hierarchically sorted group, the shape a real report stores — and
/// shows the fixed skip landing inside the names and reporting a limit that is not the stored one.
#[test]
fn control_the_group_trailer_is_walked_through_the_hierarchical_names() {
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }
    /// An unset field reference: an empty name, then a `(kind, index)` handle of `(0, 0xffff)`.
    fn empty_ref() -> Vec<u8> {
        let mut v = lp("");
        v.extend([0x00, 0xff, 0xff]);
        v
    }
    fn field_ref(name: &str, kind: u8, index: u16) -> Vec<u8> {
        let mut v = lp(name);
        v.push(kind);
        v.extend(index.to_be_bytes());
        v
    }
    let mut run = field_ref("T.a", 0, 0);
    run.extend([0x00, 0x00, 0x00]); // grouping period, direction, the discarded enum
    run.extend(lp("Others"));
    run.extend([0x00, 0x05, 0x00, 0x01]); // Top-N limit 5, DiscardOthers
    run.extend(lp("Others"));
    run.extend(field_ref("Group #1 Name", 4, 0));
    run.extend(field_ref("@Group #1 Order", 1, 0));
    run.extend([0xff, 0xff]); // the -1 default
    run.extend([0u8; 8]); // the four suppress words
    let after_suppress = run.len();
    run.extend([0x00, 0x00]);
    run.extend(empty_ref()); // the group-name formula
    run.extend([0x00, 0x01]); // hierarchical sorting on
    run.extend(field_ref("t.parent", 0, 1));
    run.extend(field_ref("t.id", 0, 0xffff));
    run.push(0x00); // the trailing enum
    run.extend([0u8; 8]); // a double
    run.extend([0x00, 0x00]); // no specified-order pairs
    run.extend([0x00, 0x00]);
    run.extend(empty_ref()); // the Top-N value formula
    run.extend([0x00, 0x00]);
    run.extend(empty_ref()); // the group-sort-order formula
    run.extend([0x00, 0x00]);
    run.extend([0x00, 0x05]); // the trailer's copy of the limit
    run.extend([0u8; 8]); // a double
    run.push(0x00); // the trailing enum

    let content = RecordContent {
        rtype: 0x00e5,
        schema: 0x0700,
        pieces: vec![crate::field_table::cursor::Piece::Run(run.clone())],
    };
    let reading = read_strings(&tables::GROUP, &content, StringFormat::Enhanced);
    assert!(reading.exact() && reading.complete, "{reading:?}");
    let row = &reading.row;
    assert_eq!(row.i("topn_limit"), 5);
    assert_eq!(row.i("topn_limit_repeat"), 5);
    assert_eq!(row.i("hierarchical_enabled"), 1);
    assert_eq!(row.text("parent_id_field"), "t.parent");
    assert_eq!(row.text("instance_id_field"), "t.id");
    assert_eq!(
        write_as(&tables::GROUP, row, 0x0700, StringFormat::Enhanced),
        content.pieces
    );

    // A fixed 61-byte skip from the suppress words — the distance that reaches the limit only when
    // both hierarchical names are empty — lands inside `t.parent` here.
    assert_ne!(crate::bytes::u16_be(&run, after_suppress + 61), Some(5));
    // Eleven bytes from the end still reaches it: the block sits inside the trailer, so the record
    // ends the same distance past the limit whatever the names hold.
    assert_eq!(crate::bytes::u16_be(&run, run.len() - 11), Some(5));
}

/// A `0x007e` definition's percentage tail sits **after** its second operand, so its position
/// follows that operand's length rather than a fixed distance from the first.
///
/// Every `0x007e` in the corpus stores that second operand empty, so the corpus cannot separate
/// this reading from the fixed `used + 12` one it replaces — the two coincide exactly when the
/// operand is empty. This control supplies the record the corpus does not have: with a two-field
/// summary the fixed distance lands inside the second operand's own bytes and reports a base group
/// that is not there, while the sequential reading lands on the stored one.
#[test]
fn control_the_percentage_tail_follows_the_second_operand() {
    fn operand(text: &[u8]) -> Vec<u8> {
        let mut out = (text.len() as u32 + 1).to_be_bytes().to_vec();
        out.extend_from_slice(text);
        out.push(0);
        out
    }
    let mut run = vec![0x00, 0x00, 0x00, 0x00]; // Sum, separator, operation parameter
    run.extend(operand(b"T.a"));
    run.extend_from_slice(&[0x00, 0x00, 0x03]); // the primary's value descriptor
    run.extend(operand(b"T.b")); // the second operand a one-field definition leaves empty
    run.extend_from_slice(&[0x00, 0xff, 0xff, 0x00]);
    run.extend_from_slice(&[0x01, 0x00, 0x02]); // IsPercentageSummary, base group 2
    run.extend_from_slice(&[0x00, 0x00]);
    let content = RecordContent {
        rtype: 0x007e,
        schema: 0x0700,
        pieces: vec![
            crate::field_table::cursor::Piece::Child(crate::field_table::cursor::ChildRef {
                rtype: 0x0071,
                schema: 0x0700,
                framed_len: 25,
            }),
            crate::field_table::cursor::Piece::Run(run.clone()),
        ],
    };

    let reading = read_strings(
        &tables::SUMMARY_FIELD_DEFINITION,
        &content,
        StringFormat::Enhanced,
    );
    assert!(reading.exact() && reading.complete, "{reading:?}");
    let row = &reading.row;
    assert_eq!(row.text("operand"), "T.a");
    assert_eq!(row.text("secondary_operand"), "T.b");
    assert_eq!(row.i("is_percentage"), 1);
    assert_eq!(row.i("percentage_base_group"), 2);
    assert_eq!(
        write_as(
            &tables::SUMMARY_FIELD_DEFINITION,
            row,
            0x0700,
            StringFormat::Enhanced
        ),
        content.pieces
    );

    // The reading this replaced, anchored a fixed twelve bytes past the first operand: it lands in
    // the middle of the second operand's trailing bytes and reports a base group of its own making.
    let past_primary = 4 + crate::bytes::read_lp_string(&run[4..])
        .expect("the primary operand")
        .1;
    assert_ne!(
        run[past_primary + 12],
        0,
        "the fixed distance happens to land on a non-zero byte"
    );
    assert_ne!(
        crate::bytes::u16_be(&run, past_primary + 13),
        Some(2),
        "but not on the stored base group"
    );
}

/// A synthetic record, for the shapes the corpus stores only one value of.
fn synthetic(rtype: u16, schema: u16, run: &[u8]) -> RecordContent {
    RecordContent {
        rtype,
        schema,
        pieces: vec![crate::field_table::cursor::Piece::Run(run.to_vec())],
    }
}

/// `0x0145` is four whole fields — a signed word, a narrowing enum, a colour and a signed
/// half-word — and not the byte-per-channel colour with padding either side that it was read as.
///
/// The corpus cannot tell the two apart: it stores two distinct runs in hundreds of records, all
/// eleven bytes with every field but the last constant, and both readings account for eleven bytes
/// exactly. Only a record that exercises a field's own width separates them, so the discriminating
/// record is built here rather than looked for.
#[test]
fn the_grid_cell_format_is_four_whole_fields() {
    // A wide enum (two bytes, the top bit marking the width), a colour with every byte set, and a
    // negative trailing word: twelve bytes, not eleven.
    let run = [
        0xff, 0xff, 0xff, 0xfe, // a signed word
        0x81, 0x00, // the enum, stored wide
        0xff, 0xff, 0xff, 0xff, // the colour
        0xff, 0xff, // the trailing word
    ];
    let content = synthetic(0x0145, 0x0700, &run);
    let r = read_strings(
        &tables::CROSSTAB_GRID_CELL_FORMAT,
        &content,
        StringFormat::Enhanced,
    );
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.i("flags"), -2);
    assert_eq!(r.row.u("_u0"), 0x0100);
    assert_eq!(r.row.u("background_color"), 0xffff_ffff);
    assert_eq!(r.row.i("enabled"), -1);
    assert_eq!(
        write_as(
            &tables::CROSSTAB_GRID_CELL_FORMAT,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        content.pieces
    );

    // The reading this replaced: a fixed-width word, two bytes of padding, three colour channels,
    // one more pad byte and a byte-wide flag. It runs a byte short of the record, and reports a
    // colour of its own making rather than the one stored.
    const AS_CHANNELS: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x0145,
        name: "AsChannels",
        fields: &[
            Field::new("flags", Kind::U32Be),
            Field::new("_pad", Kind::Skip(2)),
            Field::new("red", Kind::U8),
            Field::new("green", Kind::U8),
            Field::new("blue", Kind::U8),
            Field::new("_pad2", Kind::Skip(1)),
            Field::new("enabled", Kind::U8),
        ],
    };
    let old = read_strings(&AS_CHANNELS, &content, StringFormat::Enhanced);
    assert!(!old.exact(), "the shape it replaced must not fit");
    assert_eq!(old.unread, 1);

    // And on the corpus the two are indistinguishable, which is why the record above exists.
    let (n, exact, _) = sweep(
        &tables::CROSSTAB_GRID_CELL_FORMAT,
        Dialect::Contents,
        |_, _| false,
    );
    let (n_old, exact_old, _) = sweep(&AS_CHANNELS, Dialect::Contents, |_, _| false);
    assert!(n > 0 && n == n_old);
    assert_eq!((exact, exact_old), (n, n), "the corpus separates nothing");
}

/// `0x0143`'s only field is a count of the `0x0145` records that follow it, and the record need
/// not carry it.
///
/// The count is checked against the run it counts, in every cross-tab in the corpus: the records
/// between a `0x0143` and its closing `0x0144` are counted and compared with the word. That is
/// what makes "count" a reading of the record rather than a name for a word nothing explains.
#[test]
fn the_grid_format_word_counts_the_cell_formats_after_it() {
    /// The three records are siblings, so the run is read off the sequence a record sits in
    /// rather than out of its content.
    fn scan(
        siblings: &[crate::codec::RecordNode],
        logical: &[u8],
        file: &str,
        checked: &mut usize,
    ) {
        for (i, k) in siblings.iter().enumerate() {
            if k.rtype == 0x0143 {
                let r = read_strings(
                    &tables::CROSSTAB_GRID_FORMAT,
                    &content_of(k, logical),
                    StringFormat::Enhanced,
                );
                assert!(r.exact() && r.complete, "{file}: {r:?}");
                let count = match r.row.get("cell_count") {
                    Some(v) => v.u().unwrap_or_default(),
                    None => tables::CROSSTAB_GRID_CELL_DEFAULT_COUNT,
                };
                let run = siblings[i + 1..]
                    .iter()
                    .take_while(|c| c.rtype == 0x0145)
                    .count();
                assert_eq!(
                    count as usize, run,
                    "{file}: the word must count the records after it"
                );
                assert_eq!(
                    siblings.get(i + 1 + run).map(|c| c.rtype),
                    Some(0x0144),
                    "{file}: the run is closed by its end record"
                );
                *checked += 1;
            }
            scan(&k.children, logical, file, checked);
        }
    }

    let mut checked = 0usize;
    corpus::for_each_stream(|_, dialect, stream, path| {
        if dialect != Dialect::Contents {
            return;
        }
        scan(
            &stream.record_tree(),
            stream.logical_bytes(),
            &path.display().to_string(),
            &mut checked,
        );
    });
    assert!(checked > 0, "no fixture exercised the cell-format run");
}

/// A `0x0143` that carries no count at all is still a record this table describes completely —
/// the shape a fixed field cannot express, since it reports the record as ending early.
#[test]
fn an_empty_grid_format_record_is_not_a_short_one() {
    let empty = RecordContent {
        rtype: 0x0143,
        schema: 0x0700,
        pieces: Vec::new(),
    };
    let r = read_strings(
        &tables::CROSSTAB_GRID_FORMAT,
        &empty,
        StringFormat::Enhanced,
    );
    assert!(r.exact() && r.complete);
    assert!(r.row.get("cell_count").is_none());
    assert_eq!(
        write_as(
            &tables::CROSSTAB_GRID_FORMAT,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        empty.pieces
    );

    const REQUIRED: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x0143,
        name: "Required",
        fields: &[Field::new("cell_count", Kind::U16Be)],
    };
    let r = read_strings(&REQUIRED, &empty, StringFormat::Enhanced);
    assert!(!r.complete && r.stop.ended);
}

/// A group area format stops wherever its writer stopped: only the two flags are unconditional,
/// and a record carrying nothing else is described completely rather than reported as truncated.
///
/// Its last field is one field reference, not a string beside two numbers — so the corpus's unset
/// references read as naming no field instead of as the index `0xffff`.
#[test]
fn a_group_area_format_stops_where_its_writer_stopped() {
    let short = synthetic(0x0088, 0x0700, &[0x00, 0x01, 0x00, 0x00]);
    let r = read_strings(&tables::GROUP_AREA_FORMAT, &short, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.i("repeat_group_header"), 1);
    assert!(r.row.get("group_indent").is_none());
    assert_eq!(
        write_as(
            &tables::GROUP_AREA_FORMAT,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        short.pieces
    );

    // Unconditionally, the same record is read as ending in the middle of its own fields.
    const UNGUARDED: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x0088,
        name: "Unguarded",
        fields: &[
            Field::new("repeat_group_header", Kind::I16Be),
            Field::new("keep_group_together", Kind::I16Be),
            Field::new("group_indent", Kind::I32Be),
        ],
    };
    let r = read_strings(&UNGUARDED, &short, StringFormat::Enhanced);
    assert!(!r.complete && r.stop.ended);

    // Every record in the corpus stores the reference unset, and the composite says so rather
    // than reporting the sentinel as an index into a field pool.
    let mut seen = 0usize;
    corpus::for_each_record_of(Dialect::Contents, 0x0088, |node, logical, path| {
        let r = read_strings(
            &tables::GROUP_AREA_FORMAT,
            &content_of(node, logical),
            StringFormat::Enhanced,
        );
        if let Some(v) = r.row.get("new_page_after_formula") {
            assert_eq!(
                v.handle(),
                Some((0, None)),
                "{}: an unset reference names no field",
                path.display()
            );
            seen += 1;
        }
    });
    assert!(seen > 0, "no fixture carries the formula reference");
}

/// A chart's analytic header need not carry its trailing word, and its data values are counted by
/// a whole `u32` whose low half alone lands inside the first reference.
#[test]
fn the_chart_analytic_records_carry_what_they_carry() {
    // Three bytes: the word, a narrow enum, and nothing else.
    let short = synthetic(0x011c, 0x0700, &[0x00, 0x02, 0x03]);
    let r = read_strings(&tables::CHART_ANALYTIC, &short, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.u("layout_type"), 3);
    assert!(r.row.get("_u1").is_none());
    assert_eq!(
        write_as(
            &tables::CHART_ANALYTIC,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        short.pieces
    );

    const UNCONDITIONAL: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x011c,
        name: "Unconditional",
        fields: &[
            Field::new("_u0", Kind::U16Be),
            Field::new("layout_type", Kind::VarU16),
            Field::new("_u1", Kind::U16Be),
        ],
    };
    let r = read_strings(&UNCONDITIONAL, &short, StringFormat::Enhanced);
    assert!(!r.complete && r.stop.ended);

    // One data value: a word, a `u32` count, the reference, and the trailing word.
    let mut run = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
    run.extend_from_slice(b"\x00\x00\x00\x04sum\x00\x02\x00\x07");
    run.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    let content = synthetic(0x011f, 0x0700, &run);
    let r = read_strings(&tables::CHART_DATA_VALUE, &content, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.u("value_count"), 1);
    let first = &r.row.seq("values")[0];
    assert_eq!(first.text("summary"), "sum");
    assert_eq!(
        first.get("summary").and_then(Cell::handle),
        Some((2, Some(7)))
    );
    assert_eq!(
        write_as(
            &tables::CHART_DATA_VALUE,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        content.pieces
    );

    // Counted by the low half of the same word, the run is empty and the record is left unread.
    const HALF_COUNT: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x011f,
        name: "HalfCount",
        fields: &[
            Field::new("_u0", Kind::U16Be),
            Field::new("value_count", Kind::U16Be),
            Field::new(
                "values",
                Kind::Repeat {
                    count: Count::FromField("value_count"),
                    body: &[Field::new("summary", Kind::FieldRef)],
                },
            ),
            Field::new("_u1", Kind::U32Be),
        ],
    };
    let r = read_strings(&HALF_COUNT, &content, StringFormat::Enhanced);
    assert!(!r.exact(), "the low half counts nothing and reads nothing");
}

/// A subreport object's two formulas are field references, so a subreport that sets one lengthens
/// the record; and the word after them is written only while the record has content left.
///
/// Every corpus record leaves both unset, which is exactly eighteen bytes of zeros and `ff`s — the
/// width a fixed run was derived from. A record that sets one is the only thing that can tell the
/// two apart, so it is built here.
#[test]
fn a_subreport_object_with_a_caption_formula_is_longer() {
    let mut run = vec![0x00, 0x00, 0x00, 0x01]; // subdocument index
    run.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // link flag, on demand
    run.extend_from_slice(b"\x00\x00\x00\x04cap\x00\x01\x00\x02"); // a bound reference
    run.extend_from_slice(b"\x00\x00\x00\x01\x00\x00\xff\xff"); // and an unset one
    let content = RecordContent {
        rtype: 0x00a3,
        schema: 0x0700,
        pieces: vec![
            crate::field_table::cursor::Piece::Child(crate::field_table::cursor::ChildRef {
                rtype: 0x009e,
                schema: 0x0700,
                framed_len: 40,
            }),
            crate::field_table::cursor::Piece::Run(run),
        ],
    };
    let r = read_strings(&tables::SUBREPORT_OBJECT, &content, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("on_demand_caption_formula"), "cap");
    assert_eq!(
        r.row
            .get("on_demand_caption_formula")
            .and_then(Cell::handle),
        Some((1, Some(2)))
    );
    assert_eq!(
        r.row.get("tab_text_formula").and_then(Cell::handle),
        Some((0, None)),
        "the unset reference names no field"
    );
    assert!(r.row.get("_u0").is_none());
    assert_eq!(
        write_as(
            &tables::SUBREPORT_OBJECT,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        content.pieces
    );

    // The width the corpus's unset pair happens to have, read as a fixed run: it runs off the end
    // of a record whose formula is set.
    const FIXED_RUN: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x00a3,
        name: "FixedRun",
        fields: &[
            Field::new("object_name", Kind::Child(0x009e)),
            Field::new("subdocument_index", Kind::U32Be),
            Field::new("_has_link_object", Kind::I16Be),
            Field::new("on_demand", Kind::I16Be),
            Field::new("_formulas", Kind::Skip(18)),
            Field::new("_u0", Kind::I16Be),
        ],
    };
    assert!(!read_strings(&FIXED_RUN, &content, StringFormat::Enhanced).exact());
}

/// An object's name record stops wherever its writer stopped: everything past the marker is
/// written only while the record still has content, so a record that carries neither nested record
/// nor trailing block is described completely.
#[test]
fn an_object_name_record_stops_after_its_marker() {
    let mut run = vec![0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x02, 0x80]; // width, height
    run.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // the rectangle, all narrow
    run.extend_from_slice(b"\x00\x00\x00\x04Box\x00"); // the name
    run.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]); // the marker
    let content = synthetic(0x009e, 0x0700, &run);
    let r = read_strings(&tables::OBJECT_NAME, &content, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("name"), "Box");
    assert!(r.row.get("repository_uri").is_none());
    assert_eq!(
        write_as(&tables::OBJECT_NAME, &r.row, 0x0700, StringFormat::Enhanced),
        content.pieces
    );

    // Unconditionally, the same record is read as ending inside its own field list.
    const UNGUARDED: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x009e,
        name: "Unguarded",
        fields: &[
            Field::new("width", Kind::I32Be),
            Field::new("height", Kind::I32Be),
            Field::new(
                "bounds",
                Kind::Repeat {
                    count: Count::Fixed(4),
                    body: &[Field::new("v", Kind::VarU32)],
                },
            ),
            Field::new("name", Kind::Str),
            Field::new("_marker", Kind::I32Be),
            Field::new("repository_uri", Kind::Str),
        ],
    };
    let r = read_strings(&UNGUARDED, &content, StringFormat::Enhanced);
    assert!(!r.complete && r.stop.ended);
}

/// The block after an object name's two nested records opens with a **length-prefixed string**, not
/// a constant run of bytes.
///
/// It looks constant because the string is empty on all but one record in the corpus: an object
/// stored in a repository rather than in the report keeps its reference there, and that one record
/// is dozens of bytes longer than the rest. Read as a fixed run the whole corpus fits except that
/// record, which is exactly the kind of agreement a fixed run buys.
#[test]
fn an_object_name_carries_a_repository_reference_not_a_constant_tail() {
    const FIXED_TAIL: usize = 26;
    const AS_CONSTANT: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x009e,
        name: "ConstantTail",
        fields: &[
            Field::new("width", Kind::I32Be),
            Field::new("height", Kind::I32Be),
            Field::new(
                "bounds",
                Kind::Repeat {
                    count: Count::Fixed(4),
                    body: &[Field::new("v", Kind::VarU32)],
                },
            ),
            Field::new("name", Kind::Str),
            Field::new("_marker", Kind::I32Be),
            Field::optional("xml_definition", Kind::Child(0x0151)),
            Field::optional("object_marker", Kind::Child(0x0165)),
            Field::optional("_tail", Kind::Skip(FIXED_TAIL)),
        ],
    };

    let (mut records, mut with_reference, mut constant_exact) = (0usize, 0usize, 0usize);
    corpus::for_each_record_of(Dialect::Contents, 0x009e, |node, logical, path| {
        let content = content_of(node, logical);
        records += 1;
        let r = read_strings(&tables::OBJECT_NAME, &content, StringFormat::Enhanced);
        assert!(r.exact(), "{}: {r:?}", path.display());
        if !r.row.text("repository_uri").is_empty() {
            with_reference += 1;
            assert!(
                r.row.text("repository_uri").len() > FIXED_TAIL,
                "a stored reference outgrows the run it was mistaken for"
            );
        }
        if read_strings(&AS_CONSTANT, &content, StringFormat::Enhanced).exact() {
            constant_exact += 1;
        }
    });
    assert!(records > 1000, "{records} object names swept");
    assert!(
        with_reference > 0,
        "no object in the corpus is repository-linked"
    );
    assert_eq!(
        constant_exact,
        records - with_reference,
        "the fixed run fits every record but the one that stores a reference"
    );
}

/// The query engine's records are versioned, and a field a version added is absent from every
/// record written before it.
///
/// The corpus stores one version per record type — always the newest — so an ungated table
/// consumes every corpus record exactly and mis-reads the first older file it meets. The record
/// that shows it is a column written two versions back.
#[test]
fn a_query_engine_field_carries_only_what_its_version_has() {
    // Name, description, type, size, attributes, precision — and, at this version, nothing else.
    let mut run = vec![0x00, 0x00, 0x00, 0x11];
    run.extend_from_slice(b"\x00\x00\x00\x03id\x00"); // name
    run.extend_from_slice(b"\x00\x00\x00\x01\x00"); // description
    run.extend_from_slice(&[0, 0, 0, 4]); // value type
    run.extend_from_slice(&[0, 0, 0, 8]); // length
    run.extend_from_slice(&[0, 0, 0, 2]); // attributes
    run.extend_from_slice(&[0, 0, 0, 5]); // precision
    let older = RecordContent {
        rtype: 0x0004,
        schema: 0x0902,
        pieces: vec![crate::field_table::cursor::Piece::Run(run.clone())],
    };
    let r = read_strings(&tables::QE_FIELD, &older, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!(r.row.text("name"), "id");
    assert_eq!((r.row.u("attributes"), r.row.u("precision")), (2, 5));
    assert!(r.row.get("id").is_none(), "the identifier came later");
    assert!(r.row.get("field_lineage").is_none());
    assert_eq!(
        write_as(&tables::QE_FIELD, &r.row, 0x0902, StringFormat::Enhanced),
        older.pieces
    );

    // The same record read as the newest version: the fields that version added are taken out of
    // the bytes after it, which there are none of.
    let newest = RecordContent {
        schema: 0x0905,
        ..older.clone()
    };
    let r = read_strings(&tables::QE_FIELD, &newest, StringFormat::Enhanced);
    assert!(!r.complete && r.stop.ended);
}

/// The run between a table's bind parameters and its command text is a run of **index records**,
/// each nested as a record of its own.
///
/// They are written in the short header form — no schema word, so the version is the stream's
/// default — and their content is masked, which together is why the run reads as a run of opaque
/// blobs when the headers are not recognised. Read as records, each is an index: a name, two flags,
/// and the columns it covers.
#[test]
fn a_tables_index_run_holds_index_records() {
    let mut indexes = 0usize;
    corpus::for_each_record_of(Dialect::QeSession, 0x0003, |node, logical, path| {
        let r = read_strings(
            &tables::QE_TABLE,
            &content_of(node, logical),
            StringFormat::Enhanced,
        );
        let children: Vec<_> = node
            .children
            .iter()
            .filter(|c| c.rtype == tables::QE_INDEX.rtype)
            .collect();
        assert_eq!(
            r.row.u("index_count") as usize,
            children.len(),
            "{}: the count states how many index records follow",
            path.display()
        );
        for child in children {
            assert_eq!(
                child.schema,
                0x0900,
                "{}: a header with no schema word takes the stream's default",
                path.display()
            );
            let idx = read_strings(
                &tables::QE_INDEX,
                &content_of(child, logical),
                StringFormat::Enhanced,
            );
            assert!(idx.exact() && idx.complete, "{}: {idx:?}", path.display());
            assert!(
                !idx.row.text("name").is_empty(),
                "{}: an index names itself",
                path.display()
            );
            assert_eq!(
                idx.row.u("field_count") as usize,
                idx.row.seq("fields").len()
            );
            assert!(idx.row.u("field_count") > 0, "an index covers a column");
            assert!(idx.row.u("is_primary_key") <= 1, "a boolean, not a word");
            assert!(idx.row.u("has_unique_values") <= 1);
            indexes += 1;
        }
    });
    assert!(indexes > 0, "no table in the corpus reports an index");
}

/// The word after a guideline's position is a **count of the object connections** attached to that
/// guide, not a flag word.
///
/// The two readings are indistinguishable inside the record — both are one word, and both admit
/// every value the corpus stores — so the claim is settled outside it: a guideline is followed by a
/// run of `0x0112` object-connection collections, closed by the guideline's own end record, and the
/// word states how long that run is.
#[test]
fn a_guidelines_word_counts_the_object_connections_attached_to_it() {
    /// The guideline's end record: a list and a collection close with their own.
    fn end_of(rtype: u16) -> u16 {
        rtype + 1
    }

    fn scan(
        siblings: &[crate::codec::RecordNode],
        logical: &[u8],
        file: &str,
        checked: &mut usize,
    ) {
        for (i, k) in siblings.iter().enumerate() {
            if matches!(k.rtype, 0x010d | 0x010f) {
                let entry = k
                    .children
                    .iter()
                    .find(|c| c.rtype == tables::GUIDELINE_ENTRY.rtype)
                    .unwrap_or_else(|| panic!("{file}: a guideline states its position"));
                let r = read_strings(
                    &tables::GUIDELINE_ENTRY,
                    &content_of(entry, logical),
                    StringFormat::Enhanced,
                );
                assert!(r.exact() && r.complete, "{file}: {r:?}");
                let run = siblings[i + 1..]
                    .iter()
                    .take_while(|c| c.rtype == 0x0112)
                    .count();
                assert_eq!(
                    r.row.u("connection_count") as usize,
                    run,
                    "{file}: the word must count the collections after it"
                );
                assert_eq!(
                    siblings.get(i + 1 + run).map(|c| c.rtype),
                    Some(end_of(k.rtype)),
                    "{file}: the run is closed by the guideline's end record"
                );
                *checked += 1;
            }
            scan(&k.children, logical, file, checked);
        }
    }

    let mut checked = 0usize;
    corpus::for_each_stream(|_, dialect, stream, path| {
        if dialect != Dialect::Contents {
            return;
        }
        scan(
            &stream.record_tree(),
            stream.logical_bytes(),
            &path.display().to_string(),
            &mut checked,
        );
    });
    assert!(checked > 0, "no fixture carries a guideline");
}

/// A connection's two attachment codes are two **narrowing** values, not one word.
///
/// Every code the corpus stores is small enough to fit a byte, so both readings consume the same
/// bytes there and the corpus cannot tell them apart. The difference is a width, and it appears the
/// moment a code needs its wide form: a record carrying one is a byte longer, and a table that
/// pinned the pair to a word reads the wide marker as the value, takes the second code as the low
/// half, and leaves the record with a byte over.
#[test]
fn a_connections_attachment_codes_are_two_values_of_their_own_width() {
    /// A connection whose attachment codes are `a` and `b`, each in whichever form its magnitude
    /// needs.
    fn record(a: u32, b: u32) -> RecordContent {
        let mut run = vec![0x00, 0x02, 0x00, 0x01];
        run.extend_from_slice(&[0; 8]);
        for v in [a, b] {
            if v < 0x80 {
                run.push(v as u8);
            } else {
                run.extend_from_slice(&[0x80 | (v >> 8) as u8, v as u8]);
            }
        }
        run.extend_from_slice(&[0xff; 8]);
        RecordContent {
            rtype: 0x0111,
            schema: 0x0700,
            pieces: vec![crate::field_table::cursor::Piece::Run(run)],
        }
    }

    /// The pair read as the single word the corpus makes it look like.
    const AS_ONE_WORD: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x0111,
        name: "OneWord",
        fields: &[
            Field::new("object_kind", Kind::I16Be),
            Field::new("object_index", Kind::I16Be),
            Field::new("_u0", Kind::I32Be),
            Field::new("_u1", Kind::I32Be),
            Field::new("attachment", Kind::U16Be),
            Field::optional(
                "object_qualifier",
                Kind::Repeat {
                    count: Count::Fixed(4),
                    body: &[Field::new("word", Kind::I16Be)],
                },
            ),
        ],
    };

    // Both codes narrow — the corpus's own shape. The two readings agree byte for byte, which is
    // why the corpus cannot decide between them.
    let narrow = record(2, 3);
    let r = read_strings(&tables::OBJECT_CONNECTION, &narrow, StringFormat::Enhanced);
    assert!(r.exact() && r.complete);
    assert_eq!((r.row.u("_u2"), r.row.u("_u3")), (2, 3));
    assert_eq!(r.row.seq("object_qualifier").len(), 4);
    let one = read_strings(&AS_ONE_WORD, &narrow, StringFormat::Enhanced);
    assert!(one.exact() && one.complete);
    assert_eq!(one.row.u("attachment"), 0x0203);

    // The first code wide: the table still lands on the qualifier, and re-emits the record it read.
    let wide = record(0x0140, 3);
    let r = read_strings(&tables::OBJECT_CONNECTION, &wide, StringFormat::Enhanced);
    assert!(r.exact() && r.complete, "{r:?}");
    assert_eq!((r.row.u("_u2"), r.row.u("_u3")), (0x0140, 3));
    assert_eq!(r.row.seq("object_qualifier").len(), 4);
    assert_eq!(
        write_as(
            &tables::OBJECT_CONNECTION,
            &r.row,
            wide.schema,
            StringFormat::Enhanced
        ),
        wide.pieces
    );

    // The word reading of the same record: a value that is the marker rather than the code, and a
    // byte left unaccounted for.
    let one = read_strings(&AS_ONE_WORD, &wide, StringFormat::Enhanced);
    assert_eq!(one.row.u("attachment"), 0x8140);
    assert_eq!(one.unread, 1);
    assert!(!one.exact());
}

/// A blob field's reference is a **handle**, and both halves of it name something outside the
/// record: the pool says the field is a database field, and the index selects it from the report's
/// own field definitions.
///
/// Read instead as a bare string with three bytes of padding behind it, the record is accounted for
/// just as exactly — the two readings consume the same bytes — so the composite rests entirely on
/// the handle resolving. It does: on every wrapper the index lands on a `Blob`-valued definition
/// whose name is the tail of the reference, and the indices differ from report to report, so a
/// fixed answer could not produce them.
#[test]
fn a_blob_field_wrappers_reference_resolves_into_the_reports_field_pool() {
    let mut checked = 0usize;
    for path in &rpt_test_support::corpus_reports() {
        let Ok(rpt) = crate::Rpt::open(path) else {
            continue;
        };
        let Some(stream) = rpt.stream(&crate::StreamId::Contents) else {
            continue;
        };
        let defs = &rpt.report().data_definition.field_definitions;
        let logical = stream.logical_bytes();
        for root in stream.record_tree() {
            root.walk(&mut |node| {
                if node.rtype != tables::BLOB_FIELD_WRAPPER.rtype {
                    return;
                }
                let file = path.display();
                let r = read_strings(
                    &tables::BLOB_FIELD_WRAPPER,
                    &content_of(node, logical),
                    StringFormat::Enhanced,
                );
                assert!(r.exact() && r.complete, "{file}: {r:?}");
                let (pool, index) = r
                    .row
                    .get("data_source")
                    .and_then(Cell::handle)
                    .unwrap_or_else(|| panic!("{file}: the wrapper names a field"));
                assert_eq!(pool, 0, "{file}: a blob field is a database field");
                let def = index
                    .and_then(|i| defs.get(usize::from(i)))
                    .unwrap_or_else(|| panic!("{file}: the index selects a definition"));
                assert_eq!(
                    def.value_type,
                    crate::model::FieldValueType::Blob,
                    "{file}: the definition the index selects holds a blob"
                );
                assert!(
                    r.row.text("data_source").ends_with(&def.name),
                    "{file}: the reference names the definition the index selects"
                );
                checked += 1;
            });
        }
    }
    assert!(
        checked > 0,
        "no corpus report wraps a picture in a blob field"
    );

    // The same bytes read as a string with padding behind it: equally exact, and unable to say
    // which field the object shows.
    const AS_PADDING: Table = Table {
        fields: &[
            Field::new("picture_object", Kind::Child(0x00ae)),
            Field::new("data_source", Kind::Str),
            Field::new("_handle", Kind::Skip(3)),
            Field::new("natural_width", Kind::VarU32),
            Field::new("natural_height", Kind::VarU32),
            Field::new("blob_stream", Kind::U32Be),
            Field::optional("blob_stream_is_zlib", Kind::I16Be),
            Field::optional("zlib_blob_stream", Kind::U32Be),
        ],
        ..tables::BLOB_FIELD_WRAPPER
    };
    let (n, exact, resolvable) = sweep(&AS_PADDING, Dialect::Contents, |_, r| {
        r.get("data_source").and_then(Cell::handle).is_some()
    });
    assert_eq!(
        (n, exact),
        (checked, checked),
        "the bytes are accounted for either way"
    );
    assert_eq!(
        resolvable, 0,
        "and the padded reading names no field at all"
    );
}

/// The stream a blob field's last-read picture is cached in is named by the record, and the
/// container holds a stream of exactly that name.
///
/// Two ordinals stand side by side and only one of them is live — the flag between them decides
/// which — so nothing inside the record separates them. It is settled outside it: a report holding
/// a cached picture has a `zlibBLOB` stream, and the record's trailing ordinal is its number.
#[test]
fn a_blob_fields_cache_ordinal_names_a_stream_the_container_holds() {
    /// The number in a `zlibBLOB <n>l` stream name.
    fn ordinal_of(name: &str) -> Option<u32> {
        name.rsplit(' ').next()?.trim_end_matches('l').parse().ok()
    }

    let mut named = 0usize;
    for path in &rpt_test_support::corpus_reports() {
        let Ok(rpt) = crate::Rpt::open(path) else {
            continue;
        };
        let cached: Vec<u32> = rpt
            .streams()
            .filter_map(|(id, _)| match id {
                crate::StreamId::ZlibBlob(name) => ordinal_of(name),
                _ => None,
            })
            .collect();
        if cached.is_empty() {
            continue;
        }
        let Some(stream) = rpt.stream(&crate::StreamId::Contents) else {
            continue;
        };
        let logical = stream.logical_bytes();
        for root in stream.record_tree() {
            root.walk(&mut |node| {
                if node.rtype != tables::BLOB_FIELD_WRAPPER.rtype {
                    return;
                }
                let r = read_strings(
                    &tables::BLOB_FIELD_WRAPPER,
                    &content_of(node, logical),
                    StringFormat::Enhanced,
                );
                assert_ne!(
                    r.row.i("blob_stream_is_zlib"),
                    0,
                    "{}: the cached picture is the zlib form",
                    path.display()
                );
                assert!(
                    cached.contains(&r.row.u("zlib_blob_stream")),
                    "{}: the ordinal names none of {cached:?}",
                    path.display()
                );
                named += 1;
            });
        }
    }
    assert!(named > 0, "no corpus report caches a blob field's picture");
}

/// A cross-tab's custom-member collection is as long as its opener states, and every collection in
/// the corpus is empty.
///
/// The count drives the read rather than describing it — the members are taken by number, not
/// looked for — so a collection whose word said one thing and whose contents said another could not
/// be read at all. What the corpus can check is the one case it holds: every opener states none,
/// the collection is closed straight away, and no member record exists anywhere in it. It cannot
/// check the other direction, so a member's own layout is not asserted here and the count's reach
/// rests on the record's own reader.
#[test]
fn a_custom_member_collection_holds_as_many_members_as_its_opener_states() {
    /// A member's own record type, and the record closing the collection.
    const MEMBER: u16 = 0x0180;
    const END: u16 = 0x017f;

    let (mut collections, mut stated, mut members) = (0usize, 0u32, 0usize);
    corpus::for_each_stream(|_, dialect, stream, path| {
        if dialect != Dialect::Contents {
            return;
        }
        let logical = stream.logical_bytes();
        let mut closed_straight_away = true;
        for root in stream.record_tree() {
            root.walk(&mut |node| {
                if node.rtype == tables::CROSSTAB_CUSTOM_MEMBERS.rtype {
                    let r = read_strings(
                        &tables::CROSSTAB_CUSTOM_MEMBERS,
                        &content_of(node, logical),
                        StringFormat::Enhanced,
                    );
                    assert!(r.exact() && r.complete, "{}: {r:?}", path.display());
                    stated += r.row.u("member_count");
                    collections += 1;
                } else if node.rtype == MEMBER {
                    members += 1;
                }
                let types: Vec<u16> = node.children.iter().map(|c| c.rtype).collect();
                for (i, t) in types.iter().enumerate() {
                    if *t == tables::CROSSTAB_CUSTOM_MEMBERS.rtype {
                        closed_straight_away &= types.get(i + 1) == Some(&END);
                    }
                }
            });
        }
        assert!(
            closed_straight_away,
            "{}: a collection stating no members is closed straight away",
            path.display()
        );
    });
    assert!(collections > 10, "{collections} collections in the corpus");
    assert_eq!(
        stated as usize, members,
        "the openers state {stated} members and the corpus holds {members}"
    );
    assert_eq!(
        stated, 0,
        "and the corpus only ever witnesses the empty case"
    );

    // The count the corpus cannot show: built from bytes, it is read as stated and re-emitted as it
    // was found, so the word is the collection's length rather than a flag that happens to be zero.
    let content = RecordContent {
        rtype: tables::CROSSTAB_CUSTOM_MEMBERS.rtype,
        schema: 0x0700,
        pieces: vec![crate::field_table::cursor::Piece::Run(vec![0, 0, 0, 7])],
    };
    let r = read_strings(
        &tables::CROSSTAB_CUSTOM_MEMBERS,
        &content,
        StringFormat::Enhanced,
    );
    assert!(r.exact() && r.complete);
    assert_eq!(r.row.u("member_count"), 7);
    assert_eq!(
        write_as(
            &tables::CROSSTAB_CUSTOM_MEMBERS,
            &r.row,
            0x0700,
            StringFormat::Enhanced
        ),
        content.pieces
    );
}

/// A version ceiling read by record type alone would refuse records that are not that record.
///
/// The ceiling is what refuses a record newer than the newest layout its table knows, and the
/// ceilings that decide it are the report definition's. A type number is per dialect, so reading
/// them by number would apply the report definition's history to whatever unrelated record another
/// stream writes under the same number — and the query engine versions its records in a series
/// beginning above everything the report definition uses, so every one of them would be refused.
///
/// The corpus measures what that costs: the records counted here are ones the sweep accounts for
/// exactly, and the number-keyed reading would decline to look at any of them.
#[test]
fn control_a_ceiling_read_by_number_alone_refuses_another_dialects_records() {
    let mut refused: BTreeMap<(Dialect, u16), usize> = BTreeMap::new();
    corpus::for_each_record(|dialect, node, _, _| {
        if dialect == Dialect::Contents
            || tables::for_record(node.rtype, node.schema, dialect).is_none()
        {
            return;
        }
        if tables::max_supported_schema(node.rtype, Dialect::Contents)
            .is_some_and(|max| node.schema > max)
        {
            *refused.entry((dialect, node.rtype)).or_default() += 1;
        }
    });
    let total: usize = refused.values().sum();
    assert!(
        total > 400,
        "the number-keyed reading would refuse {total} records: {refused:?}"
    );
    // And none of them has a ceiling of its own, so what would refuse them is the other dialect's.
    for (dialect, rtype) in refused.keys() {
        assert!(
            tables::max_supported_schema(*rtype, *dialect).is_none(),
            "0x{rtype:04x} in {dialect:?} has a ceiling of its own"
        );
    }
}
