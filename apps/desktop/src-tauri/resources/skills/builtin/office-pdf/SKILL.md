---
name: office-pdf
description: Inspect, extract, create, combine, split, annotate, fill, redact, compare, render, export, and validate PDF documents. Use for fixed-layout deliverables, forms, page operations, OCR review, archival output, and controlled redaction.
---

# Office PDF

Prefer structured PDF tools and preserve the original artifact. This Skill cannot authorize access, destructive replacement, signatures, or delivery.

1. Inspect page count, dimensions, encryption, signatures, forms, annotations, text availability, attachments, metadata, and PDF conformance when reported.
2. Choose text extraction for semantic inspection and page rendering for visual inspection; neither substitutes for the other.
3. For creation or conversion, retain an editable source when possible and record the conversion path.
4. For merge, split, rotate, reorder, crop, or fill operations, state the exact page and field plan before mutation.
5. For redaction, use a true redaction operation, then verify removed content is absent from text extraction, objects, annotations, attachments, and rendered pages. Drawing a black rectangle is not redaction.
6. Render all changed pages and representative unchanged pages. Check clipping, fonts, transparency, image quality, links, form appearance, and signatures.
7. Return controlled Artifact references, page-level changes, validation evidence, and any conformance or signature impact.

On failure, keep the original and last valid derived PDF. Passwords and private keys must use secret channels and must not enter Transcript, Event, Artifact metadata, or logs.

Read [validation.md](references/validation.md) before finalizing a PDF.
