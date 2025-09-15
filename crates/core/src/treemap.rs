use crate::model::NodeId;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct TreemapItem {
    pub id: NodeId,
    pub weight: f64,
    pub rect: Rect,
}

// Simple slice-and-dice treemap: sorts by weight and slices along the longer edge
pub fn squarify(weights: &[(NodeId, f64)], area: Rect) -> Vec<TreemapItem> {
    let mut items: Vec<(NodeId, f64)> = weights
        .iter()
        .cloned()
        .filter(|(_, w)| *w > 0.0 && w.is_finite())
        .collect();
    if items.is_empty() || area.w <= 0.0 || area.h <= 0.0 {
        return Vec::new();
    }
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let total: f64 = items.iter().map(|(_, w)| w).sum();
    if total <= 0.0 || !total.is_finite() {
        return Vec::new();
    }
    let mut x = area.x;
    let mut y = area.y;
    let mut w = area.w;
    let mut h = area.h;
    let mut out = Vec::with_capacity(items.len());
    let horizontal = w >= h;
    for (id, weight) in items {
        let frac = (weight / total).max(0.0);
        if horizontal {
            let cw = (w as f64 * frac) as f32;
            if cw <= 0.5 {
                continue;
            }
            out.push(TreemapItem {
                id,
                weight,
                rect: Rect { x, y, w: cw, h },
            });
            x += cw;
            w -= cw;
        } else {
            let ch = (h as f64 * frac) as f32;
            if ch <= 0.5 {
                continue;
            }
            out.push(TreemapItem {
                id,
                weight,
                rect: Rect { x, y, w, h: ch },
            });
            y += ch;
            h -= ch;
        }
        if w <= 1.0 || h <= 1.0 {
            break;
        }
    }
    out
}
