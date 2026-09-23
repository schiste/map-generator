//! Label placement, in output pixels.
//!
//! For each region, largest first:
//! 1. straight text at the polygon's *pole of inaccessibility* (the interior
//!    point farthest from the edges, "polylabel"), then at nearby alternative
//!    positions, at 100 %, 85 % and 70 % size, as long as the text box stays
//!    inside the region and clear of labels already placed;
//! 2. for long, thin regions (Chile, Norway…), text curved along the shape's
//!    centreline;
//! 3. for small regions, text outside the region joined by a leader line,
//!    only where the text covers no other region.
//!
//! Labels that fit nowhere are dropped. Everything is deterministic: ties are
//! broken by input order.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use geo::{Area, BoundingRect, Centroid, Contains};
use geo_types::{coord, Coord, LineString, MultiPolygon, Point, Polygon};
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{RTree, AABB};

use crate::math::{atan2, cos, sin};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelOptions {
    /// Nominal font size in pixels.
    pub size: f64,
    /// Smallest allowed size, as a fraction of `size`.
    pub min_scale: f64,
    pub leaders: bool,
    pub curved: bool,
}

impl Default for LabelOptions {
    fn default() -> Self {
        LabelOptions {
            size: 11.0,
            min_scale: 0.7,
            leaders: true,
            curved: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LabelShape {
    Straight,
    /// Text outside its region; the line runs from `anchor` to the text box.
    Leader {
        anchor: (f64, f64),
        end: (f64, f64),
    },
    /// Text along a path (pixel coordinates, in reading order).
    Curved {
        path: Vec<(f64, f64)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// Index of the labelled feature.
    pub feature: usize,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub shape: LabelShape,
}

/// Average glyph advance of a sans-serif font, as a fraction of the size.
const CHAR_WIDTH: f64 = 0.56;

/// Advance of one character, as a fraction of the size: East Asian wide
/// characters (CJK ideographs, kana, Hangul, fullwidth forms) are square.
fn char_width(c: char) -> f64 {
    let wide = matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD);
    if wide {
        1.0
    } else {
        CHAR_WIDTH
    }
}

fn text_width(text: &str, size: f64) -> f64 {
    size * text.chars().map(char_width).sum::<f64>()
}

/// Places labels for `regions` (`(name, geometry in pixels)`); leader labels
/// stay within `bounds` (`[min_x, min_y, max_x, max_y]` in pixels).
pub fn place_labels(
    regions: &[(&str, &MultiPolygon<f64>)],
    bounds: [f64; 4],
    opts: &LabelOptions,
) -> Vec<Label> {
    place_labels_around(regions, bounds, opts, &[])
}

/// [`place_labels`], keeping clear of `reserved` boxes (`[min_x, min_y,
/// max_x, max_y]` in pixels), such as insets drawn over the map.
pub fn place_labels_around(
    regions: &[(&str, &MultiPolygon<f64>)],
    bounds: [f64; 4],
    opts: &LabelOptions,
    reserved: &[[f64; 4]],
) -> Vec<Label> {
    // Largest part of each region, and every part for obstacle tests.
    let mains: Vec<Option<&Polygon<f64>>> = regions
        .iter()
        .map(|(_, g)| {
            g.0.iter()
                .max_by(|a, b| a.unsigned_area().total_cmp(&b.unsigned_area()))
        })
        .collect();
    let mut order: Vec<usize> = (0..regions.len()).filter(|&i| mains[i].is_some()).collect();
    let area = |i: usize| mains[i].map_or(0.0, |p| p.unsigned_area());
    order.sort_by(|&a, &b| area(b).total_cmp(&area(a)).then(a.cmp(&b)));

    let parts: Vec<&Polygon<f64>> = regions.iter().flat_map(|(_, g)| g.0.iter()).collect();
    let obstacles: RTree<GeomWithData<Rectangle<[f64; 2]>, usize>> = RTree::bulk_load(
        parts
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.bounding_rect().map(|r| (i, r)))
            .map(|(i, r)| {
                GeomWithData::new(
                    Rectangle::from_corners([r.min().x, r.min().y], [r.max().x, r.max().y]),
                    i,
                )
            })
            .collect(),
    );
    let covers_land = |x: f64, y: f64| {
        let pt = Point::new(x, y);
        obstacles
            .locate_all_at_point(&[x, y])
            .any(|hit| parts[hit.data].contains(&pt))
    };

    let mut placed: RTree<Rectangle<[f64; 2]>> = RTree::bulk_load(
        reserved
            .iter()
            .map(|r| Rectangle::from_corners([r[0], r[1]], [r[2], r[3]]))
            .collect(),
    );
    let mut labels = Vec::new();
    let min_scale = opts.min_scale.clamp(0.1, 1.0);
    let sizes: Vec<f64> = [1.0, 0.85, 0.7]
        .into_iter()
        .filter(|s| *s >= min_scale - 1e-9)
        .map(|s| s * opts.size)
        .collect();
    let smallest = sizes[sizes.len() - 1];

    for i in order {
        let (text, _) = regions[i];
        let poly = mains[i].expect("filtered");
        if text.trim().is_empty() {
            continue;
        }
        let (pole, radius) = polylabel(poly, 0.5);

        // At each size, straight text first, then curved: a long thin region
        // gets curved text at full size rather than straight text shrunk.
        let mut found = None;
        for size in &sizes {
            let size = std::slice::from_ref(size);
            found = straight(i, text, poly, pole, size, &placed).or_else(|| {
                opts.curved
                    .then(|| curved(i, text, poly, size, &placed))
                    .flatten()
            });
            if found.is_some() {
                break;
            }
        }
        if let Some(l) = found {
            reserve(&mut placed, &l);
            labels.push(l);
            continue;
        }
        if opts.leaders && radius < 2.0 * opts.size {
            if let Some(l) = leader(i, text, pole, smallest, bounds, &placed, &covers_land) {
                reserve(&mut placed, &l);
                labels.push(l);
            }
        }
    }
    labels.sort_by_key(|l| l.feature);
    labels
}

fn boxed(x: f64, y: f64, w: f64, h: f64) -> Rectangle<[f64; 2]> {
    Rectangle::from_corners([x - w / 2.0, y - h / 2.0], [x + w / 2.0, y + h / 2.0])
}

fn collides(placed: &RTree<Rectangle<[f64; 2]>>, r: &Rectangle<[f64; 2]>) -> bool {
    let e = r.envelope_rect();
    placed.locate_in_envelope_intersecting(&e).next().is_some()
}

trait EnvelopeRect {
    fn envelope_rect(&self) -> AABB<[f64; 2]>;
}

impl EnvelopeRect for Rectangle<[f64; 2]> {
    fn envelope_rect(&self) -> AABB<[f64; 2]> {
        AABB::from_corners(self.lower(), self.upper())
    }
}

fn reserve(placed: &mut RTree<Rectangle<[f64; 2]>>, l: &Label) {
    let w = text_width(&l.text, l.size);
    let h = 1.2 * l.size;
    match &l.shape {
        LabelShape::Straight | LabelShape::Leader { .. } => placed.insert(boxed(l.x, l.y, w, h)),
        LabelShape::Curved { path } => {
            for &(x, y) in path {
                placed.insert(boxed(x, y, h, h));
            }
        }
    }
}

fn straight(
    i: usize,
    text: &str,
    poly: &Polygon<f64>,
    pole: (f64, f64),
    sizes: &[f64],
    placed: &RTree<Rectangle<[f64; 2]>>,
) -> Option<Label> {
    for &size in sizes {
        let (w, h) = (text_width(text, size), 1.2 * size);
        let offsets = [
            (0.0, 0.0),
            (0.0, -0.6 * h),
            (0.0, 0.6 * h),
            (-0.25 * w, 0.0),
            (0.25 * w, 0.0),
        ];
        for (dx, dy) in offsets {
            let (x, y) = (pole.0 + dx, pole.1 + dy);
            let fits = [
                (-0.5, -0.5),
                (0.5, -0.5),
                (0.5, 0.5),
                (-0.5, 0.5),
                (0.0, -0.5),
                (0.0, 0.5),
                (-0.5, 0.0),
                (0.5, 0.0),
            ]
            .iter()
            .all(|(fx, fy)| poly.contains(&Point::new(x + fx * w, y + fy * h)));
            if fits && !collides(placed, &boxed(x, y, w, h)) {
                return Some(Label {
                    feature: i,
                    text: text.to_owned(),
                    x,
                    y,
                    size,
                    shape: LabelShape::Straight,
                });
            }
        }
    }
    None
}

fn leader(
    i: usize,
    text: &str,
    anchor: (f64, f64),
    size: f64,
    bounds: [f64; 4],
    placed: &RTree<Rectangle<[f64; 2]>>,
    covers_land: &dyn Fn(f64, f64) -> bool,
) -> Option<Label> {
    let (w, h) = (text_width(text, size), 1.2 * size);
    // Right, left, then diagonals and vertical, at increasing distances.
    let dirs = [
        (1.0, 0.0),
        (-1.0, 0.0),
        (1.0, -1.0),
        (1.0, 1.0),
        (-1.0, -1.0),
        (-1.0, 1.0),
        (0.0, -1.0),
        (0.0, 1.0),
    ];
    for dist in [1.5, 2.5, 3.5] {
        for (ux, uy) in dirs {
            let gap = dist * size;
            // Place the box so its nearest edge is `gap` away from the anchor.
            let x = anchor.0 + ux * (gap + w / 2.0);
            let y = anchor.1 + uy * (gap + h / 2.0);
            let inside_canvas = x - w / 2.0 >= bounds[0] + 2.0
                && y - h / 2.0 >= bounds[1] + 2.0
                && x + w / 2.0 <= bounds[2] - 2.0
                && y + h / 2.0 <= bounds[3] - 2.0;
            if !inside_canvas || collides(placed, &boxed(x, y, w, h)) {
                continue;
            }
            let probes = [
                (0.0, 0.0),
                (-0.5, -0.5),
                (0.5, -0.5),
                (0.5, 0.5),
                (-0.5, 0.5),
                (-0.25, 0.0),
                (0.25, 0.0),
            ];
            if probes
                .iter()
                .any(|(fx, fy)| covers_land(x + fx * w, y + fy * h))
            {
                continue;
            }
            let end = (x - ux * w / 2.0, y - uy * h / 2.0);
            return Some(Label {
                feature: i,
                text: text.to_owned(),
                x,
                y,
                size,
                shape: LabelShape::Leader { anchor, end },
            });
        }
    }
    None
}

fn curved(
    i: usize,
    text: &str,
    poly: &Polygon<f64>,
    sizes: &[f64],
    placed: &RTree<Rectangle<[f64; 2]>>,
) -> Option<Label> {
    let (line, widths) = centerline(poly, 24)?;
    let total = path_length(&line);
    // Distance along the path of each sample, to check the width only where
    // the text will sit (the middle of the path).
    let mut along = vec![0.0];
    for seg in line.windows(2) {
        let d = ((seg[1].0 - seg[0].0).powi(2) + (seg[1].1 - seg[0].1).powi(2)).sqrt();
        along.push(along[along.len() - 1] + d);
    }
    for &size in sizes {
        let w = text_width(text, size);
        let (from, to) = ((total - w) / 2.0, (total + w) / 2.0);
        let too_narrow = along
            .iter()
            .zip(&widths)
            .any(|(&d, &wd)| d >= from && d <= to && wd < 1.1 * size);
        if total < 1.1 * w || too_narrow {
            continue;
        }
        let used = middle_portion(&line, w);
        let clear = used
            .iter()
            .all(|&(x, y)| !collides(placed, &boxed(x, y, 1.2 * size, 1.2 * size)));
        if clear {
            let mid = point_at(&line, total / 2.0);
            return Some(Label {
                feature: i,
                text: text.to_owned(),
                x: mid.0,
                y: mid.1,
                size,
                shape: LabelShape::Curved { path: line },
            });
        }
    }
    None
}

/// Centreline of an elongated polygon along its principal axis, oriented in
/// reading direction (left to right, or bottom to top when near vertical),
/// with the local width at each sample. `None` for compact shapes.
/// A centreline (pixel points) and the region's width at each point.
type Centerline = (Vec<(f64, f64)>, Vec<f64>);

fn centerline(poly: &Polygon<f64>, samples: usize) -> Option<Centerline> {
    let pts = &poly.exterior().0;
    let n = pts.len().saturating_sub(1).max(1) as f64;
    let (mx, my) = pts
        .iter()
        .take(pts.len().saturating_sub(1))
        .fold((0.0, 0.0), |(a, b), c| (a + c.x / n, b + c.y / n));
    let (mut cxx, mut cyy, mut cxy) = (0.0, 0.0, 0.0);
    for c in pts {
        let (dx, dy) = (c.x - mx, c.y - my);
        cxx += dx * dx;
        cyy += dy * dy;
        cxy += dx * dy;
    }
    let theta = 0.5 * atan2(2.0 * cxy, cxx - cyy);
    let (u, v) = ((cos(theta), sin(theta)), (-sin(theta), cos(theta)));
    let proj = |c: &Coord<f64>, d: (f64, f64)| (c.x - mx) * d.0 + (c.y - my) * d.1;
    let (tmin, tmax) = pts
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
            (a.min(proj(c, u)), b.max(proj(c, u)))
        });
    let (smin, smax) = pts
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
            (a.min(proj(c, v)), b.max(proj(c, v)))
        });
    if (tmax - tmin) < 3.0 * (smax - smin) {
        return None;
    }

    let rings: Vec<&LineString<f64>> = std::iter::once(poly.exterior())
        .chain(poly.interiors())
        .collect();
    let mut line = Vec::new();
    let mut widths = Vec::new();
    let mut prev_mid: Option<f64> = None;
    for k in 1..samples {
        let t = tmin + (tmax - tmin) * k as f64 / samples as f64;
        let origin = (mx + t * u.0, my + t * u.1);
        // Intersections of the line origin + s·v with every edge.
        let mut ss: Vec<f64> = Vec::new();
        for r in &rings {
            for seg in r.0.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                let (pa, pb) = (
                    (a.x - origin.0) * u.0 + (a.y - origin.1) * u.1,
                    (b.x - origin.0) * u.0 + (b.y - origin.1) * u.1,
                );
                if (pa <= 0.0 && pb > 0.0) || (pb <= 0.0 && pa > 0.0) {
                    let f = pa / (pa - pb);
                    let (x, y) = (a.x + f * (b.x - a.x), a.y + f * (b.y - a.y));
                    ss.push((x - origin.0) * v.0 + (y - origin.1) * v.1);
                }
            }
        }
        ss.sort_by(f64::total_cmp);
        let intervals: Vec<(f64, f64)> = ss.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let best = match prev_mid {
            None => intervals
                .iter()
                .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0))),
            Some(p) => intervals.iter().min_by(|a, b| {
                ((a.0 + a.1) / 2.0 - p)
                    .abs()
                    .total_cmp(&((b.0 + b.1) / 2.0 - p).abs())
            }),
        };
        let Some(&(s0, s1)) = best else { continue };
        let m = (s0 + s1) / 2.0;
        prev_mid = Some(m);
        line.push((origin.0 + m * v.0, origin.1 + m * v.1));
        widths.push(s1 - s0);
    }
    if line.len() < 4 {
        return None;
    }
    // Two passes of a 3-point moving average, keeping the ends.
    for _ in 0..2 {
        let copy = line.clone();
        for j in 1..copy.len() - 1 {
            line[j] = (
                (copy[j - 1].0 + copy[j].0 + copy[j + 1].0) / 3.0,
                (copy[j - 1].1 + copy[j].1 + copy[j + 1].1) / 3.0,
            );
        }
    }
    let (first, last) = (line[0], line[line.len() - 1]);
    let (dx, dy) = (last.0 - first.0, last.1 - first.1);
    let backwards = if dx.abs() < 0.3 * dy.abs() {
        dy > 0.0
    } else {
        dx < 0.0
    };
    if backwards {
        line.reverse();
        widths.reverse();
    }
    Some((line, widths))
}

/// Letters of a curved label, for renderers without `textPath` (librsvg):
/// each character's centre on `path` and the path's direction there, in
/// degrees, centred on the path like `startOffset="50%"` would. Spacing uses
/// the same width estimate as placement; the direction is taken over one
/// letter's width, smoothing the centreline's corners.
pub fn letters(path: &[(f64, f64)], text: &str, size: f64) -> Vec<(char, f64, f64, f64)> {
    let mut d = (path_length(path) - text_width(text, size)) / 2.0;
    text.chars()
        .map(|c| {
            let advance = char_width(c) * size;
            let mid = d + advance / 2.0;
            d += advance;
            let (x, y) = point_at(path, mid);
            let (a, b) = (point_at(path, d - advance), point_at(path, d));
            (c, x, y, atan2(b.1 - a.1, b.0 - a.0).to_degrees())
        })
        .collect()
}

fn path_length(p: &[(f64, f64)]) -> f64 {
    p.windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum()
}

fn point_at(p: &[(f64, f64)], dist: f64) -> (f64, f64) {
    let mut left = dist;
    for w in p.windows(2) {
        let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        if left <= l && l > 0.0 {
            let f = left / l;
            return (
                w[0].0 + f * (w[1].0 - w[0].0),
                w[0].1 + f * (w[1].1 - w[0].1),
            );
        }
        left -= l;
    }
    *p.last().unwrap_or(&(0.0, 0.0))
}

/// Points spaced one text-height apart over the central `width` of the path.
fn middle_portion(p: &[(f64, f64)], width: f64) -> Vec<(f64, f64)> {
    let total = path_length(p);
    let start = (total - width) / 2.0;
    let steps = (width / 4.0).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|k| point_at(p, start + width * k as f64 / steps as f64))
        .collect()
}

/// Pole of inaccessibility (Mapbox's polylabel): the interior point farthest
/// from the polygon's edges, and that distance, to within `precision`.
pub fn polylabel(poly: &Polygon<f64>, precision: f64) -> ((f64, f64), f64) {
    let Some(b) = poly.bounding_rect() else {
        return ((0.0, 0.0), 0.0);
    };
    let cell = b.width().min(b.height());
    if cell <= 0.0 {
        return ((b.min().x, b.min().y), 0.0);
    }

    #[derive(PartialEq)]
    struct Cell {
        x: f64,
        y: f64,
        h: f64,
        d: f64,
        max: f64,
        seq: usize,
    }
    impl Eq for Cell {}
    impl PartialOrd for Cell {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Cell {
        fn cmp(&self, o: &Self) -> Ordering {
            self.max.total_cmp(&o.max).then(o.seq.cmp(&self.seq))
        }
    }
    let mut seq = 0;
    let mut make = |x: f64, y: f64, h: f64| {
        let d = signed_distance(x, y, poly);
        seq += 1;
        Cell {
            x,
            y,
            h,
            d,
            max: d + h * std::f64::consts::SQRT_2,
            seq,
        }
    };

    let mut queue = BinaryHeap::new();
    let h = cell / 2.0;
    let mut x = b.min().x;
    while x < b.max().x {
        let mut y = b.min().y;
        while y < b.max().y {
            queue.push(make(x + h, y + h, h));
            y += cell;
        }
        x += cell;
    }
    let c = poly
        .centroid()
        .map_or((b.center().x, b.center().y), |p| (p.x(), p.y()));
    let mut best = make(c.0, c.1, 0.0);
    let bc = make(b.center().x, b.center().y, 0.0);
    if bc.d > best.d {
        best = bc;
    }
    while let Some(cur) = queue.pop() {
        if cur.d > best.d {
            best = Cell {
                x: cur.x,
                y: cur.y,
                h: 0.0,
                d: cur.d,
                max: cur.d,
                seq: cur.seq,
            };
        }
        if cur.max - best.d <= precision {
            continue;
        }
        let h = cur.h / 2.0;
        for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
            queue.push(make(cur.x + dx, cur.y + dy, h));
        }
    }
    ((best.x, best.y), best.d.max(0.0))
}

/// Distance from a point to the polygon's boundary, negative outside.
fn signed_distance(x: f64, y: f64, poly: &Polygon<f64>) -> f64 {
    let mut inside = false;
    let mut min = f64::INFINITY;
    for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
        for seg in ring.0.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            if (a.y > y) != (b.y > y) && x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
            min = min.min(seg_dist(x, y, a, b));
        }
    }
    if inside {
        min
    } else {
        -min
    }
}

fn seg_dist(px: f64, py: f64, a: Coord<f64>, b: Coord<f64>) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let t = if dx == 0.0 && dy == 0.0 {
        0.0
    } else {
        (((px - a.x) * dx + (py - a.y) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0)
    };
    let c = coord! { x: a.x + t * dx, y: a.y + t * dy };
    ((px - c.x).powi(2) + (py - c.y).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::polygon;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> MultiPolygon<f64> {
        MultiPolygon(vec![
            polygon![(x: x0, y: y0), (x: x1, y: y0), (x: x1, y: y1), (x: x0, y: y1), (x: x0, y: y0)],
        ])
    }

    #[test]
    fn polylabel_of_a_c_shape_is_inside_the_thick_part() {
        // A "C": the centroid falls in the hole; polylabel must not.
        let c = polygon![(x: 0., y: 0.), (x: 100., y: 0.), (x: 100., y: 20.), (x: 20., y: 20.), (x: 20., y: 80.), (x: 100., y: 80.), (x: 100., y: 100.), (x: 0., y: 100.), (x: 0., y: 0.)];
        let ((x, y), d) = polylabel(&c, 0.1);
        assert!(c.contains(&Point::new(x, y)));
        // Widest spot: where the spine meets an arm, a circle of radius
        // 20 / (2 - √2)... ≈ 11.7 touches both outer edges and the inner corner.
        assert!(x < 20.0 && d > 11.0 && d < 12.0, "({x}, {y}) d={d}");
        let centroid = c.centroid().unwrap();
        assert!(!c.contains(&centroid), "the centroid of a C is outside it");
    }

    #[test]
    fn letters_follow_the_path_centred() {
        // An L-shaped path of length 200: along x, then down y.
        let path = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)];
        let ls = letters(&path, "ab cd", 10.0);
        assert_eq!(ls.iter().map(|l| l.0).collect::<String>(), "ab cd");
        // 5 letters × 5.6 px, centred on the corner at distance 100.
        let (_, x, y, angle) = ls[0];
        assert!((x - (100.0 - 14.0 + 2.8)).abs() < 1e-9 && y == 0.0 && angle == 0.0);
        let (_, x, y, angle) = ls[4];
        assert!(x == 100.0 && (y - (14.0 - 2.8)).abs() < 1e-9 && angle == 90.0);
        // The middle letter straddles the corner: its direction is in between.
        assert!((ls[2].3 - 45.0).abs() < 1e-9);
    }

    #[test]
    fn wide_characters_are_square() {
        assert_eq!(text_width("法國", 10.0), 20.0);
        assert!((text_width("France", 10.0) - 33.6).abs() < 1e-9);
        assert_eq!(text_width("서울", 10.0), 20.0);
        // Letters advance by their own width.
        let ls = letters(&[(0.0, 0.0), (100.0, 0.0)], "a法", 10.0);
        assert!((ls[1].1 - ls[0].1 - (2.8 + 5.0)).abs() < 1e-9);
    }

    #[test]
    fn places_centered_labels_without_overlap() {
        let a = rect(0.0, 0.0, 200.0, 100.0);
        let b = rect(200.0, 0.0, 400.0, 100.0);
        let labels = place_labels(
            &[("Alpha", &a), ("Beta", &b)],
            [0.0, 0.0, 400.0, 100.0],
            &LabelOptions::default(),
        );
        assert_eq!(labels.len(), 2);
        assert!((labels[0].x - 100.0).abs() < 1.0 && (labels[0].y - 50.0).abs() < 1.0);
        assert!(labels
            .iter()
            .all(|l| l.shape == LabelShape::Straight && l.size == 11.0));
    }

    #[test]
    fn shrinks_text_to_fit() {
        // Wide enough for the name at 70 % but not at 100 %.
        let name = "Longname";
        let w = text_width(name, 11.0 * 0.8);
        let r = rect(0.0, 0.0, w + 1.0, 30.0);
        let labels = place_labels(
            &[(name, &r)],
            [0.0, 0.0, 400.0, 400.0],
            &LabelOptions {
                leaders: false,
                ..LabelOptions::default()
            },
        );
        assert_eq!(labels.len(), 1);
        assert!(labels[0].size < 11.0 && labels[0].size >= 7.7 - 1e-9);
    }

    #[test]
    fn small_regions_get_leader_lines_over_empty_space() {
        let tiny = rect(100.0, 100.0, 104.0, 104.0);
        let labels = place_labels(
            &[("Tiny island", &tiny)],
            [0.0, 0.0, 400.0, 400.0],
            &LabelOptions::default(),
        );
        assert_eq!(labels.len(), 1);
        assert!(matches!(labels[0].shape, LabelShape::Leader { .. }));
        let off = LabelOptions {
            leaders: false,
            ..LabelOptions::default()
        };
        assert!(place_labels(&[("Tiny island", &tiny)], [0.0, 0.0, 400.0, 400.0], &off).is_empty());
    }

    #[test]
    fn leader_labels_never_cover_other_regions() {
        let tiny = rect(195.0, 195.0, 199.0, 199.0);
        // Surround the tiny region with land in every direction.
        let land = MultiPolygon(vec![
            polygon![(x: 0., y: 0.), (x: 400., y: 0.), (x: 400., y: 400.), (x: 0., y: 400.), (x: 0., y: 0.)],
        ]);
        let labels = place_labels(
            &[("Tiny", &tiny), ("", &land)],
            [0.0, 0.0, 400.0, 400.0],
            &LabelOptions::default(),
        );
        assert!(
            labels.iter().all(|l| l.feature != 0),
            "no room for a leader label"
        );
    }

    #[test]
    fn long_thin_regions_get_curved_labels_in_reading_order() {
        // A tall thin sliver, too narrow for straight text.
        let chile = rect(100.0, 0.0, 118.0, 400.0);
        let labels = place_labels(
            &[("Chile", &chile)],
            [0.0, 0.0, 400.0, 400.0],
            &LabelOptions::default(),
        );
        assert_eq!(labels.len(), 1);
        let LabelShape::Curved { path } = &labels[0].shape else {
            panic!("{:?}", labels[0].shape)
        };
        assert!(
            path.first().unwrap().1 > path.last().unwrap().1,
            "vertical text reads bottom to top"
        );
    }

    #[test]
    fn deterministic() {
        let a = rect(0.0, 0.0, 60.0, 20.0);
        let b = rect(60.0, 0.0, 70.0, 300.0);
        let run = || {
            place_labels(
                &[("Aaaa", &a), ("Bbbbbb", &b)],
                [0.0, 0.0, 400.0, 400.0],
                &LabelOptions::default(),
            )
        };
        assert_eq!(run(), run());
    }
}
