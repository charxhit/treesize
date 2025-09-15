use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use treesize_core::model::{NodeId, Tree};
use treesize_core::scanner::{ScanMsg, Scanner};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Size,
    Name,
    Count,
}

pub struct AppState {
    pub root: Option<PathBuf>,
    pub cancel: Arc<AtomicBool>,
    pub scan_rx: Option<Receiver<ScanMsg>>,
    pub progress_bytes: u128,
    pub progress_files: u64,
    pub progress_discovered: u64,
    pub sort: SortKey,
    pub search: String,
    pub tree: Option<Tree>,
    pub current_dir: Option<NodeId>,
    pub selected: Option<NodeId>,
    pub pending_delete: Option<NodeId>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            root: None,
            cancel: Arc::new(AtomicBool::new(false)),
            scan_rx: None,
            progress_bytes: 0,
            progress_files: 0,
            progress_discovered: 0,
            sort: SortKey::Size,
            search: String::new(),
            tree: None,
            current_dir: None,
            selected: None,
            pending_delete: None,
        }
    }

    pub fn start_scan(&mut self, root: PathBuf) {
        self.root = Some(root.clone());
        self.progress_bytes = 0;
        self.progress_files = 0;
        self.progress_discovered = 0;
        self.tree = None;
        self.current_dir = None;
        self.selected = None;
        self.pending_delete = None;
        self.cancel.store(false, Ordering::Relaxed);

        let (tx, rx): (Sender<ScanMsg>, Receiver<ScanMsg>) = unbounded();
        self.scan_rx = Some(rx);
        let cancel = self.cancel.clone();

        std::thread::spawn(move || {
            let scanner = Scanner::new(cancel);
            scanner.scan(root, tx);
        });
    }

    pub fn cancel_scan(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn navigate_up(&mut self) {
        if let (Some(tree), Some(cur)) = (&self.tree, self.current_dir) {
            if let Some(parent) = tree.nodes[cur.0 as usize].parent {
                self.current_dir = Some(parent);
                self.selected = None;
            }
        }
    }

    pub fn set_current_root(&mut self) {
        if let Some(tree) = &self.tree {
            self.current_dir = Some(tree.root);
        }
    }

    pub fn request_delete(&mut self, id: NodeId) {
        self.selected = Some(id);
        self.pending_delete = Some(id);
    }

    pub fn delete_selected_and_rescan(&mut self) {
        if let (Some(tree), Some(id)) = (&self.tree, self.selected) {
            let path = &tree.nodes[id.0 as usize].path;
            if trash::delete(path).is_err() {
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
            }
            if let Some(root) = &self.root {
                self.start_scan(root.clone());
            }
        }
    }
}
