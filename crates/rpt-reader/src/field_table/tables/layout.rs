//! Layout: the areas and sections a report is built from, the objects they place, and the
//! records an object carries — its name and bounds, its position.

use super::*;

/// `0x00be ObjectPosition` — Left then Top, both narrowing twips.
///
/// The whole record is two variable-width values, so the second field's position is decided by the
/// first field's magnitude.
pub(crate) const OBJECT_POSITION: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00be,
    name: "ObjectPosition",
    fields: &[
        Field::new("left", Kind::VarU32),
        Field::new("top", Kind::VarU32),
    ],
};

/// `0x00a9 DrawingObject` — the line/box opener.
///
/// Its nested `ObjectName` comes **first** in the content, ahead of every field; the joined runs
/// hide that, since cutting the child out leaves the field run at offset 0. The bottom-right
/// corner is one `TwipPoint`, i.e. two narrowing twips, so its width follows its magnitude; the two
/// words either side of it are whole signed shorts, not byte flags.
pub(crate) const DRAWING_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00a9,
    name: "DrawingObject",
    fields: &[
        Field::new("object_name", Kind::Child(0x009e)),
        Field::new("end_section_index", Kind::I16Be),
        Field::new("right", Kind::VarU32),
        Field::new("bottom", Kind::VarU32),
        Field::new("extend_to_bottom_of_section", Kind::I16Be),
    ],
};

/// `0x0088 GroupAreaFormat` — the group's area-pair options.
///
/// A nested record sits in the middle of the field sequence: two flags and the indent, the child,
/// then the per-page group limit and the formula that can override it. Both `group_indent` and
/// `visible_groups_per_page` are whole `i32`s the engine clamps to zero when negative, not the
/// `u16` each one's low half looks like.
///
/// Only the two flags are unconditional. Everything after them is a trailing cascade of fields a
/// record need not carry — the limit and its formula are one group, and each member of it is
/// stated optional because a record that stops carries neither.
pub(crate) const GROUP_AREA_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0088,
    name: "GroupAreaFormat",
    fields: &[
        Field::new("repeat_group_header", Kind::I16Be),
        Field::new("keep_group_together", Kind::I16Be),
        Field::optional("group_indent", Kind::I32Be),
        Field::optional("xml_definition", Kind::Child(0x0151)),
        Field::optional("visible_groups_per_page", Kind::I32Be),
        Field::optional("new_page_after_formula", Kind::FieldRef),
    ],
};

/// One coordinate of a `TwipRect` — four narrowing twips, so the rectangle is eight bytes only
/// while every edge is under `0x8000` twips (about 22.7 inches).
const TWIP_RECT_EDGE: &[Field] = &[Field::new("v", Kind::VarU32)];

/// `0x009e ObjectName` — an object's size and name, then two nested records and a trailing block.
///
/// Size, then a `TwipRect`, then the name — so both the rectangle's width and the name's decide
/// where the children sit. The engine takes the absolute value of a negative width or height on
/// load; the stored number is what this table reports.
///
/// Everything past the `-1` marker is written only while the record still has content, in four
/// groups: the two nested records, then the repository reference with its two words, then a second
/// string, then a pair of markers. Both strings are variable, so neither the words after the first
/// nor the markers after the second sit at a fixed distance from the name. The groups are a
/// trailing cascade, so each member states its own presence.
///
/// The rectangle's wide `TwipRect` form and a stored second string are both unconfirmed.
pub(crate) const OBJECT_NAME: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x009e,
    name: "ObjectName",
    fields: &[
        Field::new("width", Kind::I32Be),
        Field::new("height", Kind::I32Be),
        Field::new(
            "bounds",
            Kind::Repeat {
                count: Count::Fixed(4),
                body: TWIP_RECT_EDGE,
            },
        ),
        Field::new("name", Kind::Str),
        Field::new("_marker", Kind::I32Be),
        Field::optional("xml_definition", Kind::Child(0x0151)),
        Field::optional("object_marker", Kind::Child(0x0165)),
        // The repository reference and its two words: one group, written together.
        Field::optional("repository_uri", Kind::Str),
        Field::optional("_u0", Kind::U32Be),
        Field::optional("_u1", Kind::U32Be),
        Field::optional("_u2", Kind::Str),
        // The two trailing markers: the last group.
        Field::optional("_u3", Kind::I32Be),
        Field::optional("_u4", Kind::I32Be),
    ],
};
/// `0x009b SectionCodeAreaType` — the area kind and, for a group area, its nesting level.
///
/// The kind is a narrowing enum and the level a whole `u16`, not the low byte of one.
pub(crate) const SECTION_CODE_AREA_TYPE: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x009b,
    name: "SectionCodeAreaType",
    fields: &[
        Field::new("area_type", Kind::VarU16),
        Field::new("group_level", Kind::U16Be),
    ],
};

/// `0x009f FieldObject` — the field object's opener: which field the object shows.
///
/// Like every object opener its nested `ObjectName` comes **first** in the content, ahead of every
/// field; cutting the child out leaves the reference at offset 0, which is why the joined runs hide
/// that it was ever there. The reference is one composite — the display text, the pool it names and
/// the index within it — so the pair of bytes a caller wants is a value of the reference rather than
/// a distance from the record's start.
///
/// Two counts of the object's highlighting rules follow, both `u16` and neither ever smaller than
/// the one before it; the second is the one that sizes the list, and a record that stops after the
/// first states its count once for both. The two are always equal, so which one sizes the list
/// rests on the record's own reader rather than on a confirming example.
///
/// The record then repeats the reference's handle the other way round — index first, then the pool
/// as a narrowing enum — behind a marker that decides whether it is used at all. With the marker
/// zero the handle equals the reference's own and the object takes its field from the reference. A
/// non-zero marker means the handle names the field instead, and the field's own definition then
/// runs to the end of the record; that tail is unread here.
///
/// Everything past the first count is written only while the record still has content, in two
/// groups: the second count, then the marker with the handle behind it. Each member states its own
/// presence, so a record that stops carries the rest of its group not at all.
pub(crate) const FIELD_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x009f,
    name: "FieldObject",
    fields: &[
        Field::new("object_name", Kind::Child(0x009e)),
        Field::new("data_source", Kind::FieldRef),
        Field::new("old_highlight_count", Kind::U16Be),
        Field::optional("highlight_count", Kind::U16Be),
        Field::optional("field_definition_is_stored", Kind::I16Be),
        Field::optional("field_index", Kind::U16Be),
        Field::optional("field_kind", Kind::VarU16),
    ],
};

/// `0x0166 FieldHeadingLink` — the field object a heading is the heading for, named by its object
/// name.
///
/// The record follows the text object it promotes, and the name is the whole of it. The reference
/// is the object's *name* — not a handle into a pool and not an index — so it names its target
/// outright, and no field stands behind it to be reached at a distance.
pub(crate) const FIELD_HEADING_LINK: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0166,
    name: "FieldHeadingLink",
    fields: &[Field::new("field_object_name", Kind::Str)],
};

/// `0x00ae PictureObject` — the opener a static picture, a chart placeholder and a blob-field
/// object all share.
///
/// Like every object opener it writes its nested `ObjectName` first, so its one word sits **past**
/// the child rather than at a fixed distance from the record's start, and the joined runs hide
/// that the child was ever there.
///
/// The image itself is not in this record: a static picture's bytes live in an `Embedding N`
/// storage that the `0x00bd` record beside it names, and a blob field's come from the database, so
/// nothing here varies with the picture's size or format. What the object *is* likewise comes from
/// elsewhere — its name and the wrapper it sits inside — leaving this record the same four bytes
/// whichever of the three it opens.
///
/// The word is written unconditionally and read only while the record still has content, so a
/// record that stops after the child is complete rather than short. It is always zero, which is why
/// it is named for its position and not for a meaning.
pub(crate) const PICTURE_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ae,
    name: "PictureObject",
    fields: &[
        Field::new("object_name", Kind::Child(0x009e)),
        Field::optional("_u0", Kind::U32Be),
    ],
};

/// `0x00b1 BlobFieldWrapper` — the wrapper that turns a picture opener into a blob-field object:
/// the database field the picture comes from, and where the picture last read from it is cached.
///
/// It wraps the `0x00ae` opener rather than standing beside it — the opener is the first thing in
/// its content, ahead of every field of its own — and it is what tells a blob field apart from a
/// static picture, which carries the same opener with no wrapper around it. The run closes with a
/// `0x00b2` end record.
///
/// `data_source` is the ordinary field reference: the display text, the pool it names and the index
/// within it, which resolves to the report's own field definition for the blob column.
///
/// The size is the picture's **natural** size — what the image would occupy unscaled — which is
/// what the object's cropping and scaling are computed against, not its size on the page. It stands
/// at one inch square until a picture has been read from the field, since the image comes from the
/// database and nothing about it is known at design time.
///
/// The two ordinals name the document stream holding the last picture read from the field:
/// `blob_stream` names a `BLOB` stream and `zlib_blob_stream` a `zlibBLOB` one, and the flag between
/// them says which of the two to open. The writer sets the flag and emits the second ordinal on
/// every record it writes; a record that stops after the first ordinal predates the zlib form, and
/// the one ordinal it has stands for both.
pub(crate) const BLOB_FIELD_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00b1,
    name: "BlobFieldWrapper",
    fields: &[
        Field::new("picture_object", Kind::Child(0x00ae)),
        Field::new("data_source", Kind::FieldRef),
        Field::new("natural_width", Kind::VarU32),
        Field::new("natural_height", Kind::VarU32),
        Field::new("blob_stream", Kind::U32Be),
        Field::optional("blob_stream_is_zlib", Kind::I16Be),
        Field::optional("zlib_blob_stream", Kind::U32Be),
    ],
};

/// `0x00bd OleObjectItem` — the 1-based `Embedding N` storage ordinal whose `CONTENTS` stream holds
/// a picture's bytes.
///
/// The trailing pair is optional: a record simply ends before it, and re-emitting one that did
/// reproduces the short form.
pub(crate) const OLE_OBJECT_ITEM: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00bd,
    name: "OleObjectItem",
    fields: &[
        Field::new("embedding_ordinal", Kind::U32Be),
        Field::new("_u0", Kind::Skip(4)),
        Field::new("_u1", Kind::Skip(2)),
        Field::new("_u2", Kind::Skip(2)),
    ],
};

/// `0x00a5 TextObjectContainer` — the text object's opener.
///
/// Like every object opener it writes its nested `ObjectName` **first**, ahead of every field of its
/// own: the record's sixteen field bytes are the last sixteen of its content and the child fills
/// everything before them.
///
/// `paragraph_count` states how many `0x00c0` paragraphs the object holds — the engine sizes the
/// object from it rather than counting the records that follow it. Summed over a report's text
/// objects it is exactly the number of `0x00c0` records the report stores.
///
/// `is_field_heading` promotes the object to a field heading, and a `0x0166` naming the field object
/// it heads follows only where it is set; per report the two counts agree.
///
/// The last two fields are a trailing cascade guarded on the record still having content, so a
/// record that stops after `paragraph_count` carries neither the word after it nor the heading flag.
/// That is what makes the flag the end of a sequence rather than a byte at a fixed distance: read at
/// one, a short record answers with whatever byte happens to sit there.
pub(crate) const TEXT_OBJECT_CONTAINER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00a5,
    name: "TextObjectContainer",
    fields: &[
        Field::new("object_name", Kind::Child(0x009e)),
        Field::new("_u0", Kind::I16Be),
        Field::new("_u1", Kind::I16Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("_u3", Kind::U16Be),
        Field::new("paragraph_count", Kind::U16Be),
        Field::optional("_u4", Kind::U32Be),
        Field::optional("is_field_heading", Kind::I16Be),
    ],
};

/// `0x00c0 TextObjectFormat` — one paragraph of a text object: its indents, its horizontal
/// alignment, its two element counts, and its line spacing.
///
/// The two words after the alignment are counts: the engine reads `element_count` paragraph
/// elements and `run_count` text runs out of the records that follow this one, so they state the
/// paragraph's size rather than padding it. An alignment of `0` is read back as `1`.
pub(crate) const TEXT_OBJECT_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00c0,
    name: "TextObjectFormat",
    fields: &[
        Field::new("left_indent", Kind::I32Be),
        Field::new("right_indent", Kind::I32Be),
        Field::new("first_line_indent", Kind::I32Be),
        Field::new("horizontal_alignment", Kind::VarU16),
        Field::new("element_count", Kind::U16Be),
        Field::new("run_count", Kind::U16Be),
        Field::new("line_spacing_type", Kind::VarU16),
        Field::new("line_spacing", Kind::U32Be),
        Field::new("_u0", Kind::VarU16),
    ],
};

/// `0x00c2 TextObject` — one literal-text run of a paragraph: the run's text, then the rigid extra
/// advance the engine adds after each of its characters, in twips.
///
/// The text is the record's own field and needs no finding: it is the first thing written, at the
/// stated length, whatever it holds — a run whose text is empty stores an empty string rather than
/// nothing, and one whose text holds a control byte stores it verbatim.
///
/// The spacing is written only while the record still has content, so a run saved without one ends
/// after its text and re-emits just as short. Nothing else is the run's: its font is the `0x08`
/// record after it, and a `0x00c3` closes it.
pub(crate) const TEXT_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00c2,
    name: "TextObject",
    fields: &[
        Field::new("text", Kind::Str),
        Field::optional("character_spacing", Kind::I32Be),
    ],
};

/// `0x00c4 TextEmbeddedField` — one field run of a paragraph: the field, formula or parameter the
/// run shows in place of literal text, and a `0x00c5` closes it.
///
/// The reference is the same composite a field object opens with — the display text, the pool it
/// names and the index within that pool — and it is the record's first field, so it needs no
/// finding. Both halves of the handle are values of the reference rather than bytes at a distance
/// from it, which matters for the index in particular: a special field's own code is that index,
/// and reading it as the byte beside the pool takes only its low half.
///
/// The spacing after it is the run's rigid extra advance per character, in twips — the same field a
/// literal run stores, in the same slot of the element the two record types share, under the same
/// guard.
///
/// The record then repeats the reference's handle the other way round — index first, then the pool
/// as a narrowing enum — behind a marker that decides whether it is used at all. With the marker
/// zero the run takes its field from the reference; a non-zero one means the handle names the field
/// instead, and the field's own definition then runs to the end of the record, which nothing here
/// reads.
///
/// Everything past the first word is written only while the record still has content, in two
/// groups: the spacing, then the marker with the handle behind it. Each member states its own
/// presence, so a record that stops carries the rest of its group not at all.
pub(crate) const TEXT_EMBEDDED_FIELD: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00c4,
    name: "TextEmbeddedField",
    fields: &[
        Field::new("data_source", Kind::FieldRef),
        Field::new("_u0", Kind::I32Be),
        Field::optional("character_spacing", Kind::I32Be),
        Field::optional("field_definition_is_stored", Kind::I16Be),
        Field::optional("field_index", Kind::U16Be),
        Field::optional("field_kind", Kind::VarU16),
    ],
};

/// `0x008a Area` — the layout marker that opens an area.
///
/// `section_count` is how many `0x008c` sections follow before the area's `0x008b` end marker: the
/// engine loops on it rather than scanning, so it is the record's own statement of the area's size.
/// The name, the `-1` marker and the nested `XmlDefinition` are one guarded group, and the trailing
/// word its own — a record may end before either.
pub(crate) const AREA: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x008a,
    name: "Area",
    fields: &[
        Field::new("_u0", Kind::I32Be),
        Field::new("section_count", Kind::U16Be),
        Field::new("name", Kind::Str),
        Field::new("_marker", Kind::I32Be),
        Field::new("xml_definition", Kind::Child(0x0151)),
        Field::new("_u1", Kind::I16Be),
    ],
};

/// `0x008c Section` — the layout marker that opens a section within the current area, carrying its
/// height and name.
///
/// `object_count` is how many report objects the section holds; the engine sizes its object array
/// from it before reading them. The `-1` marker after the name is the same one the area and the
/// object-name record store.
pub(crate) const SECTION: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x008c,
    name: "Section",
    fields: &[
        Field::new("height", Kind::I32Be),
        Field::new("_u0", Kind::I16Be),
        Field::new("object_count", Kind::U16Be),
        Field::new("name", Kind::Str),
        Field::new("_marker", Kind::I32Be),
        Field::new("xml_definition", Kind::Child(0x0151)),
        Field::new("_u1", Kind::I16Be),
    ],
};

/// `0x00a3 SubreportObject` — the subreport placeholder opener.
///
/// Like every object opener it writes its nested `ObjectName` first; `subdocument_index` names the
/// `Subdocument N` storage holding the subreport's own streams.
///
/// The two formulas are field references, so a subreport with either one set lengthens the record
/// and moves the word after them; an unset reference is eight bytes of zeros and `ff`s that a fixed
/// run swallows unchanged.
pub(crate) const SUBREPORT_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00a3,
    name: "SubreportObject",
    fields: &[
        Field::new("object_name", Kind::Child(0x009e)),
        Field::new("subdocument_index", Kind::U32Be),
        // Non-zero when a prompting-variable link record follows the object.
        Field::new("_has_link_object", Kind::I16Be),
        Field::new("on_demand", Kind::I16Be),
        Field::new("on_demand_caption_formula", Kind::FieldRef),
        Field::new("tab_text_formula", Kind::FieldRef),
        Field::optional("_u0", Kind::I16Be),
    ],
};

/// Whether a subreport link stores the second `(kind, index)` handle: the engine reads it only on a
/// record whose flag word is zero.
fn linked_field_stored(c: &Ctx<'_>) -> bool {
    c.row.i("link_flag") == 0
}

/// `0x0106 SubreportLink` — one link feeding a main-report field into a subreport parameter.
///
/// The link is stored as two `(kind, index)` field handles: the first follows the main-report field
/// name and re-states it as a handle into the **main** report's own field pools, the second names
/// the **subreport** field the link feeds, as a handle into the *subreport's* pools. In both,
/// `kind` selects the pool (`0` database field, `1` formula, `5` parameter) and `index` the entry
/// within it. The second handle is **gated on `link_flag`**: the engine reads it only when the flag
/// is zero, and a record whose flag is set ends after the flag.
pub(crate) const SUBREPORT_LINK: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0106,
    name: "SubreportLink",
    fields: &[
        Field::new("parameter_index", Kind::U16Be),
        Field::new("main_field", Kind::Str),
        Field::new("main_field_kind", Kind::VarU16),
        Field::new("main_field_index", Kind::U16Be),
        Field::new("link_flag", Kind::I16Be),
        Field::when("subreport_field_kind", Kind::VarU16, linked_field_stored),
        Field::when("subreport_field_index", Kind::U16Be, linked_field_stored),
    ],
};
