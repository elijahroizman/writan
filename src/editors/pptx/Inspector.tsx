import {
  AlignCenter,
  AlignCenterHorizontal,
  AlignCenterVertical,
  AlignEndHorizontal,
  AlignEndVertical,
  AlignJustify,
  AlignLeft,
  AlignRight,
  AlignStartHorizontal,
  AlignStartVertical,
  ArrowDownToLine,
  ArrowUpToLine,
  Bold,
  BringToFront,
  ChevronDown,
  Circle,
  Copy,
  Crop,
  Diamond,
  EyeOff,
  FileSpreadsheet,
  FlipHorizontal2,
  FlipVertical2,
  Group,
  Image as ImageIcon,
  Italic,
  Link,
  List,
  ListOrdered,
  Minus,
  MoveDown,
  MoveRight,
  MoveUp,
  Play,
  Plus,
  RectangleHorizontal,
  SendToBack,
  Shapes,
  Sigma,
  Square,
  Star,
  Strikethrough,
  Table,
  Trash2,
  Triangle,
  Type,
  Underline,
  Ungroup,
  Video,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import { useCollab } from '../../collab/session'
import { InspectorHeader } from '../../components/Workbench'
import type { MenuEntry } from '../../components/ui'
import * as platform from '../../lib/platform'
import { withKeys } from '../../state/shortcuts'
import { Button, ColorField, Hint, IconButton, normalizeHex, NumberField, PopoverMenu, Section, Segmented, SelectField, TableSizePicker, TextField, ToolButton } from '../../components/ui'
import type { Restack } from './arrange'
import { pickedShapes, pickedText } from './arrange'
import { SlideImage } from '../../engine/slides/SlideImage'
import { SlideSwap } from './SlideSwap'
import type { TableLook } from './table'
import { TABLE_LOOKS, isTable, themeLook } from './table'
import { DIRECTIONS, TRANSITIONS, changeTransition } from './transitions'
import { parseVideoLink } from './video'
import type { Deck, Element, Transition } from './types'
import type { ShapeLook, TextLook } from '../../engine/slides/engine'
import { LinkedExcelSection } from '../../links/LinkedExcelSection'
import type { ExcelLink, ExcelLinksController } from '../../links/excelLinks'
import { MathInspectorSection } from '../../math/MathInspectorSection'
import { hasText } from './types'

type TextStyle = Partial<TextLook>
type ShapeStyle = Partial<ShapeLook>
type ShapeKind = string

const SHAPES: { kind: ShapeKind; label: string; icon: ReactNode }[] = [
  { kind: 'rect', label: 'Rectangle', icon: <Square size={14} /> },
  { kind: 'roundRect', label: 'Rounded rectangle', icon: <RectangleHorizontal size={14} /> },
  { kind: 'ellipse', label: 'Oval', icon: <Circle size={14} /> },
  { kind: 'triangle', label: 'Triangle', icon: <Triangle size={14} /> },
  { kind: 'diamond', label: 'Diamond', icon: <Diamond size={14} /> },
  { kind: 'star5', label: 'Star', icon: <Star size={14} /> },
  { kind: 'rightArrow', label: 'Arrow', icon: <MoveRight size={14} /> },
  { kind: 'line', label: 'Line', icon: <Minus size={14} /> },
]

const FONTS = ['Helvetica Neue', 'Arial', 'Avenir Next', 'Calibri', 'Futura', 'Georgia', 'Gill Sans', 'Inter', 'Menlo', 'Times New Roman', 'Verdana']
const SWATCHES = ['FFFFFF', '000000', '6B6B70', '4F8CFF', 'FF8A3D', 'F2555A', '3ECF8E', 'A98BFF', 'F5A524']

export function Inspector({
  deck,
  tabId,
  onInsertImage,
  onInsertTable,
  onInsertVideo,
  onNewSlide,
  onPresent,
  onDeleteSlides,
  onArrange,
  canArrange,
  onCrop,
  onPlayVideo,
  excelLinks,
  onOpenExcelModal,
  onOpenExcelSource,
  onOpenMathModal,
  onEditMathModal,
}: {
  deck: Deck
  tabId: string
  onInsertImage: () => void
  onInsertTable: (rows: number, cols: number) => void
  onInsertVideo: (url: string) => Promise<boolean>
  onNewSlide: (anchor: { x: number; y: number }) => void
  onPresent: (fromStart: boolean) => void
  onDeleteSlides: (indexes: readonly number[]) => void
  onArrange: (how: Restack) => void
  canArrange: (how: Restack) => boolean
  /** Crop the selected picture; null when it can't be cropped. */
  onCrop: (() => void) | null
  /** Play a frame's video link on the slide. */
  onPlayVideo: (id: string) => void
  excelLinks: ExcelLinksController
  onOpenExcelModal: () => void
  onOpenExcelSource: (link: ExcelLink) => void
  onOpenMathModal: () => void
  onEditMathModal: (id: string) => void
}) {
  const { engine, slides, activeIndex, selectedSlides } = deck
  const [shapeMenu, setShapeMenu] = useState<{ x: number; y: number } | null>(null)
  const [themeMenu, setThemeMenu] = useState<{ x: number; y: number; opener: HTMLButtonElement } | null>(null)
  const [themeError, setThemeError] = useState(false)
  const shared = useCollab((s) => Boolean(s[tabId]))
  const [videoOpen, setVideoOpen] = useState(false)
  const [tableOpen, setTableOpen] = useState(false)
  const el = deck.selectedElement
  const count = deck.selectedIds.length
  const slide = slides[activeIndex]

  const update = (patch: Partial<Element>) => el && engine.run({ cmd: 'setBox', id: Number(el.id), x: patch.x ?? el.x, y: patch.y ?? el.y, w: patch.width ?? el.width, h: patch.height ?? el.height, rot: patch.rotation ?? el.rotation })

  return (
    <>
      <InspectorHeader kind="pptx">Presentation</InspectorHeader>

      <Section title="Insert">
        <div className="insert-grid">
          <ToolButton icon={<Type size={15} />} label="Text" onMouseDown={(e) => e.preventDefault()} onClick={() => engine.run({ cmd: 'insertTextBox' })} />
          <ToolButton
            icon={<Shapes size={15} />}
            label="Shape"
            onMouseDown={(e) => e.preventDefault()}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect()
              setShapeMenu({ x: r.left, y: r.bottom + 4 })
            }}
          />
          <ToolButton icon={<Table size={15} />} label="Table" active={tableOpen} onMouseDown={(e) => e.preventDefault()} onClick={() => setTableOpen(!tableOpen)} />
          <ToolButton icon={<ImageIcon size={15} />} label="Image" onMouseDown={(e) => e.preventDefault()} onClick={onInsertImage} />
          <ToolButton icon={<FileSpreadsheet size={15} />} label="Excel" title="Link a live range or chart from an open Excel workbook" onMouseDown={(e) => e.preventDefault()} onClick={onOpenExcelModal} />
          <ToolButton icon={<Sigma size={15} />} label="Math" title={platform.shortcut(withKeys('Insert LaTeX math', 'pptx.math'))} onMouseDown={(e) => e.preventDefault()} onClick={onOpenMathModal} />
          <ToolButton icon={<Video size={15} />} label="Video" active={videoOpen} onMouseDown={(e) => e.preventDefault()} onClick={() => setVideoOpen(!videoOpen)} />
          <ToolButton
            icon={<Plus size={15} />}
            label="Slide"
            onMouseDown={(e) => e.preventDefault()}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect()
              onNewSlide({ x: r.left, y: r.bottom + 4 })
            }}
          />
        </div>
        {tableOpen && (
          <TableSizePicker
            max={{ rows: 12, cols: 10 }}
            onPick={(rows, cols) => {
              setTableOpen(false)
              onInsertTable(rows, cols)
            }}
          />
        )}
        {videoOpen && (
          <UrlField
            placeholder="Paste a YouTube or Vimeo link"
            submitLabel="Add"
            autoFocus
            onSubmit={async (url) => {
              if (await onInsertVideo(url)) setVideoOpen(false)
            }}
          />
        )}
      </Section>

      <Section title="Design">
        <Button
          block
          aria-haspopup="menu"
          aria-expanded={!!themeMenu}
          disabled={shared}
          title={shared ? 'Themes can’t be changed while sharing' : undefined}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect()
            setThemeMenu({ x: r.right, y: r.bottom + 4, opener: e.currentTarget })
          }}
        >
          Theme <ChevronDown size={14} />
        </Button>
        {themeMenu && (
          <PopoverMenu
            anchor={{ ...themeMenu, align: 'end' }}
            opener={themeMenu.opener}
            items={engine.themes.map((name) => ({ label: name, onSelect: () => {
              try {
                engine.run({ cmd: 'applyTheme', name })
                setThemeError(false)
              } catch {
                setThemeError(true)
              }
            } }))}
            onClose={() => setThemeMenu(null)}
          />
        )}
        {themeError && <p role="alert">Couldn’t apply the theme.</p>}
      </Section>

      <Section title="Linked Excel" collapsible defaultOpen={excelLinks.links.length > 0 || !!el?.linkTag}>
        <LinkedExcelSection
          controller={excelLinks}
          activeTag={el?.linkTag}
          onOpenInsert={onOpenExcelModal}
          onOpenSource={onOpenExcelSource}
        />
      </Section>

      {count > 1 ? (
        <MultiSelection deck={deck} count={count} onArrange={onArrange} canArrange={canArrange} />
      ) : el ? (
        <>
          <MathInspectorSection mathTag={el.mathTag} onEdit={() => onEditMathModal(el.id)} />
          {isTable(el) && <TableSection deck={deck} el={el} />}
          {el.type === 'group' && <GroupTextSection deck={deck} />}
          {!el.mathTag && hasText(el) && <TextSection deck={deck} el={el} />}
          {!el.mathTag && (el.type === 'shape' || el.type === 'text' || el.type === 'connector') && <FillSection deck={deck} el={el} />}
          <ArrangeSection deck={deck} el={el} update={update} onArrange={onArrange} canArrange={canArrange} onCrop={onCrop} />
          <LinkSection deck={deck} el={el} onPlayVideo={onPlayVideo} />
        </>
      ) : selectedSlides.length > 1 ? (
        <>
          <Section title={`${selectedSlides.length} slides`}>
            <div className="grid-2">
              <Button onClick={() => engine.run({ cmd: 'duplicateSlides', indexes: selectedSlides })}>
                <Copy size={14} /> Duplicate
              </Button>
              <Button variant="danger" onClick={() => onDeleteSlides(selectedSlides)}>
                <Trash2 size={14} /> Delete
              </Button>
            </div>
            <Button block onClick={() => engine.run({ cmd: 'toggleHidden', indexes: selectedSlides })}>
              <EyeOff size={14} /> Hide or show in slideshow
            </Button>
            <Hint tone="faint" as="p">Shift-click picks a run of slides; {platform.isMac ? '⌘' : 'Ctrl'}-click adds or removes one.</Hint>
          </Section>
          <TransitionSection deck={deck} indexes={selectedSlides} />
        </>
      ) : (
        slide && (
          <>
          <Section title={`Slide ${activeIndex + 1} of ${slides.length}`}>
            {engine.layouts.length > 1 && (
              <SelectField
                label="Layout"
                value={slide.layout ?? null}
                options={engine.layouts.map((l) => ({ value: l.path, label: l.name }))}
                onChange={(path) => engine.run({ cmd: 'applyLayout', layout: path })}
              />
            )}
            <div className="grid-2">
              <Button onClick={() => engine.run({ cmd: 'duplicateSlides', indexes: [activeIndex] })}>
                <Copy size={14} /> Duplicate
              </Button>
              <Button variant="danger" onClick={() => onDeleteSlides([activeIndex])}>
                <Trash2 size={14} /> Delete
              </Button>
              <Button disabled={activeIndex === 0} onClick={() => engine.run({ cmd: 'moveSlide', from: activeIndex, to: activeIndex - 1 })}>
                <ArrowUpToLine size={14} /> Move up
              </Button>
              <Button disabled={activeIndex >= slides.length - 1} onClick={() => engine.run({ cmd: 'moveSlide', from: activeIndex, to: activeIndex + 1 })}>
                <ArrowDownToLine size={14} /> Move down
              </Button>
            </div>
            <Button block onClick={() => engine.run({ cmd: 'toggleHidden', indexes: [activeIndex] })}>
              <EyeOff size={14} /> {slide.hidden ? 'Show slide in slideshow' : 'Hide slide in slideshow'}
            </Button>
            <Hint tone="faint" as="p">Drag across the slide or shift-click to select several objects. Double-click text to edit it.</Hint>
          </Section>
          <TransitionSection deck={deck} indexes={[activeIndex]} />
          </>
        )
      )}

      <Section title="Present">
        <Button variant="primary" block disabled={!slides.length} onClick={() => onPresent(false)}>
          <Play size={14} /> Play from this slide
        </Button>
        <Button block disabled={!slides.length} onClick={() => onPresent(true)}>
          Play from start
        </Button>
      </Section>

      {shapeMenu && (
        <PopoverMenu
          anchor={shapeMenu}
          onClose={() => setShapeMenu(null)}
          items={SHAPES.map<MenuEntry>((s) => ({
            label: s.label,
            icon: s.icon,
            onSelect: () => {
              engine.run({ cmd: 'insertShape', preset: s.kind })
            },
          }))}
        />
      )}
    </>
  )
}

function UrlField({ placeholder, submitLabel, initial = '', autoFocus = false, onSubmit, onClear }: { placeholder: string; submitLabel: string; initial?: string; autoFocus?: boolean; onSubmit: (url: string) => void; onClear?: () => void }) {
  const [value, setValue] = useState(initial)
  const [busy, setBusy] = useState(false)
  useEffect(() => setValue(initial), [initial])
  const submit = async () => {
    if (!value.trim()) return
    setBusy(true)
    try {
      await onSubmit(value.trim())
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="url-field">
      <TextField
        icon={<Link size={13} className="faint" />}
        value={value}
        placeholder={placeholder}
        autoFocus={autoFocus}
        onChange={(e) => setValue(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void submit()
        }}
      />
      <div className="row">
        <Button variant="primary" disabled={!value.trim() || busy} onClick={() => void submit()}>
          {busy ? 'Adding…' : submitLabel}
        </Button>
        {onClear && initial && <Button onClick={onClear}>Remove</Button>}
      </div>
    </div>
  )
}

/** Looks offered for tables: the deck's own accent first, when it has one. */
export function tableLooks(deck: Deck): TableLook[] {
  const theme = themeLook(deck.engine.theme.colors.accent1)
  return theme ? [theme, ...TABLE_LOOKS] : TABLE_LOOKS
}

function TableSection({ deck, el }: { deck: Deck; el: Element }) {
  const { engine } = deck
  const data = el.table
  const looks = tableLooks(deck)
  if (!data) return null
  const look = looks.find((l) => l.style === data.style)
  const header = data.header
  const banded = data.banded
  const first = data.textStyle
  const fonts = Array.from(new Set([engine.theme.body, engine.theme.heading, ...FONTS].filter(Boolean)))
  const op = (op: string, value?: string | number) => engine.run({ cmd: 'table', op, value: value == null ? null : String(value) })
  const style = (patch: TextStyle) => engine.run({ cmd: 'tableTextStyle', patch })

  return (
    <Section title={`Table · ${data.rows} × ${data.columns}`}>
      <div className="table-looks">
        {looks.map((l) => (
          <button
            key={l.id}
            type="button"
            title={l.label}
            aria-label={`${l.label} table style`}
            className={`table-look ${look?.id === l.id ? 'on' : ''}`}
            onClick={() => op('style', l.style)}
          >
            <span style={{ background: l.header, borderBottom: l.open ? `1.5px solid ${l.text}` : undefined }} />
            <span style={{ background: l.band, boxShadow: `inset 0 -1px ${l.line}` }} />
            <span style={{ background: l.body, boxShadow: `inset 0 -1px ${l.line}` }} />
            <span style={{ background: l.band }} />
          </button>
        ))}
      </div>
      <div className="grid-2">
        <Button className={header ? 'active-chip' : ''} onClick={() => op('header')}>
          Header row
        </Button>
        <Button className={banded ? 'active-chip' : ''} onClick={() => op('banded')}>
          Banded rows
        </Button>
      </div>
      <div className="grid-2">
        <Stepper label="Rows" value={data.rows} min={1} max={50} onChange={(n) => op('rows', n)} />
        <Stepper label="Columns" value={data.columns} min={1} max={20} onChange={(n) => op('cols', n)} />
      </div>
      <div className="docx-font-row">
        <SelectField value={first?.fontFamily ?? null} placeholder="Theme font" options={fonts.map((f) => ({ value: f, label: f }))} onChange={(f) => style({ fontFamily: f })} />
        <NumberField label={<span className="faint">pt</span>} value={first?.fontSize ?? null} precision={1} min={4} max={200} onCommit={(v) => style({ fontSize: v })} />
      </div>
      <Segmented
        value={first?.align === 'center' || first?.align === 'right' ? first.align : 'left'}
        onChange={(a) => style({ align: a })}
        options={[
          { value: 'left', icon: <AlignLeft size={15} />, title: 'Align left' },
          { value: 'center', icon: <AlignCenter size={15} />, title: 'Center' },
          { value: 'right', icon: <AlignRight size={15} />, title: 'Align right' },
        ]}
      />
      <Hint tone="faint" as="p">Double-click a cell to type in it. Drag the frame to resize the table.</Hint>
    </Section>
  )
}

function Stepper({ label, value, min, max, onChange }: { label: string; value: number; min: number; max: number; onChange: (n: number) => void }) {
  return (
    <div className="stepper">
      <span className="field-label">{label}</span>
      <IconButton label={`Fewer ${label.toLowerCase()}`} size="sm" disabled={value <= min} onClick={() => onChange(value - 1)}>
        <Minus size={13} />
      </IconButton>
      <span className="stepper-value">{value}</span>
      <IconButton label={`More ${label.toLowerCase()}`} size="sm" disabled={value >= max} onClick={() => onChange(value + 1)}>
        <Plus size={13} />
      </IconButton>
    </div>
  )
}

function TextSection({ deck, el }: { deck: Deck; el: Element & { textStyle?: TextStyle } }) {
  const { engine } = deck
  const style: TextStyle = el.textStyle ?? {}
  const set = (patch: TextStyle) => engine.run({ cmd: 'textStyle', patch })
  const fonts = Array.from(new Set([engine.theme.body, engine.theme.heading, ...FONTS].filter(Boolean) as string[]))
  const align = style.align === 'center' || style.align === 'right' || style.align === 'justify' ? style.align : 'left'
  return (
    <Section title="Text">
      <div className="docx-font-row">
        <SelectField value={style.fontFamily ?? null} placeholder="Theme font" options={fonts.map((f) => ({ value: f, label: f }))} onChange={(f) => set({ fontFamily: f })} />
        <NumberField label={<span className="faint">pt</span>} value={style.fontSize ?? null} precision={1} min={1} max={400} onCommit={(v) => set({ fontSize: v })} />
      </div>
      <div className="toggle-row">
        <IconButton label={withKeys('Bold', 'text.bold')} active={!!style.bold} onMouseDown={(e) => e.preventDefault()} onClick={() => set({ bold: !style.bold })}>
          <Bold size={15} />
        </IconButton>
        <IconButton label={withKeys('Italic', 'text.italic')} active={!!style.italic} onMouseDown={(e) => e.preventDefault()} onClick={() => set({ italic: !style.italic })}>
          <Italic size={15} />
        </IconButton>
        <IconButton label={withKeys('Underline', 'text.underline')} active={!!style.underline} onMouseDown={(e) => e.preventDefault()} onClick={() => set({ underline: !style.underline })}>
          <Underline size={15} />
        </IconButton>
        <IconButton label="Strikethrough" active={!!style.strikethrough} onMouseDown={(e) => e.preventDefault()} onClick={() => set({ strikethrough: !style.strikethrough })}>
          <Strikethrough size={15} />
        </IconButton>
        <IconButton label="Bulleted list" onMouseDown={(e) => e.preventDefault()} onClick={() => engine.run({ cmd: 'bullets', kind: 'bullet' })}>
          <List size={15} />
        </IconButton>
        <IconButton label="Numbered list" onMouseDown={(e) => e.preventDefault()} onClick={() => engine.run({ cmd: 'bullets', kind: 'numbered' })}>
          <ListOrdered size={15} />
        </IconButton>
      </div>
      <Segmented
        value={align}
        onChange={(a) => set({ align: a })}
        options={[
          { value: 'left', icon: <AlignLeft size={15} />, title: 'Align left' },
          { value: 'center', icon: <AlignCenter size={15} />, title: 'Center' },
          { value: 'right', icon: <AlignRight size={15} />, title: 'Align right' },
          { value: 'justify', icon: <AlignJustify size={15} />, title: 'Justify' },
        ]}
      />
      <ColorField value={normalizeHex(style.color)} label="Text color" onChange={(hex) => set({ color: `#${hex}` })} />
      <Swatches value={normalizeHex(style.color)} onPick={(hex) => set({ color: `#${hex}` })} />
      <Hint tone="faint" as="p">Select words while editing to style just those.</Hint>
    </Section>
  )
}

function FillSection({ deck, el }: { deck: Deck; el: Element }) {
  const { engine } = deck
  const s: ShapeStyle = el.shapeStyle ?? {}
  const noFill = s.fillMode === 'none' || (!s.fillColor && s.fillMode !== 'solid')
  return (
    <Section title="Fill & line">
      <div className="docx-color-row">
        <ColorField value={noFill ? null : normalizeHex(s.fillColor)} label="No fill" onChange={(hex) => engine.run({ cmd: 'fill', hex })} />
        <Button className={noFill ? 'active-chip' : ''} onClick={() => engine.run({ cmd: 'fill', hex: null })}>
          None
        </Button>
      </div>
      <Swatches value={noFill ? null : normalizeHex(s.fillColor)} onPick={(hex) => engine.run({ cmd: 'fill', hex })} />
      <div className="grid-2">
        <ColorField value={normalizeHex(s.strokeColor)} label="No line" onChange={(hex) => engine.run({ cmd: 'line', hex, width: s.strokeWidth || 1 })} />
        <NumberField label="W" suffix="pt" value={s.strokeWidth ?? 0} min={0} max={50} step={0.5} precision={1} onCommit={(w) => engine.run({ cmd: 'line', hex: null, width: w })} />
      </div>
    </Section>
  )
}

function ArrangeSection({
  deck,
  el,
  update,
  onArrange,
  canArrange,
  onCrop,
}: {
  deck: Deck
  el: Element
  update: (patch: Partial<Element>) => void
  onArrange: (how: Restack) => void
  canArrange: (how: Restack) => boolean
  onCrop: (() => void) | null
}) {
  const { engine } = deck
  const kind = el.type === 'text' ? 'Text box' : el.type === 'image' ? 'Image' : el.type[0].toUpperCase() + el.type.slice(1)
  return (
    <Section title={kind}>
      <div className="grid-2">
        <NumberField label="X" value={el.x} onCommit={(x) => update({ x })} />
        <NumberField label="Y" value={el.y} onCommit={(y) => update({ y })} />
        <NumberField label="W" value={el.width} min={1} onCommit={(width) => update({ width })} />
        <NumberField label="H" value={el.height} min={1} onCommit={(height) => update({ height })} />
        <NumberField label="∠" value={el.rotation ?? 0} suffix="°" onCommit={(r) => update({ rotation: ((r % 360) + 360) % 360 })} />
        <div className="toggle-row">
          <IconButton label="Flip horizontally" onClick={() => engine.run({ cmd: 'flip', horizontal: true })}>
            <FlipHorizontal2 size={15} />
          </IconButton>
          <IconButton label="Flip vertically" onClick={() => engine.run({ cmd: 'flip', horizontal: false })}>
            <FlipVertical2 size={15} />
          </IconButton>
        </div>
      </div>
      {(el.type === 'image') && (
        <Button block disabled={!onCrop} title={onCrop ? 'Crop' : 'A rotated or flipped picture can’t be cropped here'} onClick={() => onCrop?.()}>
          <Crop size={14} /> Crop
        </Button>
      )}
      <LayerButtons onArrange={onArrange} canArrange={canArrange} />
      {el.type === 'group' && (
        <Button block onClick={() => engine.run({ cmd: 'ungroup' })} title={platform.shortcut(withKeys('Ungroup', 'pptx.ungroup'))}>
          <Ungroup size={14} /> Ungroup
        </Button>
      )}
      <div className="grid-2">
        <Button onClick={() => engine.run({ cmd: 'duplicate' })} title={platform.shortcut(withKeys('Duplicate', 'pptx.duplicate'))}>
          <Copy size={14} /> Duplicate
        </Button>
        <Button variant="danger" onClick={() => engine.run({ cmd: 'delete' })}>
          <Trash2 size={14} /> Delete
        </Button>
      </div>
    </Section>
  )
}

/** Stacking order for the selected objects, as PowerPoint's Arrange menu. */
function LayerButtons({ onArrange, canArrange }: { onArrange: (how: Restack) => void; canArrange: (how: Restack) => boolean }) {
  const layer = (how: Restack, short: string, label: string, icon: ReactNode) => (
    <Button className="layer-btn" disabled={!canArrange(how)} title={platform.shortcut(withKeys(label, `pptx.${how}`))} aria-label={label} onClick={() => onArrange(how)}>
      {icon} {short}
    </Button>
  )
  return (
    <>
      <span className="field-caption">Arrange</span>
      <div className="grid-2">
        {layer('front', 'To front', 'Bring to front', <BringToFront size={14} />)}
        {layer('back', 'To back', 'Send to back', <SendToBack size={14} />)}
        {layer('forward', 'Forward', 'Bring forward', <MoveUp size={14} />)}
        {layer('backward', 'Backward', 'Send backward', <MoveDown size={14} />)}
      </div>
    </>
  )
}

function LinkSection({ deck, el, onPlayVideo }: { deck: Deck; el: Element; onPlayVideo: (id: string) => void }) {
  const url = el.link?.url
  const video = url ? parseVideoLink(url) : null
  return (
    <Section title="Link" collapsible defaultOpen={!!url}>
      {video && video.provider !== 'other' && (
        <Button onClick={() => onPlayVideo(el.id)}>
          <Play size={14} /> Play video
        </Button>
      )}
      <UrlField
        key={el.id}
        initial={url ?? ''}
        placeholder="https://"
        submitLabel={url ? 'Update' : 'Add link'}
        onSubmit={(value) => {
          const href = /^[a-z]+:/i.test(value) ? value : `https://${value}`
          deck.engine.run({ cmd: 'link', id: Number(el.id), url: href })
        }}
        onClear={() => deck.engine.run({ cmd: 'link', id: Number(el.id), url: null })}
      />
      <Hint tone="faint" as="p">Clicking this object during a slideshow opens the link. Video links play in the slide; double-click one to play it here.</Hint>
    </Section>
  )
}

function MultiSelection({ deck, count, onArrange, canArrange }: { deck: Deck; count: number; onArrange: (how: Restack) => void; canArrange: (how: Restack) => boolean }) {
  const { engine } = deck
  return (
    <>
    <Section title={`${count} objects`}>
      <div className="grid-2">
        <Button disabled={count < 2} onClick={() => engine.run({ cmd: 'group' })} title={platform.shortcut(withKeys('Group', 'pptx.group'))}>
          <Group size={14} /> Group
        </Button>
        <Button variant="danger" onClick={() => engine.run({ cmd: 'delete' })}>
          <Trash2 size={14} /> Delete
        </Button>
      </div>
      <div className="toggle-row">
        <IconButton label="Align left" onClick={() => engine.run({ cmd: 'align', how: 'left' })}>
          <AlignStartVertical size={15} />
        </IconButton>
        <IconButton label="Align centers" onClick={() => engine.run({ cmd: 'align', how: 'center' })}>
          <AlignCenterVertical size={15} />
        </IconButton>
        <IconButton label="Align right" onClick={() => engine.run({ cmd: 'align', how: 'right' })}>
          <AlignEndVertical size={15} />
        </IconButton>
        <IconButton label="Align top" onClick={() => engine.run({ cmd: 'align', how: 'top' })}>
          <AlignStartHorizontal size={15} />
        </IconButton>
        <IconButton label="Align middles" onClick={() => engine.run({ cmd: 'align', how: 'middle' })}>
          <AlignCenterHorizontal size={15} />
        </IconButton>
        <IconButton label="Align bottom" onClick={() => engine.run({ cmd: 'align', how: 'bottom' })}>
          <AlignEndHorizontal size={15} />
        </IconButton>
      </div>
      <div className="grid-2">
        <Button disabled={count < 3} onClick={() => engine.run({ cmd: 'distribute', horizontal: true })}>
          Space across
        </Button>
        <Button disabled={count < 3} onClick={() => engine.run({ cmd: 'distribute', horizontal: false })}>
          Space down
        </Button>
      </div>
      <LayerButtons onArrange={onArrange} canArrange={canArrange} />
    </Section>
    <GroupTextSection deck={deck} />
    <GroupFillSection deck={deck} />
    </>
  )
}

/** Text styling for every selected object that holds text, all at once (one undo step). */
function GroupTextSection({ deck }: { deck: Deck }) {
  const { engine, elements, selectedIds } = deck
  const ids = new Set(selectedIds)
  const texts = pickedText(elements, ids)
  if (!texts.length) return null
  // Show a value only when every box agrees on it.
  const shared = <K extends keyof TextStyle>(key: K) => {
    const first = texts[0].textStyle?.[key]
    return texts.every((t) => t.textStyle?.[key] === first) ? first : undefined
  }
  const set = (patch: TextStyle) => engine.run({ cmd: 'textStyle', patch })
  const fonts = Array.from(new Set([engine.theme.body, engine.theme.heading, ...FONTS].filter(Boolean) as string[]))
  const all = (key: 'bold' | 'italic' | 'underline' | 'strikethrough') => texts.every((t) => !!t.textStyle?.[key])
  const color = normalizeHex(shared('color') as string | undefined)
  const align = shared('align')
  return (
    <Section title={texts.length === 1 ? 'Text' : `Text · ${texts.length} boxes`}>
      <div className="docx-font-row">
        <SelectField value={(shared('fontFamily') as string | undefined) ?? null} placeholder="Mixed fonts" options={fonts.map((f) => ({ value: f, label: f }))} onChange={(f) => set({ fontFamily: f })} />
        <NumberField label={<span className="faint">pt</span>} value={(shared('fontSize') as number | undefined) ?? null} precision={1} min={1} max={400} onCommit={(v) => set({ fontSize: v })} />
      </div>
      <div className="toggle-row">
        <IconButton label={withKeys('Bold', 'text.bold')} active={all('bold')} onClick={() => set({ bold: !all('bold') })}>
          <Bold size={15} />
        </IconButton>
        <IconButton label={withKeys('Italic', 'text.italic')} active={all('italic')} onClick={() => set({ italic: !all('italic') })}>
          <Italic size={15} />
        </IconButton>
        <IconButton label={withKeys('Underline', 'text.underline')} active={all('underline')} onClick={() => set({ underline: !all('underline') })}>
          <Underline size={15} />
        </IconButton>
        <IconButton label="Strikethrough" active={all('strikethrough')} onClick={() => set({ strikethrough: !all('strikethrough') })}>
          <Strikethrough size={15} />
        </IconButton>
      </div>
      <Segmented
        value={align === 'center' || align === 'right' || align === 'justify' || align === 'left' ? align : null}
        onChange={(a) => set({ align: a })}
        options={[
          { value: 'left', icon: <AlignLeft size={15} />, title: 'Align left' },
          { value: 'center', icon: <AlignCenter size={15} />, title: 'Center' },
          { value: 'right', icon: <AlignRight size={15} />, title: 'Align right' },
          { value: 'justify', icon: <AlignJustify size={15} />, title: 'Justify' },
        ]}
      />
      <ColorField value={color} label="Text color" onChange={(hex) => set({ color: `#${hex}` })} />
      <Swatches value={color} onPick={(hex) => set({ color: `#${hex}` })} />
    </Section>
  )
}

function GroupFillSection({ deck }: { deck: Deck }) {
  const { engine, elements, selectedIds } = deck
  const ids = new Set(selectedIds)
  const shapes = pickedShapes(elements, ids)
  if (!shapes.length) return null
  const set = (patch: ShapeStyle) => engine.run({ cmd: 'fill', hex: patch.fillMode === 'none' ? null : patch.fillColor })
  const fills = shapes.map((e) => e.shapeStyle)
  const first = normalizeHex(fills[0]?.fillMode === 'none' ? undefined : fills[0]?.fillColor)
  const value = fills.every((f) => normalizeHex(f?.fillMode === 'none' ? undefined : f?.fillColor) === first) ? first : null
  return (
    <Section title="Fill">
      <div className="docx-color-row">
        <ColorField value={value} label="Fill" onChange={(hex) => set({ fillMode: 'solid', fillColor: `#${hex}` })} />
        <Button onClick={() => set({ fillMode: 'none' })}>None</Button>
      </div>
      <Swatches value={value} onPick={(hex) => set({ fillMode: 'solid', fillColor: `#${hex}` })} />
    </Section>
  )
}

/** Transition into the picked slides, with a preview played from the slide before. */
function TransitionSection({ deck, indexes }: { deck: Deck; indexes: readonly number[] }) {
  const { engine, slides } = deck
  const [run, setRun] = useState(0)
  const first = slides[indexes[0]]
  if (!first) return null
  const current = first.transition
  const type = indexes.every((i) => (slides[i]?.transition?.kind || 'none') === (current?.kind || 'none')) ? (current?.kind || 'none') : null
  const option = TRANSITIONS.find((t) => t.value === type)
  const set = (change: (t: Transition | undefined) => Transition | undefined, targets: readonly number[] = indexes) => {
    engine.run({ cmd: 'beginGroup' })
    try {
      for (const index of targets) engine.run({ cmd: 'transition', indexes: [index], transition: change(slides[index]?.transition) ?? null })
    } finally {
      engine.run({ cmd: 'endGroup' })
    }
    setRun((r) => r + 1)
  }
  const options = TRANSITIONS.map((t) => ({ value: t.value as string, label: t.label }))
  if (type && !option) options.push({ value: type, label: type })
  const before = slides[indexes[0] - 1]
  const width = 244
  const playing = type && type !== 'none' && type !== 'cut'
  return (
    <Section title="Transition">
      <SelectField value={type} placeholder="Mixed" options={options} onChange={(v) => set((t) => changeTransition(t, v as Transition['kind']))} />
      {playing && (
        <>
          <div className="grid-2">
            {option?.directions ? (
              <SelectField value={current?.direction ?? 'l'} options={DIRECTIONS} onChange={(d) => set((t) => t && { ...t, direction: d })} />
            ) : (
              <span />
            )}
            <NumberField
              label={<span className="faint">sec</span>}
              value={(current?.durationMs ?? 700) / 1000}
              precision={2}
              step={0.1}
              min={0.1}
              max={10}
              onCommit={(v) => set((t) => t && { ...t, durationMs: Math.round(v * 1000) })}
            />
          </div>
          <button type="button" className="transition-preview" style={{ height: (width * engine.height) / engine.width }} title="Play the transition" onClick={() => setRun((r) => r + 1)}>
            <SlideSwap
              run={run}
              transition={current}
              outgoing={before ? <SlideImage engine={engine} index={indexes[0] - 1} revision={before.revision} width={width} lazy={false} /> : <div className="transition-blank" />}
              incoming={<SlideImage engine={engine} index={indexes[0]} revision={first.revision} width={width} lazy={false} />}
            />
          </button>
        </>
      )}
      <Button block disabled={slides.length < 2} onClick={() => set(() => current, slides.map((_, i) => i))}>
        Use on every slide
      </Button>
    </Section>
  )
}

function Swatches({ value, onPick }: { value: string | null; onPick: (hex: string) => void }) {
  return (
    <div className="swatches one-line">
      {SWATCHES.map((c) => (
        <button
          key={c}
          type="button"
          aria-label={`#${c}`}
          className={`swatch ${value === c ? 'on' : ''}`}
          style={{ background: `#${c}` }}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onPick(c)}
        />
      ))}
    </div>
  )
}
