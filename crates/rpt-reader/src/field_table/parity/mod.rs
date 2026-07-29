//! Corpus-wide evidence that every field table still accounts for the bytes it declares.
//!
//! For every record of a tabled type in every report of the corpus the harness reads the record
//! through its table and holds it to two things: the table accounted for the record **exactly** —
//! no unread bytes, no undeclared children — and re-emitting the row reproduces the original bytes.
//! Counts are kept per record type, so a regression names the type that moved rather than the file.
//!
//! The corpus is [`rpt_test_support::corpus_reports`] — every report tree there is, discovered
//! rather than named, because this sweep is where "no record does X" claims come from and such a
//! claim is only as wide as the files it looked at. `RPT_EXTRA_CORPUS` widens it further;
//! `RPT_FIELD_TABLE_REPORT=1` prints the summary even when everything agrees.
//!
//! What the corpus cannot witness on its own is supplied beside it. [`controls`] perturbs one
//! declaration at a time and shows which records move, so the agreement the sweep reports is a
//! measurement rather than a tautology, and builds the records no report writes. [`gates`] measures
//! how much of the version-gated declaration the corpus leaves unwitnessed, then exercises every
//! gate on records built at each version.
//!
//! A record the harness builds carries no header to declare a string form, so every reading and
//! re-emission here names the **enhanced** form — the one the record-tree reader admits — rather
//! than leaving it to be assumed.

mod controls;
mod gates;
mod sweep;

use super::cursor::RecordContent;
use super::cursor::StringFormat;
use super::table::{
    read_strings, write_as, Cell, Count, Ctx, Field, Kind, Presence, Row, Span, Table,
    UNSET_FIELD_INDEX,
};
use super::{content_of, corpus, tables};
use crate::codec::Dialect;
use std::collections::BTreeMap;

/// The counters one record type accumulates over the corpus.
#[derive(Default)]
struct Stats {
    records: usize,
    exact: usize,
    incomplete: usize,
    roundtrip_ok: usize,
    /// files holding a record that the table did not account for exactly
    inexact_files: Vec<String>,
}

/// Every table the sweep can route to, paired with the dialect it belongs to.
///
/// The vocabularies come from [`Dialect::ALL`], which is declared with the variants themselves: a
/// vocabulary added later joins the sweep rather than dropping out of it silently, which would
/// report full coverage of a set the sweep no longer covers.
fn registry() -> impl Iterator<Item = (Dialect, &'static Table)> {
    Dialect::ALL
        .iter()
        .flat_map(|&d| tables::set(d).iter().map(move |t| (d, *t)))
}

/// The tables no corpus record reaches, so that every other one is required to be swept.
///
/// `0x0007` in the query engine's dialect describes a SQL command's parameter; `0x00e9` is one
/// value of a group's specified order, which only a group that declares value ranges is followed
/// by.
const UNREACHED: &[(Dialect, u16)] = &[(Dialect::QeSession, 0x0007), (Dialect::Contents, 0x00e9)];

/// The fewest records the sweep must read to have said anything.
///
/// The committed fixtures alone reach about 82,000; a private corpus only adds to that. The floor
/// is what catches a type that still matches *some* record but has lost most of them — a table
/// that matches none at all is caught by name against [`registry`].
const SWEPT_FLOOR: usize = 75_000;
