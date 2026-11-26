use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfDocumentInput {
    pub pages: Vec<PdfPageInput>,
    pub styles: Vec<PaintStyleInput>,
    #[serde(default)]
    pub metadata: PdfMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageInput {
    pub width_pt: f64,
    pub height_pt: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub offset_x_pt: f64,
    pub offset_y_pt: f64,
    pub clip_rect_px: Option<[f64; 4]>,
    pub items: Vec<PaintItemInput>,
    #[serde(default)]
    pub svgs: Vec<SvgOverlayInput>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStyleInput {
    pub font: String,
    pub color: String,
    pub decoration: Option<String>,
    pub background: Option<String>,
    #[serde(default)]
    pub small_caps: bool,
    pub shift: Option<f64>,
    pub ascent: Option<f64>,
    pub descent: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PaintItemInput {
    Text {
        x: f64,
        y: f64,
        w: f64,
        s: usize,
        text: String,
        ws: Option<f64>,
        link: Option<String>,
        #[serde(default)]
        rtl: bool,
        #[serde(default)]
        invisible: bool,
    },
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        fill: Option<String>,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        color: String,
        w: f64,
        dash: Option<String>,
    },
    Image {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        media: String,
        crop: Option<[f64; 4]>,
        id: Option<String>,
    },
    Path {
        x: f64,
        y: f64,
        d: String,
        fill: Option<String>,
        stroke: Option<String>,
        sw: Option<f64>,
        dash: Option<String>,
        fill_rule: Option<String>,
    },
    Group {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        rot: Option<f64>,
        #[serde(default)]
        fh: bool,
        #[serde(default)]
        fv: bool,
        #[serde(default)]
        clip: bool,
        id: Option<String>,
        items: Vec<PaintItemInput>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvgOverlayInput {
    pub x_px: f64,
    pub y_px: f64,
    pub w_px: f64,
    pub h_px: f64,
    pub svg: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePdfPageInput {
    pub media_key: String,
    pub mime: String,
    pub width_pt: f64,
    pub height_pt: f64,
    pub ocr_layer: Option<OcrPageLayer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrWordBox {
    pub text: String,
    pub x_pt: f64,
    pub y_pt: f64,
    pub w_pt: f64,
    pub h_pt: f64,
    pub baseline_y_pt: Option<f64>,
    pub baseline_x0_pt: Option<f64>,
    pub baseline_y0_pt: Option<f64>,
    pub baseline_x1_pt: Option<f64>,
    pub baseline_y1_pt: Option<f64>,
    pub oriented_h_pt: Option<f64>,
    #[serde(default)]
    pub rtl: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrPageLayer {
    pub page_index: usize,
    pub page_width_pt: f64,
    pub page_height_pt: f64,
    pub words: Vec<OcrWordBox>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfInspection {
    pub page_count: usize,
    pub encrypted: bool,
    pub has_digital_signature: bool,
    pub signature_fields: Vec<String>,
    pub has_forms: bool,
    pub has_outlines: bool,
    pub annotation_count: usize,
}
