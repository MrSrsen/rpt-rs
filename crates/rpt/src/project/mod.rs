//! Projection between the record substrate and the semantic [`crate::model`].
//!
//! [`raise`] reads the substrate into a [`Report`]. Raising is total: it never drops a record —
//! anything not yet modelled is still counted in the report's record inventory and kept verbatim
//! in the substrate for round-trip.

mod raise;

pub(crate) use raise::{
    parse_report_parameters, raise, resolve_sf_handle, subreport_link_bindings, subreport_links,
    subreport_param_index_names,
};
