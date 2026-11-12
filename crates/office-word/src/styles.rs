//! Styles (`word/styles.xml`): what a paragraph's `pStyle` and a run's `rStyle` mean, and the
//! formatting text really has once styles, document defaults and direct formatting are put
//! together the way Word does.

use office_core::drawing::Theme;
use office_core::formats::docx::{read_para_props, read_run_props, val};
use office_core::model::{ParaProps, RunProps, Strike};
use office_core::xml::{ns, Element, XmlDoc};
use serde::{Deserialize, Serialize};

/// Word toggles not all represented in the shared run model yet. Kept here so style
/// resolution preserves them without changing office-core's model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunToggles {
    pub strike: Option<bool>,
    pub double_strike: Option<bool>,
    pub outline: Option<bool>,
    pub shadow: Option<bool>,
    pub emboss: Option<bool>,
    pub imprint: Option<bool>,
    pub vanish: Option<bool>,
}

impl RunToggles {
    fn read(rpr: &Element) -> Self {
        let toggle = |name: &str| rpr.child(ns::W, name).map(|e| val(e).and_then(office_core::units::parse_bool).unwrap_or(true));
        Self {
            strike: toggle("strike"),
            double_strike: toggle("dstrike"),
            outline: toggle("outline"),
            shadow: toggle("shadow"),
            emboss: toggle("emboss"),
            imprint: toggle("imprint"),
            vanish: toggle("vanish"),
        }
    }

    fn apply(&mut self, over: &Self, style: bool) {
        macro_rules! take { ($($f:ident),*) => { $(if let Some(v) = over.$f {
            self.$f = Some(if style { self.$f.unwrap_or(false) ^ v } else { v });
        })* }; }
        take!(strike, double_strike, outline, shadow, emboss, imprint, vanish);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedRun {
    pub props: RunProps,
    pub toggles: RunToggles,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    pub id: String,
    pub name: String,
    pub kind: StyleKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    /// The style the next paragraph gets when you press return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// The kind's default style (Normal, Default Paragraph Font, Normal Table).
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub para: ParaProps,
    #[serde(default)]
    pub run: RunProps,
    #[serde(default)]
    pub run_toggles: RunToggles,
    /// The list a paragraph style puts its paragraphs in (`w:numPr/w:numId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_id: Option<String>,
    /// Shown in Word's style gallery.
    #[serde(default)]
    pub quick: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Styles {
    pub styles: Vec<Style>,
    /// The document defaults (`w:docDefaults`).
    pub default_para: ParaProps,
    pub default_run: RunProps,
    pub default_run_toggles: RunToggles,
}

impl Styles {
    pub fn parse(doc: &XmlDoc) -> Styles {
        let root = &doc.root;
        let defaults = root.child(ns::W, "docDefaults");
        let default_run = defaults.and_then(|d| d.path(ns::W, &["rPrDefault", "rPr"])).map(read_run_props).unwrap_or_default();
        let default_para = defaults.and_then(|d| d.path(ns::W, &["pPrDefault", "pPr"])).map(read_para_props).unwrap_or_default();
        let styles = root.children_named(ns::W, "style").filter_map(parse_style).collect();
        let default_run_toggles = defaults.and_then(|d| d.path(ns::W, &["rPrDefault", "rPr"])).map(RunToggles::read).unwrap_or_default();
        Styles { styles, default_para, default_run, default_run_toggles }
    }

    pub fn get(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id)
    }

    /// The style named `name` (as Word shows it: "heading 1"), ignoring case.
    pub fn by_name(&self, name: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// The default style of a kind: Normal for paragraphs.
    pub fn default_of(&self, kind: StyleKind) -> Option<&Style> {
        self.styles.iter().find(|s| s.kind == kind && s.default)
    }

    /// A style and the styles it's based on, the root first. Stops at loops.
    pub fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out: Vec<&Style> = Vec::new();
        let mut next = self.get(id);
        while let Some(s) = next {
            if out.iter().any(|o| o.id == s.id) {
                break;
            }
            out.push(s);
            next = s.based_on.as_deref().and_then(|b| self.get(b));
        }
        out.reverse();
        out
    }

    /// A paragraph's formatting with its style and the document defaults filled in.
    pub fn resolve_para(&self, direct: &ParaProps) -> ParaProps {
        let style = direct.style.clone().or_else(|| self.default_of(StyleKind::Paragraph).map(|s| s.id.clone()));
        let mut p = self.default_para.clone();
        for s in style.as_deref().map(|id| self.chain(id)).unwrap_or_default() {
            over_para(&mut p, &s.para);
        }
        over_para(&mut p, direct);
        p.style = style;
        p
    }

    /// A run's formatting as it shows: document defaults, then the paragraph's style, then
    /// the run's character style, then its own. Bold, italic, caps and strikethrough set in
    /// styles toggle, as Word has it: a character style's bold on a bold heading makes it plain
    /// again. Theme fonts (`+mn-lt`) are turned into their names.
    pub fn resolve_run(&self, para: &ParaProps, direct: &RunProps, theme: Option<&Theme>) -> RunProps {
        self.resolve_run_with_toggles(para, direct, None, theme).props
    }

    /// Full Word run formatting, including toggles awaiting fields in the shared model.
    /// `direct_xml` is the run's raw w:rPr for those additional direct properties.
    pub fn resolve_run_with_toggles(
        &self,
        para: &ParaProps,
        direct: &RunProps,
        direct_xml: Option<&Element>,
        theme: Option<&Theme>,
    ) -> ResolvedRun {
        let para_style = para.style.as_deref().or_else(|| self.default_of(StyleKind::Paragraph).map(|s| s.id.as_str()));
        let mut r = self.default_run.clone();
        let mut toggles = self.default_run_toggles.clone();
        if toggles.strike.is_none() && toggles.double_strike.is_none() {
            match r.strike {
                Some(Strike::Single) => toggles.strike = Some(true),
                Some(Strike::Double) => toggles.double_strike = Some(true),
                Some(Strike::None) => {
                    toggles.strike = Some(false);
                    toggles.double_strike = Some(false);
                }
                None => {}
            }
        }
        for id in [para_style, direct.style.as_deref()].into_iter().flatten() {
            for s in self.chain(id) {
                let (bold, italic, caps, small_caps) = (r.bold, r.italic, r.caps, r.small_caps);
                let (bold_cs, italic_cs) = (r.bold_complex, r.italic_complex);
                over_run(&mut r, &s.run);
                let toggle = |base: Option<bool>, over: Option<bool>| over.map(|v| base.unwrap_or(false) ^ v).or(base);
                r.bold = toggle(bold, s.run.bold);
                r.italic = toggle(italic, s.run.italic);
                r.bold_complex = toggle(bold_cs, s.run.bold_complex);
                r.italic_complex = toggle(italic_cs, s.run.italic_complex);
                r.caps = toggle(caps, s.run.caps);
                r.small_caps = toggle(small_caps, s.run.small_caps);
                toggles.apply(&s.run_toggles, true);
            }
        }
        r.strike = if toggles.double_strike == Some(true) {
            Some(Strike::Double)
        } else if toggles.strike == Some(true) {
            Some(Strike::Single)
        } else if toggles.strike.is_some() || toggles.double_strike.is_some() {
            Some(Strike::None)
        } else {
            None
        };
        if let Some(strike) = direct.strike {
            toggles.strike = Some(strike == Strike::Single);
            toggles.double_strike = Some(strike == Strike::Double);
        }
        if let Some(xml) = direct_xml {
            toggles.apply(&RunToggles::read(xml), false);
        }
        over_run(&mut r, direct);
        r.style = direct.style.clone();
        if let Some(theme) = theme {
            for f in [&mut r.font, &mut r.font_east_asian, &mut r.font_complex] {
                if let Some(name) = f.as_deref().and_then(|t| theme.font(t)) {
                    *f = Some(name.to_string());
                }
            }
        }
        ResolvedRun { props: r, toggles }
    }
}

fn parse_style(el: &Element) -> Option<Style> {
    let kind = match el.attr_ns(ns::W, "type").unwrap_or("paragraph") {
        "paragraph" => StyleKind::Paragraph,
        "character" => StyleKind::Character,
        "table" => StyleKind::Table,
        "numbering" => StyleKind::Numbering,
        _ => return None,
    };
    let child_val = |local: &str| el.child(ns::W, local).and_then(val).map(str::to_string);
    let ppr = el.child(ns::W, "pPr");
    Some(Style {
        id: el.attr_ns(ns::W, "styleId")?.to_string(),
        name: child_val("name").unwrap_or_default(),
        kind,
        based_on: child_val("basedOn"),
        next: child_val("next"),
        default: el.attr_ns(ns::W, "default").and_then(office_core::units::parse_bool).unwrap_or(false),
        para: ppr.map(read_para_props).unwrap_or_default(),
        run: el.child(ns::W, "rPr").map(read_run_props).unwrap_or_default(),
        run_toggles: el.child(ns::W, "rPr").map(RunToggles::read).unwrap_or_default(),
        num_id: ppr.and_then(|p| p.path(ns::W, &["numPr", "numId"])).and_then(val).map(str::to_string),
        quick: el.child(ns::W, "qFormat").is_some(),
    })
}

pub fn over_run(base: &mut RunProps, over: &RunProps) {
    base.overlay(over);
}

pub fn over_para(base: &mut ParaProps, over: &ParaProps) {
    base.overlay(over);
}

/// The styles Word has built in, as Word 2016 and later define them, for documents that use
/// one without defining it (a converted workbook's sheet names are Heading 1).
pub fn builtin(id: &str) -> Option<Element> {
    let w = |local: &str, attrs: &[(&str, &str)]| attrs.iter().fold(Element::new(ns::W, local), |el, (k, v)| el.with_attr(ns::W, k, *v));
    let style = |kind: &str, name: &str| {
        w("style", &[("type", kind), ("styleId", id)])
            .with_child(w("name", &[("val", name)]))
            .with_child(w("basedOn", &[("val", if kind == "character" { "DefaultParagraphFont" } else { "Normal" })]))
    };
    let heading = |level: u8, before: &str, size: &str, color: &str, shade: &str, italic: bool| {
        let mut rpr = Element::new(ns::W, "rPr").with_child(w(
            "rFonts",
            &[("asciiTheme", "majorHAnsi"), ("eastAsiaTheme", "majorEastAsia"), ("hAnsiTheme", "majorHAnsi"), ("cstheme", "majorBidi")],
        ));
        if italic {
            rpr.push(Element::new(ns::W, "i"));
            rpr.push(Element::new(ns::W, "iCs"));
        }
        rpr.push(w("color", &[("val", color), ("themeColor", "accent1"), ("themeShade", shade)]));
        if size != "22" {
            rpr.push(w("sz", &[("val", size)]));
            rpr.push(w("szCs", &[("val", size)]));
        }
        style("paragraph", &format!("heading {level}"))
            .with_child(w("next", &[("val", "Normal")]))
            .with_child(w("uiPriority", &[("val", "9")]))
            .with_child(Element::new(ns::W, "qFormat"))
            .with_child(
                Element::new(ns::W, "pPr")
                    .with_child(Element::new(ns::W, "keepNext"))
                    .with_child(Element::new(ns::W, "keepLines"))
                    .with_child(w("spacing", &[("before", before), ("after", "0")]))
                    .with_child(w("outlineLvl", &[("val", &(level - 1).to_string())])),
            )
            .with_child(rpr)
    };
    Some(match id {
        "Heading1" => heading(1, "240", "32", "2F5496", "BF", false),
        "Heading2" => heading(2, "40", "26", "2F5496", "BF", false),
        "Heading3" => heading(3, "40", "24", "1F3763", "7F", false),
        "Heading4" => heading(4, "40", "22", "2F5496", "BF", true),
        "Heading5" => heading(5, "40", "22", "2F5496", "BF", false),
        "Heading6" => heading(6, "40", "22", "1F3763", "7F", false),
        "Title" => style("paragraph", "Title")
            .with_child(w("next", &[("val", "Normal")]))
            .with_child(Element::new(ns::W, "qFormat"))
            .with_child(
                Element::new(ns::W, "pPr")
                    .with_child(w("spacing", &[("after", "0"), ("line", "240"), ("lineRule", "auto")]))
                    .with_child(Element::new(ns::W, "contextualSpacing")),
            )
            .with_child(
                Element::new(ns::W, "rPr")
                    .with_child(w(
                        "rFonts",
                        &[
                            ("asciiTheme", "majorHAnsi"),
                            ("eastAsiaTheme", "majorEastAsia"),
                            ("hAnsiTheme", "majorHAnsi"),
                            ("cstheme", "majorBidi"),
                        ],
                    ))
                    .with_child(w("spacing", &[("val", "-10")]))
                    .with_child(w("kern", &[("val", "28")]))
                    .with_child(w("sz", &[("val", "56")]))
                    .with_child(w("szCs", &[("val", "56")])),
            ),
        "Subtitle" => style("paragraph", "Subtitle")
            .with_child(w("next", &[("val", "Normal")]))
            .with_child(Element::new(ns::W, "qFormat"))
            .with_child(
                Element::new(ns::W, "rPr")
                    .with_child(w("color", &[("val", "5A5A5A"), ("themeColor", "text1"), ("themeTint", "A5")]))
                    .with_child(w("spacing", &[("val", "15")])),
            ),
        "ListParagraph" => style("paragraph", "List Paragraph").with_child(Element::new(ns::W, "qFormat")).with_child(
            Element::new(ns::W, "pPr").with_child(w("ind", &[("left", "720")])).with_child(Element::new(ns::W, "contextualSpacing")),
        ),
        "Quote" => style("paragraph", "Quote")
            .with_child(w("next", &[("val", "Normal")]))
            .with_child(Element::new(ns::W, "qFormat"))
            .with_child(
                Element::new(ns::W, "pPr")
                    .with_child(w("spacing", &[("before", "200"), ("after", "160")]))
                    .with_child(w("ind", &[("left", "864"), ("right", "864")]))
                    .with_child(w("jc", &[("val", "center")])),
            )
            .with_child(
                Element::new(ns::W, "rPr")
                    .with_child(Element::new(ns::W, "i"))
                    .with_child(Element::new(ns::W, "iCs"))
                    .with_child(w("color", &[("val", "404040"), ("themeColor", "text1"), ("themeTint", "BF")])),
            ),
        "Hyperlink" => style("character", "Hyperlink").with_child(
            Element::new(ns::W, "rPr")
                .with_child(w("color", &[("val", "0563C1"), ("themeColor", "hyperlink")]))
                .with_child(w("u", &[("val", "single")])),
        ),
        "Strong" => style("character", "Strong")
            .with_child(Element::new(ns::W, "qFormat"))
            .with_child(Element::new(ns::W, "rPr").with_child(Element::new(ns::W, "b")).with_child(Element::new(ns::W, "bCs"))),
        "Emphasis" => style("character", "Emphasis")
            .with_child(Element::new(ns::W, "qFormat"))
            .with_child(Element::new(ns::W, "rPr").with_child(Element::new(ns::W, "i")).with_child(Element::new(ns::W, "iCs"))),
        "TableGrid" => {
            let border = |side: &str| w(side, &[("val", "single"), ("sz", "4"), ("space", "0"), ("color", "auto")]);
            let borders = ["top", "left", "bottom", "right", "insideH", "insideV"]
                .iter()
                .fold(Element::new(ns::W, "tblBorders"), |el, s| el.with_child(border(s)));
            w("style", &[("type", "table"), ("styleId", id)])
                .with_child(w("name", &[("val", "Table Grid")]))
                .with_child(w("basedOn", &[("val", "TableNormal")]))
                .with_child(Element::new(ns::W, "pPr").with_child(w("spacing", &[("after", "0"), ("line", "240"), ("lineRule", "auto")])))
                .with_child(Element::new(ns::W, "tblPr").with_child(borders))
        }
        _ => return None,
    })
}

/// Adds the built-in styles in `ids` that the style sheet doesn't have; returns whether it
/// added any.
pub fn add_missing(styles: &mut XmlDoc, ids: &[String]) -> bool {
    let have: Vec<String> =
        styles.root.children_named(ns::W, "style").filter_map(|s| s.attr_ns(ns::W, "styleId")).map(str::to_string).collect();
    let mut added = false;
    for id in ids {
        if have.contains(id) {
            continue;
        }
        if let Some(el) = builtin(id) {
            styles.root.push(el);
            added = true;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_core::xml::parse;

    const STYLES: &str = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="240" w:after="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Heading1"/><w:rPr><w:sz w:val="26"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Loop"><w:name w:val="Loop"/><w:basedOn w:val="Loop"/></w:style>
</w:styles>"#;

    #[test]
    fn resolves_like_word() {
        let s = Styles::parse(&parse(STYLES.as_bytes()).unwrap());
        assert_eq!(s.by_name("Heading 2").unwrap().id, "Heading2");
        assert_eq!(s.chain("Heading2").iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["Normal", "Heading1", "Heading2"]);
        assert_eq!(s.chain("Loop").len(), 1, "a style based on itself stops");

        let h2 = ParaProps { style: Some("Heading2".into()), ..Default::default() };
        let p = s.resolve_para(&h2);
        assert_eq!(p.space_before, Some(office_core::units::Emu::from_twips(240)));
        assert_eq!(p.space_after, Some(office_core::units::Emu(0)));
        let theme = Theme::office();
        let r = s.resolve_run(&h2, &RunProps::default(), Some(&theme));
        assert_eq!((r.bold, r.size, r.font.as_deref()), (Some(true), Some(office_core::units::FontSize(1300)), Some("Calibri")));
        // Strong's bold on a bold heading toggles it off; direct bold is bold whatever.
        let strong = RunProps { style: Some("Strong".into()), ..Default::default() };
        assert_eq!(s.resolve_run(&h2, &strong, None).bold, Some(false));
        let direct = RunProps { style: Some("Strong".into()), bold: Some(true), ..Default::default() };
        assert_eq!(s.resolve_run(&h2, &direct, None).bold, Some(true));
        // A paragraph without a style is Normal's.
        assert_eq!(s.resolve_para(&ParaProps::default()).style.as_deref(), Some("Normal"));
        assert_eq!(s.resolve_run(&ParaProps::default(), &RunProps::default(), None).size, Some(office_core::units::FontSize(1100)));
    }

    #[test]
    fn adds_built_in_styles() {
        let mut doc = parse(STYLES.as_bytes()).unwrap();
        assert!(add_missing(&mut doc, &["Heading1".into(), "Heading3".into(), "Title".into(), "Nonsense".into()]));
        let s = Styles::parse(&doc);
        assert_eq!(s.styles.iter().filter(|s| s.id == "Heading1").count(), 1, "the document's own stays");
        assert_eq!(s.get("Heading3").unwrap().based_on.as_deref(), Some("Normal"));
        assert_eq!(s.get("Title").unwrap().run.size, Some(office_core::units::FontSize(2800)));
        assert!(s.get("Nonsense").is_none());
        assert!(!add_missing(&mut doc, &["Heading3".into()]));
    }
}
