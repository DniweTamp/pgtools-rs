import { BxmXmlNode } from "../types";

export function stripInvalidXmlChars(s: string): string {
  return (s ?? "").replace(/[\x00-\x08\x0B\x0C\x0E-\x1F]/g, "");
}

export function escapeXmlAttr(s: string): string {
  return stripInvalidXmlChars(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

export function escapeXmlText(s: string): string {
  return stripInvalidXmlChars(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function parseXmlToBxmNode(xmlStr: string): BxmXmlNode | null {
  try {
    const parser = new DOMParser();
    const doc = parser.parseFromString(xmlStr, "application/xml");
    if (doc.querySelector("parsererror")) return null;
    const rootEl = doc.documentElement;
    if (!rootEl) return null;
    const convertEl = (el: Element): BxmXmlNode => {
      const attributes: [string, string][] = [];
      for (let i = 0; i < el.attributes.length; i++) {
        const attr = el.attributes[i];
        attributes.push([attr.name, attr.value]);
      }
      const children: BxmXmlNode[] = [];
      let directText = "";
      for (let i = 0; i < el.childNodes.length; i++) {
        const cn = el.childNodes[i];
        if (cn.nodeType === Node.ELEMENT_NODE) {
          children.push(convertEl(cn as Element));
        } else if (cn.nodeType === Node.TEXT_NODE || cn.nodeType === Node.CDATA_SECTION_NODE) {
          directText += cn.nodeValue ?? "";
        }
      }
      const trimmedText = directText.trim();
      return {
        name: el.tagName,
        attributes,
        value: trimmedText.length > 0 ? trimmedText : null,
        children,
      };
    };
    return convertEl(rootEl);
  } catch {
    return null;
  }
}

export function serializeBxmNodeToXml(node: BxmXmlNode, indent = 0): string {
  const pad = "  ".repeat(indent);
  const tag = (node.name || "Node").trim() || "Node";
  const attrs = [...(node.attributes || [])]
    .filter(([k]) => k && k.trim().length > 0)
    .map(([k, v]) => ` ${k.trim()}="${escapeXmlAttr(v ?? "")}"`)
    .join("");
  const children = node.children || [];
  const val = node.value !== null && node.value !== undefined ? String(node.value) : "";

  if (children.length === 0 && val.length === 0) {
    return `${pad}<${tag}${attrs}/>`;
  }
  if (children.length === 0) {
    return `${pad}<${tag}${attrs}>${escapeXmlText(val)}</${tag}>`;
  }
  const inner: string[] = [];
  if (val.length > 0) {
    inner.push(`${pad}  ${escapeXmlText(val)}`);
  }
  for (const child of children) {
    inner.push(serializeBxmNodeToXml(child, indent + 1));
  }
  return `${pad}<${tag}${attrs}>\n${inner.join("\n")}\n${pad}</${tag}>`;
}

export function bxmTreeToXmlDocument(root: BxmXmlNode | null): string {
  if (!root) return '<?xml version="1.0" encoding="utf-8"?>\n<Root/>\n';
  return `<?xml version="1.0" encoding="utf-8"?>\n${serializeBxmNodeToXml(root, 0)}\n`;
}


export function detectBxmTabular(root: BxmXmlNode | null): boolean {
    if (!root || !Array.isArray(root.children)) return false;
    if (root.children.length >= 2) {
      const firstTag = root.children[0].name;
      const sameTagCount = root.children.filter((c) => c.name === firstTag).length;
      if (sameTagCount >= 2 && (root.children[0].attributes.length > 0 || root.children[0].children.length > 0)) {
        return true;
      }
    }
    return false;
  }

export function getBxmNodeByPath(root: BxmXmlNode | null, pathStr: string): BxmXmlNode | null {
    if (!root) return null;
    if (pathStr === "0") return root;
    const indices = pathStr
      .split(".")
      .slice(1)
      .map((x) => parseInt(x, 10));
    let cur: BxmXmlNode = root;
    for (const idx of indices) {
      if (!cur.children || !cur.children[idx]) return null;
      cur = cur.children[idx];
    }
    return cur;
  }
