use crate::model::NodeId;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect { pub x:f32, pub y:f32, pub w:f32, pub h:f32 }

#[derive(Clone, Debug)]
pub struct TreemapItem { pub id: NodeId, pub weight: f64, pub rect: Rect }

pub fn squarify(_weights: &[(NodeId, f64)], _area: Rect) -> Vec<TreemapItem> {
    // Stub: replace with squarified treemap algorithm implementation.
    vec![]
}
