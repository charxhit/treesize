use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use treesize_core::model::{NodeId, NodeKind, Tree};
use treesize_core::scanner::{ScanMsg, Scanner};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Size,
    Name,
    Count,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewTab {
    Tree,
    Files,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    Pdf,
}

pub struct SearchFilter {
    pub direct_matches: Vec<bool>,
    pub subtree_matches: Vec<bool>,
}

impl SearchFilter {
    pub fn matches_node(&self, id: NodeId) -> bool {
        self.direct_matches[id.0 as usize]
    }

    pub fn matches_subtree(&self, id: NodeId) -> bool {
        self.subtree_matches[id.0 as usize]
    }

    pub fn build(needle: &str, tree: &Tree) -> Self {
        let needle_lc = needle.to_ascii_lowercase();
        let n = tree.nodes.len();
        let mut direct = vec![false; n];

        for (i, node) in tree.nodes.iter().enumerate() {
            let mut matched = node.name.to_ascii_lowercase().contains(&needle_lc);
            if !matched {
                let path_lc = node.path.to_string_lossy().to_ascii_lowercase();
                matched = path_lc.contains(&needle_lc);
            }
            direct[i] = matched;
        }

        let mut subtree = vec![false; n];
        fn dfs(tree: &Tree, direct: &[bool], subtree: &mut [bool], id: NodeId) -> bool {
            let idx = id.0 as usize;
            let node = &tree.nodes[idx];
            let mut any = direct[idx];
            for &child in &node.children {
                if dfs(tree, direct, subtree, child) {
                    any = true;
                }
            }
            subtree[idx] = any;
            any
        }
        if !tree.nodes.is_empty() {
            dfs(tree, &direct, &mut subtree, tree.root);
        }

        SearchFilter {
            direct_matches: direct,
            subtree_matches: subtree,
        }
    }
}

pub struct AppState {
    pub root: Option<PathBuf>,
    pub cancel: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
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
    pub pending_properties: Option<NodeId>,
    pub search_filter: Option<SearchFilter>,
    pub view_tab: ViewTab,
    pub file_nodes: Vec<NodeId>,
    pub filtered_file_nodes: Vec<NodeId>,
    pub export_format: ExportFormat,
    pub export_status: Option<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            root: None,
            cancel: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
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
            pending_properties: None,
            search_filter: None,
            view_tab: ViewTab::Tree,
            file_nodes: Vec::new(),
            filtered_file_nodes: Vec::new(),
            export_format: ExportFormat::Csv,
            export_status: None,
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
        self.pending_properties = None;
        self.search_filter = None;
        self.file_nodes.clear();
        self.filtered_file_nodes.clear();
        self.view_tab = ViewTab::Tree;
        self.export_status = None;
        self.cancel.store(false, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);

        let (tx, rx): (Sender<ScanMsg>, Receiver<ScanMsg>) = unbounded();
        self.scan_rx = Some(rx);
        let cancel = self.cancel.clone();
        let paused = self.paused.clone();

        std::thread::spawn(move || {
            let scanner = Scanner::new(cancel, paused);
            scanner.scan(root, tx);
        });
    }

    pub fn cancel_scan(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn pause_or_resume(&self) {
        let was = self.paused.load(Ordering::Relaxed);
        self.paused.store(!was, Ordering::Relaxed);
    }

    pub fn reset_to_initial(&mut self) {
        self.scan_rx = None;
        self.tree = None;
        self.current_dir = None;
        self.selected = None;
        self.progress_bytes = 0;
        self.progress_discovered = 0;
        self.progress_files = 0;
        self.search_filter = None;
        self.file_nodes.clear();
        self.filtered_file_nodes.clear();
        self.view_tab = ViewTab::Tree;
        self.export_status = None;
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
        self.pending_properties = None;
    }

    pub fn request_properties(&mut self, id: NodeId) {
        self.selected = Some(id);
        self.pending_properties = Some(id);
        self.pending_delete = None;
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

    pub fn rebuild_file_cache(&mut self) {
        self.file_nodes.clear();
        if let Some(tree) = &self.tree {
            for (idx, node) in tree.nodes.iter().enumerate() {
                if matches!(node.kind, NodeKind::File) {
                    self.file_nodes.push(NodeId(idx as u64));
                }
            }
        }
        self.apply_search();
    }

    pub fn sort_file_lists(&mut self) {
        let Some(tree) = &self.tree else {
            return;
        };
        let mut sort_ids = |ids: &mut Vec<NodeId>| match self.sort {
            SortKey::Size => ids.sort_by(|a, b| {
                tree.nodes[b.0 as usize]
                    .size
                    .cmp(&tree.nodes[a.0 as usize].size)
            }),
            SortKey::Name => ids.sort_by(|a, b| {
                tree.nodes[a.0 as usize]
                    .name
                    .cmp(&tree.nodes[b.0 as usize].name)
            }),
            SortKey::Count => ids.sort_by(|a, b| {
                tree.nodes[b.0 as usize]
                    .file_count
                    .cmp(&tree.nodes[a.0 as usize].file_count)
            }),
        };
        sort_ids(&mut self.file_nodes);
        sort_ids(&mut self.filtered_file_nodes);
    }

    pub fn refresh_filtered_files(&mut self) {
        if let Some(tree) = &self.tree {
            if let Some(filter) = &self.search_filter {
                self.filtered_file_nodes = self
                    .file_nodes
                    .iter()
                    .copied()
                    .filter(|id| filter.matches_node(*id))
                    .collect();
            } else {
                self.filtered_file_nodes = self.file_nodes.clone();
            }
            self.sort_file_lists();
        } else {
            self.filtered_file_nodes.clear();
        }
    }

    pub fn apply_search(&mut self) {
        if let Some(tree) = &self.tree {
            let trimmed = self.search.trim();
            if trimmed.is_empty() {
                self.search_filter = None;
            } else {
                self.search_filter = Some(SearchFilter::build(trimmed, tree));
            }
            self.refresh_filtered_files();
        } else {
            self.search_filter = None;
            self.filtered_file_nodes.clear();
        }
    }
}
