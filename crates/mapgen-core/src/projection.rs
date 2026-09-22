use crate::antimeridian::{center_longitude, wrap_longitude};
use crate::feature::MapFeature;

/// Authalic (equal-area) radius of the WGS84 ellipsoid, in metres.
pub const AUTHALIC_RADIUS_M: f64 = 6_371_007.181;

/// Forward map projection from WGS84 degrees to planar metres.
pub trait Projection {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64);
}

/// Spherical Lambert Azimuthal Equal-Area projection.
///
/// Centred on the region being drawn, it preserves area everywhere and keeps
/// shape distortion low near the centre, which makes it a sane default for any
/// country or subdivision without a per-region EPSG lookup table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LambertAzimuthalEqualArea {
    pub lon0: f64,
    pub lat0: f64,
}

impl Projection for LambertAzimuthalEqualArea {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        let dlon = wrap_longitude(lon - self.lon0).to_radians();
        let (phi, phi0) = (lat.to_radians(), self.lat0.to_radians());
        let denom = 1.0 + phi0.sin() * phi.sin() + phi0.cos() * phi.cos() * dlon.cos();
        // `denom` is 0 only at the antipode of the centre, which never lies
        // inside a region whose centre was chosen from its own extent.
        let k = (2.0 / denom.max(f64::EPSILON)).sqrt();
        let x = AUTHALIC_RADIUS_M * k * phi.cos() * dlon.sin();
        let y =
            AUTHALIC_RADIUS_M * k * (phi0.cos() * phi.sin() - phi0.sin() * phi.cos() * dlon.cos());
        (x, y)
    }
}

/// Picks a LAEA projection centred on the given features.
pub fn select_projection(features: &[MapFeature]) -> LambertAzimuthalEqualArea {
    let mut lons = Vec::new();
    let (mut min_lat, mut max_lat) = (f64::INFINITY, f64::NEG_INFINITY);
    for f in features {
        for poly in &f.geometry {
            for c in poly.exterior() {
                lons.push(c.x);
                min_lat = min_lat.min(c.y);
                max_lat = max_lat.max(c.y);
            }
        }
    }
    LambertAzimuthalEqualArea {
        lon0: center_longitude(&lons),
        lat0: (min_lat + max_lat) / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_maps_to_origin() {
        let p = LambertAzimuthalEqualArea {
            lon0: 2.35,
            lat0: 48.85,
        };
        let (x, y) = p.project(2.35, 48.85);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
    }

    #[test]
    fn east_is_positive_x_north_is_positive_y() {
        let p = LambertAzimuthalEqualArea {
            lon0: 0.0,
            lat0: 0.0,
        };
        assert!(p.project(1.0, 0.0).0 > 0.0);
        assert!(p.project(0.0, 1.0).1 > 0.0);
    }

    #[test]
    fn continuous_across_antimeridian() {
        let p = LambertAzimuthalEqualArea {
            lon0: 179.0,
            lat0: -17.0,
        };
        let (xa, _) = p.project(179.9, -17.0);
        let (xb, _) = p.project(-179.9, -17.0);
        // 0.2° of longitude apart, so ~20 km, not ~40 000 km.
        assert!((xb - xa).abs() < 30_000.0);
    }
}
