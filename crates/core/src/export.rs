use crate::model::*;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::SystemTime;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("pdf error: {0}")]
    Pdf(#[from] printpdf::Error),
}

#[derive(Serialize)]
struct ExportRow {
    path: String,
    kind: &'static str,
    size_bytes: u128,
    files: u64,
    folders: u64,
    modified: String,
}

fn build_rows(tree: &Tree) -> Vec<ExportRow> {
    let dir_counts = compute_dir_counts(tree);
    tree.nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let kind = match node.kind {
                NodeKind::File => "file",
                NodeKind::Dir => "dir",
            };
            let (files, dirs) = if matches!(node.kind, NodeKind::File) {
                (0, 0)
            } else {
                (node.file_count, dir_counts[idx])
            };
            let modified = format_modified(node.modified);
            ExportRow {
                path: node.path.display().to_string(),
                kind,
                size_bytes: node.size,
                files,
                folders: dirs,
                modified,
            }
        })
        .collect()
}

fn compute_dir_counts(tree: &Tree) -> Vec<u64> {
    let mut counts = vec![0; tree.nodes.len()];
    for idx in (0..tree.nodes.len()).rev() {
        if matches!(tree.nodes[idx].kind, NodeKind::Dir) {
            let mut total = 0;
            for &child in &tree.nodes[idx].children {
                let cidx = child.0 as usize;
                if matches!(tree.nodes[cidx].kind, NodeKind::Dir) {
                    total += 1 + counts[cidx];
                }
            }
            counts[idx] = total;
        }
    }
    counts
}

fn format_modified(modified: Option<std::time::SystemTime>) -> String {
    modified
        .map(|ts| {
            DateTime::<Local>::from(ts)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "".to_string())
}

pub fn export_csv(tree: &Tree, path: &Path) -> Result<(), ExportError> {
    let rows = build_rows(tree);
    let file = File::create(path)?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(file));
    writer.write_record(["path", "kind", "size_bytes", "files", "folders", "modified"])?;
    for row in rows {
        writer.write_record([
            row.path,
            row.kind.to_string(),
            row.size_bytes.to_string(),
            row.files.to_string(),
            row.folders.to_string(),
            row.modified,
        ])?;
    }
    writer.flush()?;
    Ok(())
}

pub fn export_json(tree: &Tree, path: &Path) -> Result<(), ExportError> {
    let rows = build_rows(tree);
    let file = File::create(path)?;
    serde_json::to_writer_pretty(BufWriter::new(file), &rows)?;
    Ok(())
}

pub fn export_pdf(tree: &Tree, path: &Path) -> Result<(), ExportError> {
    use printpdf::*;
    let rows = build_rows(tree);
    let (doc, page, layer) = PdfDocument::new("TreeSize Export", Mm(210.0), Mm(297.0), "Layer 1");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let mut current_page = page;
    let mut current_layer = doc.get_page(current_page).get_layer(layer);
    let mut y = Mm(280.0);
    current_layer.use_text("TreeSize Export", 14.0, Mm(10.0), y, &font);
    y -= Mm(10.0);
    let line_height = Mm(5.0);
    for row in rows {
        let line = format!(
            "{} | {} | size={} | files={} | folders={} | {}",
            row.path, row.kind, row.size_bytes, row.files, row.folders, row.modified
        );
        if y.0 < 20.0 {
            let (new_page, new_layer) = doc.add_page(Mm(210.0), Mm(297.0), "Layer");
            current_page = new_page;
            current_layer = doc.get_page(current_page).get_layer(new_layer);
            y = Mm(280.0);
        }
        current_layer.use_text(line, 6.0, Mm(10.0), y, &font);
        y -= line_height;
    }
    let mut buf = BufWriter::new(File::create(path)?);
    doc.save(&mut buf)?;
    Ok(())
}
