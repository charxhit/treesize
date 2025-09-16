use crossbeam_channel::Sender;
use ignore::{WalkBuilder, WalkState};
use std::collections::HashMap;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use crate::model::*;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum ScanMsg {
    Progress {
        scanned: u64,
        discovered: u64,
        bytes: u128,
    },
    DirDone {
        path: PathBuf,
        bytes: u128,
        files: u64,
        dirs: u64,
    },
    File {
        path: PathBuf,
        bytes: u64,
    },
    Done(Tree),
    Error(String),
}

pub struct Scanner {
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl Scanner {
    pub fn new(cancel: Arc<AtomicBool>, paused: Arc<AtomicBool>) -> Self {
        Self { cancel, paused }
    }

    pub fn scan(&self, root: PathBuf, tx: Sender<ScanMsg>) {
        use parking_lot::Mutex;

        let cancel = self.cancel.clone();
        let paused = self.paused.clone();

        // Shared progress counters
        let discovered = Arc::new(AtomicU64::new(0));
        let scanned = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(Mutex::new(0u128));

        // Collected files for final tree assembly
        let files: Arc<Mutex<Vec<(PathBuf, u64)>>> = Arc::new(Mutex::new(Vec::with_capacity(4096)));

        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_global(false)
            .follow_links(false)
            .threads(num_cpus::get());

        let walker = builder.build_parallel();
        walker.run(|| {
            let paused_outer = paused.clone();
            let cancel = cancel.clone();
            let tx = tx.clone();
            let discovered = discovered.clone();
            let scanned = scanned.clone();
            let bytes = bytes.clone();
            let files = files.clone();
            Box::new(move |entry| {
                while paused_outer.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        return WalkState::Quit;
                    }
                    sleep(Duration::from_millis(40));
                }
                if cancel.load(Ordering::Relaxed) {
                    return WalkState::Quit;
                }
                match entry {
                    Ok(ent) => {
                        if ent.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                            discovered.fetch_add(1, Ordering::Relaxed);
                            let path = ent.path().to_path_buf();
                            match ent.metadata() {
                                Ok(md) => {
                                    let sz = md.len() as u64;
                                    scanned.fetch_add(1, Ordering::Relaxed);
                                    {
                                        let mut b = bytes.lock();
                                        *b = b.saturating_add(sz as u128);
                                        let _ = tx.send(ScanMsg::Progress {
                                            scanned: scanned.load(Ordering::Relaxed),
                                            discovered: discovered.load(Ordering::Relaxed),
                                            bytes: *b,
                                        });
                                    }
                                    let _ = tx.send(ScanMsg::File {
                                        path: path.clone(),
                                        bytes: sz,
                                    });
                                    files.lock().push((path, sz));
                                }
                                Err(_) => {
                                    // Still count as scanned, but no size
                                    scanned.fetch_add(1, Ordering::Relaxed);
                                    let b = *bytes.lock();
                                    let _ = tx.send(ScanMsg::Progress {
                                        scanned: scanned.load(Ordering::Relaxed),
                                        discovered: discovered.load(Ordering::Relaxed),
                                        bytes: b,
                                    });
                                }
                            }
                        }
                        WalkState::Continue
                    }
                    Err(e) => {
                        let _ = tx.send(ScanMsg::Error(e.to_string()));
                        WalkState::Continue
                    }
                }
            })
        });

        // Assemble a tree from the collected file list
        let files = Arc::try_unwrap(files)
            .map(|m| m.into_inner())
            .unwrap_or_else(|arc| arc.lock().clone());
        let tree = build_tree(&root, files);
        let _ = tx.send(ScanMsg::Done(tree));
    }
}

fn build_tree(root: &Path, files: Vec<(PathBuf, u64)>) -> Tree {
    use crate::model::{NodeId, NodeKind, Tree, TreeNode};

    let root = root.to_path_buf();
    let mut nodes: Vec<TreeNode> = Vec::with_capacity(1024);
    let mut id_by_path: HashMap<PathBuf, NodeId> = HashMap::new();

    // Helper to ensure a directory node exists (and link it to its parent)
    fn ensure_dir(
        path: &Path,
        root: &Path,
        nodes: &mut Vec<TreeNode>,
        id_by_path: &mut HashMap<PathBuf, NodeId>,
    ) -> NodeId {
        if let Some(id) = id_by_path.get(path).cloned() {
            return id;
        }
        let parent_id = if path == root {
            None
        } else {
            let parent = path.parent().unwrap_or(root);
            Some(ensure_dir(parent, root, nodes, id_by_path))
        };
        let id = NodeId(nodes.len() as u64);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_else(|| path.as_os_str().to_str().unwrap_or(""))
            .to_string();
        nodes.push(TreeNode {
            id,
            parent: parent_id,
            path: path.to_path_buf(),
            name,
            kind: NodeKind::Dir,
            size: 0,
            file_count: 0,
            children: Vec::new(),
            modified: None,
        });
        id_by_path.insert(path.to_path_buf(), id);
        if let Some(pid) = parent_id {
            // Link as child of parent
            if let Some(p) = nodes.get_mut(pid.0 as usize) {
                p.children.push(id);
            }
        }
        id
    }

    // create root dir node
    let root_id = ensure_dir(&root, &root, &mut nodes, &mut id_by_path);

    // Add files and propagate sizes
    for (path, sz) in files {
        let parent_dir = path.parent().unwrap_or(&root);
        let pid = ensure_dir(parent_dir, &root, &mut nodes, &mut id_by_path);
        let id = NodeId(nodes.len() as u64);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        nodes.push(TreeNode {
            id,
            parent: Some(pid),
            path: path.clone(),
            name,
            kind: NodeKind::File,
            size: sz as u128,
            file_count: 1,
            children: Vec::new(),
            modified: None,
        });
        if let Some(p) = nodes.get_mut(pid.0 as usize) {
            p.children.push(id);
        }

        // Propagate to ancestors
        let mut cur = Some(parent_dir.to_path_buf());
        while let Some(dir) = cur {
            if let Some(did) = id_by_path.get(&dir).cloned() {
                if let Some(node) = nodes.get_mut(did.0 as usize) {
                    node.size = node.size.saturating_add(sz as u128);
                    node.file_count = node.file_count.saturating_add(1);
                }
            }
            if dir == root {
                break;
            }
            cur = dir.parent().map(|p| p.to_path_buf());
        }
    }

    Tree {
        root: root_id,
        nodes,
    }
}
