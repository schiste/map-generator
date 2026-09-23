use crate::antimeridian::wrap_longitude;
use crate::math::{asin, cos, sin};

/// Authalic (equal-area) radius of the WGS84 ellipsoid, in metres.
pub const AUTHALIC_RADIUS_M: f64 = 6_371_007.181;

/// Forward map projection from WGS84 degrees to planar metres (y up).
pub trait Projection {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64);
}

/// Which projection to use; `Auto` picks LAEA for regions and Equal Earth for
/// world maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectionChoice {
    #[default]
    Auto,
    Laea,
    EqualEarth,
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
        let denom = 1.0 + sin(phi0) * sin(phi) + cos(phi0) * cos(phi) * cos(dlon);
        // `denom` is 0 only at the antipode of the centre, which never lies
        // inside a region whose centre was chosen from its own extent.
        let k = (2.0 / denom.max(f64::EPSILON)).sqrt();
        let x = AUTHALIC_RADIUS_M * k * cos(phi) * sin(dlon);
        let y = AUTHALIC_RADIUS_M * k * (cos(phi0) * sin(phi) - sin(phi0) * cos(phi) * cos(dlon));
        (x, y)
    }
}

/// Equal Earth pseudo-cylindrical projection (Šavrič, Patterson & Jenny, 2018).
///
/// Its seam is at `lon0 ± 180°`; geometries must go through
/// [`crate::antimeridian::split_at_seam`] first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqualEarth {
    pub lon0: f64,
}

impl Projection for EqualEarth {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        const A1: f64 = 1.340264;
        const A2: f64 = -0.081106;
        const A3: f64 = 0.000893;
        const A4: f64 = 0.003796;
        let d = lon - self.lon0;
        // Seam-split input sits in [-180, 180]; keep ±180 on its own side
        // instead of letting `wrap_longitude` fold +180 onto -180.
        let dlon = if d.abs() <= 180.0 + 1e-6 {
            d.clamp(-180.0, 180.0)
        } else {
            wrap_longitude(d)
        };
        let lam = dlon.to_radians();
        let m = 3f64.sqrt() / 2.0;
        let theta = asin(m * sin(lat.to_radians()));
        let t2 = theta * theta;
        let t6 = t2 * t2 * t2;
        let x = AUTHALIC_RADIUS_M * 2.0 * 3f64.sqrt() * lam * cos(theta)
            / (3.0 * (9.0 * A4 * t6 * t2 + 7.0 * A3 * t6 + 3.0 * A2 * t2 + A1));
        let y = AUTHALIC_RADIUS_M * theta * (A1 + A2 * t2 + t6 * (A3 + A4 * t2));
        (x, y)
    }
}

/// The projection actually used for a map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MapProjection {
    Laea(LambertAzimuthalEqualArea),
    EqualEarth(EqualEarth),
}

impl MapProjection {
    pub fn has_seam(&self) -> bool {
        matches!(self, MapProjection::EqualEarth(_))
    }

    pub fn lon0(&self) -> f64 {
        match self {
            MapProjection::Laea(p) => p.lon0,
            MapProjection::EqualEarth(p) => p.lon0,
        }
    }
}

impl Projection for MapProjection {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        match self {
            MapProjection::Laea(p) => p.project(lon, lat),
            MapProjection::EqualEarth(p) => p.project(lon, lat),
        }
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

    #[test]
    fn equal_earth_symmetry_and_seam() {
        let p = EqualEarth { lon0: 0.0 };
        let (x0, y0) = p.project(0.0, 0.0);
        assert!(x0.abs() < 1e-9 && y0.abs() < 1e-9);
        let (xe, _) = p.project(180.0, 0.0);
        let (xw, _) = p.project(-180.0, 0.0);
        assert!(
            xe > 0.0 && (xe + xw).abs() < 1e-6,
            "±180 must stay on their own sides"
        );
        let (_, yn) = p.project(0.0, 90.0);
        let (_, ys) = p.project(0.0, -90.0);
        assert!((yn + ys).abs() < 1e-6 && yn > 0.0);
    }
}
