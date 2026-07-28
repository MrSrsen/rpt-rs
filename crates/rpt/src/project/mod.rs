//! Projection between the record substrate and the semantic [`crate::model`].
//!
//! [`raise`] reads the substrate into a [`Report`]. Raising is total: it never drops a record —
//! anything not yet modelled is kept verbatim in the substrate for round-trip and is still
//! reachable via the on-demand record DOM ([`build_record_dom`]) and inventory ([`build_inventory`]).

pub(crate) mod raise;

pub(crate) use raise::{
    build_inventory, build_record_dom, parse_report_parameters, raise, raise_subreports,
    resolve_sf_handle, subreport_links,
};
