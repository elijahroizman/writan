//! SmartArt: the shapes a diagram shows. Office caches the drawing it laid out
//! (`dsp:drawing`, found through the data part's `dsp:dataModelExt`); those shapes come back as
//! PresentationML shapes (`p:sp`, `p:grpSp`) in the diagram's own coordinates, so any DrawingML
//! shape renderer can draw them. A diagram without a cached drawing gets a plain layout of its
//! top-level points (boxes in a row, with arrows for processes), so its text still shows.

use std::collections::HashMap;
use std::sync::Arc;

use crate::opc::Package;
use crate::units::Emu;
use crate::xml::{ns, parse_element, Element, Node};

/// Whether `graphic_data` (`a:graphicData`) holds a SmartArt diagram.
pub fn is_diagram(graphic_data: &Element) -> bool {
    graphic_data.attr("uri") == Some(ns::DGM) || graphic_data.child(ns::DGM, "relIds").is_some()
}

/// The shapes of the diagram in `graphic_data`, which sits in `part`, as `p:sp` and `p:grpSp`
/// elements positioned in EMUs from the diagram's top left. `size` is the diagram's extent,
/// which the fallback layout fills.
pub fn shapes(pkg: &Package, part: &str, graphic_data: &Element, size: (Emu, Emu)) -> Vec<Element> {
    let Some(ids) = graphic_data.child(ns::DGM, "relIds") else { return Vec::new() };
    let Ok(rels) = pkg.rels(part) else { return Vec::new() };
    let target = |id: &str| rels.get(id).and_then(|rel| pkg.target(part, rel)).filter(|t| pkg.has(t));
    let Some(data) = ids.attr_ns(ns::R, "dm").and_then(target).and_then(|t| pkg.xml(&t).ok()) else { return Vec::new() };
    let drawing =
        data.root.descendant(ns::DSP, "dataModelExt").and_then(|e| e.attr("relId")).and_then(target).and_then(|t| pkg.xml(&t).ok());
    let cached = drawing.and_then(|d| d.root.child(ns::DSP, "spTree").map(cached_shapes)).unwrap_or_default();
    if !cached.is_empty() {
        return cached;
    }
    let kind = data.root.descendant(ns::DGM, "prSet").and_then(|p| p.attr("loTypeId").or(p.attr("loCatId"))).unwrap_or("");
    fallback(&data.root, kind.contains("process") || kind.contains("Process"), size)
}

/// The cached drawing's shapes, renamed into PresentationML. A shape whose text has its own box
/// (`dsp:txXfrm`) becomes the shape without its text and, above it, an unfilled shape with the
/// text in that box.
fn cached_shapes(tree: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    for el in tree.elements() {
        if el.is(ns::DSP, "sp") {
            out.extend(split_text(el));
        } else if el.is(ns::DSP, "grpSp") {
            let mut g = el.clone();
            // Only the group's own properties and its shapes: anything else (a nested graphic
            // frame naming this diagram again) is dropped, so drawing can't recurse.
            g.children.retain(|c| matches!(c, Node::Element(e) if e.is(ns::DSP, "nvGrpSpPr") || e.is(ns::DSP, "grpSpPr")));
            for c in cached_shapes(el) {
                g.push(c);
            }
            out.push(to_pml(g));
        }
    }
    out
}

fn split_text(sp: &Element) -> Vec<Element> {
    let (Some(tx), Some(_)) = (sp.child(ns::DSP, "txXfrm"), sp.child(ns::DSP, "txBody")) else {
        return vec![to_pml(sp.clone())];
    };
    let mut shape = sp.clone();
    shape.children.retain(|c| !matches!(c, Node::Element(e) if e.is(ns::DSP, "txBody") || e.is(ns::DSP, "txXfrm")));
    let mut xfrm = tx.clone();
    xfrm.prefix = Some("a".into());
    xfrm.ns = Some(Arc::from(ns::A));
    xfrm.local = "xfrm".into();
    let sp_pr = Element::new(ns::DSP, "spPr")
        .with_child(xfrm)
        .with_child(Element::new(ns::A, "prstGeom").with_attr("", "prst", "rect").with_child(Element::new(ns::A, "avLst")))
        .with_child(Element::new(ns::A, "noFill"))
        .with_child(Element::new(ns::A, "ln").with_child(Element::new(ns::A, "noFill")));
    let mut text = Element::new(ns::DSP, "sp");
    for c in sp.elements() {
        if c.is(ns::DSP, "spPr") {
            text.push(sp_pr.clone());
        } else if c.is(ns::DSP, "nvSpPr") || c.is(ns::DSP, "style") || c.is(ns::DSP, "txBody") {
            text.push(c.clone());
        }
    }
    vec![to_pml(shape), to_pml(text)]
}

/// Moves an element and its descendants from the diagram drawing's namespace to
/// PresentationML's, which has the same shape markup.
fn to_pml(mut el: Element) -> Element {
    if el.in_ns(ns::DSP) {
        el.ns = Some(Arc::from(ns::P));
        el.prefix = Some("p".into());
    }
    for c in el.children.iter_mut() {
        if let Node::Element(e) = c {
            *e = to_pml(std::mem::replace(e, Element::new("", "")));
        }
    }
    el
}

/// A point's text, one string per paragraph, without empty paragraphs.
fn point_text(pt: &Element) -> Vec<String> {
    pt.child(ns::DGM, "t")
        .map(|t| {
            t.children_named(ns::A, "p")
                .map(|p| {
                    p.elements()
                        .filter(|r| r.is(ns::A, "r") || r.is(ns::A, "fld"))
                        .filter_map(|r| r.child(ns::A, "t"))
                        .map(Element::text)
                        .collect::<String>()
                })
                .filter(|s| !s.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The data model's top-level points with their text, and each one's children's text.
fn outline(data: &Element) -> Vec<(Vec<String>, Vec<String>)> {
    let pts: HashMap<&str, &Element> = data
        .child(ns::DGM, "ptLst")
        .into_iter()
        .flat_map(|l| l.children_named(ns::DGM, "pt"))
        .filter_map(|p| Some((p.attr("modelId")?, p)))
        .collect();
    let is_node = |id: &str| pts.get(id).is_some_and(|p| matches!(p.attr("type"), None | Some("node") | Some("asst")));
    let mut children: HashMap<&str, Vec<(i64, &str)>> = HashMap::new();
    for c in data.child(ns::DGM, "cxnLst").into_iter().flat_map(|l| l.children_named(ns::DGM, "cxn")) {
        if !matches!(c.attr("type"), None | Some("parOf")) {
            continue;
        }
        let (Some(src), Some(dest)) = (c.attr("srcId"), c.attr("destId")) else { continue };
        if is_node(dest) {
            children.entry(src).or_default().push((c.attr("srcOrd").and_then(|o| o.parse().ok()).unwrap_or(0), dest));
        }
    }
    for list in children.values_mut() {
        list.sort();
    }
    let Some(root) = pts.values().find(|p| p.attr("type") == Some("doc")).and_then(|p| p.attr("modelId")) else { return Vec::new() };
    let kids = |id: &str| children.get(id).map(|l| l.iter().map(|(_, d)| *d).collect::<Vec<_>>()).unwrap_or_default();
    kids(root)
        .into_iter()
        .map(|id| {
            let own = pts.get(id).map(|p| point_text(p)).unwrap_or_default();
            let sub = kids(id).into_iter().filter_map(|c| pts.get(c)).flat_map(|p| point_text(p)).collect();
            (own, sub)
        })
        .filter(|(own, sub): &(Vec<String>, Vec<String>)| !own.is_empty() || !sub.is_empty())
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Boxes in a row for the top-level points, their children's text under their own, joined by
/// arrows when the diagram is a process.
fn fallback(data: &Element, process: bool, (w, h): (Emu, Emu)) -> Vec<Element> {
    let nodes = outline(data);
    let n = nodes.len();
    if n == 0 || w.0 <= 0 || h.0 <= 0 {
        return Vec::new();
    }
    let (w, h) = (w.0 as f64, h.0 as f64);
    let gap_share = if process { 0.3 } else { 0.1 };
    let bw = w / (n as f64 + (n as f64 - 1.0) * gap_share);
    let gap = bw * gap_share;
    let deep = nodes.iter().any(|(_, sub)| !sub.is_empty());
    let bh = h.min(bw * if deep { 1.3 } else { 0.75 });
    let y = (h - bh) / 2.0;
    let lines = nodes.iter().map(|(own, sub)| own.len() + sub.len()).max().unwrap_or(1).max(1) as f64;
    let longest = nodes.iter().flat_map(|(own, sub)| own.iter().chain(sub)).map(|s| s.chars().count()).max().unwrap_or(1).max(1) as f64;
    // Points (EMU / 12700), shrunk until the tallest box's lines and the longest word fit.
    let pt = ((bh / 12700.0) / (lines * 1.8)).min((bw / 12700.0) * 0.85 / (longest.min(14.0) * 0.55)).clamp(8.0, 28.0);
    let para = |text: &str, size: f64, bold: bool| {
        format!(
            r#"<a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="{}"{}><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p>"#,
            (size * 100.0).round() as i64,
            if bold { r#" b="1""# } else { "" },
            escape(text)
        )
    };
    let mut out = Vec::new();
    let mut id = 1;
    let mut push = |xml: String| {
        let wrapped = format!(r#"<p:sp xmlns:p="{}" xmlns:a="{}">{}</p:sp>"#, ns::P, ns::A, xml);
        if let Ok(el) = parse_element(&wrapped) {
            out.push(el);
        }
    };
    for (i, (own, sub)) in nodes.iter().enumerate() {
        let x = i as f64 * (bw + gap);
        let mut paras: String = own.iter().map(|t| para(t, pt, !sub.is_empty())).collect();
        paras.extend(sub.iter().map(|t| para(t, pt * 0.8, false)));
        push(format!(
            r#"<p:nvSpPr><p:cNvPr id="{id}" name=""/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"/></a:solidFill><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></p:spPr><p:txBody><a:bodyPr wrap="square" lIns="45720" tIns="45720" rIns="45720" bIns="45720" anchor="ctr"><a:noAutofit/></a:bodyPr><a:lstStyle/>{paras}</p:txBody>"#,
            x.round() as i64,
            y.round() as i64,
            bw.round() as i64,
            bh.round() as i64,
        ));
        id += 1;
        if process && i + 1 < n {
            let aw = gap * 0.7;
            let ah = (bw * 0.25).min(bh * 0.5);
            push(format!(
                r#"<p:nvSpPr><p:cNvPr id="{id}" name=""/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rightArrow"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"><a:tint val="60000"/></a:schemeClr></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr>"#,
                (x + bw + (gap - aw) / 2.0).round() as i64,
                (h / 2.0 - ah / 2.0).round() as i64,
                aw.round() as i64,
                ah.round() as i64,
            ));
            id += 1;
        }
    }
    out
}

/// For tests: adds a SmartArt diagram to `host` in `bytes`: its data part (two top-level points, "Plan"
/// with a child "Scope", then "Build") and, if `cached`, Office's cached drawing of one shape
/// reading "Cached". `place` puts the diagram's `a:graphicData` into the host part's XML.
#[doc(hidden)]
pub fn test_diagram(bytes: &[u8], host: &str, cached: bool, place: impl Fn(&str, &str) -> String) -> Vec<u8> {
    let mut pkg = Package::read(bytes).unwrap();
    let dir = &host[..host.find('/').unwrap()];
    let (data_part, drawing_part) = (format!("{dir}/diagrams/data1.xml"), format!("{dir}/diagrams/drawing1.xml"));
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
    let ext = if cached {
        pkg.put(&drawing_part, br#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:spTree><dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/><dsp:sp modelId="{1}"><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="3048000" cy="1524000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></dsp:spPr><dsp:style><a:lnRef idx="0"><a:scrgbClr r="0" g="0" b="0"/></a:lnRef><a:fillRef idx="1"><a:scrgbClr r="0" g="0" b="0"/></a:fillRef><a:effectRef idx="0"><a:scrgbClr r="0" g="0" b="0"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></dsp:style><dsp:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="2000"/><a:t>Cached</a:t></a:r></a:p></dsp:txBody><dsp:txXfrm><a:off x="76200" y="76200"/><a:ext cx="2895600" cy="1371600"/></dsp:txXfrm></dsp:sp></dsp:spTree></dsp:drawing>"#.to_vec(), "application/vnd.ms-office.drawingml.diagramDrawing+xml");
        let id = pkg.relate(host, "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing", &drawing_part).unwrap();
        format!(
            r#"<dgm:extLst><a:ext uri="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:dataModelExt xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" relId="{id}"/></a:ext></dgm:extLst>"#
        )
    } else {
        String::new()
    };
    let data = format!(
        r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="0" type="doc"><dgm:prSet loTypeId="urn:microsoft.com/office/officeart/2005/8/layout/process3"/></dgm:pt><dgm:pt modelId="1"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Plan</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="11"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Scope</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="2"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Build</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst><dgm:cxnLst><dgm:cxn modelId="4" srcId="0" destId="1" srcOrd="0"/><dgm:cxn modelId="5" srcId="0" destId="2" srcOrd="1"/><dgm:cxn modelId="6" srcId="1" destId="11" srcOrd="0"/></dgm:cxnLst>{ext}</dgm:dataModel>"#
    );
    pkg.put(&data_part, data.into_bytes(), "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml");
    let dm = pkg.relate(host, &format!("{rel}diagramData"), &data_part).unwrap();
    let graphic_data = format!(
        r#"<a:graphicData xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="{}" r:dm="{dm}" r:lo="" r:qs="" r:cs=""/></a:graphicData>"#,
        ns::R
    );
    let xml = String::from_utf8(pkg.part(host).unwrap().to_vec()).unwrap();
    pkg.replace(host, place(&xml, &graphic_data).into_bytes()).unwrap();
    pkg.to_bytes().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="0" type="doc"><dgm:prSet loTypeId="urn:microsoft.com/office/officeart/2005/8/layout/process3"/></dgm:pt><dgm:pt modelId="1"><dgm:t><a:p><a:r><a:t>Plan</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="11"><dgm:t><a:p><a:r><a:t>Scope</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="2"><dgm:t><a:p><a:r><a:t>Build</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="9" type="pres"><dgm:t><a:p><a:r><a:t>hidden</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst><dgm:cxnLst><dgm:cxn modelId="5" srcId="0" destId="2" srcOrd="1"/><dgm:cxn modelId="4" srcId="0" destId="1" srcOrd="0"/><dgm:cxn modelId="6" srcId="1" destId="11" srcOrd="0"/><dgm:cxn modelId="7" type="presOf" srcId="1" destId="9"/></dgm:cxnLst></dgm:dataModel>"#;

    #[test]
    fn outline_follows_parent_connections_in_order() {
        let data = parse_element(DATA).unwrap();
        assert_eq!(outline(&data), vec![(vec!["Plan".to_string()], vec!["Scope".to_string()]), (vec!["Build".to_string()], vec![])]);
    }

    #[test]
    fn fallback_lays_out_boxes_and_arrows_inside_the_extent() {
        let data = parse_element(DATA).unwrap();
        let shapes = fallback(&data, true, (Emu(6_096_000), Emu(4_064_000)));
        let geoms: Vec<&str> = shapes.iter().filter_map(|s| s.descendant(ns::A, "prstGeom").and_then(|g| g.attr("prst"))).collect();
        assert_eq!(geoms, ["roundRect", "rightArrow", "roundRect"]);
        for s in &shapes {
            assert!(s.is(ns::P, "sp"));
            let x = crate::drawing::Xfrm::parse(s.descendant(ns::A, "xfrm").unwrap());
            assert!(x.x.0 >= 0 && x.x.0 + x.width.0 <= 6_096_001 && x.y.0 >= 0 && x.y.0 + x.height.0 <= 4_064_001);
        }
        assert_eq!(shapes[0].text(), "PlanScope");
    }

    #[test]
    fn cached_groups_keep_only_shapes() {
        let tree = parse_element(r#"<dsp:spTree xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:grpSp><dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/><dsp:graphicFrame><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic></dsp:graphicFrame><dsp:sp><dsp:spPr/></dsp:sp></dsp:grpSp></dsp:spTree>"#).unwrap();
        let shapes = cached_shapes(&tree);
        let kids: Vec<&str> = shapes[0].elements().map(|e| e.local.as_str()).collect();
        assert_eq!(kids, ["nvGrpSpPr", "grpSpPr", "sp"]);
        assert!(shapes[0].elements().all(|e| e.in_ns(ns::P)));
    }

    #[test]
    fn cached_text_box_becomes_its_own_unfilled_shape() {
        let tree = parse_element(r#"<dsp:spTree xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:sp><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm><a:prstGeom prst="ellipse"/></dsp:spPr><dsp:txBody><a:bodyPr/><a:p><a:r><a:t>Hi</a:t></a:r></a:p></dsp:txBody><dsp:txXfrm><a:off x="10" y="20"/><a:ext cx="80" cy="60"/></dsp:txXfrm></dsp:sp></dsp:spTree>"#).unwrap();
        let shapes = cached_shapes(&tree);
        assert_eq!(shapes.len(), 2);
        assert!(shapes.iter().all(|s| s.is(ns::P, "sp") && s.child(ns::P, "spPr").is_some()));
        assert!(shapes[0].child(ns::P, "txBody").is_none());
        let text = &shapes[1];
        assert_eq!(text.text(), "Hi");
        let x = crate::drawing::Xfrm::parse(text.path(ns::P, &["spPr"]).unwrap().child(ns::A, "xfrm").unwrap());
        assert_eq!((x.x.0, x.y.0, x.width.0, x.height.0), (10, 20, 80, 60));
        assert!(text.child(ns::P, "spPr").and_then(|s| s.child(ns::A, "noFill")).is_some());
    }
}
