//! Empty areas of a map, where a legend, title or credit can go without
//! covering the mapped regions (`Rendered::legend_slots`).
//!
//! The canvas is the inset-placement raster (`LandGrid`, 6 px cells) with the
//! regions of the main map, the insets, the labels and the credit marked as
//! occupied. Neighbouring countries don't count: a legend over them is fine.
//! For each of nine positions, the result is the largest empty rectangle
//! anchored there that is at least `MIN_SIDE` px on each side (a sliver of
//! sea along the coast holds no legend).

use geo_types::Rect;

use crate::labels::{text_width, Label, LabelShape};

/// Nine anchor positions, as `(name, horizontal, vertical)` with 0 = start,
/// 1 = centre, 2 = end.
const POSITIONS: [(&str, u8, u8); 9] = [
    ("top-left", 0, 0),
    ("top-center", 1, 0),
    ("top-right", 2, 0),
    ("middle-left", 0, 1),
    ("center", 1, 1),
    ("middle-right", 2, 1),
    ("bottom-left", 0, 2),
    ("bottom-center", 1, 2),
    ("bottom-right", 2, 2),
];

/// Smallest usable slot side, in pixels.
const MIN_SIDE: f64 = 48.0;

/// An empty area of the canvas, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendSlot {
    /// `top-left`, `top-center`… `bottom-right`.
    pub position: &'static str,
    pub x: f64,
    pub y: f64,
    /// `0` when nothing fits at this position.
    pub width: f64,
    pub height: f64,
    /// Share of a box a third of the canvas wide and high, at this position,
    /// that covers the mapped regions or an inset: how much a legend placed
    /// there without looking would hide.
    pub land_share: f64,
}

/// The occupied cells of the canvas.
pub(crate) struct Occupancy {
    cell: f64,
    cols: usize,
    rows: usize,
    land: Vec<bool>,
    used: Vec<bool>,
}

impl Occupancy {
    pub(crate) fn new(cell: f64, cols: usize, rows: usize, land: Vec<bool>) -> Self {
        let used = land.clone();
        Occupancy {
            cell,
            cols,
            rows,
            land,
            used,
        }
    }

    /// Marks a pixel rectangle (grown by `pad`) as occupied; `solid` ones
    /// (insets) also count in `land_share`.
    pub(crate) fn mark(&mut self, r: Rect<f64>, pad: f64, solid: bool) {
        let c0 = ((r.min().x - pad) / self.cell).floor().max(0.0) as usize;
        let r0 = ((r.min().y - pad) / self.cell).floor().max(0.0) as usize;
        let c1 = (((r.max().x + pad) / self.cell).ceil().max(0.0) as usize).min(self.cols);
        let r1 = (((r.max().y + pad) / self.cell).ceil().max(0.0) as usize).min(self.rows);
        for row in r0..r1 {
            for col in c0..c1 {
                self.used[row * self.cols + col] = true;
                if solid {
                    self.land[row * self.cols + col] = true;
                }
            }
        }
    }

    pub(crate) fn mark_labels(&mut self, labels: &[Label]) {
        for l in labels {
            let r = match &l.shape {
                LabelShape::Curved { path } => {
                    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for &(x, y) in path {
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    }
                    let h = 0.6 * l.size;
                    rect(x0, y0 - h, x1, y1 + h)
                }
                _ => {
                    let (w, h) = (text_width(&l.text, l.size) / 2.0, 0.6 * l.size);
                    rect(l.x - w, l.y - h, l.x + w, l.y + h)
                }
            };
            self.mark(r, 2.0, false);
        }
    }

    pub(crate) fn slots(&self) -> Vec<LegendSlot> {
        // Summed-area table of used cells: any rectangle checked in O(1).
        let (cols, rows) = (self.cols, self.rows);
        let mut sum = vec![0u32; (cols + 1) * (rows + 1)];
        for r in 0..rows {
            for c in 0..cols {
                sum[(r + 1) * (cols + 1) + c + 1] = u32::from(self.used[r * cols + c])
                    + sum[r * (cols + 1) + c + 1]
                    + sum[(r + 1) * (cols + 1) + c]
                    - sum[r * (cols + 1) + c];
            }
        }
        let used_in = |c0: usize, r0: usize, w: usize, h: usize| {
            let (c1, r1) = (c0 + w, r0 + h);
            sum[r1 * (cols + 1) + c1] + sum[r0 * (cols + 1) + c0]
                - sum[r0 * (cols + 1) + c1]
                - sum[r1 * (cols + 1) + c0]
        };
        // One empty cell of margin from the canvas edge.
        let m = 1;
        let min_cells = (MIN_SIDE / self.cell).ceil() as usize;
        let origin = |anchor: u8, size: usize, total: usize| match anchor {
            0 => m,
            1 => (total - size) / 2,
            _ => total - m - size,
        };
        let mut slots: Vec<(usize, LegendSlot)> = POSITIONS
            .iter()
            .map(|&(position, ax, ay)| {
                let mut best = (0, 0, 0); // area, width, height, in cells
                if cols > 2 * m && rows > 2 * m {
                    for w in 1..=cols - 2 * m {
                        let c0 = origin(ax, w, cols);
                        let mut h = 0;
                        while h < rows - 2 * m
                            && used_in(c0, origin(ay, h + 1, rows), w, h + 1) == 0
                        {
                            h += 1;
                        }
                        if w >= min_cells && h >= min_cells && w * h > best.0 {
                            best = (w * h, w, h);
                        }
                    }
                }
                let (area, w, h) = best;
                let (c0, r0) = (origin(ax, w, cols), origin(ay, h, rows));
                let slot = LegendSlot {
                    position,
                    x: c0 as f64 * self.cell,
                    y: r0 as f64 * self.cell,
                    width: w as f64 * self.cell,
                    height: h as f64 * self.cell,
                    land_share: self.land_share(ax, ay),
                };
                (area, slot)
            })
            .collect();
        // Largest first; ties keep the reading order of POSITIONS.
        slots.sort_by_key(|s| std::cmp::Reverse(s.0));
        slots.into_iter().map(|(_, s)| s).collect()
    }

    fn land_share(&self, ax: u8, ay: u8) -> f64 {
        let (w, h) = ((self.cols / 3).max(1), (self.rows / 3).max(1));
        let pick = |anchor: u8, size: usize, total: usize| match anchor {
            0 => 0,
            1 => (total - size) / 2,
            _ => total - size,
        };
        let (c0, r0) = (pick(ax, w, self.cols), pick(ay, h, self.rows));
        let land = (r0..r0 + h)
            .flat_map(|r| (c0..c0 + w).map(move |c| r * self.cols + c))
            .filter(|&i| self.land[i])
            .count();
        (land as f64 / (w * h) as f64 * 1000.0).round() / 1000.0
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect<f64> {
    Rect::new(
        geo_types::coord! { x: x0, y: y0 },
        geo_types::coord! { x: x1, y: y1 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20 × 20-cell (120 px) canvas with land in the left half.
    fn half_land() -> Occupancy {
        let land = (0..400).map(|i| i % 20 < 10).collect();
        Occupancy::new(6.0, 20, 20, land)
    }

    #[test]
    fn right_side_is_free_left_side_is_land() {
        let slots = half_land().slots();
        let get = |p: &str| slots.iter().find(|s| s.position == p).unwrap().clone();
        // Right: columns 10..18 and rows 1..18, keeping a one-cell margin.
        let r = get("middle-right");
        assert_eq!((r.x, r.width, r.y, r.height), (60.0, 54.0, 6.0, 108.0));
        assert_eq!(r.land_share, 0.0);
        let l = get("top-left");
        assert_eq!((l.width, l.height), (0.0, 0.0));
        assert_eq!(l.land_share, 1.0);
        assert_eq!(slots.len(), 9);
        assert!(slots
            .windows(2)
            .all(|w| w[0].width * w[0].height >= w[1].width * w[1].height));
    }

    #[test]
    fn marked_boxes_are_occupied_and_narrow_slots_dropped() {
        let mut occ = half_land();
        // An inset in the bottom-right quarter.
        occ.mark(rect(72.0, 72.0, 120.0, 120.0), 0.0, true);
        let slots = occ.slots();
        let get = |p: &str| slots.iter().find(|s| s.position == p).unwrap().clone();
        let br = get("bottom-right");
        assert_eq!(br.width * br.height, 0.0);
        assert_eq!(br.land_share, 1.0);
        // Above the inset there is still room at the top right.
        let tr = get("top-right");
        assert!(tr.y + tr.height <= 72.0 && tr.width >= MIN_SIDE, "{tr:?}");
        // A 6-cell strip is below MIN_SIDE: nothing.
        let strip = Occupancy::new(6.0, 20, 20, (0..400).map(|i| i % 20 < 14).collect());
        assert!(strip.slots().iter().all(|s| s.width == 0.0));
    }
}
