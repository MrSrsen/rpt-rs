//! Which stream's record vocabulary a record is read in.
//!
//! A record type number is meaningful only per stream — the report definition and the query-engine
//! session both write `0x0003`, for unrelated records — so every lookup keyed on a type number
//! takes a [`Dialect`] alongside it: the record's name, its field table, whether the type is
//! identified at all, and (for the types that have no table) the scan heuristics that stand in.
//!
//! The heuristics live here because they are the same kind of fact as the vocabulary: an
//! observation about what one component writes, not a rule of the format.

/// The high byte every `Contents` record's schema word is observed to carry.
///
/// **A scan heuristic, not a rule of the format.** Nothing validates it: a schema word is an
/// opaque version number ([`RecordNode::schema`]) compared numerically against what a reader
/// supports, and the format permits any value. This byte is constant across the corpus only
/// because one component writes the whole stream at one version, and it earns its place here for
/// one reason — a reader that probes every byte offset for a header needs something that field
/// data rarely imitates, and this is it. It is consulted only where a record type has no field
/// table to declare its children, and a record whose version does not begin with it is lost.
const CONTENTS_SCHEMA_PREFIX: u8 = 0x07;

/// The high byte every `QESession` record's schema word is observed to carry — the same kind of
/// scan heuristic as [`CONTENTS_SCHEMA_PREFIX`], and retired the same way, differing only because
/// the query engine versions its records independently of the report definition. It is an
/// observation about what that component writes, not a rule of the format, so a stream that yields
/// nothing under it is re-read without it. Dropping it does not merely admit more records: field
/// data begins to frame as one, including headers that start inside a field and swallow the rest
/// of it.
const QE_SESSION_SCHEMA_PREFIX: u8 = 0x09;

/// The high byte every `ReportParametersStream` record's schema word is observed to carry — the
/// same kind of scan heuristic as [`CONTENTS_SCHEMA_PREFIX`], and the same value, because the
/// parameter values are written by the report definition's own archive machinery and versioned in
/// its series. It is a separate observation about a separate stream all the same: a component that
/// came to version this stream on its own would move this byte and leave the other where it is.
const REPORT_PARAMETERS_SCHEMA_PREFIX: u8 = 0x07;

/// The default schema of a `Contents` record — the version the report-definition writer opens its
/// archive at, and so the version of a record whose header omits the word. It is the version the
/// stream-header record itself is written at.
///
/// **No record has been read at this default.** The two arms of [`Dialect::default_schema`] are
/// not equally attested: every `Contents` header read states its version, the great majority of
/// them already at this very number, so the value here is inferred from what the writer stamps and
/// never confirmed by a record that omits the word — whereas [`QE_SESSION_DEFAULT_SCHEMA`] is
/// taken by real records. Nor can a fixture settle it: a report written now is written in the
/// current era, and the current era states the word, so only an older file would exercise this
/// path.
///
/// Being wrong here would fail silently — a schema-less `Contents` record would be read at a
/// version it is not written at, and a version decides a field's presence and width. Nothing on
/// the write path leans on it: [`crate::field_table::framing`] states the word for every `Contents`
/// record, so a writer built on it never emits the short form and never consults this constant.
pub(super) const CONTENTS_DEFAULT_SCHEMA: u16 = 0x0700;

/// The default schema of a `QESession` record, the same way round: the query engine versions its
/// records independently of the report definition, and its readers gate every field they have
/// added on a version **above** this one, so this is the floor of that series — the version a
/// record type that has never been revised still carries, and the one its header therefore omits.
///
/// Unlike [`CONTENTS_DEFAULT_SCHEMA`], this default is one records take: the query engine writes
/// its index records (`0x0008`) in the short form, so every report with a table index reads them
/// at this version.
pub(super) const QE_SESSION_DEFAULT_SCHEMA: u16 = 0x0900;

/// States the vocabularies once, as both the [`Dialect`] variants and [`Dialect::ALL`].
///
/// A consumer that must ask its question of every vocabulary needs the list of them, and
/// `#[non_exhaustive]` means no `match` it writes can be made exhaustive by the compiler — so a
/// hand-written list silently loses a vocabulary added later, which is how a whole stream's record
/// names once became unreachable from the CLI. Writing the variants and the list from one statement
/// leaves nowhere to add a variant without adding it to the list.
macro_rules! dialects {
    ($( $(#[$doc:meta])* $variant:ident ),+ $(,)?) => {
        /// Which stream's record vocabulary is being read. Record type numbers are per stream, not
        /// global — the report definition and the query-engine session both use `0x0003`, for
        /// unrelated records — so this selects both the field tables that may declare a child and
        /// the scan heuristic that stands in where none does.
        ///
        /// It is equally the input to every lookup keyed on a record type: a name, a field table,
        /// whether the type is identified at all. A lookup given only the number answers for one
        /// stream and is wrong for the others.
        ///
        /// The variants are the vocabularies that have been read, not a set the format closes: a
        /// stream this reader comes to read in a numbering of its own adds one. Hence
        /// `#[non_exhaustive]`, which asks a downstream match for a wildcard once instead of
        /// breaking it each time one arrives. It relaxes nothing here — a match inside this crate
        /// must still name every dialect, which is where the answer differs per dialect and a
        /// missed one is a decode read in the wrong vocabulary.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Dialect {
            $( $(#[$doc])* $variant, )+
        }

        impl Dialect {
            /// Every record vocabulary, in declaration order — the list a consumer asks its
            /// question of when the question is not about one stream.
            ///
            /// Declared with the variants themselves, so it cannot omit one: there is no way to
            /// add a vocabulary that does not also add it here. That is what a `#[non_exhaustive]`
            /// enum otherwise costs a downstream list, which the compiler will not check.
            pub const ALL: &'static [Dialect] = &[ $( Dialect::$variant, )+ ];
        }
    };
}

dialects! {
    /// The report definition — the `Contents` stream and every subreport's own.
    Contents,
    /// The query-engine session: connections, tables, fields, indexes, links.
    QeSession,
    /// The saved-data `DataSourceManager` catalog: a QE-framed stream written by several
    /// components at once. Their schema words share no prefix, so the flag byte is the whole
    /// filter.
    Catalog,
    /// The saved current parameter values: the `ReportParametersStream`, framed like the report
    /// definition and versioned in the same series, but numbered in a vocabulary of its own —
    /// `0x0030`, `0x0031` and `0x003b` name the records of a parameter's entry here and unrelated
    /// report-definition records there.
    ReportParameters,
}

impl Dialect {
    /// The schema-word high byte a scanned header is required to carry — a heuristic
    /// ([`CONTENTS_SCHEMA_PREFIX`]), and `None` where there is not even one to lean on.
    pub(crate) fn scan_schema_prefix(self) -> Option<u8> {
        match self {
            Dialect::Contents => Some(CONTENTS_SCHEMA_PREFIX),
            Dialect::QeSession => Some(QE_SESSION_SCHEMA_PREFIX),
            Dialect::Catalog => None,
            Dialect::ReportParameters => Some(REPORT_PARAMETERS_SCHEMA_PREFIX),
        }
    }

    /// Whether the scan may take a header with no length field at all — the form an empty record is
    /// framed in, and the narrowest the format has, so the least of it is evidence that it is one.
    ///
    /// Admitting it costs real decodes wherever field data imitates it, so it is answered per
    /// stream: the report definition and the parameter-values stream both frame their end-marker
    /// records this way, and refusing it there loses records that are really present.
    pub(crate) fn scans_empty_records(self) -> bool {
        matches!(self, Dialect::Contents | Dialect::ReportParameters)
    }

    /// The version a record of this stream carries when its header states none.
    ///
    /// A record's header states its schema only when that schema differs from the default the
    /// writing archive was opened at; the reader of a header without the word takes that same
    /// default back. So the answer is a property of the component that wrote the stream, and a
    /// stream written by several components at once has no single one — hence `None`, which
    /// refuses the schema-less form outright rather than picking one of them.
    ///
    /// `None` is equally the answer for a stream whose records all state the word: a default is
    /// only visible in a record that takes it, so a writer that never omits it establishes none.
    /// That is the parameter-values stream, where every record carries its version — the schema-less
    /// form is refused rather than assigned a version nothing has shown.
    ///
    /// The two answers that are not `None` are not equally attested; which is which, and what it
    /// costs, is on [`CONTENTS_DEFAULT_SCHEMA`].
    pub(crate) fn default_schema(self) -> Option<u16> {
        match self {
            Dialect::Contents => Some(CONTENTS_DEFAULT_SCHEMA),
            Dialect::QeSession => Some(QE_SESSION_DEFAULT_SCHEMA),
            Dialect::Catalog | Dialect::ReportParameters => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list names each vocabulary once. It is generated from the variants, so this states what
    /// that generation is for: the four streams this reader reads are all reachable through it, and
    /// none of them twice.
    #[test]
    fn every_vocabulary_is_listed_once() {
        for expected in [
            Dialect::Contents,
            Dialect::QeSession,
            Dialect::Catalog,
            Dialect::ReportParameters,
        ] {
            assert_eq!(
                Dialect::ALL.iter().filter(|&&d| d == expected).count(),
                1,
                "{expected:?} must appear in Dialect::ALL exactly once"
            );
        }
    }
}
