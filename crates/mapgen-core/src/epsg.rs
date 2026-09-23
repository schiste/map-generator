//! Any EPSG coordinate reference system through PROJ (cargo feature `proj`).
//!
//! PROJ computes with the platform's math library, so maps in an EPSG
//! projection are deterministic on a given platform but, unlike the built-in
//! projections, not guaranteed bit-identical across platforms.

use std::fmt;
use std::rc::Rc;

use proj::Proj;

use crate::error::{Error, Result};
use crate::projection::Projection;

#[derive(Clone)]
pub struct EpsgProjection {
    pub code: u32,
    /// Longitude used for antimeridian handling (the region's centre).
    pub lon0: f64,
    proj: Rc<Proj>,
}

impl EpsgProjection {
    pub fn new(code: u32, lon0: f64) -> Result<Self> {
        let proj = Proj::new_known_crs("EPSG:4326", &format!("EPSG:{code}"), None)
            .map_err(|e| Error::Projection(format!("EPSG:{code}: {e}")))?;
        Ok(EpsgProjection {
            code,
            lon0,
            proj: Rc::new(proj),
        })
    }
}

impl fmt::Debug for EpsgProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EpsgProjection(EPSG:{})", self.code)
    }
}

impl Projection for EpsgProjection {
    /// Points PROJ cannot transform come back as NaN; the pipeline drops the
    /// rings that contain them.
    fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        self.proj
            .convert((lon, lat))
            .unwrap_or((f64::NAN, f64::NAN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_to_conus_albers() {
        // EPSG:5070 (NAD83 / Conus Albers) puts 96°W 23°N at the origin.
        let p = EpsgProjection::new(5070, -96.0).unwrap();
        let (x, y) = p.project(-96.0, 23.0);
        assert!(x.abs() < 1.0 && y.abs() < 1.0, "({x}, {y})");
        assert!(p.project(-90.0, 40.0).0 > 0.0);
    }

    #[test]
    fn unknown_code_is_an_error() {
        assert!(EpsgProjection::new(999_999, 0.0).is_err());
    }
}
