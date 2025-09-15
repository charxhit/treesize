use eframe::egui::{self, Ui};
use treesize_core::human::human_bytes;
use treesize_core::scanner::ScanMsg;

use crate::state::{AppState, SortKey};

pub fn draw(app: &mut AppState, ctx: &egui::Context) {
    poll_scan(app, ctx);

    // Ensure the UI keeps repainting during active scans
    if app.scan_rx.is_some() {
        ctx.request_repaint();
    }

    egui::TopBottomPanel::top("top").show(ctx, |ui| {
        top_bar(ui, app);
    });

    egui::SidePanel::left("sidebar").resizable(true).default_width(320.0).show(ctx, |ui| {
        ui.heading("Folders");
        if let Some(root) = &app.root { ui.label(root.display().to_string()); }
        else { ui.label("Choose a folder to start"); }
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Overview");
        ui.separator();
        ui.label(format!("Files: {}", app.progress_files));
        ui.label(format!("Bytes: {}", human_bytes(app.progress_bytes)));
        let progress = if app.progress_discovered > 0 {
            (app.progress_files as f32 / app.progress_discovered as f32).clamp(0.0, 1.0)
        } else { 0.0 };
        ui.add(egui::ProgressBar::new(progress).show_percentage().text("Scanning…"));

        ui.separator();
        if let Some(tree) = &app.tree {
            ui.heading("Root Contents");
            if let Some(root) = tree.nodes.get(tree.root.0 as usize) {
                let mut children = root.children.clone();
                match app.sort {
                    SortKey::Size => children.sort_by(|a, b| tree.nodes[b.0 as usize].size.cmp(&tree.nodes[a.0 as usize].size)),
                    SortKey::Name => children.sort_by(|a, b| tree.nodes[a.0 as usize].name.cmp(&tree.nodes[b.0 as usize].name)),
                    SortKey::Count => children.sort_by(|a, b| tree.nodes[b.0 as usize].file_count.cmp(&tree.nodes[a.0 as usize].file_count)),
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for id in children {
                        let n = &tree.nodes[id.0 as usize];
                        let label = match n.kind {
                            treesize_core::model::NodeKind::Dir => format!("📁 {}  ({} files, {})", n.name, n.file_count, human_bytes(n.size)),
                            treesize_core::model::NodeKind::File => format!("📄 {}  ({})", n.name, human_bytes(n.size)),
                        };
                        ui.label(label);
                    }
                });
            }
        }
    });
}

fn top_bar(ui: &mut Ui, app: &mut AppState) {
    ui.horizontal(|ui| {
        if ui.button("Choose Folder").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() { app.start_scan(path); }
        }
        if ui.button("Cancel").clicked() { app.cancel_scan(); }
        ui.separator();
        ui.label("Sort by:");
        egui::ComboBox::from_label("")
            .selected_text(match app.sort { SortKey::Size=>"Size", SortKey::Name=>"Name", SortKey::Count=>"Files"})
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.sort, SortKey::Size, "Size");
                ui.selectable_value(&mut app.sort, SortKey::Name, "Name");
                ui.selectable_value(&mut app.sort, SortKey::Count, "Files");
            });
        ui.separator();
        ui.label("Search:");
        ui.text_edit_singleline(&mut app.search);
    });
}

fn poll_scan(app: &mut AppState, ctx: &egui::Context) {
    // Take ownership of the receiver to avoid borrowing while we might assign to it.
    let Some(rx) = app.scan_rx.take() else { return; };
    let mut had_msg = false;
    let mut finished = false;
    while let Ok(msg) = rx.try_recv() {
        had_msg = true;
        match msg {
            ScanMsg::Progress { scanned, discovered, bytes } => {
                app.progress_files = scanned;
                app.progress_discovered = discovered;
                app.progress_bytes = bytes;
            }
            ScanMsg::File { .. } => {}
            ScanMsg::DirDone { .. } => {}
            ScanMsg::Done(tree) => {
                app.tree = Some(tree);
                finished = true;
                break;
            }
            ScanMsg::Error(_e) => {}
        }
    }
    if !finished {
        // Put the receiver back to keep polling next frame
        app.scan_rx = Some(rx);
    }
    if had_msg {
        ctx.request_repaint();
    }
}
