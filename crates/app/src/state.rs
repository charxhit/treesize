use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use treesize_core::scanner::{ScanMsg, Scanner};
use treesize_core::model::Tree;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey { Size, Name, Count }

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
        }
    }

    pub fn start_scan(&mut self, root: PathBuf) {
        self.root = Some(root.clone());
        self.progress_bytes = 0;
        self.progress_files = 0;
        self.progress_discovered = 0;
        self.tree = None;
        self.cancel.store(false, Ordering::Relaxed);

        let (tx, rx): (Sender<ScanMsg>, Receiver<ScanMsg>) = unbounded();
        self.scan_rx = Some(rx);
        let cancel = self.cancel.clone();

        std::thread::spawn(move || {
            let scanner = Scanner::new(cancel);
            scanner.scan(root, tx);
        });
    }

    pub fn cancel_scan(&self) { self.cancel.store(true, Ordering::Relaxed); }
}
