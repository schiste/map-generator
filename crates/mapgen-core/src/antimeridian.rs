//! Antimeridian (±180°) handling.
//!
//! With an azimuthal projection centred on the region, ±180° is *not* a seam:
//! every point is projected relative to the centre (`wrap_longitude(lon - lon0)`),
//! so Fiji or Chukotka stay continuous. The real hazard is choosing that centre.
//! A naive bounding box of Fiji spans -180..180 and puts the centre near
//! Greenwich, on the far side of the planet.
//!
//! Cylindrical and conic projections (planned for world/continent maps) *do*
//! have a seam and will need true ring splitting at the seam longitude.

/// Wraps a longitude difference or absolute longitude into `[-180, 180)`.
pub fn wrap_longitude(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Returns the longitude at the centre of the smallest arc of the circle
/// that contains every input longitude.
///
/// Inputs are in degrees, in any range. The result is in `[-180, 180)`.
/// An empty slice returns `0.0`.
pub fn center_longitude(lons: &[f64]) -> f64 {
    if lons.is_empty() {
        return 0.0;
    }
    // TODO(contributor): this midpoint-of-extremes version is wrong for any
    // region straddling ±180° (Fiji, Russia, Kiribati, Alaska's Aleutians).
    // Replace it with the "largest empty gap" approach: sort the wrapped
    // longitudes, find the widest gap between neighbours (including the
    // wrap-around gap from last back to first + 360), and return the
    // midpoint of the arc that is the complement of that gap.
    // Then remove the #[ignore] on `fiji_centre_is_near_180` below.
    let min = lons.iter().copied().fold(f64::INFINITY, f64::min);
    let max = lons.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    wrap_longitude((min + max) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn angular_distance(a: f64, b: f64) -> f64 {
        wrap_longitude(a - b).abs()
    }

    #[test]
    fn wraps_into_range() {
        assert_eq!(wrap_longitude(190.0), -170.0);
        assert_eq!(wrap_longitude(-190.0), 170.0);
        assert_eq!(wrap_longitude(180.0), -180.0);
        assert_eq!(wrap_longitude(45.0), 45.0);
    }

    #[test]
    fn france_centre() {
        let c = center_longitude(&[-5.0, 8.0, 2.0]);
        assert!(angular_distance(c, 1.5) < 1e-9);
    }

    #[test]
    #[ignore = "enable once center_longitude handles ±180° (see TODO)"]
    fn fiji_centre_is_near_180() {
        let c = center_longitude(&[177.0, 178.5, -179.8, -178.2]);
        assert!(angular_distance(c, 179.4) < 1e-9, "got {c}");
    }
}
