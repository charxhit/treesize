use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl Default for NodeId {
    fn default() -> Self {
        NodeId(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DirStats {
    pub bytes: u128,
    pub files: u64,
    pub dirs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeKind {
    File,
    Dir,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub path: std::path::PathBuf,
    pub name: String,
    pub kind: NodeKind,
    pub size: u128,
    pub file_count: u64,
    pub children: Vec<NodeId>,
    pub modified: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tree {
    pub root: NodeId,
    pub nodes: Vec<TreeNode>,
}
