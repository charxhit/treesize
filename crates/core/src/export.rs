use crate::model::*;

pub fn to_csv(tree: &Tree, mut w: impl std::io::Write) -> csv::Result<()> {
    let mut writer = csv::Writer::from_writer(&mut w);
    writer
        .write_record(["path", "name", "kind", "size", "files", "modified"])
        .ok();
    for n in &tree.nodes {
        let kind: String = match n.kind {
            crate::model::NodeKind::File => "file",
            crate::model::NodeKind::Dir => "dir",
        }
        .to_string();
        let modified: String = n.modified.map(|_| "some".to_string()).unwrap_or_default();
        writer.write_record([
            n.path.display().to_string(),
            n.name.clone(),
            kind,
            n.size.to_string(),
            n.file_count.to_string(),
            modified,
        ])?;
    }
    writer.flush()?;
    Ok(())
}

pub fn to_json(tree: &Tree) -> serde_json::Value {
    serde_json::json!({
        "root": tree.root.0,
        "nodes": tree.nodes.iter().map(|n| serde_json::json!({
            "id": n.id.0,
            "parent": n.parent.as_ref().map(|p| p.0),
            "path": n.path,
            "name": n.name,
            "kind": match n.kind { crate::model::NodeKind::File => "file", crate::model::NodeKind::Dir => "dir"},
            "size": n.size,
            "file_count": n.file_count,
            "children": n.children.iter().map(|c| c.0).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

pub fn to_pdf(_tree: &Tree, out: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use printpdf::*;
    let (doc, page1, layer1) = PdfDocument::new("TreeSize Report", Mm(210.0), Mm(297.0), "Layer 1");
    let layer = doc.get_page(page1).get_layer(layer1);
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    layer.use_text("TreeSize Report", 14.0, Mm(15.0), Mm(280.0), &font);
    let file = std::fs::File::create(out)?;
    let mut buf = std::io::BufWriter::new(file);
    doc.save(&mut buf)?;
    Ok(())
}
