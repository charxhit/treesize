use chrono::{DateTime, Local};
use eframe::egui::{
    self, collapsing_header::CollapsingState, Align2, Color32, Id, Pos2, ScrollArea, Sense,
    TextStyle, Ui,
};
use std::path::PathBuf;
use std::time::SystemTime;
use treesize_core::human::human_bytes;
use treesize_core::model::{NodeId, NodeKind, Tree, TreeNode};
use treesize_core::scanner::ScanMsg;

use crate::state::{AppState, SortKey};

const GB_FACTOR: f64 = 1024.0 * 1024.0 * 1024.0;
const MIN_SLICE_RATIO: f64 = 0.04;
const MAX_PRIMARY_SLICES: usize = 6;

#[derive(Default)]
struct FolderTreeActions {
    select: Option<NodeId>,
    open: Option<NodeId>,
    delete: Option<NodeId>,
    properties: Option<NodeId>,
}

#[derive(Default)]
struct PieActions {
    select: Option<NodeId>,
    open: Option<NodeId>,
    delete: Option<NodeId>,
    properties: Option<NodeId>,
}

struct PieSlice {
    id: Option<NodeId>,
    name: String,
    kind: NodeKind,
    bytes: u128,
    ratio: f64,
    color: Color32,
    path: PathBuf,
    modified: Option<SystemTime>,
    file_count: u64,
}

pub fn draw(app: &mut AppState, ctx: &egui::Context) {
    poll_scan(app, ctx);

    if app.scan_rx.is_some() {
        ctx.request_repaint();
    }

    egui::TopBottomPanel::top("top").show(ctx, |ui| {
        top_bar(ui, app);
    });

    egui::SidePanel::left("sidebar")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.heading("Folders");
            if let Some(root) = &app.root {
                ui.label(root.display().to_string());
            } else {
                ui.label("Choose a folder to start");
            }
            ui.separator();
            if ui.button("Up").clicked() {
                app.navigate_up();
            }
            if app.selected.is_some() && ui.button("Delete Selected").clicked() {
                if let Some(id) = app.selected {
                    app.request_delete(id);
                }
            }
            ui.separator();
            if let Some(tree) = &app.tree {
                let actions = draw_folder_tree(ui, app, tree);
                apply_folder_actions(app, actions);
            } else {
                ui.label("No folders scanned yet");
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Overview");
        ui.separator();
        ui.label(format!("Files: {}", app.progress_files));
        ui.label(format!("Bytes: {}", human_bytes(app.progress_bytes)));
        let progress = if app.progress_discovered > 0 {
            (app.progress_files as f32 / app.progress_discovered as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let progress_label = if app.scan_rx.is_some() {
            "Scanning..."
        } else {
            "Scan complete"
        };
        ui.add(
            egui::ProgressBar::new(progress)
                .show_percentage()
                .text(progress_label),
        );

        ui.separator();

        if app.current_dir.is_none() {
            if let Some(root_id) = app.tree.as_ref().map(|t| t.root) {
                app.current_dir = Some(root_id);
            }
        }

        if let Some(tree) = &app.tree {
            if let Some(cur) = app.current_dir {
                let node = &tree.nodes[cur.0 as usize];
                ui.horizontal(|ui| {
                    ui.strong("Dir:");
                    ui.label(node.path.display().to_string());
                });

                let mut children = node.children.clone();
                if !app.search.trim().is_empty() {
                    let needle = app.search.trim();
                    children.retain(|cid| {
                        let n = &tree.nodes[cid.0 as usize];
                        treesize_core::search::fuzzy_score(needle, &n.name).is_some()
                            || treesize_core::search::fuzzy_score(
                                needle,
                                &n.path.display().to_string(),
                            )
                            .is_some()
                    });
                }

                match app.sort {
                    SortKey::Size => children.sort_by(|a, b| {
                        tree.nodes[b.0 as usize]
                            .size
                            .cmp(&tree.nodes[a.0 as usize].size)
                    }),
                    SortKey::Name => children.sort_by(|a, b| {
                        tree.nodes[a.0 as usize]
                            .name
                            .cmp(&tree.nodes[b.0 as usize].name)
                    }),
                    SortKey::Count => children.sort_by(|a, b| {
                        tree.nodes[b.0 as usize]
                            .file_count
                            .cmp(&tree.nodes[a.0 as usize].file_count)
                    }),
                }

                let slices = collect_pie_slices(tree, &children);
                if slices.is_empty() {
                    ui.label("Nothing to display for this folder yet.");
                } else {
                    let actions = draw_pie_chart(ui, &slices, app.selected, app.current_dir);
                    apply_pie_actions(app, actions);
                }
            }
        }
    });

    show_delete_confirmation(ctx, app);
    show_properties_panel(ctx, app);
}

fn top_bar(ui: &mut Ui, app: &mut AppState) {
    ui.horizontal(|ui| {
        if ui.button("Choose Folder").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                app.start_scan(path);
            }
        }
        if ui.button("Cancel").clicked() {
            app.cancel_scan();
        }
        if app.selected.is_some() && ui.button("Delete Selected").clicked() {
            if let Some(id) = app.selected {
                app.request_delete(id);
            }
        }
        ui.separator();
        ui.label("Sort by:");
        egui::ComboBox::from_label("")
            .selected_text(match app.sort {
                SortKey::Size => "Size",
                SortKey::Name => "Name",
                SortKey::Count => "Files",
            })
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
    let Some(rx) = app.scan_rx.take() else {
        return;
    };
    let mut had_msg = false;
    let mut finished = false;
    while let Ok(msg) = rx.try_recv() {
        had_msg = true;
        match msg {
            ScanMsg::Progress {
                scanned,
                discovered,
                bytes,
            } => {
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
        app.scan_rx = Some(rx);
    }
    if had_msg {
        ctx.request_repaint();
    }
}

fn draw_folder_tree(ui: &mut Ui, app: &AppState, tree: &Tree) -> FolderTreeActions {
    let mut actions = FolderTreeActions::default();
    ScrollArea::vertical()
        .id_source("folder_tree_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            render_folder_node(
                ui,
                tree,
                tree.root,
                app.selected,
                app.current_dir,
                app.sort,
                &mut actions,
            );
        });
    actions
}

fn render_folder_node(
    ui: &mut Ui,
    tree: &Tree,
    node_id: NodeId,
    selected: Option<NodeId>,
    current: Option<NodeId>,
    sort: SortKey,
    actions: &mut FolderTreeActions,
) {
    ui.push_id(node_id.0, |ui| {
        render_folder_node_contents(ui, tree, node_id, selected, current, sort, actions);
    });
}

fn render_folder_node_contents(
    ui: &mut Ui,
    tree: &Tree,
    node_id: NodeId,
    selected: Option<NodeId>,
    current: Option<NodeId>,
    sort: SortKey,
    actions: &mut FolderTreeActions,
) {
    let node = &tree.nodes[node_id.0 as usize];
    if !matches!(node.kind, NodeKind::Dir) {
        return;
    }

    let id = ui.make_persistent_id(("folder_node", node_id.0));
    let state = CollapsingState::load_with_default_open(ui.ctx(), id, node.parent.is_none());
    let is_selected = selected == Some(node_id) || current == Some(node_id);
    let mut delete_clicked = false;
    let mut header_label_response = None;
    let label_text = format!("{} ({})", node.name, human_bytes(node.size));
    let header = state.show_header(ui, |ui| {
        ui.horizontal(|ui| {
            let response = ui.selectable_label(is_selected, label_text.clone());
            header_label_response = Some(response.clone());
            ui.add_space(6.0);
            if ui
                .small_button("Del")
                .on_hover_text("Delete this directory")
                .clicked()
            {
                delete_clicked = true;
            }
            response
        })
        .inner
    });
    let (_toggle, header_inner, _) = header.body(|ui| {
        let mut dir_children = Vec::new();
        let mut file_children = Vec::new();
        for &child in &node.children {
            let child_node = &tree.nodes[child.0 as usize];
            match child_node.kind {
                NodeKind::Dir => dir_children.push(child),
                NodeKind::File => file_children.push(child),
            }
        }

        sort_node_ids(&mut dir_children, tree, sort);
        sort_node_ids(&mut file_children, tree, sort);

        for child in dir_children {
            render_folder_node(ui, tree, child, selected, current, sort, actions);
        }

        for child in file_children {
            render_file_entry(ui, tree, child, selected, actions);
        }
    });
    if delete_clicked {
        actions.select = Some(node_id);
        actions.delete = Some(node_id);
    }

    let response = header_inner.response;

    if let Some(resp) = header_label_response {
        resp.on_hover_ui(|ui| show_node_metadata(ui, node));
    }
    response
        .clone()
        .on_hover_ui(|ui| show_node_metadata(ui, node));

    if response.clicked() {
        actions.select = Some(node_id);
        actions.open = Some(node_id);
    }

    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            actions.select = Some(node_id);
            actions.open = Some(node_id);
            ui.close_menu();
        }
        if ui.button("Delete").clicked() {
            actions.select = Some(node_id);
            actions.delete = Some(node_id);
            ui.close_menu();
        }
        if ui.button("Properties").clicked() {
            actions.select = Some(node_id);
            actions.properties = Some(node_id);
            ui.close_menu();
        }
    });
}

fn render_file_entry(
    ui: &mut Ui,
    tree: &Tree,
    node_id: NodeId,
    selected: Option<NodeId>,
    actions: &mut FolderTreeActions,
) {
    let node = &tree.nodes[node_id.0 as usize];
    let label = format!("{} ({})", node.name, human_bytes(node.size));
    let response = ui.selectable_label(selected == Some(node_id), label);
    let hover_response = response.clone();
    hover_response.on_hover_ui(|ui| show_node_metadata(ui, node));

    if response.clicked() {
        actions.select = Some(node_id);
    }

    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            let _ = open::that(&node.path);
            ui.close_menu();
        }
        if ui.button("Delete").clicked() {
            actions.select = Some(node_id);
            actions.delete = Some(node_id);
            ui.close_menu();
        }
        if ui.button("Properties").clicked() {
            actions.select = Some(node_id);
            actions.properties = Some(node_id);
            ui.close_menu();
        }
    });
}

fn show_node_metadata(ui: &mut Ui, node: &TreeNode) {
    ui.label(format!("Path: {}", node.path.display()));
    ui.label(match node.kind {
        NodeKind::Dir => format!("Kind: Directory"),
        NodeKind::File => format!("Kind: File"),
    });
    ui.label(format!("Size: {}", human_bytes(node.size)));
    if matches!(node.kind, NodeKind::Dir) {
        ui.label(format!("Files: {}", node.file_count));
    }
    ui.label(format!("Modified: {}", format_modified(node.modified)));
}

fn show_slice_metadata(ui: &mut Ui, slice: &PieSlice) {
    ui.label(format!("Name: {}", slice.name));
    ui.label(format!("Size: {}", human_bytes(slice.bytes)));
    match slice.id {
        Some(_) => {
            ui.label(format!("Path: {}", slice.path.display()));
            if matches!(slice.kind, NodeKind::Dir) {
                ui.label(format!("Files: {}", slice.file_count));
            }
            ui.label(format!("Modified: {}", format_modified(slice.modified)));
        }
        None => {
            ui.label("Aggregated from remaining items");
            if slice.file_count > 0 {
                ui.label(format!("Combined files: {}", slice.file_count));
            }
        }
    }
}

fn format_modified(modified: Option<SystemTime>) -> String {
    match modified {
        Some(time) => {
            let datetime: DateTime<Local> = time.into();
            datetime.format("%Y-%m-%d %H:%M:%S").to_string()
        }
        None => "Unknown".to_string(),
    }
}

fn truncate_middle(value: &str, max_len: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_len {
        return value.to_string();
    }
    if max_len <= 3 {
        return value.chars().take(max_len).collect();
    }
    let chars: Vec<char> = value.chars().collect();
    let keep = max_len - 3;
    let first = keep / 2;
    let second = keep - first;
    let start: String = chars[..first].iter().collect();
    let end: String = chars[char_count - second..].iter().collect();
    format!("{}...{}", start, end)
}

fn sort_node_ids(nodes: &mut Vec<NodeId>, tree: &Tree, sort: SortKey) {
    match sort {
        SortKey::Size => nodes.sort_by(|a, b| {
            tree.nodes[b.0 as usize]
                .size
                .cmp(&tree.nodes[a.0 as usize].size)
        }),
        SortKey::Name => nodes.sort_by(|a, b| {
            tree.nodes[a.0 as usize]
                .name
                .cmp(&tree.nodes[b.0 as usize].name)
        }),
        SortKey::Count => nodes.sort_by(|a, b| {
            tree.nodes[b.0 as usize]
                .file_count
                .cmp(&tree.nodes[a.0 as usize].file_count)
        }),
    }
}

fn show_delete_confirmation(ctx: &egui::Context, app: &mut AppState) {
    let delete_id = match app.pending_delete {
        Some(id) => id,
        None => return,
    };

    let (path_display, item_label, item_kind, size_label) = app
        .tree
        .as_ref()
        .and_then(|tree| tree.nodes.get(delete_id.0 as usize))
        .map(|node| {
            (
                node.path.display().to_string(),
                node.name.clone(),
                node.kind.clone(),
                human_bytes(node.size),
            )
        })
        .unwrap_or_else(|| {
            (
                String::from("(unknown)"),
                String::from("item"),
                NodeKind::File,
                String::new(),
            )
        });

    let mut confirm = false;
    let mut cancel = false;
    let mut open = true;
    egui::Window::new("Confirm Delete")
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            let kind_text = match item_kind {
                NodeKind::Dir => "folder",
                NodeKind::File => "file",
            };
            ui.heading(format!("Delete {kind_text}?"));
            ui.label(format!("Name: {item_label}"));
            ui.label(format!("Path: {path_display}"));
            if !size_label.is_empty() {
                ui.label(format!("Size: {size_label}"));
            }
            ui.separator();
            ui.label("This action cannot be undone.");
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui
                    .add(egui::Button::new("Delete").fill(Color32::from_rgb(170, 50, 50)))
                    .clicked()
                {
                    confirm = true;
                }
            });
        });

    if confirm {
        app.delete_selected_and_rescan();
        app.pending_delete = None;
        ctx.request_repaint();
    } else if cancel || !open {
        app.pending_delete = None;
    }
}

fn show_properties_panel(ctx: &egui::Context, app: &mut AppState) {
    let properties_id = match app.pending_properties {
        Some(id) => id,
        None => return,
    };

    let Some(tree) = app.tree.as_ref() else {
        app.pending_properties = None;
        return;
    };
    let Some(node) = tree.nodes.get(properties_id.0 as usize) else {
        app.pending_properties = None;
        return;
    };

    let mut open = true;
    egui::Window::new("Properties")
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.heading(&node.name);
            show_node_metadata(ui, node);
            if matches!(node.kind, NodeKind::Dir) {
                ui.label(format!("Contains: {} files", node.file_count));
            }
            ui.separator();
            if ui.button("Open Externally").clicked() {
                let _ = open::that(&node.path);
            }
        });

    if !open {
        app.pending_properties = None;
    }
}

fn collect_pie_slices(tree: &Tree, children: &[NodeId]) -> Vec<PieSlice> {
    let mut items: Vec<_> = children
        .iter()
        .map(|cid| {
            let node = &tree.nodes[cid.0 as usize];
            (*cid, node)
        })
        .filter(|(_, node)| node.size > 0)
        .collect();

    if items.is_empty() {
        return Vec::new();
    }

    items.sort_by(|a, b| b.1.size.cmp(&a.1.size));

    let total: f64 = items.iter().map(|(_, node)| node.size as f64).sum();
    if total == 0.0 {
        return Vec::new();
    }

    let mut slices = Vec::new();
    let mut other_bytes: u128 = 0;
    let mut other_ratio = 0.0;
    let mut other_files: u64 = 0;

    for (index, (id, node)) in items.iter().enumerate() {
        let ratio = node.size as f64 / total;
        let file_count = if matches!(node.kind, NodeKind::Dir) {
            node.file_count
        } else {
            node.file_count.max(1)
        };
        if slices.len() < MAX_PRIMARY_SLICES
            && (index < MAX_PRIMARY_SLICES || ratio >= MIN_SLICE_RATIO)
        {
            let color = palette_color(slices.len());
            slices.push(PieSlice {
                id: Some(*id),
                name: node.name.clone(),
                kind: node.kind.clone(),
                bytes: node.size,
                ratio,
                color,
                path: node.path.clone(),
                modified: node.modified,
                file_count,
            });
        } else {
            other_bytes += node.size;
            other_ratio += ratio;
            other_files += file_count;
        }
    }

    if other_bytes > 0 {
        slices.push(PieSlice {
            id: None,
            name: "Other".to_string(),
            kind: NodeKind::Dir,
            bytes: other_bytes,
            ratio: other_ratio,
            color: Color32::from_gray(110),
            path: PathBuf::new(),
            modified: None,
            file_count: other_files,
        });
    }

    slices
}

fn draw_pie_chart(
    ui: &mut Ui,
    slices: &[PieSlice],
    selected: Option<NodeId>,
    current: Option<NodeId>,
) -> PieActions {
    let mut actions = PieActions::default();

    let legend_width = 220.0;
    let tooltip_id = Id::new("pie_slice_tooltip");
    ui.horizontal(|ui| {
        let available_width = ui.available_width();
        let available_height = ui.available_height();
        let mut chart_side = if available_width > legend_width + 200.0 {
            (available_width - legend_width).min(available_height)
        } else {
            available_width.min(available_height)
        };
        chart_side = chart_side.max(220.0);
        chart_side = chart_side.min(available_width - 20.0).max(200.0);

        let (chart_rect, response) =
            ui.allocate_exact_size(egui::vec2(chart_side, chart_side), Sense::click());
        let painter = ui.painter().with_clip_rect(chart_rect);
        let center = chart_rect.center();
        let radius = ((chart_rect.width().min(chart_rect.height()) / 2.0) - 14.0).max(0.0);
        let tau = std::f32::consts::TAU;

        let hovered_index = response
            .hover_pos()
            .and_then(|pos| slice_at_pos(slices, pos, center, radius, tau));
        let clicked_index = if response.clicked() {
            response
                .interact_pointer_pos()
                .and_then(|pos| slice_at_pos(slices, pos, center, radius, tau))
        } else {
            None
        };

        if let Some(idx) = clicked_index {
            if let Some(id) = slices[idx].id {
                actions.select = Some(id);
                if matches!(slices[idx].kind, NodeKind::Dir) {
                    actions.open = Some(id);
                }
            }
        }

        if let Some(idx) = hovered_index {
            egui::show_tooltip(ui.ctx(), ui.layer_id(), tooltip_id, |ui| {
                show_slice_metadata(ui, &slices[idx]);
            });
        }

        let mut start_angle = 0.0f32;
        for (index, slice) in slices.iter().enumerate() {
            let sweep = (slice.ratio as f32).max(0.0) * tau;
            if sweep <= 0.0 {
                continue;
            }

            let mut color = slice.color;
            if Some(index) == hovered_index {
                color = lighten(color, 35);
            }
            if slice.id.is_some() && (selected == slice.id || current == slice.id) {
                color = lighten(color, 20);
            }

            let points = wedge_points(center, radius, start_angle, sweep);
            painter.add(egui::Shape::convex_polygon(
                points,
                color,
                egui::Stroke::new(1.0, Color32::BLACK),
            ));

            if sweep > 0.1 {
                let mid = start_angle + sweep / 2.0;
                let label_pos = Pos2::new(
                    center.x + radius * 0.6 * mid.cos(),
                    center.y + radius * 0.6 * mid.sin(),
                );
                let name_label = truncate_middle(&slice.name, 28);
                let label = format!("{}\n{}", name_label, format_gb(slice.bytes));
                painter.text(
                    label_pos,
                    Align2::CENTER_CENTER,
                    label,
                    TextStyle::Small.resolve(ui.style()),
                    Color32::WHITE,
                );
            }

            start_angle += sweep;
        }

        let hovered_for_menu = hovered_index;
        response.context_menu(|ui| {
            if let Some(idx) = hovered_for_menu {
                if let Some(id) = slices[idx].id {
                    if ui.button("Open").clicked() {
                        actions.select = Some(id);
                        if matches!(slices[idx].kind, NodeKind::Dir) {
                            actions.open = Some(id);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Delete").clicked() {
                        actions.select = Some(id);
                        actions.delete = Some(id);
                        ui.close_menu();
                    }
                    if ui.button("Properties").clicked() {
                        actions.select = Some(id);
                        actions.properties = Some(id);
                        ui.close_menu();
                    }
                } else {
                    ui.label("No actions available");
                }
            } else {
                ui.label("Hover an item for actions");
            }
        });

        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_min_width(legend_width);
            ui.strong("Breakdown");
            ui.add_space(6.0);
            for slice in slices {
                let percentage = slice.ratio * 100.0;
                ui.horizontal(|ui| {
                    let (color_rect, _color_resp) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(color_rect, 2.0, slice.color);
                    ui.add_space(4.0);
                    ui.label(format!(
                        "{name} - {size} - {percent:.1}%",
                        name = &slice.name,
                        size = format_gb(slice.bytes),
                        percent = percentage
                    ));
                });
            }
        });
    });

    actions
}

fn slice_at_pos(
    slices: &[PieSlice],
    pos: Pos2,
    center: Pos2,
    radius: f32,
    tau: f32,
) -> Option<usize> {
    let dx = pos.x - center.x;
    let dy = pos.y - center.y;
    let dist_sq = dx * dx + dy * dy;
    if dist_sq > radius * radius || radius <= 0.0 {
        return None;
    }
    let mut angle = dy.atan2(dx);
    if angle < 0.0 {
        angle += tau;
    }
    let mut start = 0.0f32;
    for (index, slice) in slices.iter().enumerate() {
        let sweep = (slice.ratio as f32).max(0.0) * tau;
        let end = start + sweep;
        if angle >= start && angle <= end {
            return Some(index);
        }
        start = end;
    }
    if slices.is_empty() {
        None
    } else {
        Some(slices.len() - 1)
    }
}

fn wedge_points(center: Pos2, radius: f32, start_angle: f32, sweep: f32) -> Vec<Pos2> {
    let segments = ((sweep.abs() * radius) / 20.0).ceil().max(2.0) as usize;
    let mut points = Vec::with_capacity(segments + 2);
    points.push(center);
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let angle = start_angle + sweep * t;
        points.push(Pos2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    points
}

fn format_gb(bytes: u128) -> String {
    let gb = bytes as f64 / GB_FACTOR;
    if gb >= 100.0 {
        format!("{:.0} GB", gb)
    } else if gb >= 10.0 {
        format!("{:.1} GB", gb)
    } else {
        format!("{:.2} GB", gb)
    }
}

fn lighten(color: Color32, amount: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r().saturating_add(amount),
        color.g().saturating_add(amount),
        color.b().saturating_add(amount),
        color.a(),
    )
}

fn palette_color(index: usize) -> Color32 {
    const PALETTE: [Color32; 10] = [
        Color32::from_rgb(0x5B, 0x8C, 0xCB),
        Color32::from_rgb(0xE7, 0x84, 0x3C),
        Color32::from_rgb(0x76, 0xB7, 0xB2),
        Color32::from_rgb(0xED, 0xC9, 0x79),
        Color32::from_rgb(0xBC, 0x65, 0x81),
        Color32::from_rgb(0x7F, 0xC8, 0xA9),
        Color32::from_rgb(0xF2, 0xA6, 0x76),
        Color32::from_rgb(0x86, 0x99, 0xC7),
        Color32::from_rgb(0xA1, 0xD9, 0xCE),
        Color32::from_rgb(0xF5, 0xB7, 0xB1),
    ];
    PALETTE[index % PALETTE.len()]
}

fn apply_pie_actions(app: &mut AppState, actions: PieActions) {
    if let Some(id) = actions.select {
        app.selected = Some(id);
    }
    if let Some(id) = actions.open {
        app.current_dir = Some(id);
    }
    if let Some(id) = actions.delete {
        app.request_delete(id);
    }
    if let Some(id) = actions.properties {
        app.request_properties(id);
    }
}

fn apply_folder_actions(app: &mut AppState, actions: FolderTreeActions) {
    if let Some(id) = actions.select {
        app.selected = Some(id);
    }
    if let Some(id) = actions.open {
        app.current_dir = Some(id);
    }
    if let Some(id) = actions.delete {
        app.request_delete(id);
    }
    if let Some(id) = actions.properties {
        app.request_properties(id);
    }
}
