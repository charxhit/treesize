use eframe::egui::{
    self, collapsing_header::CollapsingState, Align2, Color32, Pos2, ScrollArea, Sense, TextStyle,
    Ui,
};
use treesize_core::human::human_bytes;
use treesize_core::model::{NodeId, NodeKind, Tree};
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
}

#[derive(Default)]
struct PieActions {
    select: Option<NodeId>,
    open: Option<NodeId>,
    delete: Option<NodeId>,
}

struct PieSlice {
    id: Option<NodeId>,
    name: String,
    kind: NodeKind,
    bytes: u128,
    ratio: f64,
    color: Color32,
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
                app.delete_selected_and_rescan();
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
        ui.add(
            egui::ProgressBar::new(progress)
                .show_percentage()
                .text("Scanning..."),
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
    actions: &mut FolderTreeActions,
) {
    let node = &tree.nodes[node_id.0 as usize];
    if !matches!(node.kind, NodeKind::Dir) {
        return;
    }

    let id = ui.make_persistent_id(("folder_node", node_id.0));
    let state = CollapsingState::load_with_default_open(ui.ctx(), id, node.parent.is_none());
    let is_selected = selected == Some(node_id) || current == Some(node_id);
    let header = state.show_header(ui, |ui| {
        ui.selectable_label(
            is_selected,
            format!("{} ({})", node.name, human_bytes(node.size)),
        )
    });
    let (_toggle, header_inner, _) = header.body(|ui| {
        for &child in &node.children {
            render_folder_node(ui, tree, child, selected, current, actions);
        }
    });
    let response = header_inner.response;

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
    });
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

    for (index, (id, node)) in items.iter().enumerate() {
        let ratio = node.size as f64 / total;
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
            });
        } else {
            other_bytes += node.size;
            other_ratio += ratio;
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
    ui.horizontal(|ui| {
        let available_width = ui.available_width();
        let available_height = ui.available_height();
        let mut chart_side = if available_width > legend_width + 140.0 {
            (available_width - legend_width).min(available_height)
        } else {
            available_width.min(available_height)
        };
        chart_side = chart_side.max(160.0);

        let (chart_rect, response) =
            ui.allocate_exact_size(egui::vec2(chart_side, chart_side), Sense::click());
        let painter = ui.painter().with_clip_rect(chart_rect);
        let center = chart_rect.center();
        let radius = (chart_rect.width().min(chart_rect.height()) / 2.0 - 14.0).max(0.0);
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
                let label = format!("{}\n{}", slice.name, format_gb(slice.bytes));
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
        app.selected = Some(id);
        app.delete_selected_and_rescan();
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
        app.selected = Some(id);
        app.delete_selected_and_rescan();
    }
}
