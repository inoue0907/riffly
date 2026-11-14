//! HTML documents: [`parse`] turns a page into blocks, keeping its structure
//! and inline formatting, and [`RifflyEditor::to_html`] writes a document
//! back as a plain, standalone page.
//!
//! Only the document is carried over. Styling, scripts and layout are not:
//! a `<div>` is looked through for the headings, paragraphs, lists, tables
//! and images inside it.

use std::fmt::Write as _;

use gpui_kit::App;
use scraper::{ElementRef, Html, Node};

use super::block::{BlockAttrs, BlockContent, types};
use super::grid::table_content;
use super::mark::{Mark, MarkKind, MarkList};
use super::view::RifflyEditor;

// ------------------------------------------------------------------ reading

/// Read an HTML document (or fragment) into blocks.
pub fn parse(source: &str) -> Vec<BlockContent> {
    let document = Html::parse_document(source);
    let mut reader = Reader::default();
    let root = document.root_element();
    let body = root
        .children()
        .filter_map(ElementRef::wrap)
        .find(|el| el.value().name() == "body")
        .unwrap_or(root);
    reader.walk(body);
    reader.flush();
    reader.blocks
}

/// Text with the marks laid over it, gathered from inline content.
#[derive(Default)]
struct Inline {
    text: String,
    marks: Vec<Mark>,
}

impl Inline {
    /// Append text, collapsing runs of white space the way a browser does.
    fn push_text(&mut self, raw: &str, active: &[MarkKind]) {
        let mut normalized = String::with_capacity(raw.len());
        let mut in_space = self.text.is_empty() || self.text.ends_with([' ', '\n']);
        for ch in raw.chars() {
            if ch.is_whitespace() {
                if !in_space {
                    normalized.push(' ');
                    in_space = true;
                }
            } else {
                normalized.push(ch);
                in_space = false;
            }
        }
        self.push_raw(&normalized, active);
    }

    /// Append text as it is, for `<pre>` and line breaks.
    fn push_raw(&mut self, text: &str, active: &[MarkKind]) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        for kind in active {
            self.marks.push(Mark::new(kind.clone(), start..end));
        }
    }

    /// The finished text, trimmed, with marks kept on what is left.
    fn finish(mut self) -> (String, MarkList) {
        let trimmed_end = self.text.trim_end().len();
        self.text.truncate(trimmed_end);
        let lead = self.text.len() - self.text.trim_start().len();
        let text = self.text[lead..].to_string();
        let marks = self
            .marks
            .into_iter()
            .filter_map(|mark| {
                let start = mark.range.start.saturating_sub(lead).min(text.len());
                let end = mark.range.end.saturating_sub(lead).min(text.len());
                (start < end).then(|| Mark::new(mark.kind, start..end))
            })
            .collect();
        (text, MarkList::from_marks(marks))
    }

    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// The mark an inline element applies, if it is one that formats text.
fn inline_mark(el: &ElementRef) -> Option<MarkKind> {
    let value = el.value();
    Some(match value.name() {
        "strong" | "b" => MarkKind::Bold,
        "em" | "i" | "cite" | "dfn" | "var" => MarkKind::Italic,
        "u" | "ins" => MarkKind::Underline,
        "s" | "del" | "strike" => MarkKind::Strike,
        "code" | "kbd" | "samp" | "tt" => MarkKind::Code,
        "mark" => MarkKind::Highlight(None),
        "sup" => MarkKind::Superscript,
        "sub" => MarkKind::Subscript,
        "a" => MarkKind::Link(value.attr("href")?.to_string().into()),
        _ => return None,
    })
}

/// Elements whose content is never shown.
fn is_hidden(name: &str) -> bool {
    matches!(
        name,
        "head" | "script" | "style" | "template" | "noscript" | "title" | "meta" | "link"
            | "button" | "input" | "select" | "textarea" | "svg" | "canvas" | "iframe"
    )
}

/// Elements that start a block of their own.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "blockquote" | "pre"
            | "hr" | "table" | "img" | "figure" | "div" | "section" | "article" | "main"
            | "header" | "footer" | "nav" | "aside" | "details" | "summary" | "dl" | "dt" | "dd"
            | "address" | "form" | "fieldset" | "body" | "html"
    )
}

/// Gather the inline content of an element, skipping nested lists (which
/// are blocks of their own) and treating any other block as inline.
fn collect_inline(el: ElementRef, out: &mut Inline, active: &[MarkKind]) {
    for child in el.children() {
        match child.value() {
            Node::Text(text) => out.push_text(text, active),
            Node::Element(element) => {
                let name = element.name();
                if is_hidden(name) || name == "ul" || name == "ol" {
                    continue;
                }
                let Some(child) = ElementRef::wrap(child) else { continue };
                if name == "br" {
                    out.push_raw("\n", active);
                    continue;
                }
                if name == "img" {
                    if let Some(alt) = element.attr("alt") {
                        out.push_text(alt, active);
                    }
                    continue;
                }
                match inline_mark(&child) {
                    Some(mark) => {
                        let mut nested = active.to_vec();
                        nested.push(mark);
                        collect_inline(child, out, &nested);
                    }
                    None => {
                        // A paragraph inside a list item reads as a line of it.
                        if is_block(name) && !out.is_blank() && !out.text.ends_with('\n') {
                            out.push_raw(" ", active);
                        }
                        collect_inline(child, out, active);
                    }
                }
            }
            _ => {}
        }
    }
}

fn plain_text(el: ElementRef) -> String {
    let mut inline = Inline::default();
    collect_inline(el, &mut inline, &[]);
    inline.finish().0
}

#[derive(Default)]
struct Reader {
    blocks: Vec<BlockContent>,
    /// Loose inline content between blocks, which becomes a paragraph.
    pending: Inline,
}

impl Reader {
    fn flush(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if !pending.is_blank() {
            let (text, marks) = pending.finish();
            self.blocks.push(BlockContent::paragraph(text).with_marks(marks));
        }
    }

    fn push_inline_block(&mut self, el: ElementRef, ty: &str, attrs: BlockAttrs, indent: usize) {
        let mut inline = Inline::default();
        collect_inline(el, &mut inline, &[]);
        let (text, marks) = inline.finish();
        self.blocks.push(
            BlockContent::new(ty.to_string(), text)
                .with_attrs(attrs)
                .with_marks(marks)
                .with_indent(indent),
        );
    }

    fn walk(&mut self, el: ElementRef) {
        for child in el.children() {
            match child.value() {
                Node::Text(text) => self.pending.push_text(text, &[]),
                Node::Element(element) => {
                    let name = element.name();
                    if is_hidden(name) {
                        continue;
                    }
                    let Some(child) = ElementRef::wrap(child) else { continue };
                    if !is_block(name) {
                        // Inline content directly inside a container.
                        if name == "br" {
                            self.pending.push_raw("\n", &[]);
                        } else {
                            let mut active = Vec::new();
                            if let Some(mark) = inline_mark(&child) {
                                active.push(mark);
                            }
                            collect_inline(child, &mut self.pending, &active);
                        }
                        continue;
                    }
                    self.flush();
                    self.block(child, name);
                }
                _ => {}
            }
        }
    }

    fn block(&mut self, el: ElementRef, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<u8>().unwrap_or(1).min(3);
                self.push_inline_block(el, types::HEADING, BlockAttrs::level(level), 0);
            }
            "p" | "dt" | "dd" | "address" | "summary" => {
                self.push_inline_block(el, types::PARAGRAPH, BlockAttrs::default(), 0)
            }
            "ul" | "ol" => self.list(el, name == "ol", 0),
            "li" => self.list_item(el, false, 0),
            "blockquote" => {
                let ty = if el.value().attr("class").is_some_and(|c| c.contains("callout")) {
                    types::CALLOUT
                } else {
                    types::BLOCKQUOTE
                };
                self.push_inline_block(el, ty, BlockAttrs::default(), 0);
            }
            "aside" if el.value().attr("class").is_some_and(|c| c.contains("callout")) => {
                self.push_inline_block(el, types::CALLOUT, BlockAttrs::default(), 0)
            }
            "pre" => {
                let code = el
                    .children()
                    .filter_map(ElementRef::wrap)
                    .find(|child| child.value().name() == "code");
                let language = code
                    .and_then(|code| code.value().attr("class"))
                    .and_then(|class| {
                        class
                            .split_whitespace()
                            .find_map(|c| c.strip_prefix("language-").or(c.strip_prefix("lang-")))
                    })
                    .map(str::to_string);
                let text: String = el.text().collect();
                let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
                let attrs = language.map(BlockAttrs::language).unwrap_or_default();
                self.blocks.push(BlockContent::new(types::CODE_BLOCK, text).with_attrs(attrs));
            }
            "hr" => self.blocks.push(BlockContent::new(types::HORIZONTAL_RULE, "")),
            "img" => {
                let value = el.value();
                if let Some(src) = value.attr("src") {
                    let attrs = BlockAttrs {
                        src: Some(src.to_string().into()),
                        alt: Some(value.attr("alt").unwrap_or_default().to_string().into()),
                        ..Default::default()
                    };
                    self.blocks.push(BlockContent::new(types::IMAGE, "").with_attrs(attrs));
                }
            }
            "table" => self.table(el),
            "details" => {
                if let Some(summary) = el
                    .children()
                    .filter_map(ElementRef::wrap)
                    .find(|child| child.value().name() == "summary")
                {
                    self.push_inline_block(summary, types::TOGGLE, BlockAttrs::default(), 0);
                }
                for child in el.children().filter_map(ElementRef::wrap) {
                    if child.value().name() != "summary" {
                        let name = child.value().name().to_string();
                        if is_block(&name) {
                            self.block(child, &name);
                        }
                    }
                }
            }
            // Containers: look through them.
            _ => {
                self.walk(el);
                self.flush();
            }
        }
    }

    fn list(&mut self, el: ElementRef, ordered: bool, depth: usize) {
        for item in el.children().filter_map(ElementRef::wrap) {
            match item.value().name() {
                "li" => self.list_item(item, ordered, depth),
                "ul" | "ol" => {
                    let ordered = item.value().name() == "ol";
                    self.list(item, ordered, depth + 1);
                }
                _ => {}
            }
        }
    }

    fn list_item(&mut self, item: ElementRef, ordered: bool, depth: usize) {
        let checkbox = item.descendants().filter_map(ElementRef::wrap).find(|el| {
            el.value().name() == "input" && el.value().attr("type") == Some("checkbox")
        });
        let (ty, attrs) = match checkbox {
            Some(checkbox) => (
                types::TASK_LIST,
                BlockAttrs {
                    checked: checkbox.value().attr("checked").is_some(),
                    ..Default::default()
                },
            ),
            None if ordered => (types::ORDERED_LIST, BlockAttrs::default()),
            None => (types::BULLET_LIST, BlockAttrs::default()),
        };
        self.push_inline_block(item, ty, attrs, depth);
        for nested in item.children().filter_map(ElementRef::wrap) {
            let name = nested.value().name();
            if name == "ul" || name == "ol" {
                self.list(nested, name == "ol", depth + 1);
            }
        }
    }

    fn table(&mut self, el: ElementRef) {
        let rows: Vec<Vec<String>> = el
            .descendants()
            .filter_map(ElementRef::wrap)
            .filter(|row| row.value().name() == "tr")
            .map(|row| {
                row.children()
                    .filter_map(ElementRef::wrap)
                    .filter(|cell| matches!(cell.value().name(), "td" | "th"))
                    .map(plain_text)
                    .collect::<Vec<_>>()
            })
            .filter(|row| !row.is_empty())
            .collect();
        if rows.is_empty() {
            return;
        }
        let width = rows.iter().map(Vec::len).max().unwrap_or(1);
        let rows: Vec<Vec<&str>> = rows
            .iter()
            .map(|row| {
                let mut cells: Vec<&str> = row.iter().map(String::as_str).collect();
                cells.resize(width, "");
                cells
            })
            .collect();
        let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
        self.blocks.push(table_content(&rows));
    }
}

// ------------------------------------------------------------------ writing

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn open_tag(kind: &MarkKind) -> Option<String> {
    Some(match kind {
        MarkKind::Bold => "<strong>".into(),
        MarkKind::Italic => "<em>".into(),
        MarkKind::Underline => "<u>".into(),
        MarkKind::Strike => "<s>".into(),
        MarkKind::Code => "<code>".into(),
        MarkKind::Highlight(_) => "<mark>".into(),
        MarkKind::Superscript => "<sup>".into(),
        MarkKind::Subscript => "<sub>".into(),
        MarkKind::Link(href) => format!("<a href=\"{}\">", escape(href)),
        MarkKind::Mention(_) => "<span class=\"mention\">".into(),
        MarkKind::TextColor(_) | MarkKind::Comment(_) => return None,
    })
}

fn close_tag(kind: &MarkKind) -> Option<&'static str> {
    Some(match kind {
        MarkKind::Bold => "</strong>",
        MarkKind::Italic => "</em>",
        MarkKind::Underline => "</u>",
        MarkKind::Strike => "</s>",
        MarkKind::Code => "</code>",
        MarkKind::Highlight(_) => "</mark>",
        MarkKind::Superscript => "</sup>",
        MarkKind::Subscript => "</sub>",
        MarkKind::Link(_) => "</a>",
        MarkKind::Mention(_) => "</span>",
        MarkKind::TextColor(_) | MarkKind::Comment(_) => return None,
    })
}

/// Text with its marks as nested inline elements. Every stretch between two
/// mark boundaries is wrapped on its own, which keeps the nesting valid.
fn inline_html(text: &str, marks: &MarkList) -> String {
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for mark in marks.iter() {
        cuts.push(mark.range.start.min(text.len()));
        cuts.push(mark.range.end.min(text.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = String::new();
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        let kinds: Vec<&MarkKind> = marks
            .iter()
            .filter(|mark| mark.range.start <= start && mark.range.end >= end)
            .map(|mark| &mark.kind)
            .collect();
        for kind in &kinds {
            if let Some(tag) = open_tag(kind) {
                out.push_str(&tag);
            }
        }
        out.push_str(&escape(&text[start..end]).replace('\n', "<br>"));
        for kind in kinds.iter().rev() {
            if let Some(tag) = close_tag(kind) {
                out.push_str(tag);
            }
        }
    }
    out
}

impl RifflyEditor {
    /// The document as a standalone HTML page.
    /// (`App` is taken to match `to_markdown`, for callers that pick one.)
    pub fn to_html(&self, _cx: &App) -> String {
        let title = self
            .outline()
            .into_iter()
            .map(|item| item.text)
            .find(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "Document".to_string());

        let mut body = String::new();
        // Open lists, outermost first; each level keeps its last <li> open
        // so that a deeper list nests inside it.
        let mut lists: Vec<&'static str> = Vec::new();
        let close_lists = |lists: &mut Vec<&'static str>, keep: usize, body: &mut String| {
            while lists.len() > keep {
                let tag = lists.pop().unwrap();
                let _ = writeln!(body, "</li></{tag}>");
            }
        };

        for block in &self.blocks {
            let ty = block.ty.as_ref();
            let list_tag = match ty {
                types::BULLET_LIST | types::TASK_LIST => Some("ul"),
                types::ORDERED_LIST => Some("ol"),
                _ => None,
            };
            let inline = inline_html(&block.text, &block.marks);

            if let Some(tag) = list_tag {
                let depth = block.indent;
                close_lists(&mut lists, depth + 1, &mut body);
                if lists.len() == depth + 1 {
                    if lists[depth] == tag {
                        body.push_str("</li>\n");
                    } else {
                        close_lists(&mut lists, depth, &mut body);
                    }
                }
                while lists.len() < depth + 1 {
                    let _ = writeln!(body, "<{tag}>");
                    lists.push(tag);
                }
                if ty == types::TASK_LIST {
                    let checked = if block.attrs.checked { " checked" } else { "" };
                    let _ = write!(body, "<li><input type=\"checkbox\" disabled{checked}> {inline}");
                } else {
                    let _ = write!(body, "<li>{inline}");
                }
                continue;
            }
            close_lists(&mut lists, 0, &mut body);

            match ty {
                types::HEADING => {
                    let level = block.attrs.level.clamp(1, 6);
                    let _ = writeln!(body, "<h{level}>{inline}</h{level}>");
                }
                types::PARAGRAPH if block.text.is_empty() => {}
                types::PARAGRAPH => {
                    let _ = writeln!(body, "<p>{inline}</p>");
                }
                types::BLOCKQUOTE => {
                    let _ = writeln!(body, "<blockquote><p>{inline}</p></blockquote>");
                }
                types::CALLOUT => {
                    let _ = writeln!(body, "<aside class=\"callout\"><p>{inline}</p></aside>");
                }
                types::TOGGLE => {
                    let _ = writeln!(body, "<details><summary>{inline}</summary></details>");
                }
                types::CODE_BLOCK => {
                    let class = block
                        .attrs
                        .language
                        .as_ref()
                        .map(|language| format!(" class=\"language-{}\"", escape(language)))
                        .unwrap_or_default();
                    let _ = writeln!(body, "<pre><code{class}>{}</code></pre>", escape(&block.text));
                }
                types::HORIZONTAL_RULE => body.push_str("<hr>\n"),
                types::IMAGE => {
                    let src = block.attrs.src.clone().unwrap_or_default();
                    let alt = block.attrs.alt.clone().unwrap_or_default();
                    let _ = writeln!(body, "<img src=\"{}\" alt=\"{}\">", escape(&src), escape(&alt));
                }
                types::TABLE => {
                    if let Some(grid) = self.grid(block.id) {
                        body.push_str("<table>\n");
                        for row in 0..grid.rows() {
                            let cell = if row == 0 { "th" } else { "td" };
                            body.push_str("<tr>");
                            for item in grid.row(row).unwrap_or_default() {
                                let _ = write!(body, "<{cell}>{}</{cell}>", escape(item.text()));
                            }
                            body.push_str("</tr>\n");
                        }
                        body.push_str("</table>\n");
                    }
                }
                _ => {
                    let _ = writeln!(body, "<p>{inline}</p>");
                }
            }
        }
        close_lists(&mut lists, 0, &mut body);

        format!(
            "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>{}</title>\n</head>\n<body>\n{body}</body>\n</html>\n",
            escape(&title)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(blocks: &[BlockContent]) -> Vec<(String, String, usize)> {
        blocks
            .iter()
            .map(|b| (b.ty.to_string(), b.text.clone(), b.indent))
            .collect()
    }

    #[test]
    fn reads_structure_through_containers() {
        let html = r#"<!doctype html><html><head><title>t</title><style>p{}</style></head>
        <body><div class="wrap"><h1>Title</h1>
        <p>Some <strong>bold</strong> and <a href="https://x.dev">a link</a>.</p>
        <ul><li>one<ul><li>nested</li></ul></li><li><input type="checkbox" checked> done</li></ul>
        <ol><li>first</li></ol>
        <blockquote><p>quoted</p></blockquote>
        <pre><code class="language-rust">fn main() {}
</code></pre>
        <hr><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>
        <img src="cat.png" alt="cat"><script>alert(1)</script>
        loose text</div></body></html>"#;
        let blocks = parse(html);
        assert_eq!(
            kinds(&blocks),
            vec![
                ("heading".into(), "Title".into(), 0),
                ("paragraph".into(), "Some bold and a link.".into(), 0),
                ("bulletList".into(), "one".into(), 0),
                ("bulletList".into(), "nested".into(), 1),
                ("taskList".into(), "done".into(), 0),
                ("orderedList".into(), "first".into(), 0),
                ("blockquote".into(), "quoted".into(), 0),
                ("codeBlock".into(), "fn main() {}".into(), 0),
                ("horizontalRule".into(), "".into(), 0),
                ("table".into(), "".into(), 0),
                ("image".into(), "".into(), 0),
                ("paragraph".into(), "loose text".into(), 0),
            ]
        );
        assert!(blocks[4].attrs.checked);
        assert_eq!(blocks[7].attrs.language.as_deref(), Some("rust"));

        let para = &blocks[1];
        let bold = 5..9;
        assert!(para.marks.has(&MarkKind::Bold, &bold));
        let link = para.text.find("a link").unwrap();
        assert!(para.marks.has(&MarkKind::Link("https://x.dev".into()), &(link..link + 6)));
    }

    #[test]
    fn collapses_white_space_like_a_browser() {
        let blocks = parse("<p>  many \n   spaces <em> here </em> </p>");
        assert_eq!(blocks[0].text, "many spaces here");
        assert!(blocks[0].marks.has(&MarkKind::Italic, &(12..16)));
    }

    #[test]
    fn writes_nested_inline_html() {
        let marks = MarkList::from_marks(vec![
            Mark::new(MarkKind::Bold, 0..4),
            Mark::new(MarkKind::Italic, 2..6),
        ]);
        assert_eq!(
            inline_html("ab<d e", &marks),
            "<strong>ab</strong><strong><em>&lt;d</em></strong><em> e</em>"
        );
    }
}
