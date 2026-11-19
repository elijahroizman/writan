use super::*;

fn shape(name: &str) -> Geometry {
    preset(name, 200.0, 100.0, &[]).unwrap_or_else(|| panic!("missing {name}"))
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 0.011, "{actual} != {expected}");
}

// Validate the explicit absolute-command subset emitted here: lexical numbers,
// arities, flags, finite coordinates, two-decimal precision and a leading M.
fn validate(d: &str) -> Vec<(u8, Vec<f64>)> {
    assert!(!d.is_empty());
    let bytes = d.as_bytes();
    let mut at = 0;
    let mut commands = Vec::new();
    while at < bytes.len() {
        let cmd = bytes[at];
        at += 1;
        let arity = match cmd {
            b'M' | b'L' => 2,
            b'Q' => 4,
            b'C' => 6,
            b'A' => 7,
            b'Z' => 0,
            _ => panic!("invalid command in {d}"),
        };
        let mut args = Vec::new();
        for i in 0..arity {
            if i > 0 {
                assert_eq!(bytes.get(at), Some(&b','), "{d}");
                at += 1;
            }
            let start = at;
            if bytes.get(at) == Some(&b'-') {
                at += 1;
            }
            let digits = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            assert!(at > digits, "invalid number in {d}");
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                let fraction = at;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                assert!(at > fraction && at - fraction <= 2, "invalid precision in {d}");
            }
            let n: f64 = d[start..at].parse().unwrap();
            assert!(n.is_finite());
            args.push(n);
        }
        if cmd == b'A' {
            assert!(args[0] >= 0.0 && args[1] >= 0.0);
            assert!(matches!(args[3], 0.0 | 1.0) && matches!(args[4], 0.0 | 1.0));
        }
        commands.push((cmd, args));
    }
    assert_eq!(commands[0].0, b'M');
    commands
}

#[test]
fn rect_and_text_box() {
    let g = shape("rect");
    assert_eq!(g.text_rect, (0.0, 0.0, 200.0, 100.0));
    assert_eq!(g.paths.len(), 1);
    assert_eq!(g.paths[0].d, "M0,0L200,0L200,100L0,100Z");
    assert_eq!(g.paths[0].fill, PathFill::Norm);
    assert!(g.paths[0].stroke);
}

#[test]
fn round_rect_default_and_override() {
    let g = shape("roundRect");
    let cmds = validate(&g.paths[0].d);
    near(cmds[0].1[1], 16.667);
    assert_eq!(cmds.iter().filter(|c| c.0 == b'A').count(), 4);
    near(cmds[1].1[0], 16.667);
    let g = preset("roundRect", 200.0, 100.0, &[("adj".into(), "val 25000".into())]).unwrap();
    let cmds = validate(&g.paths[0].d);
    assert_eq!(cmds[0].1, [0.0, 25.0]);
    assert_eq!(cmds[1].1, [25.0, 25.0, 0.0, 0.0, 1.0, 25.0, 0.0]);
    near(g.text_rect.0, 7.32225);
    near(g.text_rect.2, 192.67775);
    let clamped = preset("roundRect", 200.0, 100.0, &[("adj".into(), "val 90000".into())]).unwrap();
    assert_eq!(validate(&clamped.paths[0].d)[0].1, [0.0, 50.0]);
}

#[test]
fn ellipse_arcs_and_inscribed_text_box() {
    let g = shape("ellipse");
    let commands = validate(&g.paths[0].d);
    let arcs: Vec<_> = commands.iter().filter(|c| c.0 == b'A').collect();
    assert_eq!(arcs.len(), 4);
    for (arc, expected) in arcs.iter().zip([[100.0, 0.0], [200.0, 50.0], [100.0, 100.0], [0.0, 50.0]]) {
        assert_eq!(&arc.1[..5], [100.0, 50.0, 0.0, 0.0, 1.0]);
        assert_eq!(arc.1[5..], expected);
        let (x, y) = (arc.1[5], arc.1[6]);
        near(((x - 100.0) / 100.0).powi(2) + ((y - 50.0) / 50.0).powi(2), 1.0);
    }
    let inset = 1.0 - std::f64::consts::FRAC_1_SQRT_2;
    near(g.text_rect.0, 100.0 * inset);
    near(g.text_rect.1, 50.0 * inset);
    near(g.text_rect.2, 200.0 - 100.0 * inset);
    near(g.text_rect.3, 100.0 - 50.0 * inset);
}

#[test]
fn collapsed_ellipses_retain_their_axis_extents() {
    assert_eq!(preset("ellipse", 0.0, 100.0, &[]).unwrap().paths[0].d, "M0,50L0,0L0,50L0,100L0,50Z");
    assert_eq!(preset("ellipse", 200.0, 0.0, &[]).unwrap().paths[0].d, "M0,0L100,0L200,0L100,0L0,0Z");
}

#[test]
fn right_arrow_adjustments() {
    let g = preset("rightArrow", 200.0, 100.0, &[("adj1".into(), "val 25000".into()), ("adj2".into(), "val 30000".into())]).unwrap();
    assert_eq!(g.paths[0].d, "M0,37.5L170,37.5L170,0L200,50L170,100L170,62.5L0,62.5Z");
    assert_eq!(g.text_rect, (0.0, 37.5, 192.5, 62.5));
}

#[test]
fn star5_spec_vertices() {
    let g = shape("star5");
    let cmds = validate(&g.paths[0].d);
    assert_eq!(cmds.len(), 11);
    assert_eq!(cmds[2].1, [100.0, 0.0]);
    near(cmds[0].1[0], 0.0);
    near(cmds[0].1[1], 38.1966);
    near(cmds[4].1[0], 200.0);
    near(cmds[4].1[1], 38.1966);
    assert!(g.text_rect.0 < 100.0 && g.text_rect.2 > 100.0);
    let changed = preset("star5", 200.0, 100.0, &[("adj".into(), "val 35000".into())]).unwrap();
    assert_ne!(g.paths[0].d, changed.paths[0].d);
}

#[test]
fn rounded_callout_keeps_pointer_and_rounding() {
    let g = shape("wedgeRoundRectCallout");
    let cmds = validate(&g.paths[0].d);
    assert_eq!(cmds.iter().filter(|c| c.0 == b'A').count(), 4);
    assert!(cmds.iter().any(|c| c.0 == b'L' && c.1 == [58.33, 112.5]));
    near(g.text_rect.0, 4.8816);
    let moved =
        preset("wedgeRoundRectCallout", 200.0, 100.0, &[("adj1".into(), "val 75000".into()), ("adj2".into(), "val 0".into())]).unwrap();
    assert!(validate(&moved.paths[0].d).iter().any(|c| c.0 == b'L' && c.1 == [250.0, 50.0]));
}

#[test]
fn every_connector_accepts_zero_dimensions() {
    for name in [
        "line",
        "straightConnector1",
        "bentConnector2",
        "bentConnector3",
        "bentConnector4",
        "bentConnector5",
        "curvedConnector2",
        "curvedConnector3",
        "curvedConnector4",
        "curvedConnector5",
    ] {
        for (w, h) in [(200.0, 0.0), (0.0, 100.0), (0.0, 0.0)] {
            let g = preset(name, w, h, &[]).unwrap();
            for p in g.paths {
                validate(&p.d);
            }
        }
    }
    assert_eq!(preset("line", 200.0, 0.0, &[]).unwrap().paths[0].d, "M0,0L200,0");
    assert_eq!(preset("bentConnector2", 200.0, 0.0, &[]).unwrap().paths[0].d, "M0,0L200,0L200,0");
}

#[test]
fn chevron() {
    let g = shape("chevron");
    assert_eq!(g.paths[0].d, "M0,0L150,0L200,50L150,100L0,100L50,50Z");
    assert_eq!(g.text_rect, (50.0, 0.0, 150.0, 100.0));
}

fn custom_xml(body: &str) -> Element {
    office_core::xml::parse_element(&format!("<a:custGeom xmlns:a=\"{}\">{body}</a:custGeom>", ns::A)).unwrap()
}

#[test]
fn custom_geometry_guides_spaces_beziers_and_paint() {
    let xml = custom_xml(
        r#"
      <a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
      <a:gdLst><a:gd name="inset" fmla="*/ w adj 100000"/></a:gdLst>
      <a:rect l="inset" t="t" r="r" b="b"/>
      <a:pathLst>
        <a:path w="100" h="100" fill="darkenLess" stroke="0" extrusionOk="false">
          <a:moveTo><a:pt x="0" y="0"/></a:moveTo>
          <a:quadBezTo><a:pt x="50" y="0"/><a:pt x="100" y="50"/></a:quadBezTo>
          <a:cubicBezTo><a:pt x="100" y="75"/><a:pt x="50" y="100"/><a:pt x="0" y="100"/></a:cubicBezTo>
          <a:close/>
        </a:path>
        <a:path fill="none" stroke="true">
          <a:moveTo><a:pt x="inset" y="0"/></a:moveTo>
          <a:lnTo><a:pt x="9525" y="9525"/></a:lnTo>
        </a:path>
      </a:pathLst>"#,
    );
    let g = custom(&xml, 200.0, 100.0).unwrap();
    assert_eq!(g.text_rect, (50.0, 0.0, 200.0, 100.0));
    assert_eq!(g.paths[0].d, "M0,0Q100,0,200,50C200,75,100,100,0,100Z");
    assert_eq!(g.paths[0].fill, PathFill::DarkenLess);
    assert!(!g.paths[0].stroke);
    assert_eq!(g.paths[1].d, "M50,0L1,1");
    assert_eq!(g.paths[1].fill, PathFill::None);
    assert!(g.paths[1].stroke);
    let def = Definition::from_xml(&xml).unwrap();
    assert!(!def.paths[0]._extrusion_ok);
    assert!(def.paths[1]._extrusion_ok);
    for path in g.paths {
        validate(&path.d);
    }
}

#[test]
fn non_cardinal_elliptical_arc_uses_polar_angles() {
    // A 45-degree ray meets the 100x50 ellipse at (44.72,44.72).
    let start = 100.0 / 5.0_f64.sqrt();
    let xml = custom_xml(&format!(
        r#"<a:pathLst><a:path w="200" h="100">
      <a:moveTo><a:pt x="{start}" y="{start}"/></a:moveTo>
      <a:arcTo wR="100" hR="50" stAng="2700000" swAng="5400000"/>
      </a:path></a:pathLst>"#
    ));
    let g = custom(&xml, 200.0, 100.0).unwrap();
    let cmds = validate(&g.paths[0].d);
    let end = &cmds[1].1[5..];
    near(end[0], -start);
    near(end[1], start);
    near(end[0].powi(2) / 10000.0 + end[1].powi(2) / 2500.0, 1.0);
    // Scale the path nonuniformly. Its polar angles belong to the unscaled
    // path coordinate space, so converting them after scaling would be wrong.
    let stretched = custom(&xml, 400.0, 100.0).unwrap();
    let cmds = validate(&stretched.paths[0].d);
    near(cmds[1].1[0], 200.0);
    near(cmds[1].1[5], -2.0 * start);
    near(cmds[1].1[6], start);
}

#[test]
fn guide_dependencies_shadowing_and_strict_namespace() {
    let mut xml = custom_xml(
        r#"
      <a:avLst><a:gd name="adjustment" fmla="val 25000"/></a:avLst>
      <a:gdLst>
        <a:gd name="x" fmla="*/ w adjustment 100000"/>
        <a:gd name="previous" fmla="val x"/>
        <a:gd name="x" fmla="+- x 9525 0"/>
      </a:gdLst>
      <a:pathLst><a:path>
        <a:moveTo><a:pt x="previous" y="0"/></a:moveTo>
        <a:lnTo><a:pt x="x" y="h"/></a:lnTo>
      </a:path></a:pathLst>"#,
    );
    assert_eq!(custom(&xml, 200.0, 100.0).unwrap().paths[0].d, "M50,0L51,100");
    // Namespace matching must also accept strict OOXML and arbitrary prefixes.
    xml = office_core::xml::parse_element(
        &xml.to_xml_string()
            .replace(ns::A, "http://purl.oclc.org/ooxml/drawingml/main")
            .replace("a:", "other:")
            .replace("xmlns:a", "xmlns:other"),
    )
    .unwrap();
    assert_eq!(custom(&xml, 200.0, 100.0).unwrap().paths[0].d, "M50,0L51,100");
}

#[test]
fn full_turn_negative_sweep_and_close_restore_current_point() {
    let xml = custom_xml(
        r#"<a:pathLst><a:path w="200" h="100">
      <a:moveTo><a:pt x="200" y="50"/></a:moveTo>
      <a:arcTo wR="100" hR="50" stAng="0" swAng="-21600000"/>
      <a:lnTo><a:pt x="10" y="20"/></a:lnTo><a:close/>
      <a:arcTo wR="100" hR="50" stAng="0" swAng="5400000"/>
      </a:path></a:pathLst>"#,
    );
    let g = custom(&xml, 200.0, 100.0).unwrap();
    let cmds = validate(&g.paths[0].d);
    assert_eq!(cmds[1].1, [100.0, 50.0, 0.0, 0.0, 0.0, 0.0, 50.0]);
    assert_eq!(cmds[2].1, [100.0, 50.0, 0.0, 0.0, 0.0, 200.0, 50.0]);
    assert_eq!(cmds.last().unwrap().1, [100.0, 50.0, 0.0, 0.0, 1.0, 100.0, 100.0]);
}

#[test]
fn all_formula_operators_and_builtins() {
    for (formula, expected) in [
        ("*/ 6 7 3", 14.0),
        ("+- 6 7 3", 10.0),
        ("+/ 6 7 2", 6.5),
        ("?: 1 2 3", 2.0),
        ("?: 0 2 3", 3.0),
        ("?: -1 2 3", 3.0),
        ("abs -2", 2.0),
        ("at2 0 1", 5_400_000.0),
        ("cat2 10 3 4", 6.0),
        ("cos 10 cd2", -10.0),
        ("max 2 3", 3.0),
        ("min 2 3", 2.0),
        ("mod 2 3 6", 7.0),
        ("pin 2 0 3", 2.0),
        ("pin 2 4 3", 3.0),
        ("pin 2 2.5 3", 2.5),
        ("sat2 10 3 4", 8.0),
        ("sin 10 cd4", 10.0),
        ("sqrt 9", 3.0),
        ("tan 10 cd8", 10.0),
        ("val 42", 42.0),
        ("*/ 2 3 0", 0.0),
        ("+/ 1 2 0", 0.0),
        ("sqrt -1", 0.0),
    ] {
        near(Formula::compile(formula, &[]).unwrap().evaluate(&[], 200.0, 100.0), expected);
    }
    for (name, expected) in [
        ("l", 0.0),
        ("t", 0.0),
        ("w", 200.0),
        ("r", 200.0),
        ("h", 100.0),
        ("b", 100.0),
        ("hc", 100.0),
        ("vc", 50.0),
        ("ss", 100.0),
        ("ls", 200.0),
        ("wd3", 200.0 / 3.0),
        ("wd32", 6.25),
        ("hd8", 12.5),
        ("ssd16", 6.25),
        ("cd2", 10_800_000.0),
        ("3cd4", 16_200_000.0),
        ("7cd8", 18_900_000.0),
    ] {
        near(Operand::compile(name, &[]).unwrap().value(&[], 200.0, 100.0), expected);
    }
}

#[test]
fn every_preset_evaluates_to_valid_paths() {
    let names: Vec<_> = preset_names().collect();
    assert_eq!(names.len(), 187);
    assert!(names.windows(2).all(|w| w[0] < w[1]));
    for name in names {
        for (w, h) in [(200.0, 100.0), (10.0, 300.0), (0.0, 100.0), (200.0, 0.0), (0.0, 0.0)] {
            let geometry = preset(name, w, h, &[]).unwrap_or_else(|| panic!("{name} at {w}x{h}"));
            assert!(!geometry.paths.is_empty(), "{name}");
            for path in geometry.paths {
                validate(&path.d);
            }
            let (l, t, r, b) = geometry.text_rect;
            assert!([l, t, r, b].iter().all(|n| n.is_finite()), "{name}");
        }
    }
}

#[test]
fn all_fill_modes_and_compact_numbers() {
    for (name, expected) in [
        ("norm", PathFill::Norm),
        ("none", PathFill::None),
        ("lighten", PathFill::Lighten),
        ("lightenLess", PathFill::LightenLess),
        ("darken", PathFill::Darken),
        ("darkenLess", PathFill::DarkenLess),
    ] {
        let xml = custom_xml(&format!(r#"<a:pathLst><a:path fill="{name}"><a:moveTo><a:pt x="0" y="0"/></a:moveTo></a:path></a:pathLst>"#));
        assert_eq!(custom(&xml, 100.0, 100.0).unwrap().paths[0].fill, expected);
    }
    let mut s = String::new();
    command(&mut s, 'C', &[-0.00001, 1.0, 1.2, 1.234, -2.456, 0.009]);
    assert_eq!(s, "C0,1,1.2,1.23,-2.46,0.01");
}

#[test]
fn malformed_input_returns_none() {
    assert!(preset("unknown", 200.0, 100.0, &[]).is_none());
    assert!(preset("rect", -1.0, 100.0, &[]).is_none());
    assert!(preset("rect", f64::NAN, 100.0, &[]).is_none());
    assert!(preset("roundRect", 200.0, 100.0, &[("adj".into(), "oops 1".into())]).is_none());
    for body in [
        "<a:pathLst><a:path><a:moveTo><a:pt x=\"unknown\" y=\"0\"/></a:moveTo></a:path></a:pathLst>",
        "<a:gdLst><a:gd name=\"x\" fmla=\"val x\"/></a:gdLst><a:pathLst/>",
        "<a:pathLst><a:path><a:quadBezTo><a:pt x=\"0\" y=\"0\"/></a:quadBezTo></a:path></a:pathLst>",
        "<a:pathLst><a:path fill=\"invalid\"/></a:pathLst>",
    ] {
        assert!(custom(&custom_xml(body), 100.0, 100.0).is_none());
    }
    assert!(custom(&Element::new("", "custGeom"), 100.0, 100.0).is_none());
}

#[test]
fn embedded_presets_include_rect() {
    let rect = preset("rect", 10.0, 5.0, &[]).expect("rect preset");
    assert!(!rect.paths.is_empty());
    assert!(preset_names().any(|name| name == "roundRect"));
}

#[test]
fn presets_from_a_crlf_checkout_keep_their_names() {
    let crlf: &'static str = Box::leak(include_str!("presets.dat").replace('\n', "\r\n").into_boxed_str());
    let definitions = parse_definitions(crlf);
    assert_eq!(definitions.len(), parse_definitions(include_str!("presets.dat")).len());
    assert!(definitions.iter().any(|(name, _)| *name == "rect"));
    assert!(definitions.iter().all(|(name, _)| !name.ends_with('\r')));
}
