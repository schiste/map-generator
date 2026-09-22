//! Deciding what part of the world the map shows.

use geo::{Area, BooleanOps, BoundingRect};
use geo_types::{coord, LineString, MultiPolygon, Polygon, Rect};

use crate::error::{Error, Result};
use crate::feature::MapFeature;

/// A WGS84 bounding box. `west > east` means it crosses the antimeridian.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoBBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

/// Named frames for continent and region maps.
pub const BBOX_PRESETS: [(&str, GeoBBox); 8] = [
    (
        "europe",
        GeoBBox {
            west: -25.0,
            south: 34.0,
            east: 45.0,
            north: 72.0,
        },
    ),
    (
        "africa",
        GeoBBox {
            west: -26.0,
            south: -37.0,
            east: 60.0,
            north: 38.0,
        },
    ),
    (
        "asia",
        GeoBBox {
            west: 25.0,
            south: -12.0,
            east: 150.0,
            north: 60.0,
        },
    ),
    (
        "north-america",
        GeoBBox {
            west: -170.0,
            south: 5.0,
            east: -50.0,
            north: 75.0,
        },
    ),
    (
        "south-america",
        GeoBBox {
            west: -93.0,
            south: -57.0,
            east: -32.0,
            north: 14.0,
        },
    ),
    (
        "oceania",
        GeoBBox {
            west: 110.0,
            south: -50.0,
            east: -175.0,
            north: 5.0,
        },
    ),
    (
        "middle-east",
        GeoBBox {
            west: 25.0,
            south: 12.0,
            east: 63.0,
            north: 42.0,
        },
    ),
    (
        "caribbean",
        GeoBBox {
            west: -88.0,
            south: 9.0,
            east: -59.0,
            north: 27.0,
        },
    ),
];

impl GeoBBox {
    /// Parses `west,south,east,north` or a preset name such as `europe`.
    pub fn parse(s: &str) -> Result<GeoBBox> {
        let norm = s.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        if let Some((_, b)) = BBOX_PRESETS.iter().find(|(n, _)| *n == norm) {
            return Ok(*b);
        }
        let err = || {
            let names: Vec<&str> = BBOX_PRESETS.iter().map(|(n, _)| *n).collect();
            Error::InvalidBBox(s.to_owned(), names.join(", "))
        };
        let v: Vec<f64> = s
            .split(',')
            .map(|p| p.trim().parse::<f64>())
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| err())?;
        let [west, south, east, north] = v[..] else {
            return Err(err());
        };
        let valid = (-90.0..=90.0).contains(&south)
            && (-90.0..=90.0).contains(&north)
            && south < north
            && (-180.0..=180.0).contains(&west)
            && (-180.0..=180.0).contains(&east)
            && west != east;
        if !valid {
            return Err(err());
        }
        Ok(GeoBBox {
            west,
            south,
            east,
            north,
        })
    }

    /// The box outline, densified so it projects to a smooth curve.
    pub fn outline(&self) -> Polygon<f64> {
        let east = if self.east < self.west {
            self.east + 360.0
        } else {
            self.east
        };
        let steps = 32;
        let mut pts = Vec::with_capacity(4 * steps + 1);
        let lerp = |a: f64, b: f64, t: usize| a + (b - a) * t as f64 / steps as f64;
        for i in 0..steps {
            pts.push(coord! { x: lerp(self.west, east, i), y: self.south });
        }
        for i in 0..steps {
            pts.push(coord! { x: east, y: lerp(self.south, self.north, i) });
        }
        for i in 0..steps {
            pts.push(coord! { x: lerp(east, self.west, i), y: self.north });
        }
        for i in 0..steps {
            pts.push(coord! { x: self.west, y: lerp(self.north, self.south, i) });
        }
        pts.push(pts[0]);
        Polygon::new(LineString(pts), vec![])
    }
}

/// How the map frame is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FrameMode {
    /// Frame the main landmass: parts more than [`CLUSTER_GAP_KM`] away from
    /// the largest cluster (overseas territories, remote islands) are left out.
    #[default]
    Auto,
    /// Frame every feature.
    All,
    /// Frame a fixed box.
    BBox(GeoBBox),
    /// The whole globe (implies Equal Earth).
    World,
}

/// Polygons closer than this belong to the same cluster in [`FrameMode::Auto`].
pub const CLUSTER_GAP_KM: f64 = 500.0;

/// Geometry (WGS84) whose extent defines the frame; `None` for world maps.
pub fn anchor(subject: &[MapFeature], mode: FrameMode) -> Option<MultiPolygon<f64>> {
    let all = || subject.iter().flat_map(|f| f.geometry.0.iter().cloned());
    match mode {
        FrameMode::World => None,
        FrameMode::BBox(b) => Some(MultiPolygon(vec![b.outline()])),
        FrameMode::All => Some(MultiPolygon(all().collect())),
        FrameMode::Auto => {
            let polys: Vec<Polygon<f64>> = all().collect();
            let keep = main_cluster(&polys);
            Some(MultiPolygon(
                keep.into_iter().map(|i| polys[i].clone()).collect(),
            ))
        }
    }
}

/// Indices of the polygons in the cluster with the largest total area, where
/// polygons join a cluster when their bounding boxes are within
/// [`CLUSTER_GAP_KM`] (single linkage).
fn main_cluster(polys: &[Polygon<f64>]) -> Vec<usize> {
    let boxes: Vec<Option<Rect<f64>>> = polys.iter().map(|p| p.bounding_rect()).collect();
    let n = polys.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }

    // Sweep over latitude (not circular) to avoid comparing every pair.
    let mut order: Vec<usize> = (0..n).filter(|&i| boxes[i].is_some()).collect();
    order.sort_by(|&a, &b| {
        let (ra, rb) = (boxes[a].unwrap(), boxes[b].unwrap());
        ra.min().y.total_cmp(&rb.min().y).then(a.cmp(&b))
    });
    let lat_gap_deg = CLUSTER_GAP_KM / 111.2;
    for (oi, &i) in order.iter().enumerate() {
        let ri = boxes[i].unwrap();
        for &j in &order[oi + 1..] {
            let rj = boxes[j].unwrap();
            if rj.min().y > ri.max().y + lat_gap_deg {
                break;
            }
            if bbox_gap_km(ri, rj) <= CLUSTER_GAP_KM {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }

    let mut weight = vec![0.0; n];
    for i in 0..n {
        let r = find(&mut parent, i);
        let lat = boxes[i].map_or(0.0, |b| b.center().y);
        weight[r] += polys[i].unsigned_area() * lat.to_radians().cos().abs();
    }
    let best = (0..n).max_by(|&a, &b| weight[a].total_cmp(&weight[b]).then(b.cmp(&a)));
    match best {
        Some(root) => (0..n).filter(|&i| find(&mut parent, i) == root).collect(),
        None => Vec::new(),
    }
}

/// Approximate distance in km between two lon/lat boxes (0 if they overlap),
/// aware of the antimeridian.
fn bbox_gap_km(a: Rect<f64>, b: Rect<f64>) -> f64 {
    let dlat = (b.min().y - a.max().y).max(a.min().y - b.max().y).max(0.0);
    let dlon = [-360.0, 0.0, 360.0]
        .iter()
        .map(|s| {
            (b.min().x + s - a.max().x)
                .max(a.min().x - (b.max().x + s))
                .max(0.0)
        })
        .fold(f64::INFINITY, f64::min);
    let lat = (a.center().y + b.center().y) / 2.0;
    let kx = 111.2 * lat.to_radians().cos().abs().max(0.05);
    (dlat * 111.2).hypot(dlon * kx)
}

/// Clips a projected geometry to the frame. Geometries entirely inside are
/// returned as-is (so shared borders stay bit-identical).
pub fn clip_to_rect(mp: &MultiPolygon<f64>, frame: Rect<f64>) -> MultiPolygon<f64> {
    let Some(b) = mp.bounding_rect() else {
        return MultiPolygon(vec![]);
    };
    let (fmin, fmax) = (frame.min(), frame.max());
    if b.min().x >= fmin.x && b.min().y >= fmin.y && b.max().x <= fmax.x && b.max().y <= fmax.y {
        return mp.clone();
    }
    if b.max().x < fmin.x || b.min().x > fmax.x || b.max().y < fmin.y || b.min().y > fmax.y {
        return MultiPolygon(vec![]);
    }
    mp.intersection(&frame.to_polygon())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Polygon<f64> {
        Rect::new(coord! { x: x, y: y }, coord! { x: x + s, y: y + s }).to_polygon()
    }

    #[test]
    fn parses_bbox_and_presets() {
        assert_eq!(GeoBBox::parse("-5, 41, 10, 52").unwrap().east, 10.0);
        assert_eq!(GeoBBox::parse("Europe").unwrap(), BBOX_PRESETS[0].1);
        assert!(GeoBBox::parse("north_america").is_ok());
        assert!(GeoBBox::parse("1,2,3").is_err());
        assert!(GeoBBox::parse("0,50,10,40").is_err());
    }

    #[test]
    fn auto_frame_drops_far_islands() {
        // "Mainland" of two touching squares + a nearby island + a far territory.
        let polys = vec![
            square(-2.0, 44.0, 4.0),
            square(2.0, 44.0, 4.0),
            square(8.5, 41.5, 1.0),  // ~200 km away, like Corsica
            square(-54.0, 3.0, 4.5), // French Guiana-sized, far away
        ];
        let mut keep = main_cluster(&polys);
        keep.sort();
        assert_eq!(keep, vec![0, 1, 2]);
    }

    #[test]
    fn clusters_across_antimeridian() {
        let polys = vec![square(178.0, -18.0, 1.5), square(-180.0, -17.0, 1.0)];
        assert_eq!(main_cluster(&polys).len(), 2);
    }
}
