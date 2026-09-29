//! Pixel comparison of two screenshots: a per-pixel colour distance in YIQ
//! space with an anti-aliasing filter (the pixelmatch recipe), then the
//! changed pixels grouped into connected regions the tree half of the diff
//! can map back to elements.

use image::{Rgba, RgbaImage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Changed pixels inside the box.
    pub changed: u32,
    /// Changed pixels over the box area.
    pub ratio: f64,
}

#[derive(Debug, Clone)]
pub struct PixelDiff {
    pub width: u32,
    pub height: u32,
    pub changed: u32,
    pub total: u32,
    pub ratio: f64,
    /// Sizes differed; the canvas is the union and the missing area counts as changed.
    pub size_mismatch: bool,
    pub regions: Vec<Region>,
    /// `a` faded to grey with the changed pixels painted red.
    pub image: RgbaImage,
}

fn rgb2y(p: &Rgba<u8>) -> f64 {
    p[0] as f64 * 0.29889531 + p[1] as f64 * 0.58662247 + p[2] as f64 * 0.11448223
}
fn rgb2i(p: &Rgba<u8>) -> f64 {
    p[0] as f64 * 0.59597799 - p[1] as f64 * 0.27417610 - p[2] as f64 * 0.32180189
}
fn rgb2q(p: &Rgba<u8>) -> f64 {
    p[0] as f64 * 0.21147017 - p[1] as f64 * 0.52261711 + p[2] as f64 * 0.31114694
}

fn blend(p: &Rgba<u8>) -> Rgba<u8> {
    // Composite over white so alpha differences count as colour differences.
    let a = p[3] as f64 / 255.0;
    let c = |v: u8| (255.0 + (v as f64 - 255.0) * a).round() as u8;
    Rgba([c(p[0]), c(p[1]), c(p[2]), 255])
}

/// Squared YIQ distance; the maximum (black vs white) is 35215.
fn color_delta(a: &Rgba<u8>, b: &Rgba<u8>) -> f64 {
    if a == b {
        return 0.0;
    }
    let (a, b) = (blend(a), blend(b));
    let y = rgb2y(&a) - rgb2y(&b);
    let i = rgb2i(&a) - rgb2i(&b);
    let q = rgb2q(&a) - rgb2q(&b);
    0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q
}

fn pixel(img: &RgbaImage, x: i64, y: i64) -> Option<Rgba<u8>> {
    if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
        return None;
    }
    Some(*img.get_pixel(x as u32, y as u32))
}

/// A pixel is anti-aliasing when it sits between two strongly different
/// neighbours along its brightness gradient and has few equal neighbours.
fn is_antialiased(img: &RgbaImage, x: u32, y: u32, other: &RgbaImage) -> bool {
    let center = *img.get_pixel(x, y);
    let mut zeroes = 0;
    let (mut min, mut max) = (0.0f64, 0.0f64);
    let (mut min_pos, mut max_pos) = ((0i64, 0i64), (0i64, 0i64));
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let Some(neighbour) = pixel(img, x as i64 + dx, y as i64 + dy) else {
                continue;
            };
            let delta = rgb2y(&blend(&center)) - rgb2y(&blend(&neighbour));
            if delta == 0.0 {
                zeroes += 1;
                if zeroes > 2 {
                    return false;
                }
            } else if delta < min {
                min = delta;
                min_pos = (x as i64 + dx, y as i64 + dy);
            } else if delta > max {
                max = delta;
                max_pos = (x as i64 + dx, y as i64 + dy);
            }
        }
    }
    if min == 0.0 || max == 0.0 {
        return false;
    }
    let has_siblings = |img: &RgbaImage, (px, py): (i64, i64)| {
        let Some(p) = pixel(img, px, py) else {
            return false;
        };
        let mut count = 0;
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                if (dx != 0 || dy != 0) && pixel(img, px + dx, py + dy) == Some(p) {
                    count += 1;
                    if count > 2 {
                        return true;
                    }
                }
            }
        }
        false
    };
    (has_siblings(img, min_pos) && has_siblings(other, min_pos))
        || (has_siblings(img, max_pos) && has_siblings(other, max_pos))
}

/// `threshold` is the pixelmatch scale (0..1, default 0.1).
pub fn diff(a: &RgbaImage, b: &RgbaImage, threshold: f64) -> PixelDiff {
    let width = a.width().max(b.width());
    let height = a.height().max(b.height());
    let size_mismatch = a.dimensions() != b.dimensions();
    let max_delta = 35215.0 * threshold * threshold;
    let mut mask = vec![false; (width * height) as usize];
    let mut image = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    let mut changed = 0u32;
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let (pa, pb) = (pixel(a, x as i64, y as i64), pixel(b, x as i64, y as i64));
            let differs = match (pa, pb) {
                (Some(pa), Some(pb)) => {
                    let delta = color_delta(&pa, &pb);
                    delta > max_delta && !(is_antialiased(a, x, y, b) || is_antialiased(b, x, y, a))
                }
                _ => true,
            };
            if let Some(pa) = pa {
                let grey = (rgb2y(&blend(&pa)) * 0.35 + 255.0 * 0.65)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                image.put_pixel(x, y, Rgba([grey, grey, grey, 255]));
            }
            if differs {
                mask[idx] = true;
                changed += 1;
                image.put_pixel(x, y, Rgba([255, 0, 64, 255]));
            }
        }
    }
    let total = width * height;
    PixelDiff {
        width,
        height,
        changed,
        total,
        ratio: if total == 0 {
            0.0
        } else {
            changed as f64 / total as f64
        },
        size_mismatch,
        regions: regions(&mask, width, height),
        image,
    }
}

/// Connected components of the mask (8-neighbourhood, bridged across up
/// to two empty pixels), boxed, largest first. Regions under 4 pixels are
/// dropped.
pub fn regions(mask: &[bool], width: u32, height: u32) -> Vec<Region> {
    const GAP: i64 = 3;
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut count = 0u32;
        while let Some(idx) = stack.pop() {
            let (x, y) = ((idx as u32) % width, (idx as u32) / width);
            count += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            for dy in -GAP..=GAP {
                for dx in -GAP..=GAP {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let n = (ny as u32 * width + nx as u32) as usize;
                    if mask[n] && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        if count < 4 {
            continue;
        }
        let (w, h) = (max_x - min_x + 1, max_y - min_y + 1);
        out.push(Region {
            x: min_x,
            y: min_y,
            w,
            h,
            changed: count,
            ratio: count as f64 / (w * h) as f64,
        });
    }
    out.sort_by_key(|region| std::cmp::Reverse(region.changed));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(color))
    }

    #[test]
    fn identical_images_have_no_changes() {
        let a = solid(20, 20, [10, 20, 30, 255]);
        let result = diff(&a, &a.clone(), 0.1);
        assert_eq!(result.changed, 0);
        assert!(result.regions.is_empty());
        assert!(!result.size_mismatch);
    }

    #[test]
    fn a_painted_block_becomes_one_region_and_faint_noise_is_ignored() {
        let a = solid(40, 30, [255, 255, 255, 255]);
        let mut b = a.clone();
        for y in 5..15 {
            for x in 10..20 {
                b.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        // One-off near-white pixel: under the threshold.
        b.put_pixel(30, 25, Rgba([250, 250, 250, 255]));
        let result = diff(&a, &b, 0.1);
        assert_eq!(result.changed, 100);
        assert_eq!(result.regions.len(), 1);
        let region = &result.regions[0];
        assert_eq!((region.x, region.y, region.w, region.h), (10, 5, 10, 10));
        assert_eq!(region.changed, 100);
        assert_eq!(result.image.get_pixel(10, 5), &Rgba([255, 0, 64, 255]));
    }

    #[test]
    fn size_mismatch_counts_the_missing_area() {
        let a = solid(10, 10, [0, 0, 0, 255]);
        let b = solid(10, 12, [0, 0, 0, 255]);
        let result = diff(&a, &b, 0.1);
        assert!(result.size_mismatch);
        assert_eq!(result.changed, 20);
        assert_eq!((result.width, result.height), (10, 12));
    }

    #[test]
    fn regions_bridge_small_gaps() {
        let (w, h) = (20u32, 5u32);
        let mut mask = vec![false; (w * h) as usize];
        for x in [2u32, 3, 4, 5, 8, 9, 10, 11] {
            mask[(2 * w + x) as usize] = true;
            mask[(3 * w + x) as usize] = true;
        }
        let out = regions(&mask, w, h);
        assert_eq!(out.len(), 1, "a 2 px gap joins the two blocks");
        assert_eq!(out[0].w, 10);
    }
}
