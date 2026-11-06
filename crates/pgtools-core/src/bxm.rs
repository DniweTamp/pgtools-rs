//! BXM / XML Binary Format (`BXM\0` / `XML\0`) reader, writer, and XML converter.
//!
//! Matches `lib/bayonetta/bxm.rb`. Used across Bayonetta 1, Bayonetta 2, Vanquish,
//! Metal Gear Rising, The Wonderful 101, Nier: Automata, and Astral Chain for engine
//! configuration trees, state machines, shader metadata, and cloth/facial parameters.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::io::{BinReader, BinWriter, Endian};
use crate::{PgError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BxmXmlNode {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub value: Option<String>,
    pub children: Vec<BxmXmlNode>,
}

impl BxmXmlNode {
    pub fn to_pretty_xml(&self) -> String {
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        self.write_xml(&mut out, 0);
        out
    }

    fn write_xml(&self, out: &mut String, indent: usize) {
        let pad = "  ".repeat(indent);
        out.push_str(&pad);
        out.push('<');
        out.push_str(&escape_xml_tag(&self.name));
        for (k, v) in &self.attributes {
            out.push(' ');
            out.push_str(&escape_xml_tag(k));
            out.push_str("=\"");
            out.push_str(&escape_xml_attr(v));
            out.push('"');
        }
        let val_str = self.value.as_deref().unwrap_or("");
        if self.children.is_empty() && val_str.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push('>');
        if self.children.is_empty() {
            out.push_str(&escape_xml_text(val_str));
            out.push_str("</");
            out.push_str(&escape_xml_tag(&self.name));
            out.push_str(">\n");
        } else {
            out.push('\n');
            if !val_str.is_empty() {
                out.push_str(&pad);
                out.push_str("  ");
                out.push_str(&escape_xml_text(val_str));
                out.push('\n');
            }
            for child in &self.children {
                child.write_xml(out, indent + 1);
            }
            out.push_str(&pad);
            out.push_str("</");
            out.push_str(&escape_xml_tag(&self.name));
            out.push_str(">\n");
        }
    }
}

fn escape_xml_tag(s: &str) -> String {
    if s.is_empty() {
        "_empty".to_string()
    } else {
        s.to_string()
    }
}

fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        if let Some(semi) = after.find(';') {
            let entity = &after[..=semi];
            match entity {
                "&quot;" => out.push('"'),
                "&apos;" => out.push('\''),
                "&lt;" => out.push('<'),
                "&gt;" => out.push('>'),
                "&amp;" => out.push('&'),
                _ => out.push_str(entity),
            }
            rest = &after[semi + 1..];
        } else {
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BxmFile {
    pub tag: String,
    pub flags: u32,
    pub root: Option<BxmXmlNode>,
}

#[derive(Debug, Clone, Copy)]
struct RawNodeInfo {
    num_children: u16,
    first_child_index: u16,
    num_attributes: u16,
    data_index: u16,
}

#[derive(Debug, Clone, Copy)]
struct RawDataOffsets {
    name_offset: u16,
    value_offset: u16,
}

impl BxmFile {
    /// Parse a `.bxm` (big-endian by default in PlatinumGames titles).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut r = BinReader::new(bytes, Endian::Big);
        let magic = r.read_bytes(4)?;
        if magic != b"BXM\0" && magic != b"XML\0" {
            return Err(PgError::InvalidMagic {
                expected: "BXM\\0 or XML\\0",
                actual: magic.to_vec(),
            });
        }
        let tag = String::from_utf8_lossy(&magic[..3]).into_owned();

        let flags = r.read_u32()?;
        let num_nodes = r.read_u16()? as usize;
        let num_data = r.read_u16()? as usize;
        let data_size = r.read_u32()? as usize;

        let mut node_infos = Vec::with_capacity(num_nodes);
        for _ in 0..num_nodes {
            node_infos.push(RawNodeInfo {
                num_children: r.read_u16()?,
                first_child_index: r.read_u16()?,
                num_attributes: r.read_u16()?,
                data_index: r.read_u16()?,
            });
        }

        let mut data_offsets = Vec::with_capacity(num_data);
        for _ in 0..num_data {
            data_offsets.push(RawDataOffsets {
                name_offset: r.read_u16()?,
                value_offset: r.read_u16()?,
            });
        }

        let string_block_start = r.pos();
        if string_block_start + data_size > bytes.len() {
            return Err(PgError::InvalidData(format!(
                "BXM string block exceeds file size: offset 0x{:X} + {} > {}",
                string_block_start,
                data_size,
                bytes.len()
            )));
        }

        let read_str = |offset: u16| -> Result<Option<String>> {
            if offset == 0xFFFF {
                return Ok(None);
            }
            let s = r.read_cstring_at(string_block_start + offset as usize)?;
            if s.is_empty() {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        };

        fn build_node(
            idx: usize,
            node_infos: &[RawNodeInfo],
            data_offsets: &[RawDataOffsets],
            read_str: &dyn Fn(u16) -> Result<Option<String>>,
        ) -> Result<BxmXmlNode> {
            let info = node_infos
                .get(idx)
                .ok_or_else(|| PgError::InvalidData(format!("Invalid BXM node index {idx}")))?;
            let data_idx = info.data_index as usize;
            let node_data = data_offsets
                .get(data_idx)
                .ok_or_else(|| PgError::InvalidData(format!("Invalid BXM data index {data_idx}")))?;

            let name = read_str(node_data.name_offset)?.unwrap_or_default();
            let value = read_str(node_data.value_offset)?;

            let mut attributes = Vec::with_capacity(info.num_attributes as usize);
            for a in 0..info.num_attributes as usize {
                let attr_data = data_offsets.get(data_idx + 1 + a).ok_or_else(|| {
                    PgError::InvalidData(format!(
                        "Invalid BXM attribute data index {}",
                        data_idx + 1 + a
                    ))
                })?;
                let attr_name = read_str(attr_data.name_offset)?.unwrap_or_default();
                let attr_val = read_str(attr_data.value_offset)?.unwrap_or_default();
                attributes.push((attr_name, attr_val));
            }

            let mut children = Vec::with_capacity(info.num_children as usize);
            let first_child = info.first_child_index as usize;
            for c in 0..info.num_children as usize {
                children.push(build_node(
                    first_child + c,
                    node_infos,
                    data_offsets,
                    read_str,
                )?);
            }

            Ok(BxmXmlNode {
                name,
                attributes,
                value,
                children,
            })
        }

        let root = if num_nodes > 0 {
            Some(build_node(0, &node_infos, &data_offsets, &read_str)?)
        } else {
            None
        };

        Ok(Self {
            tag,
            flags,
            root,
        })
    }

    pub fn to_xml_string(&self) -> String {
        match &self.root {
            Some(r) => r.to_pretty_xml(),
            None => String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"),
        }
    }

    /// Parse UTF-8 XML text into a `BxmFile` ready for binary encoding.
    pub fn from_xml_str(xml: &str, tag: &str) -> Result<Self> {
        let root = parse_xml_document(xml)?;
        Ok(Self {
            tag: if tag.starts_with("XML") {
                "XML".to_string()
            } else {
                "BXM".to_string()
            },
            flags: 0,
            root: Some(root),
        })
    }

    /// Serialize this `BxmFile` back into binary BXM bytes (1:1 with `BXMFile#dump_nodes`).
    pub fn to_bytes(&self, endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::new(endian);
        let magic: [u8; 4] = if self.tag.starts_with("XML") {
            *b"XML\0"
        } else {
            *b"BXM\0"
        };

        let Some(ref root_node) = self.root else {
            w.write_bytes(&magic);
            w.write_u32(self.flags);
            w.write_u16(0);
            w.write_u16(0);
            w.write_u32(0);
            return w.into_inner();
        };

        let mut flat_nodes: Vec<&BxmXmlNode> = Vec::new();
        let mut node_infos: Vec<RawNodeInfo> = Vec::new();
        let mut unique_strings: Vec<String> = Vec::new();
        let mut seen_strings: HashSet<String> = HashSet::new();

        fn add_unique_str(
            s: &str,
            unique_strings: &mut Vec<String>,
            seen_strings: &mut HashSet<String>,
        ) {
            if !s.is_empty() && seen_strings.insert(s.to_string()) {
                unique_strings.push(s.to_string());
            }
        }

        fn process<'a>(
            node: &'a BxmXmlNode,
            node_idx: usize,
            flat_nodes: &mut Vec<&'a BxmXmlNode>,
            node_infos: &mut Vec<RawNodeInfo>,
            unique_strings: &mut Vec<String>,
            seen_strings: &mut HashSet<String>,
        ) {
            add_unique_str(&node.name, unique_strings, seen_strings);
            if let Some(ref val) = node.value {
                add_unique_str(val, unique_strings, seen_strings);
            }
            for (k, v) in &node.attributes {
                add_unique_str(k, unique_strings, seen_strings);
                add_unique_str(v, unique_strings, seen_strings);
            }

            let first_child_idx = flat_nodes.len();
            node_infos[node_idx].num_children = node.children.len() as u16;
            node_infos[node_idx].first_child_index = if node.children.is_empty() {
                flat_nodes.len() as u16
            } else {
                first_child_idx as u16
            };
            node_infos[node_idx].num_attributes = node.attributes.len() as u16;

            for child in &node.children {
                flat_nodes.push(child);
                node_infos.push(RawNodeInfo {
                    num_children: 0,
                    first_child_index: 0,
                    num_attributes: 0,
                    data_index: 0,
                });
            }

            for (i, child) in node.children.iter().enumerate() {
                process(
                    child,
                    first_child_idx + i,
                    flat_nodes,
                    node_infos,
                    unique_strings,
                    seen_strings,
                );
            }
        }

        flat_nodes.push(root_node);
        node_infos.push(RawNodeInfo {
            num_children: 0,
            first_child_index: 0,
            num_attributes: 0,
            data_index: 0,
        });
        process(
            root_node,
            0,
            &mut flat_nodes,
            &mut node_infos,
            &mut unique_strings,
            &mut seen_strings,
        );

        let mut string_offsets: HashMap<String, u16> = HashMap::new();
        let mut string_blob: Vec<u8> = Vec::new();
        for s in unique_strings {
            let offset = string_blob.len().min(0xFFFE) as u16;
            string_offsets.insert(s.clone(), offset);
            string_blob.extend_from_slice(s.as_bytes());
            string_blob.push(0);
        }

        let str_off = |s: &str| -> u16 {
            if s.is_empty() {
                0xFFFF
            } else {
                *string_offsets.get(s).unwrap_or(&0xFFFF)
            }
        };

        let mut data_offsets: Vec<RawDataOffsets> = Vec::new();
        for (idx, node) in flat_nodes.iter().enumerate() {
            node_infos[idx].data_index = data_offsets.len() as u16;
            data_offsets.push(RawDataOffsets {
                name_offset: str_off(&node.name),
                value_offset: str_off(node.value.as_deref().unwrap_or("")),
            });
            for (k, v) in &node.attributes {
                data_offsets.push(RawDataOffsets {
                    name_offset: str_off(k),
                    value_offset: str_off(v),
                });
            }
        }

        w.write_bytes(&magic);
        w.write_u32(self.flags);
        w.write_u16(node_infos.len() as u16);
        w.write_u16(data_offsets.len() as u16);
        w.write_u32(string_blob.len() as u32);

        for info in &node_infos {
            w.write_u16(info.num_children);
            w.write_u16(info.first_child_index);
            w.write_u16(info.num_attributes);
            w.write_u16(info.data_index);
        }
        for d in &data_offsets {
            w.write_u16(d.name_offset);
            w.write_u16(d.value_offset);
        }
        w.write_bytes(&string_blob);

        w.into_inner()
    }
}

/// Lightweight recursive XML parser for BXM XML documents.
fn parse_xml_document(xml: &str) -> Result<BxmXmlNode> {
    let mut s = xml.trim();
    while s.starts_with("<?") || s.starts_with("<!--") {
        if s.starts_with("<?") {
            let end = s
                .find("?>")
                .ok_or_else(|| PgError::Xml("Unclosed XML declaration".into()))?;
            s = s[end + 2..].trim_start();
        } else if s.starts_with("<!--") {
            let end = s
                .find("-->")
                .ok_or_else(|| PgError::Xml("Unclosed XML comment".into()))?;
            s = s[end + 3..].trim_start();
        }
    }
    let (node, _) = parse_xml_element(s)?;
    Ok(node)
}

fn parse_xml_element(mut s: &str) -> Result<(BxmXmlNode, &str)> {
    s = s.trim_start();
    while s.starts_with("<!--") {
        let end = s
            .find("-->")
            .ok_or_else(|| PgError::Xml("Unclosed XML comment".into()))?;
        s = s[end + 3..].trim_start();
    }
    if !s.starts_with('<') {
        return Err(PgError::Xml("Expected '<' at start of XML element".into()));
    }
    let open_end = s
        .find('>')
        .ok_or_else(|| PgError::Xml("Unclosed opening tag".into()))?;
    let inside = &s[1..open_end];
    let self_closing = inside.ends_with('/');
    let inside = inside.trim_end_matches('/').trim();

    let (raw_name, attr_str) = match inside.find(char::is_whitespace) {
        Some(idx) => (&inside[..idx], inside[idx..].trim()),
        None => (inside, ""),
    };
    let name = if raw_name == "_empty" {
        String::new()
    } else {
        raw_name.to_string()
    };
    let attributes = parse_attributes(attr_str);

    let mut rest = &s[open_end + 1..];
    if self_closing {
        return Ok((
            BxmXmlNode {
                name,
                attributes,
                value: None,
                children: Vec::new(),
            },
            rest,
        ));
    }

    let mut children = Vec::new();
    let mut text_acc = String::new();
    let close_tag = format!("</{raw_name}>");

    loop {
        let trimmed = rest.trim_start();
        if trimmed.starts_with("<!--") {
            let end = trimmed
                .find("-->")
                .ok_or_else(|| PgError::Xml("Unclosed XML comment".into()))?;
            rest = &trimmed[end + 3..];
            continue;
        }
        if let Some(after_close) = trimmed.strip_prefix(&close_tag) {
            rest = after_close;
            break;
        }
        if trimmed.starts_with("</") {
            let end = trimmed
                .find('>')
                .ok_or_else(|| PgError::Xml("Unclosed closing tag".into()))?;
            let found_close = &trimmed[..=end];
            return Err(PgError::Xml(format!(
                "Mismatched closing tag: expected {close_tag}, found {found_close}"
            )));
        }
        if trimmed.starts_with('<') {
            let (child, after_child) = parse_xml_element(trimmed)?;
            children.push(child);
            rest = after_child;
        } else {
            let next_lt = rest
                .find('<')
                .ok_or_else(|| PgError::Xml("Unexpected EOF inside XML element".into()))?;
            text_acc.push_str(&unescape_xml(rest[..next_lt].trim()));
            rest = &rest[next_lt..];
        }
    }

    let value = if text_acc.is_empty() {
        None
    } else {
        Some(text_acc)
    };

    Ok((
        BxmXmlNode {
            name,
            attributes,
            value,
            children,
        },
        rest,
    ))
}

fn parse_attributes(mut s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    while !s.is_empty() {
        s = s.trim_start();
        let Some(eq) = s.find('=') else {
            break;
        };
        let key = s[..eq].trim().to_string();
        s = s[eq + 1..].trim_start();
        if let Some(stripped) = s.strip_prefix('"') {
            if let Some(end_q) = stripped.find('"') {
                let val = unescape_xml(&stripped[..end_q]);
                attrs.push((key, val));
                s = &stripped[end_q + 1..];
            } else {
                break;
            }
        } else if let Some(stripped) = s.strip_prefix('\'') {
            if let Some(end_q) = stripped.find('\'') {
                let val = unescape_xml(&stripped[..end_q]);
                attrs.push((key, val));
                s = &stripped[end_q + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }
    attrs
}
