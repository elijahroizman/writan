//! Laying out a list of blocks (paragraphs and tables, tables holding more blocks) down a
//! column of a given width. Word pages are cut from this flow; a slide's table and a shape's
//! text are drawn from it whole.
//!
//! The layer says how each paragraph and table looks through a [`Resolver`]; laid-out
//! paragraphs are cached by their [`ParaSpec::key`], so after an edit only the paragraphs that
//! changed are laid out again.

use std::collections::HashMap;
use std::sync::Arc;

use office_core::model::{Block, Paragraph, Table};

use crate::para::{self, Ctx, ParaLayout, ParaSpec};
use crate::pos::Path;

/// What decides where a paragraph may be cut across pages or columns.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Keep {
    /// Stay on the same page as the next paragraph (headings).
    pub with_next: bool,
    /// Don't split the paragraph's lines across pages.
    pub together: bool,
    /// Start on a new page.
    pub page_break_before: bool,
    /// Avoid a paragraph's first or last line alone on a page.
    pub widows: bool,
}

/// A line around a table cell.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderLine {
    pub width: f64,
    pub color: String,
    /// `solid`, `double`, `dashed`, `dotted`.
    pub style: &'static str,
}

/// How a table looks, as its layer resolves it (table style, direct formatting, theme).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableLook {
    /// Each grid column's width, in pixels.
    pub columns: Vec<f64>,
    /// Shift from the column's start edge.
    pub indent: f64,
    /// Per row, per grid column: the cell's look (covered slots are ignored).
    pub cells: Vec<Vec<CellLook>>,
    /// Per row: (height, exact?) where the row has one.
    pub heights: Vec<Option<(f64, bool)>>,
    /// The first column is at the right (Word's `w:bidiVisual`). Each cell's look is mirrored
    /// with it: its start margin and border go on the right.
    pub rtl: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellLook {
    pub fill: Option<String>,
    /// Top, right, bottom, left.
    pub borders: [Option<BorderLine>; 4],
    /// Top, right, bottom, left padding.
    pub margins: [f64; 4],
    /// 0 top, 1 middle, 2 bottom.
    pub valign: u8,
}

/// What a layer tells the flow about its blocks.
pub trait Resolver {
    /// How the paragraph at `path` looks, laid out at `width`. Shared, so a layer can keep and
    /// hand back the same spec while the paragraph doesn't change.
    fn para(&mut self, ctx: &mut Ctx, path: &Path, p: &Paragraph, width: f64) -> (Arc<ParaSpec>, Keep);
    /// How the table at `path` looks in a column `width` wide.
    fn table(&mut self, path: &Path, t: &Table, width: f64) -> TableLook;
}

/// Laid-out paragraphs: by spec (a layer hands back the same shared spec while a paragraph
/// doesn't change, so most lookups are a pointer), and by what decided their layout (for a spec
/// made again, or two paragraphs alike).
/// A spec, its layout and the pass that last used them.
type SpecLayout = (Arc<ParaSpec>, Arc<ParaLayout>, u32);

#[derive(Default)]
pub struct Cache {
    by_spec: HashMap<(usize, u64), SpecLayout>,
    paras: HashMap<u64, (Arc<ParaLayout>, u32)>,
    generation: u32,
}

impl Cache {
    pub fn new() -> Cache {
        Cache::default()
    }

    /// Starts a layout pass; layouts no pass has used for a while are dropped.
    pub fn next_pass(&mut self) {
        self.generation += 1;
        if self.generation.is_multiple_of(8) {
            let keep = self.generation.saturating_sub(8);
            self.paras.retain(|_, (_, g)| *g >= keep);
            self.by_spec.retain(|_, (_, _, g)| *g >= keep);
        }
    }

    /// Forgets everything (fonts changed: every measurement may be different now).
    pub fn clear(&mut self) {
        self.paras.clear();
        self.by_spec.clear();
    }

    pub fn layout(&mut self, ctx: &mut Ctx, spec: &Arc<ParaSpec>, width: f64) -> Arc<ParaLayout> {
        let generation = self.generation;
        // The entry holds the spec, so its address can't be reused by another while it's here.
        let ptr = (Arc::as_ptr(spec) as usize, width.to_bits());
        if let Some((s, l, g)) = self.by_spec.get_mut(&ptr) {
            if Arc::ptr_eq(s, spec) {
                *g = generation;
                return l.clone();
            }
        }
        let key = spec.key(width);
        let l = match self.paras.get_mut(&key) {
            Some((l, g)) => {
                *g = generation;
                l.clone()
            }
            None => {
                let l = Arc::new(para::layout(ctx, spec, width));
                self.paras.insert(key, (l.clone(), generation));
                l
            }
        };
        self.by_spec.insert(ptr, (spec.clone(), l.clone(), generation));
        l
    }
}

#[derive(Clone, Debug)]
pub enum BlockBox {
    Para(ParaBox),
    Table(TableBox),
    /// Markup with nothing to lay out.
    Empty {
        path: Path,
    },
}

#[derive(Clone, Debug)]
pub struct ParaBox {
    pub path: Path,
    pub spec: Arc<ParaSpec>,
    pub layout: Arc<ParaLayout>,
    pub keep: Keep,
    /// From the flow's top to the first line's top (the space before is above it).
    pub y: f64,
}

#[derive(Clone, Debug)]
pub struct TableBox {
    pub path: Path,
    pub x: f64,
    pub y: f64,
    /// Grid lines' x, from the table's left: one more than there are columns.
    pub cols: Vec<f64>,
    pub rows: Vec<RowBox>,
    pub cells: Vec<CellBox>,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug)]
pub struct RowBox {
    /// From the table's top.
    pub y: f64,
    pub height: f64,
    pub header: bool,
}

#[derive(Clone, Debug)]
pub struct CellBox {
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
    /// From the table's top left.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub look: CellLook,
    /// The cell's blocks, laid out inside its margins.
    pub content: FlowLayout,
    /// From the cell's top to the content's top (vertical alignment and the top margin).
    pub content_y: f64,
}

#[derive(Clone, Debug, Default)]
pub struct FlowLayout {
    pub blocks: Vec<BlockBox>,
    pub height: f64,
}

/// Lays out `blocks` (at `container` in the story) across `width`.
pub fn layout(
    ctx: &mut Ctx,
    cache: &mut Cache,
    resolver: &mut dyn Resolver,
    blocks: &[Block],
    container: &[u32],
    width: f64,
) -> FlowLayout {
    layout_from(ctx, cache, resolver, blocks, container, 0, width)
}

/// Lays out `blocks`, the story's blocks from index `first` on (a Word section's share of the body).
pub fn layout_from(
    ctx: &mut Ctx,
    cache: &mut Cache,
    resolver: &mut dyn Resolver,
    blocks: &[Block],
    container: &[u32],
    first: u32,
    width: f64,
) -> FlowLayout {
    let mut out = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        let mut path = container.to_vec();
        path.push(first + i as u32);
        out.push(layout_block(ctx, cache, resolver, block, Path(path), width));
    }
    let height = restack(&mut out);
    FlowLayout { blocks: out, height }
}

/// Lays out one block at `path`, at the top of the flow (see [`restack`] for placing it).
pub fn layout_block(ctx: &mut Ctx, cache: &mut Cache, resolver: &mut dyn Resolver, block: &Block, path: Path, width: f64) -> BlockBox {
    match block {
        Block::Paragraph(p) => {
            let (spec, keep) = resolver.para(ctx, &path, p, width);
            let layout = cache.layout(ctx, &spec, width);
            BlockBox::Para(ParaBox { path, spec, layout, keep, y: 0.0 })
        }
        Block::Table(t) => {
            let look = resolver.table(&path, t, width);
            BlockBox::Table(table(ctx, cache, resolver, t, &look, &path, 0.0))
        }
        Block::Raw { .. } => BlockBox::Empty { path },
    }
}

/// Places laid-out blocks one under another (Word adds the space after one paragraph and the
/// space before the next) and answers the flow's height.
pub fn restack(blocks: &mut [BlockBox]) -> f64 {
    let mut y = 0.0;
    let mut prev_after = 0.0;
    for b in blocks {
        match b {
            BlockBox::Para(p) => {
                y += prev_after + p.spec.space_before;
                prev_after = p.spec.space_after;
                p.y = y;
                y += p.layout.height;
            }
            BlockBox::Table(t) => {
                y += prev_after;
                prev_after = 0.0;
                t.y = y;
                y += t.height;
            }
            BlockBox::Empty { .. } => {}
        }
    }
    y + prev_after
}

impl BlockBox {
    pub fn path(&self) -> &Path {
        match self {
            BlockBox::Para(p) => &p.path,
            BlockBox::Table(t) => &t.path,
            BlockBox::Empty { path } => path,
        }
    }

    /// Moves the block `delta` places in its story: the first number of its path, and of every
    /// path inside it, changes by `delta`.
    pub fn shift(&mut self, delta: i64) {
        fn shift_path(p: &mut Path, delta: i64) {
            if let Some(first) = p.0.first_mut() {
                *first = (*first as i64 + delta).max(0) as u32;
            }
        }
        match self {
            BlockBox::Para(p) => shift_path(&mut p.path, delta),
            BlockBox::Empty { path } => shift_path(path, delta),
            BlockBox::Table(t) => {
                shift_path(&mut t.path, delta);
                for cell in &mut t.cells {
                    for b in &mut cell.content.blocks {
                        b.shift(delta);
                    }
                }
            }
        }
    }
}

fn table(ctx: &mut Ctx, cache: &mut Cache, resolver: &mut dyn Resolver, t: &Table, look: &TableLook, path: &Path, y: f64) -> TableBox {
    let ncols = t.columns.len().max(t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
    let mut cols = Vec::with_capacity(ncols + 1);
    let mut x = 0.0;
    cols.push(0.0);
    for c in 0..ncols {
        x += look.columns.get(c).copied().unwrap_or(48.0);
        cols.push(x);
    }
    // Lay out each cell's content at its width; rows grow to their tallest cell.
    let mut cells = Vec::new();
    let mut heights: Vec<f64> = look.heights.iter().map(|h| h.map(|(v, _)| v).unwrap_or(0.0)).collect();
    heights.resize(t.rows.len(), 0.0);
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if cell.covered {
                continue;
            }
            let cs = (cell.col_span.max(1) as usize).min(ncols.saturating_sub(c).max(1));
            let rs = (cell.row_span.max(1) as usize).min(t.rows.len() - r);
            let mut cl = look.cells.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
            if look.rtl {
                cl.margins.swap(1, 3);
                cl.borders.swap(1, 3);
            }
            let width = cols[(c + cs).min(ncols)] - cols[c];
            let inner = (width - cl.margins[1] - cl.margins[3]).max(1.0);
            let mut container = path.0.clone();
            container.extend([r as u32, c as u32]);
            let content = layout(ctx, cache, resolver, &cell.blocks, &container, inner);
            let need = content.height + cl.margins[0] + cl.margins[2];
            let exact = look.heights.get(r).copied().flatten().is_some_and(|(_, e)| e);
            if rs == 1 && !exact {
                heights[r] = heights[r].max(need);
            }
            cells.push(CellBox {
                row: r as u32,
                col: c as u32,
                row_span: rs as u32,
                col_span: cs as u32,
                x: if look.rtl { cols[ncols] - cols[c] - width } else { cols[c] },
                y: 0.0,
                width,
                height: need,
                look: cl,
                content,
                content_y: 0.0,
            });
        }
    }
    // A cell spanning rows that needs more room than they have stretches the last of them.
    for cell in &cells {
        if cell.row_span > 1 {
            let (r0, r1) = (cell.row as usize, (cell.row + cell.row_span) as usize);
            let have: f64 = heights[r0..r1].iter().sum();
            if cell.height > have {
                heights[r1 - 1] += cell.height - have;
            }
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    let mut ry = 0.0;
    for (r, h) in heights.iter().enumerate() {
        rows.push(RowBox { y: ry, height: *h, header: t.rows[r].header });
        ry += h;
    }
    for cell in &mut cells {
        let (r0, r1) = (cell.row as usize, (cell.row + cell.row_span) as usize);
        cell.y = rows[r0].y;
        let need = cell.height;
        cell.height = heights[r0..r1].iter().sum();
        let room = cell.height - need;
        cell.content_y = cell.look.margins[0]
            + match cell.look.valign {
                1 => room / 2.0,
                2 => room,
                _ => 0.0,
            }
            .max(0.0);
    }
    TableBox { path: path.clone(), x: look.indent, y, width: *cols.last().unwrap_or(&0.0), cols, rows, cells, height: ry }
}

impl FlowLayout {
    /// The paragraph box for `path`, searching into table cells, with its offset from this
    /// flow's top left.
    pub fn find(&self, path: &Path) -> Option<(&ParaBox, f64, f64)> {
        self.find_from(path, 0.0, 0.0)
    }

    fn find_from(&self, path: &Path, dx: f64, dy: f64) -> Option<(&ParaBox, f64, f64)> {
        let depth = self.depth();
        let index = *path.0.get(depth)? as usize;
        let first = self.first_index();
        match self.blocks.get(index.checked_sub(first)?)? {
            BlockBox::Para(p) if p.path == *path => Some((p, dx, dy)),
            BlockBox::Table(t) => {
                let (r, c) = (*path.0.get(depth + 1)?, *path.0.get(depth + 2)?);
                let cell = t.cells.iter().find(|cell| cell.row == r && cell.col == c)?;
                cell.content.find_from(path, dx + t.x + cell.x + cell.look.margins[3], dy + t.y + cell.y + cell.content_y)
            }
            _ => None,
        }
    }

    /// The story index of this flow's first block.
    fn first_index(&self) -> usize {
        let depth = self.depth();
        self.blocks
            .first()
            .map(|b| match b {
                BlockBox::Para(p) => p.path.0[depth] as usize,
                BlockBox::Table(t) => t.path.0[depth] as usize,
                BlockBox::Empty { path } => path.0[depth] as usize,
            })
            .unwrap_or(0)
    }

    /// How deep in the story this flow's blocks are, from their paths.
    fn depth(&self) -> usize {
        self.blocks.first().map(|b| b.path().0.len() - 1).unwrap_or(0)
    }
}

/// The position nearest (`x`, `y`) in a laid-out flow.
pub fn hit(ctx: &mut Ctx, flow: &FlowLayout, x: f64, y: f64) -> Option<crate::pos::Pos> {
    // The block whose extent (with its spacing) holds y, or the nearest one.
    let mut best: Option<&BlockBox> = None;
    for b in &flow.blocks {
        let (top, bottom) = match b {
            BlockBox::Para(p) => (p.y - p.spec.space_before, p.y + p.layout.height + p.spec.space_after),
            BlockBox::Table(t) => (t.y, t.y + t.height),
            BlockBox::Empty { .. } => continue,
        };
        if y < bottom || best.is_none() {
            best = Some(b);
        }
        if y < bottom {
            let _ = top;
            break;
        }
    }
    match best? {
        BlockBox::Para(p) => {
            let ly = y - p.y;
            let li = p.layout.lines.iter().position(|l| ly < l.top + l.height).unwrap_or(p.layout.lines.len().saturating_sub(1));
            let off = p.layout.offset_at(ctx, &p.spec, li, x);
            Some(crate::pos::Pos::new(p.path.clone(), off))
        }
        BlockBox::Table(t) => {
            let tx = x - t.x;
            let ty = y - t.y;
            let cell = t
                .cells
                .iter()
                .find(|c| tx >= c.x && tx < c.x + c.width && ty >= c.y && ty < c.y + c.height)
                .or_else(|| {
                    t.cells.iter().filter(|c| ty >= c.y && ty < c.y + c.height).min_by(|a, b| (a.x - tx).abs().total_cmp(&(b.x - tx).abs()))
                })
                .or_else(|| t.cells.last())?;
            hit(ctx, &cell.content, tx - cell.x - cell.look.margins[3], ty - cell.y - cell.content_y)
        }
        BlockBox::Empty { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::{FixedMeasurer, Measure};
    use crate::para::{StyleId, Styles};
    use crate::style::TextStyle;
    use office_core::drawing::{Rgb, Rgba};
    use office_core::model::{Cell, Row, RunProps};
    use office_core::units::FontSize;

    struct Plain(StyleId);
    impl Resolver for Plain {
        fn para(&mut self, _: &mut Ctx, _: &Path, p: &Paragraph, _: f64) -> (Arc<ParaSpec>, Keep) {
            let mut spec = ParaSpec::plain(&p.plain_text(), self.0);
            spec.space_after = 5.0;
            (Arc::new(spec), Keep::default())
        }
        fn table(&mut self, _: &Path, t: &Table, _: f64) -> TableLook {
            TableLook {
                columns: vec![50.0; t.columns.len()],
                cells: t
                    .rows
                    .iter()
                    .map(|r| r.cells.iter().map(|_| CellLook { margins: [1.0, 2.0, 1.0, 2.0], ..Default::default() }).collect())
                    .collect(),
                ..Default::default()
            }
        }
    }

    #[test]
    fn paragraphs_and_tables_stack() {
        let mut host = FixedMeasurer::default();
        let mut measure = Measure::new();
        let mut styles = Styles::new();
        let s = styles.add(
            &mut measure,
            TextStyle::from_props(&RunProps { size: Some(FontSize::from_points(7.5)), ..Default::default() }, 10.0, &|_| Rgba {
                rgb: Rgb::BLACK,
                alpha: 1.0,
            }),
        );
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let table = Table {
            columns: vec![Default::default(); 2],
            rows: vec![
                Row { cells: vec![Cell { row_span: 2, ..Cell::text("tall") }, Cell::text("a")], ..Default::default() },
                Row { cells: vec![Cell { covered: true, ..Cell::text("") }, Cell::text("b b b b b b b b")], ..Default::default() },
            ],
            ..Default::default()
        };
        let blocks = vec![Block::Paragraph(Paragraph::text("first")), Block::Table(table), Block::Paragraph(Paragraph::text("last"))];
        let mut cache = Cache::new();
        let flow = layout(&mut ctx, &mut cache, &mut Plain(s), &blocks, &[], 300.0);
        let BlockBox::Table(t) = &flow.blocks[1] else { panic!() };
        // "first" is 10 px tall with 5 after.
        assert_eq!(t.y, 15.0);
        // Row 0: "a" = 10 + 5 after + 2 margins; row 1: 8 words in 46 px wrap to 2 lines = 20 + 5 + 2.
        assert_eq!(t.rows[0].height, 17.0);
        assert_eq!(t.rows[1].height, 27.0);
        let tall = t.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
        assert_eq!(tall.height, 44.0);
        let (pb, _, dy) = flow.find(&Path(vec![1, 1, 1, 0])).unwrap();
        assert_eq!(pb.layout.lines.len(), 2);
        assert_eq!(dy, 15.0 + 17.0 + 1.0);
        // Cached: a second pass lays nothing out again.
        cache.next_pass();
        let again = layout(&mut ctx, &mut cache, &mut Plain(s), &blocks, &[], 300.0);
        let (BlockBox::Para(a), BlockBox::Para(b)) = (&flow.blocks[0], &again.blocks[0]) else { panic!() };
        assert!(Arc::ptr_eq(&a.layout, &b.layout));
    }
}
