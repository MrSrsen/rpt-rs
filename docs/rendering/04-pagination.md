# Section-break & pagination controls

`rpt-layout` paginates band-by-band (a band never splits mid-section: one that would overflow the body moves whole to a
new page). On top of that it honours the section/group format flags decoded onto `SectionAreaFormatBase` /
`GroupAreaFormat`:

- **New Page Before / After** (`new_page_before` / `new_page_after`) — start a fresh page before a band, or after it
  (deferred to the next flow band so a trailing break leaves no blank page). Applied on both the single-column band path
  and the multi-column detail path; a break at the top of a fresh page is skipped so no leading blank page appears.
- **Keep Group Together** (`GroupAreaFormat.keep_group_together`) — before emitting a group header, the group's subtree
  (header + details + footer, nested subgroups recursively) is pre-measured from static design heights; if it would
  split across the current page boundary but fits on a page by itself, the whole group moves to a fresh page. A group
  taller than a full page is left to paginate naturally. The pre-measure deliberately ignores can-grow growth —
  resolving it would re-fire `WhilePrintingRecords` variable writes.
- **Records per page** (`AreaFormat.visible_records_per_page`, on the Detail area) — a page carries at most N *visible*
  detail records. It is a hard break, not a ceiling that height also has to allow: the page breaks after the Nth record
  even when the body has room for more. The count runs continuously across group boundaries and resets only at a page
  top, and a record suppressed (statically, conditionally, or as a blank section) does not count. The break falls before
  the next record *and* before the next group's header, so a page with no quota left starts the following group on a
  fresh page instead of orphaning its header.
- **Groups per page** (`GroupAreaFormat.visible_groups_per_page`, on a group-header area) — a page carries at most N
  group instances at that level. A group carried onto a page by a break inside it counts against that page's quota, so a
  page opened mid-group has one slot already taken; the counter is per group level.
- Both limits store `0` when the designer's checkbox is off, which means *no limit* — as in every corpus report that
  does not set one. They count bands rather than measure them, so they are disabled while formatting an inline subreport
  (where the whole report flows onto one tall page).
- **Print at Bottom of Page** (`print_at_bottom_of_page`) — pin a group/report footer against the body bottom (above the
  page footer), then treat the page as full so the next band starts fresh.
- **Reset Page Number After** (`reset_page_number_after`) — restart the page-number counter at the next page top, giving
  per-group page numbering. `PageNumber` / `Page N of M` follow the reset; `TotalPageCount` stays the whole-document
  count (a per-section total would need a second pass).
- **`TotalPageCount` / `PageNofM`** are a forward reference the single layout pass cannot know up front. Each placed run
  is recorded and rewritten with the true final page count once pagination completes (its stored advance recomputed so a
  right/centre-aligned footer re-anchors). The displayed page number is preserved (it already honoured any reset).
- **Underlay Following Sections** (`SectionAreaFormatBase.underlay_section`, SDK `EnableUnderlaySection`) — an underlay
  band is a background for the sections that follow it: after it emits, the flow cursor stays at the band's top so the
  next band overlays it in the same vertical space rather than being pushed below. That span is **bounded**: it ends at
  the band's *companion*, which is not itself underlaid — a group header's span closes at the group footer of the same
  level, a report header's at the report footer — and closing it drops the cursor to the underlay's own bottom, so a
  footer shorter than the watermark it backs prints *after* it rather than on top of it. A page header has no companion
  the flow can reach (its page footer is pinned to the body bottom), so it backs the whole page. A span left open at a
  page turn is dropped: an underlay is drawn once, on the page it lands on.
- **Suppress If Blank Section** (`SectionFormat.suppress_if_blank`, SDK `EnableSuppressIfBlank`) — a section whose
  objects all resolve to no visible output is dropped and reserves no vertical space, so it neither renders nor pushes
  following bands (and cannot force an extra page). A section is "blank" when every object is suppressed or is an empty
  text/field/heading with no drawn border and no visible (opaque, non-white) fill; any shape, picture, chart, cross-tab,
  blob, subreport, or non-empty text keeps it. Its formulas still evaluate (needed to decide blankness), so their
  record-time side effects fire.
- **Hierarchical grouping** (`Group.hierarchical_options`, Report ▸ Hierarchical Grouping Options) — a group whose
  instances form a parent/child tree over their own records (an org chart's `manager_id → employee_id`). `rpt-data`
  rearranges that level's instances into the tree: each instance's `InstanceIDField` value identifies it, its
  `ParentIDField` value names its parent, and the children hang off it as `GroupInstance::hierarchy_children` — the same
  group level, so they print through the same header/footer bands. The result is a depth-first pre-order walk from the
  roots (parent null, or naming no instance), siblings keeping the group's own sort order. `rpt-layout` then brackets
  each instance's whole subtree between its header and footer, and shifts every object in its bands right by
  `depth × GroupIndent` — a pure translation of header, details *and* footer, so a full-width line moves past the right
  margin and clips rather than shrinking. The offset composes with a multi-column detail's column offset. A malformed
  hierarchy lays out rather than failing: an orphan and a self-parenting instance are roots, a parent cycle terminates
  (its entry point becomes a root), and every instance prints exactly once.
- **Group-footer level order.** Group footers are stored innermost-first in the report (the canonical
  `GH1..GHN, Detail, GFN..GF1` area order) while group headers are outermost-first; the footer list is reversed at
  collect time so both index by group level.
- **Band record context.** A non-detail band still resolves its field/formula objects against a "current record", the
  way Crystal does — otherwise a header/footer `{table.field}` (or a formula reading one) would evaluate to `Null` and
  render blank. Each band picks its record: report header → the report's first record, report footer → its last, group
  header → the group's first record, group footer → its last, page header/footer → the record straddling the page
  boundary (tracked as detail rows print). Summary/`GroupName`/special objects don't need a row and resolve from the
  print state, the report's stored facts, or the as-of instant regardless.
- **Embedded references in a text object** are carried by the **run**, not by the flattened text: a run with a
  `field_ref` holds the engine's placeholder rendering of the reference, which is `{alias.field}` for a database field
  but the bare `PrintDate` for a special and `GroupName ({cond})` for a group name. The renderer therefore walks the run
  tree and replaces each field run wholesale — substituting on the flattened string would print a special's own name and
  leave a group-name placeholder's `GroupName (…)` wrapper on the page. `GroupName ({cond})` resolves the level whose
  condition field it names, which need not be the nearest enclosing group, and prints at **that group's granularity**: a
  date group's key is the period's start date, so a month-granular period (monthly / quarterly / semi-annually) drops
  the day for `M/YYYY` and an annual one prints the bare year, while the four day-granular periods (daily / weekly /
  bi-weekly / semi-monthly) print the full date — on the 1st of a month like any other day. The grain comes from the
  group's decoded condition, never from the key, which cannot distinguish the periods that share it.
  `{?Param}` resolves through the same expression evaluation a *placed* parameter field object uses, so the two can
  never disagree about a parameter's value. This context depends on the area being classified correctly — the area kind
  comes from the band-marker record (`0x8d`–`0x99`), not the area name, so a group area a report tool named after its
  group field (e.g.
  `nameHeader`) still lays out as a group band.

Within a band, box objects are emitted before the band's text/field/image ops so a shading box underlays the row content
even when stored after it, and a section-spanning box (its design bottom reaching most of the section height) grows with
a can-grow band so its fill/frame tracks the actual rendered row height.

The conditional variants of these *pagination* flags are decoded but not yet applied. Section condition formulas are
evaluated — `Section_Visibility` and the colour conditions are — so what is missing is the pagination subset, not the
plumbing under it.

---

← [The Page IR (`rpt-pages`)](03-page-ir.md) · [Index](README.md) ·
**Next:** [Format resolution](05-format-resolution.md) →
