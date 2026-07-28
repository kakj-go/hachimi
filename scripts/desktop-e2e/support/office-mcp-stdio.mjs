import { copyFileSync, writeFileSync } from "node:fs";
import { basename, extname, join } from "node:path";
import { createInterface } from "node:readline";

const root = process.cwd();
const templates = join(root, "office-stdio-templates");

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

function resultFor(request) {
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
    const extension = extensions[name];
    const artifactId = `desktop-e2e-${name}`;
    const template = join(templates, `template.${extension}`);
    const destination = join(root, `${artifactId}.${extension}`);
    copyFileSync(template, destination);
    return {
      content: [{ type: "text", text: `Created and validated controlled artifact ${artifactId}` }],
      structuredContent: {
        artifactId,
        validated: true,
        extension,
        fileName: basename(destination),
        mediaType: extname(destination),
      },
      isError: false,
    };
  }
  if (name === "inspect_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
    const extension = artifactId.endsWith("create_pdf") ? "pdf" : "docx";
    return {
      content: [{ type: "text", text: `Inspected bounded metadata for ${artifactId}` }],
      structuredContent: { artifactId, extension, contentIncluded: false },
      isError: false,
    };
  }
  if (name === "modify_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
    copyFileSync(join(templates, "modified.docx"), join(root, `${artifactId}.docx`));
    return {
      content: [{ type: "text", text: `Modified and revalidated ${artifactId}` }],
      structuredContent: { artifactId, modified: true, validated: true },
      isError: false,
    };
  }
  if (name === "diff_artifact") {
    const artifactId = String(argumentsValue.artifactId ?? "");
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
    copyFileSync(join(root, `${artifactId}.pdf`), join(root, "desktop-e2e-exported.pdf"));
    return {
      content: [{ type: "text", text: `Exported controlled artifact ${artifactId}` }],
      structuredContent: {
        artifactId,
        format: "pdf",
        fileName: "desktop-e2e-exported.pdf",
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
    const result = resultFor(request);
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
