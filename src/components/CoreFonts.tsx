import { Check } from 'lucide-react'
import { useState } from 'react'
import { CORE_FONTS, useExtraFonts } from '../state/fonts'
import { formatSize } from '../state/plugins'
import { Button, Card, Checkbox, Hint, Spinner } from './ui'

/** Said wherever the core fonts are offered: what can't be had, and what stands in for it. */
export const NO_CALIBRI = 'Calibri and Cambria, Office’s newer defaults, can’t be downloaded anywhere, so Writan keeps drawing them with Carlito and Caladea, which take the same space.'

/**
 * Microsoft's license, a box to accept it and the download (state/fonts.ts CORE_FONTS), in the
 * welcome tour and in Settings › Plugins. Nothing downloads until the box is ticked.
 */
export function CoreFontsOffer() {
  const installed = useExtraFonts((s) => !!s.installed[CORE_FONTS.id])
  const busy = useExtraFonts((s) => !!s.busy[CORE_FONTS.id])
  const error = useExtraFonts((s) => s.errors[CORE_FONTS.id])
  const [accepted, setAccepted] = useState(false)

  if (installed) {
    return (
      <Hint className="core-fonts-done">
        <Check size={14} /> Installed. Documents set in Arial, Times New Roman and the rest now use Microsoft’s own fonts.
      </Hint>
    )
  }

  return (
    <div className="core-fonts-offer">
      <pre className="license-text core-fonts-license" tabIndex={0} aria-label="Microsoft’s license for its core fonts">
        {CORE_FONTS.license}
      </pre>
      <Checkbox checked={accepted} disabled={busy} onChange={setAccepted} label="I accept Microsoft’s license agreement" />
      <div className="core-fonts-actions">
        <Button variant="primary" disabled={!accepted || busy} onClick={() => void useExtraFonts.getState().install(CORE_FONTS.id, accepted)}>
          {busy ? 'Downloading…' : `Download the fonts (${formatSize(CORE_FONTS.size)})`}
        </Button>
        {busy && <Spinner size={16} />}
      </div>
      {error && <span className="plugin-error">{error}</span>}
    </div>
  )
}

/** The tour's picture for the fonts step: each family set in itself, so it shows the change. */
export function CoreFontsSpecimen() {
  return (
    <Card className="core-fonts-specimen">
      {['Arial', 'Times New Roman', 'Courier New', 'Georgia', 'Verdana', 'Trebuchet MS'].map((family) => (
        <span key={family} style={{ fontFamily: `'${family}', ${SPECIMEN_FALLBACK[family] ?? 'sans-serif'}` }}>
          {family}
        </span>
      ))}
    </Card>
  )
}

/** Before the download these fall back to the bundled look-alikes, as documents do. */
const SPECIMEN_FALLBACK: Record<string, string> = {
  Arial: "'Liberation Sans', sans-serif",
  'Times New Roman': "'Liberation Serif', serif",
  'Courier New': "'Liberation Mono', monospace",
  Georgia: "'Liberation Serif', serif",
}
