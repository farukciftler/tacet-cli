# audits/ — what is current, what is stale, and why

Last checked: 27 July 2026, during the English-language round.

`tacet-small-model-architecture-audit.md` is the **authority**. The three binary
deliverables are renderings of it (or of the verdict data derived from it) and they
do not all track it equally well. This file records which is which, so nobody has
to open a 590 KB PDF to find out it is out of date.

| File | Language | State |
|---|---|---|
| `tacet-small-model-architecture-audit.md` | English | **current** — the source of truth |
| `tacet-audit-summary.md` | English | **current** — the 23-item implementation summary |
| `tacet-small-model-architecture-audit.docx` | English | **regenerated 27 Jul 2026** from the `.md` above |
| `tacet-audit-report.xlsx` | English, except the verbatim model replies | **regenerated 27 Jul 2026**; see "The workbook" below |
| `tacet-small-model-architecture-audit.pdf` | **Turkish** | **STALE — DO NOT CITE.** See "The PDF" below |

## How the two regenerated files were produced

Not by hand and not by a converter: by the repository's own OOXML code, so that
the deliverables and the app produce the same kind of file.

- `.xlsx` — `Tacet/Tools/ExcelEngine.swift` (OOXML SpreadsheetML, `inlineStr`
  cells, no sharedStrings) packaged by `Tacet/Services/ZipStore.swift` (a STORE
  zip), escaped by `Tacet/Tools/OoxmlEscape.swift`.
- `.docx` — `Tacet/Tools/DocxEngine.swift` (WordprocessingML), same packager.

Those files were compiled unchanged into a throwaway command-line driver outside
the Xcode target; the only thing written for the job was the host scaffolding the
engines need (`Table`/`Row`, `DocumentFormat`, the `DocumentEngine` protocol),
which produces no OOXML of its own. Each file was then **read back with the same
engine that wrote it** — `ExcelEngine.read` returns 15 columns × 404 rows,
`DocxEngine.read` returns 135,924 characters — and every XML part in both packages
parses.

**The cost of doing it this way, stated plainly:** the previous `.docx` had been
produced by a full word processor and carried a theme, a numbering definition, a
style sheet and a thumbnail (17 package parts). `DocxEngine` writes three parts
and one paragraph style. The new file is correct, complete and English; it is not
typeset. Markdown that the app's engine does not interpret (`## `, `| … |`,
```` ``` ````) stands as literal text in the paragraphs.

## The workbook (`tacet-audit-report.xlsx`)

The data file the first export was built from (`verdicts-full.json`, cited in
`tacet-audit-summary.md` §0) **no longer exists in the repository**. So the
workbook was not re-derived from source data — it was carried over cell by cell
from the previous Turkish export and translated. No verdict, no score and no
measurement was re-run, restated or recomputed.

Translated: column headers, section labels, run labels (`ONCE-ham` →
`BEFORE-raw`, and the three others), category names, mode names, issue codes, and
the entire prose of the 23-row AUDIT block.

**`Case ID` was rewritten from the source, not from a guess.** The eval case names
were renamed to English in `Tacet/Services/Eval*.swift` while this round was
running, which supplies an authoritative mapping: for each of those files the case
names of the pre-rename revision were paired *in order* with the names of the
current revision. 1005 pairs, 0 conflicts, and the count matched file by file
(247/247, 72/72, 142/142, 152/152, 215/215, 115/115, 41/41, 31/31) — a mismatch
would have meant a case was added or removed and the pairing could not be trusted.
204 of the workbook's 205 ids were rewritten this way.

**The one that resisted:** `mcp-kapi-reddi` is built inline in
`Tacet/Services/EvalMCP.swift:441` rather than declared as a `TestCase`, so it was
not part of the rename and had no mapping. It is left **verbatim** rather than
guessed at. If that line is ever translated, this one cell goes stale and should be
updated to match.

**Deliberately left in Turkish** — the workbook says so itself, in a `NOTE` block
above the data:

1. **`Model response`** is the small model's own output, quoted verbatim. It is
   the evidence. A translation of it would be a paraphrase, and a paraphrase is
   not a measurement.
2. The **payload after an issue code** (`fabrication:Paris`,
   `reply-lacks:Mercimek`) is a fragment quoted out of that same reply, for the
   same reason.

Carried over unchanged: the `Score` column is **empty in every EVAL row**. It was
empty in the previous export too — the exporter never wrote a per-case score. The
category averages are in `tacet-audit-summary.md` §5, not here.

## The PDF (`tacet-small-model-architecture-audit.pdf`)

**Still Turkish, still pre-rename, and it was NOT regenerated.** It is kept rather
than deleted because it is the only surviving rendering of that round and deleting
it would destroy a record; but nothing in it should be cited. It freezes the
identifiers and directory names from before the English round (`Tacet/Araclar/`,
`BelgeOlusturAraci`, `ModelServisi`, …), so a reader who searches the source for a
symbol it names will not find it.

Why it was not regenerated, measured rather than assumed: `Tacet/Tools/PdfEngine.swift`
draws with `UIGraphicsPDFRenderer`, `UIFont` and `UIColor` and therefore
`import UIKit`. Compiling it for the host toolchain fails at that line:

```
PdfEngine.swift:10:8: error: no such module 'UIKit'
```

The Rust side cannot stand in either: `tacet-tools`' `DocumentFormat` is
`{Excel, Markdown, Text}` — there is no PDF writer in the workspace. There is no
`pandoc` and no LibreOffice on this machine, and adding one would be a new
dependency.

**To regenerate it** the engine has to run where UIKit exists: call
`DocumentEngines.engine(.pdf).write(...)` from the app (or from a test target) on
an iOS simulator, feeding it `tacet-small-model-architecture-audit.md`, and copy
the result back over this file. Until that happens, this row of the table stays
red.
