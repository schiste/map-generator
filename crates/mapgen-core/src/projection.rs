use crate::antimeridian::wrap_longitude;
use crate::math::{asin, cos, ln, pow, sin, tan};

/// Authalic (equal-area) radius of the WGS84 ellipsoid, in metres.
pub const AUTHALIC_RADIUS_M: f64 = 6_371_007.181;

/// Forward map projection from WGS84 degrees to planar metres (y up).
pub trait Projection {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64);
}

/// Which projection to use.
///
/// `Auto` picks Equal Earth for world maps, Albers equal-area conic for
/// regions that are wide in mid-latitudes (the US, Canada, Russia, China...),
/// and Lambert azimuthal equal-area otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ProjectionChoice {
    #[default]
    Auto,
    Laea,
    EqualEarth,
    /// Albers equal-area conic; standard parallels from the region unless given.
    Albers {
        parallels: Option<(f64, f64)>,
    },
    /// Lambert conformal conic; standard parallels from the region unless given.
    Lcc {
        parallels: Option<(f64, f64)>,
    },
    /// Any EPSG coordinate reference system, through PROJ (`proj` feature).
    Epsg(u32),
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

/// Spherical Albers equal-area conic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Albers {
    pub lon0: f64,
    pub lat0: f64,
    pub parallels: (f64, f64),
}

impl Projection for Albers {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        let (p1, p2) = (self.parallels.0.to_radians(), self.parallels.1.to_radians());
        let n = (sin(p1) + sin(p2)) / 2.0;
        let c = cos(p1) * cos(p1) + 2.0 * n * sin(p1);
        let rho = |phi: f64| AUTHALIC_RADIUS_M * (c - 2.0 * n * sin(phi)).max(0.0).sqrt() / n;
        let theta = n * wrap_longitude(lon - self.lon0).to_radians();
        let (r, r0) = (rho(lat.to_radians()), rho(self.lat0.to_radians()));
        (r * sin(theta), r0 - r * cos(theta))
    }
}

/// Spherical Lambert conformal conic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LambertConformalConic {
    pub lon0: f64,
    pub lat0: f64,
    pub parallels: (f64, f64),
}

impl Projection for LambertConformalConic {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        use std::f64::consts::FRAC_PI_4;
        let (p1, p2) = (self.parallels.0.to_radians(), self.parallels.1.to_radians());
        let t = |phi: f64| tan(FRAC_PI_4 + phi / 2.0);
        let n = if (p1 - p2).abs() < 1e-10 {
            sin(p1)
        } else {
            ln(cos(p1) / cos(p2)) / ln(t(p2) / t(p1))
        };
        let f = cos(p1) * pow(t(p1), n) / n;
        // The pole away from the cone's apex is at infinity; stop just short.
        let clamp = |phi: f64| phi.clamp(-89.5_f64.to_radians(), 89.5_f64.to_radians());
        let rho = |phi: f64| AUTHALIC_RADIUS_M * f / pow(t(clamp(phi)), n);
        let theta = n * wrap_longitude(lon - self.lon0).to_radians();
        let (r, r0) = (rho(lat.to_radians()), rho(self.lat0.to_radians()));
        (r * sin(theta), r0 - r * cos(theta))
    }
}

/// Standard parallels by the one-sixth rule: 1/6 of the latitude range in
/// from each edge. Degenerate cases (straddling the equator symmetrically)
/// are nudged so the cone constant stays away from zero.
pub fn one_sixth_parallels(south: f64, north: f64) -> (f64, f64) {
    let d = (north - south) / 6.0;
    let (a, b) = (south + d, north - d);
    if (a + b).abs() < 1.0 {
        (a.max(1.0), b.max(a.max(1.0) + 1.0))
    } else {
        (a, b)
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
#[derive(Debug, Clone)]
pub enum MapProjection {
    Laea(LambertAzimuthalEqualArea),
    EqualEarth(EqualEarth),
    Albers(Albers),
    Lcc(LambertConformalConic),
    #[cfg(feature = "proj")]
    Epsg(crate::epsg::EpsgProjection),
}

impl MapProjection {
    /// True when the projection has a seam at `lon0 ± 180°` that geometries
    /// must be cut along (only Equal Earth among the built-in projections;
    /// regional projections never reach their seam).
    pub fn has_seam(&self) -> bool {
        matches!(self, MapProjection::EqualEarth(_))
    }

    pub fn lon0(&self) -> f64 {
        match self {
            MapProjection::Laea(p) => p.lon0,
            MapProjection::EqualEarth(p) => p.lon0,
            MapProjection::Albers(p) => p.lon0,
            MapProjection::Lcc(p) => p.lon0,
            #[cfg(feature = "proj")]
            MapProjection::Epsg(p) => p.lon0,
        }
    }

    /// Short name: `laea`, `equal-earth`, `albers`, `lcc` or `epsg:<code>`.
    pub fn name(&self) -> String {
        match self {
            MapProjection::Laea(_) => "laea".into(),
            MapProjection::EqualEarth(_) => "equal-earth".into(),
            MapProjection::Albers(_) => "albers".into(),
            MapProjection::Lcc(_) => "lcc".into(),
            #[cfg(feature = "proj")]
            MapProjection::Epsg(p) => format!("epsg:{}", p.code),
        }
    }

    /// Projection centre `[lon, lat]`.
    pub fn center(&self) -> [f64; 2] {
        match self {
            MapProjection::Laea(p) => [p.lon0, p.lat0],
            MapProjection::EqualEarth(p) => [p.lon0, 0.0],
            MapProjection::Albers(p) => [p.lon0, p.lat0],
            MapProjection::Lcc(p) => [p.lon0, p.lat0],
            #[cfg(feature = "proj")]
            MapProjection::Epsg(p) => [p.lon0, 0.0],
        }
    }
}

impl Projection for MapProjection {
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        match self {
            MapProjection::Laea(p) => p.project(lon, lat),
            MapProjection::EqualEarth(p) => p.project(lon, lat),
            MapProjection::Albers(p) => p.project(lon, lat),
            MapProjection::Lcc(p) => p.project(lon, lat),
            #[cfg(feature = "proj")]
            MapProjection::Epsg(p) => p.project(lon, lat),
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
    fn conics_are_centred_and_oriented() {
        let par = one_sixth_parallels(25.0, 49.0);
        assert_eq!(par, (29.0, 45.0));
        let a = Albers {
            lon0: -96.0,
            lat0: 37.0,
            parallels: par,
        };
        let l = LambertConformalConic {
            lon0: -96.0,
            lat0: 37.0,
            parallels: par,
        };
        for p in [&a as &dyn Projection, &l] {
            let (x, y) = p.project(-96.0, 37.0);
            assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
            assert!(p.project(-90.0, 37.0).0 > 0.0, "east is +x");
            assert!(p.project(-96.0, 40.0).1 > 0.0, "north is +y");
        }
    }

    #[test]
    fn albers_preserves_area() {
        // Two 1°×1° cells at the same latitude but different longitudes, and a
        // cell's area vs. the sphere's: equal-area means the ratio is constant.
        let a = Albers {
            lon0: -96.0,
            lat0: 37.0,
            parallels: (29.5, 45.5),
        };
        let cell = |lon: f64, lat: f64| {
            let pts = [
                (lon, lat),
                (lon + 1.0, lat),
                (lon + 1.0, lat + 1.0),
                (lon, lat + 1.0),
            ];
            let p: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| a.project(x, y)).collect();
            let mut s = 0.0;
            for i in 0..4 {
                let (x1, y1) = p[i];
                let (x2, y2) = p[(i + 1) % 4];
                s += x1 * y2 - x2 * y1;
            }
            s.abs() / 2.0
        };
        let sphere = |lat: f64| {
            let r = AUTHALIC_RADIUS_M;
            r * r * 1f64.to_radians() * ((lat + 1.0).to_radians().sin() - lat.to_radians().sin())
        };
        for (lon, lat) in [(-120.0, 30.0), (-80.0, 30.0), (-100.0, 48.0)] {
            let ratio = cell(lon, lat) / sphere(lat);
            assert!((ratio - 1.0).abs() < 2e-3, "ratio {ratio} at {lon},{lat}");
        }
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
