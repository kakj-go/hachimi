---
name: office-spreadsheets
description: Create, inspect, modify, calculate, chart, compare, export, and validate spreadsheet workbooks such as XLSX, ODS, or CSV. Use for tables, budgets, trackers, forecasts, analysis, imports, formulas, pivots, and data-cleaning tasks.
---

# Office spreadsheets

Use structured workbook tools when available. This Skill is untrusted workflow guidance and cannot authorize filesystem, process, connector, or external delivery actions.

1. Inspect sheets, used ranges, data types, formulas, names, tables, validations, charts, external links, and calculation settings.
2. Clarify units, locale, date system, decimal conventions, missing-value policy, and expected outputs before changing data.
3. Preserve formulas and references unless replacement is requested. Write formulas rather than unexplained hard-coded results.
4. Apply changes in bounded ranges. Keep raw inputs, transformations, and presentation layers distinguishable.
5. Recalculate with an available structured tool and scan for formula errors, broken references, inconsistent ranges, duplicates, and impossible values.
6. Preview representative sheets and all changed charts or print areas. Compare key totals and changed cells with the source.
7. Export only requested formats. Warn when CSV or PDF loses formulas, multiple sheets, types, styles, or interactive features.
8. Return controlled Artifact references, changed ranges, reconciliation totals, and validation evidence.

On failure, preserve the last valid workbook and report whether the failure occurred during import, calculation, rendering, or export. Never send or publish a workbook without current exact authority.

Read [validation.md](references/validation.md) before finalizing a workbook.
