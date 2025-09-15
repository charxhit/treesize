use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use treesize_core::scanner::{ScanMsg, Scanner};

#[derive(Parser, Debug)]
#[command(name = "treesize-cli", about = "TreeSize report generator")]
struct Args {
    /// Root directory to scan
    root: PathBuf,
    /// Output JSON report path
    #[arg(short, long)]
    json: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = crossbeam_channel::unbounded::<ScanMsg>();
    let scanner = Scanner::new(cancel);
    std::thread::spawn({
        let root = args.root.clone();
        move || scanner.scan(root, tx)
    });

    let mut files = 0u64;
    let mut discovered = 0u64;
    let mut bytes = 0u128;
    while let Ok(msg) = rx.recv() {
        match msg {
            ScanMsg::Progress { scanned, discovered: d, bytes: b } => { files = scanned; discovered = d; bytes = b; }
            ScanMsg::Done(tree) => {
                if let Some(path) = args.json {
                    let json = treesize_core::export::to_json(&tree);
                    std::fs::write(path, serde_json::to_string_pretty(&json).unwrap()).ok();
                }
                break;
            }
            _ => {}
        }
    }
    println!("Scanned {} / {} files, {} bytes", files, discovered.max(files), bytes);
}
