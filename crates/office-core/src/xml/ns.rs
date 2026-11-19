//! Namespace URIs used across Office files, and the prefixes Office writes them with.
//!
//! Strict Open XML (ISO 29500 strict) uses different URIs for the same namespaces. Lookups compare
//! namespaces through [`canonical`], so a strict file matches the transitional constants here.

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
pub const X: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
pub const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
pub const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
pub const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
/// Office Math (OMML), Word's equations.
pub const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
/// SmartArt's data, layout, colors and style parts.
pub const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
/// The drawing Office caches for a SmartArt diagram.
pub const DSP: &str = "http://schemas.microsoft.com/office/drawing/2008/diagram";
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
pub const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub const CP: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
pub const DC: &str = "http://purl.org/dc/elements/1.1/";
pub const DCTERMS: &str = "http://purl.org/dc/terms/";
pub const DCMITYPE: &str = "http://purl.org/dc/dcmitype/";
pub const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
pub const EP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
pub const VT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes";
pub const XML: &str = "http://www.w3.org/XML/1998/namespace";
pub const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

/// The prefix Office uses for a namespace, for elements the compiler creates.
pub fn prefix_for(uri: &str) -> Option<&'static str> {
    Some(match canonical(uri) {
        W => "w",
        A => "a",
        P => "p",
        // SpreadsheetML is the default namespace in Excel's parts; this is for mixing it into others.
        X => "x",
        R => "r",
        PIC => "pic",
        WP => "wp",
        XDR => "xdr",
        C => "c",
        MC => "mc",
        CP => "cp",
        DC => "dc",
        DCTERMS => "dcterms",
        DCMITYPE => "dcmitype",
        XSI => "xsi",
        VT => "vt",
        XML => "xml",
        _ => return None,
    })
}

/// The transitional URI for a strict one; any other URI unchanged.
pub fn canonical(uri: &str) -> &str {
    let Some(rest) = uri.strip_prefix("http://purl.oclc.org/ooxml/") else { return uri };
    match rest {
        "wordprocessingml/main" => W,
        "drawingml/main" => A,
        "presentationml/main" => P,
        "spreadsheetml/main" => X,
        "officeDocument/relationships" => R,
        "drawingml/picture" => PIC,
        "drawingml/wordprocessingDrawing" => WP,
        "drawingml/spreadsheetDrawing" => XDR,
        "drawingml/chart" => C,
        "officeDocument/math" => M,
        "officeDocument/extendedProperties" => EP,
        "officeDocument/docPropsVTypes" => VT,
        _ => uri,
    }
}

/// Whether two namespace URIs name the same namespace.
pub fn same(a: &str, b: &str) -> bool {
    a == b || canonical(a) == canonical(b)
}

/// The strict URI for an Office namespace, leaving other namespaces alone.
pub fn strict(uri: &str) -> &str {
    match uri {
        W => "http://purl.oclc.org/ooxml/wordprocessingml/main",
        A => "http://purl.oclc.org/ooxml/drawingml/main",
        P => "http://purl.oclc.org/ooxml/presentationml/main",
        X => "http://purl.oclc.org/ooxml/spreadsheetml/main",
        R => "http://purl.oclc.org/ooxml/officeDocument/relationships",
        PIC => "http://purl.oclc.org/ooxml/drawingml/picture",
        WP => "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing",
        XDR => "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing",
        C => "http://purl.oclc.org/ooxml/drawingml/chart",
        EP => "http://purl.oclc.org/ooxml/officeDocument/extendedProperties",
        VT => "http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes",
        _ => uri,
    }
}
