//! 3-D chart renderers.
//!
//! The inherently three-dimensional families ([`rpt_model::ChartGraphType::Riser3D`]/`Surface3D`,
//! [`rpt_model::ChartDefinition::is_3d`]) are drawn as extruded boxes over the shared Num+Ord frame:
//! categories run along X and data series recede along Z, projected with the native perspective
//! transform ([`projection`]) — a view-angle rotation then a perspective divide.
//! Faces are emitted as filled [`rpt_pages::DrawOp::Polygon`]s so they render through every backend
//! with no new dependency, exactly like the 2-D renderers, and are painter-sorted back-to-front. The
//! per-chart view angle is not currently decoded, so a single preset is used and the caller records
//! a diagnostic.

mod area3d;
mod projection;
mod riser;
mod scene;
mod surface;

pub(crate) use area3d::area_3d;
pub(crate) use riser::riser_3d;
pub(crate) use surface::surface_3d;
