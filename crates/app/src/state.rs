use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use treesize_core::model::{NodeId, Tree};
use treesize_core::scanner::{ScanMsg, Scanner};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Size,
    Name,
    Count,
}

pub struct SearchFilter {
    pub needle: String,
    pub node_count: usize,
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

    /// Build a filter in a single pass over all nodes plus a DFS aggregation for subtrees.
    /// The matcher is intentionally simple (case-insensitive substring) for responsiveness.
    pub fn build(needle: &str, tree: &Tree) -> Self {
        let needle_lc = needle.to_ascii_lowercase();
        let n = tree.nodes.len();
        let mut direct = vec![false; n];

        for (i, node) in tree.nodes.iter().enumerate() {
            let mut matched = node.name.to_ascii_lowercase().contains(&needle_lc);
            if !matched {
                // Avoid allocating unless needed
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

        SearchFilter { needle: needle.to_string(), node_count: n, direct_matches: direct, subtree_matches: subtree }
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
    // Debounce + identity tracking for search filter rebuilds
    last_search_built: Option<String>,
    search_ready_at: Option<Instant>,
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
            last_search_built: None,
            search_ready_at: None,
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
        self.last_search_built = None;
        self.search_ready_at = None;
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

    pub fn cancel_scan(&self) { self.cancel.store(true, Ordering::Relaxed); }

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
        self.last_search_built = None;
        self.search_ready_at = None;
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
        if let Some(tree) = &self.tree { self.current_dir = Some(tree.root); }
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
                let _ = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            }
            if let Some(root) = &self.root { self.start_scan(root.clone()); }
        }
    }

    /// Notify that the search input changed; starts a short debounce window.
    pub fn on_search_changed(&mut self) {
        self.search_ready_at = Some(Instant::now() + Duration::from_millis(120));
        // Invalidate identity so we rebuild once due
        self.last_search_built = None;
    }

    /// Rebuild the cached search filter if due and inputs changed.
    pub fn update_search_filter_if_due(&mut self) {
        let Some(tree) = &self.tree else { self.search_filter = None; return; };
        let needle = self.search.trim();
        if needle.is_empty() {
            self.search_filter = None;
            self.last_search_built = None;
            return;
        }
        if let Some(ready) = self.search_ready_at {
            if Instant::now() < ready { return; }
        }
        if self.last_search_built.as_deref() == Some(needle)
            && self.search_filter.as_ref().is_some_and(|f| f.node_count == tree.nodes.len())
        {
            return;
        }
        let filter = SearchFilter::build(needle, tree);
        self.last_search_built = Some(needle.to_string());
        self.search_filter = Some(filter);
    }
}