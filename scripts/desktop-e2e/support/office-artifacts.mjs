import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { extname, isAbsolute, relative, resolve } from "node:path";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function u16(value) {
  const buffer = Buffer.allocUnsafe(2);
  buffer.writeUInt16LE(value, 0);
  return buffer;
}

function u32(value) {
  const buffer = Buffer.allocUnsafe(4);
  buffer.writeUInt32LE(value >>> 0, 0);
  return buffer;
}

function zip(entries) {
  const localParts = [];
  const centralParts = [];
  let offset = 0;
  const dosDate = ((2020 - 1980) << 9) | (1 << 5) | 1;
  for (const [name, value] of entries) {
    const nameBytes = encoder.encode(name);
    const data = typeof value === "string" ? encoder.encode(value) : value;
    const checksum = crc32(data);
    const local = Buffer.concat([
      u32(0x04034b50),
      u16(20),
      u16(0x0800),
      u16(0),
      u16(0),
      u16(dosDate),
      u32(checksum),
      u32(data.length),
      u32(data.length),
      u16(nameBytes.length),
      u16(0),
      nameBytes,
      data,
    ]);
    localParts.push(local);
    centralParts.push(
      Buffer.concat([
        u32(0x02014b50),
        u16(20),
        u16(20),
        u16(0x0800),
        u16(0),
        u16(0),
        u16(dosDate),
        u32(checksum),
        u32(data.length),
        u32(data.length),
        u16(nameBytes.length),
        u16(0),
        u16(0),
        u16(0),
        u16(0),
        u32(0),
        u32(offset),
        nameBytes,
      ]),
    );
    offset += local.length;
  }
  const central = Buffer.concat(centralParts);
  return Buffer.concat([
    ...localParts,
    central,
    u32(0x06054b50),
    u16(0),
    u16(0),
    u16(entries.length),
    u16(entries.length),
    u32(central.length),
    u32(offset),
    u16(0),
  ]);
}

function unzipStored(buffer) {
  if (buffer.length > 64 * 1024 * 1024) throw new Error("Office fixture exceeds the size limit");
  const entries = new Map();
  let offset = 0;
  while (offset + 4 <= buffer.length && buffer.readUInt32LE(offset) === 0x04034b50) {
    const compression = buffer.readUInt16LE(offset + 8);
    const compressedSize = buffer.readUInt32LE(offset + 18);
    const uncompressedSize = buffer.readUInt32LE(offset + 22);
    const nameLength = buffer.readUInt16LE(offset + 26);
    const extraLength = buffer.readUInt16LE(offset + 28);
    if (compressedSize === 0 ? uncompressedSize > 0 : uncompressedSize / compressedSize > 100) {
      throw new Error("Office fixture ZIP has an abnormal compression ratio");
    }
    if (compression !== 0) throw new Error("Office fixture ZIP must use stored entries");
    const nameStart = offset + 30;
    const dataStart = nameStart + nameLength + extraLength;
    const dataEnd = dataStart + compressedSize;
    if (dataEnd > buffer.length) throw new Error("Office fixture ZIP entry is truncated");
    const name = decoder.decode(buffer.subarray(nameStart, nameStart + nameLength));
    if (
      name.startsWith("/") ||
      name.includes("\\") ||
      name.split("/").some((segment) => segment === ".." || segment === ".")
    ) {
      throw new Error(`Office fixture ZIP path traversal rejected for ${name}`);
    }
    if (entries.has(name)) throw new Error(`Office fixture ZIP has duplicate part ${name}`);
    if (entries.size >= 2_048) throw new Error("Office fixture ZIP has too many parts");
    const data = buffer.subarray(dataStart, dataEnd);
    if (crc32(data) !== buffer.readUInt32LE(offset + 14)) {
      throw new Error(`Office fixture ZIP CRC mismatch for ${name}`);
    }
    entries.set(name, data);
    offset = dataEnd;
  }
  if (entries.size === 0) throw new Error("Office fixture is not an OOXML ZIP package");
  return entries;
}

function decodeXml(value) {
  return value
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&amp;", "&");
}

function xmlTexts(value, prefix) {
  return [
    ...value.matchAll(new RegExp(`<${prefix}:t(?:\\s[^>]*)?>([\\s\\S]*?)<\\/${prefix}:t>`, "gu")),
  ]
    .map((match) => decodeXml(match[1]))
    .filter(Boolean);
}

function pdfTexts(value) {
  return [...value.matchAll(/\(((?:\\.|[^\\)])*)\)\s*Tj/gu)].map((match) =>
    match[1].replaceAll(/\\([\\()])/gu, "$1"),
  );
}

function xml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function rootRelationships(target) {
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="${target}"/>
</Relationships>`;
}

function createDocx(title, body) {
  return zip([
    [
      "[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>`,
    ],
    ["_rels/.rels", rootRelationships("word/document.xml")],
    [
      "word/document.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
  <w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>${xml(title)}</w:t></w:r></w:p>
  <w:p><w:r><w:t xml:space="preserve">${xml(body)}</w:t></w:r></w:p>
  <w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
</w:body></w:document>`,
    ],
  ]);
}

function createXlsx(title, body) {
  return zip([
    [
      "[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>`,
    ],
    ["_rels/.rels", rootRelationships("xl/workbook.xml")],
    [
      "xl/workbook.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Summary" sheetId="1" r:id="rId1"/></sheets></workbook>`,
    ],
    [
      "xl/_rels/workbook.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>`,
    ],
    [
      "xl/worksheets/sheet1.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
  <row r="1"><c r="A1" t="inlineStr"><is><t>${xml(title)}</t></is></c></row>
  <row r="2"><c r="A2" t="inlineStr"><is><t>${xml(body)}</t></is></c></row>
</sheetData></worksheet>`,
    ],
    [
      "xl/styles.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="1"><xf xfId="0"/></cellXfs></styleSheet>`,
    ],
  ]);
}

function createPptx(title, body) {
  return zip([
    [
      "[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
  <Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
  <Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
</Types>`,
    ],
    ["_rels/.rels", rootRelationships("ppt/presentation.xml")],
    [
      "ppt/presentation.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>`,
    ],
    [
      "ppt/_rels/presentation.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>`,
    ],
    [
      "ppt/slides/slide1.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="3200"/><a:t>${xml(title)}</a:t></a:r></a:p><a:p><a:r><a:rPr lang="en-US" sz="1800"/><a:t>${xml(body)}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>`,
    ],
    [
      "ppt/slides/_rels/slide1.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>`,
    ],
    [
      "ppt/slideLayouts/slideLayout1.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank"><p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>`,
    ],
    [
      "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>`,
    ],
    [
      "ppt/slideMasters/slideMaster1.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:sldLayoutIdLst><p:sldLayoutId id="1" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>`,
    ],
    [
      "ppt/slideMasters/_rels/slideMaster1.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>`,
    ],
    [
      "ppt/theme/theme1.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Hachimi E2E"><a:themeElements><a:clrScheme name="Hachimi"><a:dk1><a:srgbClr val="202124"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="5B8DEF"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="70AD47"/></a:accent3><a:accent4><a:srgbClr val="A5A5A5"/></a:accent4><a:accent5><a:srgbClr val="FFC000"/></a:accent5><a:accent6><a:srgbClr val="4472C4"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Hachimi"><a:majorFont><a:latin typeface="Aptos Display"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/></a:minorFont></a:fontScheme><a:fmtScheme name="Hachimi"><a:fillStyleLst/><a:lnStyleLst/><a:effectStyleLst/><a:bgFillStyleLst/></a:fmtScheme></a:themeElements></a:theme>`,
    ],
  ]);
}

function pdfText(value) {
  return String(value)
    .normalize("NFKD")
    .replaceAll(/[^\x20-\x7e]/gu, "?")
    .replaceAll("\\", "\\\\")
    .replaceAll("(", "\\(")
    .replaceAll(")", "\\)");
}

function createPdf(title, body) {
  const stream = `BT /F1 18 Tf 72 720 Td (${pdfText(title)}) Tj 0 -30 Td /F1 11 Tf (${pdfText(body)}) Tj ET`;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let output = "%PDF-1.4\n%Hachimi-E2E\n";
  const offsets = [0];
  for (let index = 0; index < objects.length; index += 1) {
    offsets.push(Buffer.byteLength(output));
    output += `${index + 1} 0 obj\n${objects[index]}\nendobj\n`;
  }
  const xref = Buffer.byteLength(output);
  output += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  output += offsets
    .slice(1)
    .map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`)
    .join("");
  output += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(output, "ascii");
}

export function createOfficeArtifact(path, kind, title, body) {
  const creator = { docx: createDocx, xlsx: createXlsx, pptx: createPptx, pdf: createPdf }[kind];
  if (!creator) throw new Error(`Unsupported Office fixture type: ${kind}`);
  writeFileSync(path, creator(title, body));
  return validateOfficeArtifact(path);
}

export function validateOfficeArtifact(path) {
  const extension = extname(path).slice(1).toLowerCase();
  const buffer = readFileSync(path);
  if (extension === "pdf") {
    const text = buffer.toString("ascii");
    if (!text.startsWith("%PDF-1.4") || !text.includes("xref\n") || !text.endsWith("%%EOF\n")) {
      throw new Error("PDF fixture is missing its header, xref, or EOF marker");
    }
    return {
      kind: extension,
      byteLength: buffer.length,
      parts: 5,
      semantics: { pages: [{ page: 1, texts: pdfTexts(text) }] },
    };
  }
  const entries = unzipStored(buffer);
  for (const name of entries.keys()) {
    if (/vbaProject\.bin$|macrosheets\//iu.test(name)) {
      throw new Error(`OOXML macro content is not allowed: ${name}`);
    }
  }
  const required = {
    docx: ["[Content_Types].xml", "_rels/.rels", "word/document.xml"],
    xlsx: [
      "[Content_Types].xml",
      "_rels/.rels",
      "xl/workbook.xml",
      "xl/_rels/workbook.xml.rels",
      "xl/worksheets/sheet1.xml",
    ],
    pptx: [
      "[Content_Types].xml",
      "_rels/.rels",
      "ppt/presentation.xml",
      "ppt/_rels/presentation.xml.rels",
      "ppt/slides/slide1.xml",
      "ppt/slides/_rels/slide1.xml.rels",
    ],
  }[extension];
  if (!required) throw new Error(`Unsupported Office artifact extension: ${extension}`);
  for (const part of required) {
    if (!entries.has(part)) throw new Error(`OOXML fixture is missing ${part}`);
  }
  const contentTypes = decoder.decode(entries.get("[Content_Types].xml"));
  if (!contentTypes.includes("openxmlformats")) {
    throw new Error("OOXML fixture has invalid content types");
  }
  if (/macroEnabled|vbaProject/iu.test(contentTypes)) {
    throw new Error("OOXML macro-enabled content type is not allowed");
  }
  const semantics = {
    docx: () => {
      const texts = xmlTexts(decoder.decode(entries.get("word/document.xml")), "w");
      return { title: texts[0] ?? "", paragraphs: texts.slice(1) };
    },
    xlsx: () => {
      const workbook = decoder.decode(entries.get("xl/workbook.xml"));
      const sheetName = decodeXml(workbook.match(/<sheet\s+name="([^"]+)"/u)?.[1] ?? "");
      const worksheet = decoder.decode(entries.get("xl/worksheets/sheet1.xml"));
      const cells = Object.fromEntries(
        [
          ...worksheet.matchAll(
            /<c\s+r="([A-Z]+\d+)"[^>]*>[\s\S]*?<t(?:\s[^>]*)?>([\s\S]*?)<\/t>[\s\S]*?<\/c>/gu,
          ),
        ].map(([, reference, value]) => [reference, decodeXml(value)]),
      );
      return { sheets: [{ name: sheetName, cells }] };
    },
    pptx: () => ({
      slides: [
        {
          slide: 1,
          texts: xmlTexts(decoder.decode(entries.get("ppt/slides/slide1.xml")), "a"),
        },
      ],
    }),
  }[extension]();
  return { kind: extension, byteLength: buffer.length, parts: entries.size, semantics };
}

export function createOfficePackageFixtureForTest(entries) {
  return zip(entries);
}

export function validateOfficeOutputTarget(root, target, { overwrite = false } = {}) {
  const resolvedRoot = resolve(root);
  const resolvedTarget = resolve(target);
  const relativeTarget = relative(resolvedRoot, resolvedTarget);
  if (
    relativeTarget === "" ||
    relativeTarget === ".." ||
    relativeTarget.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) ||
    isAbsolute(relativeTarget)
  ) {
    throw new Error("Office output target escaped the authorized directory");
  }
  if (!overwrite && existsSync(resolvedTarget)) {
    throw new Error("Office output target already exists and overwrite was not authorized");
  }
  return resolvedTarget;
}
