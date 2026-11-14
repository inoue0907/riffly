//! Documents on disk are markdown: [`parse`] turns a file into blocks, and
//! [`RifflyEditor::to_markdown`] writes them back.
//!
//! The format is the one the clipboard already speaks (`selection.rs`), with
//! the node types it has no syntax for spelled the way common markdown
//! dialects spell them: an image is `![alt](src)` and a callout is a GitHub
//! alert, `> [!NOTE]`. Inline marks are not written, so a file keeps the
//! document's structure and text but not its formatting or its comments.

use gpui_kit::App;

use super::block::{BlockAttrs, BlockContent, types};
use super::grid::table_content;
use super::view::RifflyEditor;

/// The callout marker, a GitHub alert.
const CALLOUT_PREFIX: &str = "[!NOTE]";

impl RifflyEditor {
    /// The whole document as markdown, block by block, blank-line separated.
    pub fn to_markdown(&self, cx: &App) -> String {
        let mut out: Vec<String> = Vec::with_capacity(self.blocks.len());
        for block in &self.blocks {
            let indent = "    ".repeat(block.indent);
            let text = match block.ty.as_ref() {
                types::IMAGE => {
                    let src = block.attrs.src.clone().unwrap_or_default();
                    let alt = block.attrs.alt.clone().unwrap_or_default();
                    format!("![{alt}]({src})")
                }
                types::CALLOUT => format!(
                    "> {CALLOUT_PREFIX}\n{}",
                    quote_lines(&block.text)
                ),
                types::BLOCKQUOTE => quote_lines(&block.text),
                types::PARAGRAPH if block.text.is_empty() => continue,
                types::PARAGRAPH => format!("{indent}{}", block.text),
                _ => self.markdown_of(&[block.id], cx),
            };
            out.push(text);
        }
        let mut text = out.join("\n\n");
        text.push('\n');
        text
    }
}

fn quote_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How deep a line is nested: four columns a level, or two when the file
/// indents by two.
fn indent_of(line: &str) -> (usize, &str) {
    let mut width = 0;
    let mut bytes = 0;
    for ch in line.chars() {
        match ch {
            ' ' => width += 1,
            '\t' => width += 4,
            _ => break,
        }
        bytes += ch.len_utf8();
    }
    let level = if width % 4 == 0 { width / 4 } else { width / 2 };
    (level, &line[bytes..])
}

fn is_rule(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| *c != ' ').collect();
    marks.len() >= 3
        && matches!(marks[0], '-' | '*' | '_')
        && marks.iter().all(|c| *c == marks[0])
}

fn is_table_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed.contains('-')
        && trimmed
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(|cell| cell.trim().to_string()).collect()
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if rest.is_empty() {
        return Some((hashes as u8, ""));
    }
    rest.strip_prefix(' ').map(|text| (hashes as u8, text.trim_end()))
}

fn image(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let src = rest.strip_suffix(')')?;
    Some((alt.to_string(), src.to_string()))
}

/// A list item's type, attributes and text, if the line is one.
fn list_item(line: &str) -> Option<(&'static str, BlockAttrs, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            for (marker, checked) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                if let Some(text) = rest.strip_prefix(marker) {
                    let attrs = BlockAttrs {
                        checked,
                        ..Default::default()
                    };
                    return Some((types::TASK_LIST, attrs, text));
                }
            }
            for (marker, checked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
                if rest == marker {
                    let attrs = BlockAttrs {
                        checked,
                        ..Default::default()
                    };
                    return Some((types::TASK_LIST, attrs, ""));
                }
            }
            return Some((types::BULLET_LIST, BlockAttrs::default(), rest));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        for marker in [". ", ") "] {
            if let Some(text) = rest.strip_prefix(marker) {
                return Some((types::ORDERED_LIST, BlockAttrs::default(), text));
            }
        }
    }
    None
}

/// Read a markdown document into blocks.
pub fn parse(source: &str) -> Vec<BlockContent> {
    let lines: Vec<&str> = source.lines().collect();
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut ix = 0;

    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<BlockContent>| {
        if !paragraph.is_empty() {
            blocks.push(BlockContent::paragraph(paragraph.join("\n")));
            paragraph.clear();
        }
    };

    while ix < lines.len() {
        let raw = lines[ix];
        let (indent, line) = indent_of(raw);

        if line.trim().is_empty() {
            flush(&mut paragraph, &mut blocks);
            ix += 1;
            continue;
        }

        // A fenced code block runs to its closing fence, verbatim.
        if let Some(language) = line.strip_prefix("```") {
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            ix += 1;
            while ix < lines.len() && !lines[ix].trim_start().starts_with("```") {
                code.push(lines[ix]);
                ix += 1;
            }
            ix += 1; // the closing fence
            let language = language.trim();
            let attrs = if language.is_empty() {
                BlockAttrs::default()
            } else {
                BlockAttrs::language(language.to_string())
            };
            blocks.push(BlockContent::new(types::CODE_BLOCK, code.join("\n")).with_attrs(attrs));
            continue;
        }

        if let Some((level, text)) = heading(line) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(
                BlockContent::new(types::HEADING, text).with_attrs(BlockAttrs::level(level)),
            );
            ix += 1;
            continue;
        }

        if is_rule(line) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(BlockContent::new(types::HORIZONTAL_RULE, ""));
            ix += 1;
            continue;
        }

        if line.trim_start().starts_with('|')
            && lines.get(ix + 1).is_some_and(|next| is_table_rule(next))
        {
            flush(&mut paragraph, &mut blocks);
            let mut rows = vec![table_cells(line)];
            ix += 2;
            while ix < lines.len() && lines[ix].trim_start().starts_with('|') {
                rows.push(table_cells(lines[ix]));
                ix += 1;
            }
            let rows: Vec<Vec<&str>> = rows
                .iter()
                .map(|row| row.iter().map(String::as_str).collect())
                .collect();
            let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
            blocks.push(table_content(&rows));
            continue;
        }

        if let Some((alt, src)) = image(line) {
            flush(&mut paragraph, &mut blocks);
            let attrs = BlockAttrs {
                src: Some(src.into()),
                alt: Some(alt.into()),
                ..Default::default()
            };
            blocks.push(BlockContent::new(types::IMAGE, "").with_attrs(attrs));
            ix += 1;
            continue;
        }

        if line.starts_with('>') {
            flush(&mut paragraph, &mut blocks);
            let mut quoted = Vec::new();
            while ix < lines.len() {
                let (_, next) = indent_of(lines[ix]);
                let Some(rest) = next.strip_prefix('>') else {
                    break;
                };
                quoted.push(rest.strip_prefix(' ').unwrap_or(rest));
                ix += 1;
            }
            let (ty, quoted) = match quoted.first() {
                Some(first) if first.trim() == CALLOUT_PREFIX => (types::CALLOUT, &quoted[1..]),
                Some(first) if first.starts_with(CALLOUT_PREFIX) => {
                    // `> [!NOTE] text` on one line.
                    let mut rest = vec![first[CALLOUT_PREFIX.len()..].trim_start()];
                    rest.extend_from_slice(&quoted[1..]);
                    blocks.push(BlockContent::new(types::CALLOUT, rest.join("\n")));
                    continue;
                }
                _ => (types::BLOCKQUOTE, &quoted[..]),
            };
            blocks.push(BlockContent::new(ty, quoted.join("\n")));
            continue;
        }

        if let Some((ty, attrs, text)) = list_item(line) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(
                BlockContent::new(ty, text)
                    .with_attrs(attrs)
                    .with_indent(indent),
            );
            ix += 1;
            continue;
        }

        paragraph.push(line);
        ix += 1;
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(blocks: &[BlockContent]) -> Vec<(String, String)> {
        blocks
            .iter()
            .map(|b| (b.ty.to_string(), b.text.clone()))
            .collect()
    }

    #[test]
    fn reads_every_node_type() {
        let source = "# Title\n\nSome text\nover two lines\n\n## Lists\n\n- one\n    - nested\n1. first\n- [ ] todo\n- [x] done\n\n> quoted\n\n> [!NOTE]\n> a callout\n\n```rust\nfn main() {}\n```\n\n---\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n![cat](cat.png)\n";
        let blocks = parse(source);
        assert_eq!(
            kinds(&blocks),
            vec![
                ("heading".into(), "Title".into()),
                ("paragraph".into(), "Some text\nover two lines".into()),
                ("heading".into(), "Lists".into()),
                ("bulletList".into(), "one".into()),
                ("bulletList".into(), "nested".into()),
                ("orderedList".into(), "first".into()),
                ("taskList".into(), "todo".into()),
                ("taskList".into(), "done".into()),
                ("blockquote".into(), "quoted".into()),
                ("callout".into(), "a callout".into()),
                ("codeBlock".into(), "fn main() {}".into()),
                ("horizontalRule".into(), "".into()),
                ("table".into(), "".into()),
                ("image".into(), "".into()),
            ]
        );
        assert_eq!(blocks[0].attrs.level, 1);
        assert_eq!(blocks[2].attrs.level, 2);
        assert_eq!(blocks[4].indent, 1);
        assert!(!blocks[6].attrs.checked);
        assert!(blocks[7].attrs.checked);
        assert_eq!(blocks[10].attrs.language.as_deref(), Some("rust"));
        assert_eq!(blocks[13].attrs.src.as_deref(), Some("cat.png"));
    }

    #[test]
    fn a_heading_needs_its_space() {
        let blocks = parse("#hashtag");
        assert_eq!(kinds(&blocks), vec![("paragraph".into(), "#hashtag".into())]);
    }

    #[test]
    fn an_empty_file_has_no_blocks() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
    }
}
