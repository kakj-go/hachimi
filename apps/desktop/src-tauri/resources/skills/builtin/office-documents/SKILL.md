---
name: office-documents
description: Create, read, revise, compare, preview, export, and validate professional text documents such as DOCX or ODT. Use for reports, proposals, letters, policies, contracts, redlines, comments, and other paginated document work.
---

# Office documents

Use only enabled Workspace or MCP tools. This Skill supplies workflow guidance and never grants file, process, connector, or delivery authority.

1. Inspect the source, requested output format, required structure, style constraints, and available structured tools.
2. Preserve existing semantics, styles, comments, tracked changes, fields, links, and accessibility metadata unless the request changes them.
3. For creation, establish headings, page geometry, typography, tables, lists, references, and metadata before writing detailed content.
4. For revision, make the smallest coherent edits and retain a reviewable source-to-result mapping. Prefer a structured document operation over raw archive/XML edits.
5. Render or preview the result. Check pagination, clipping, orphaned headings, tables, image placement, fonts, links, and required sections.
6. Compare the final artifact with the source or requested outline. Export only to the requested formats and preserve the editable source when possible.
7. Return controlled Artifact references, validation evidence, and a concise change summary. Do not claim success from tool text alone.

If a conversion, preview, or validation fails, keep the last valid artifact, report the stable failure, and retry only with a bounded corrective change. Sending, sharing, publishing, or deleting always requires current authority.

Read [validation.md](references/validation.md) before finalizing a document.
