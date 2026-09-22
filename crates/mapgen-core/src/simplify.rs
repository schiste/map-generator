use geo::SimplifyVwPreserve;
use geo_types::MultiPolygon;

/// Converts a tolerance in output pixels into a Visvalingam–Whyatt area
/// threshold in projected units² (VW removes vertices whose effective
/// triangle area is below the threshold).
pub fn vw_epsilon(tolerance_px: f64, pixels_per_unit: f64) -> f64 {
    let t = tolerance_px / pixels_per_unit;
    t * t
}

/// Topology-preserving Visvalingam–Whyatt simplification of one geometry.
///
/// This keeps each polygon valid (no self-intersections), but it does not yet
/// guarantee that a border shared by two features is simplified identically on
/// both sides; see `docs/architecture.md` ("Shared-border topology").
pub fn simplify(geometry: &MultiPolygon<f64>, epsilon: f64) -> MultiPolygon<f64> {
    geometry.simplify_vw_preserve(&epsilon)
}
