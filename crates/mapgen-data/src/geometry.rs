use geo_types::{Geometry, MultiPolygon};

/// Keeps only the areal parts of a geometry. Points and lines are dropped
/// because the pipeline renders filled regions.
pub(crate) fn into_multipolygon(g: Geometry<f64>) -> MultiPolygon<f64> {
    match g {
        Geometry::Polygon(p) => MultiPolygon(vec![p]),
        Geometry::MultiPolygon(mp) => mp,
        Geometry::GeometryCollection(gc) => MultiPolygon(
            gc.0.into_iter()
                .flat_map(|g| into_multipolygon(g).0)
                .collect(),
        ),
        _ => MultiPolygon(vec![]),
    }
}
