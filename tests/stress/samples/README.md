# Corpus fix samples

These DOCX files are original generated fixtures. They contain no downloaded document text or images. Regenerate them on bbb with `python3 tests/stress/samples/make-corpus-samples.py`. The script uses only Python's standard library and fixes ZIP timestamps for reproducibility.

- `table-content-controls.docx` covers editable cells before, between, and after raw checkbox controls, raw wrapped rows before/between/after modeled rows, and a merged cell. `office-core/tests/docx_table_order.rs` checks unchanged saves, edits, repeated saves, and model additions/removals.
- `cjk-list-markers.docx` contains independent traditional and zodiac lists. `office-word/tests/cjk_numbering.rs` checks every marker in the display list before and after saving, including the switch to decimal beyond each finite ideograph sequence.
- `tracked-changes.docx` has an insertion, a deletion and a formatting change by two authors. `tests/stress/ui/tracked-changes.mjs` accepts and rejects them from the Word inspector.

The raw content controls are preserved on save. Rendering/editing their contents remains a separate limitation.
