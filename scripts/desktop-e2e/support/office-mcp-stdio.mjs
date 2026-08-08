import { constants, copyFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { basename, extname, join } from "node:path";
import { createInterface } from "node:readline";

const root = process.cwd();
const templates = join(root, "office-stdio-templates");
const artifactExtensions = new Map([
  ["desktop-e2e-create_document", "docx"],
  ["desktop-e2e-create_spreadsheet", "xlsx"],
  ["desktop-e2e-create_presentation", "pptx"],
  ["desktop-e2e-create_pdf", "pdf"],
]);

function artifactPath(artifactId, expectedExtension) {
  if (!/^desktop-e2e-[a-z_]+$/u.test(artifactId)) throw new Error("invalid_artifact_id");
  const extension = artifactExtensions.get(artifactId);
  if (!extension || (expectedExtension && extension !== expectedExtension)) {
    throw new Error("artifact_extension_mismatch");
  }
  return join(root, `${artifactId}.${extension}`);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function copyIdempotent(source, destination) {
  if (existsSync(destination)) {
    if (sha256(source) !== sha256(destination)) throw new Error("overwrite_conflict");
    return true;
  }
  copyFileSync(source, destination, constants.COPYFILE_EXCL);
  return false;
}

const tools = [
  ["create_document", "docx"],
  ["create_spreadsheet", "xlsx"],
  ["create_presentation", "pptx"],
  ["create_pdf", "pdf"],
].map(([name]) => ({
  name,
  description: `Create a deterministic ${name} artifact through restricted stdio MCP`,
  inputSchema: {
    type: "object",
    properties: { title: { type: "string" }, body: { type: "string" } },
    required: ["title", "body"],
    additionalProperties: false,
  },
}));
tools.push(
  {
    name: "inspect_artifact",
    description: "Read bounded metadata for a controlled Office artifact",
    inputSchema: {
      type: "object",
      properties: { artifactId: { type: "string" } },
      required: ["artifactId"],
      additionalProperties: false,
    },
  },
  {
    name: "modify_artifact",
    description: "Modify and revalidate a controlled Office artifact",
    inputSchema: {
      type: "object",
      properties: { artifactId: { type: "string" }, body: { type: "string" } },
      required: ["artifactId", "body"],
      additionalProperties: false,
    },
  },
  {
    name: "diff_artifact",
    description: "Produce a bounded per-file artifact diff summary",
    inputSchema: {
      type: "object",
      properties: { artifactId: { type: "string" } },
      required: ["artifactId"],
      additionalProperties: false,
    },
  },
  {
    name: "export_artifact",
    description: "Export a controlled PDF artifact",
    inputSchema: {
      type: "object",
      properties: {
        artifactId: { type: "string" },
        format: { type: "string", enum: ["pdf"] },
      },
      required: ["artifactId", "format"],
      additionalProperties: false,
    },
  },
  {
    name: "preview_file_plan",
    description: "Preview an authorized file organization plan without mutation",
    inputSchema: {
      type: "object",
      properties: {
        root: { type: "string" },
        actions: { type: "array", items: { type: "string" } },
      },
      required: ["root", "actions"],
      additionalProperties: false,
    },
  },
  {
    name: "send_artifact",
    description: "Write a deterministic delivery receipt for the authorized target",
    inputSchema: {
      type: "object",
      properties: { artifactId: { type: "string" }, target: { type: "string" } },
      required: ["artifactId", "target"],
      additionalProperties: false,
    },
  },
);

async function resultFor(request) {
  const method = request.method;
  if (method === "initialize") {
    return {
      protocolVersion: "2025-06-18",
      capabilities: { tools: {} },
      serverInfo: { name: "Hachimi Restricted Office E2E", version: "1.0.0" },
    };
  }
  if (method === "tools/list") return { tools };
  if (method === "ping") return {};
  if (method !== "tools/call") throw new Error("method_not_found");

  const name = String(request.params?.name ?? "");
  const argumentsValue = request.params?.arguments ?? {};
  const extensions = {
    create_document: "docx",
    create_spreadsheet: "xlsx",
    create_presentation: "pptx",
    create_pdf: "pdf",
  };
  if (name in extensions) {
    const interruptionMarker = join(root, "desktop-e2e-office-interruption.marker");
    if (
      name === "create_document" &&
      argumentsValue.body === "interrupt-before-receipt" &&
      !existsSync(interruptionMarker)
    ) {
      writeFileSync(interruptionMarker, "dispatched_without_receipt", "utf8");
      await new Promise(() => {});
    }
    if (name === "create_document" && argumentsValue.body === "interrupt-before-receipt") {
      const destination = join(root, "desktop-e2e-interruption-document.docx");
      const replayed = copyIdempotent(join(templates, "template.docx"), destination);
      return {
        content: [{ type: "text", text: "Recovered interrupted Office document creation" }],
        structuredContent: {
          artifactId: "desktop-e2e-interruption-document",
          validated: true,
          extension: "docx",
          fileName: basename(destination),
          revision: 1,
          sha256: sha256(destination),
          changedParts: ["package:create"],
          replayed,
        },
        isError: false,
      };
    }
    const extension = extensions[name];
    const artifactId = `desktop-e2e-${name}`;
    const template = join(templates, `template.${extension}`);
    const destination = artifactPath(artifactId, extension);
    const replayed = copyIdempotent(template, destination);
    return {
      content: [{ type: "text", text: `Created and validated controlled artifact ${artifactId}` }],
      structuredContent: {
        artifactId,
        validated: true,
        extension,
        fileName: basename(destination),
        mediaType: extname(destination),
        revision: 1,
        sha256: sha256(destination),
        changedParts: extension === "pdf" ? ["page:1"] : ["package:create"],
        replayed,
      },
      isError: false,
    };
  }
  if (name === "inspect_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
    const extension = artifactExtensions.get(artifactId);
    const path = artifactPath(artifactId, extension);
    return {
      content: [{ type: "text", text: `Inspected bounded metadata for ${artifactId}` }],
      structuredContent: {
        artifactId,
        extension,
        revision: 1,
        sha256: sha256(path),
        contentIncluded: false,
      },
      isError: false,
    };
  }
  if (name === "modify_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
    const destination = artifactPath(artifactId, "docx");
    copyFileSync(join(templates, "modified.docx"), destination);
    return {
      content: [{ type: "text", text: `Modified and revalidated ${artifactId}` }],
      structuredContent: {
        artifactId,
        modified: true,
        validated: true,
        revision: 2,
        sha256: sha256(destination),
        changedParts: ["word/document.xml"],
      },
      isError: false,
    };
  }
  if (name === "diff_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
    artifactPath(artifactId);
    const diff = {
      artifactId,
      status: "modified",
      beforeRevision: "created",
      afterRevision: "revalidated",
      changedParts: ["word/document.xml"],
      contentIncluded: false,
    };
    writeFileSync(join(root, "desktop-e2e-artifact-diff.json"), JSON.stringify(diff), "utf8");
    return {
      content: [{ type: "text", text: `Produced bounded diff summary for ${artifactId}` }],
      structuredContent: diff,
      isError: false,
    };
  }
  if (name === "export_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
    const source = artifactPath(artifactId, "pdf");
    const exported = join(root, "desktop-e2e-exported.pdf");
    const replayed = copyIdempotent(source, exported);
    return {
      content: [{ type: "text", text: `Exported controlled artifact ${artifactId}` }],
      structuredContent: {
        artifactId,
        format: "pdf",
        fileName: "desktop-e2e-exported.pdf",
        revision: 1,
        sha256: sha256(exported),
        replayed,
      },
      isError: false,
    };
  }
  if (name === "preview_file_plan") {
    const preview = {
      root: String(argumentsValue.root ?? "authorized-fixture"),
      actions: Array.isArray(argumentsValue.actions) ? argumentsValue.actions : [],
      inventory: ["incoming/report.docx", "incoming/report (1).docx"],
      conflictPolicy: "suffix",
      conflictExample: "organized/report (2).docx",
      authorizedRootBoundary: { normalizedRoot: "authorized-fixture", outsideRootRejected: true },
      previewOnly: true,
    };
    const rollback = {
      version: 1,
      previewOnly: true,
      operations: preview.actions.map((action, index) => ({
        order: index,
        action,
        rollback: `undo:${action}`,
      })),
    };
    writeFileSync(join(root, "desktop-e2e-file-plan.json"), JSON.stringify(preview), "utf8");
    writeFileSync(join(root, "desktop-e2e-file-rollback.json"), JSON.stringify(rollback), "utf8");
    return {
      content: [{ type: "text", text: "File organization plan previewed without mutation" }],
      structuredContent: preview,
      isError: false,
    };
  }
  if (name === "send_artifact") {
    const receipt = {
      artifactId: String(argumentsValue.artifactId ?? ""),
      target: String(argumentsValue.target ?? ""),
      delivered: true,
    };
    artifactPath(receipt.artifactId);
    writeFileSync(join(root, "desktop-e2e-office-delivery.json"), JSON.stringify(receipt), "utf8");
    return {
      content: [{ type: "text", text: "Deterministic external delivery completed" }],
      structuredContent: receipt,
      isError: false,
    };
  }
  return {
    content: [{ type: "text", text: "Unknown restricted Office E2E tool" }],
    isError: true,
  };
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (!line.trim()) continue;
  let request;
  try {
    request = JSON.parse(line);
  } catch {
    continue;
  }
  if (!("id" in request)) continue;
  try {
    const result = await resultFor(request);
    process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id: request.id, result })}\n`);
  } catch (error) {
    process.stdout.write(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: request.id,
        error: { code: -32601, message: error instanceof Error ? error.message : "failed" },
      })}\n`,
    );
  }
}
