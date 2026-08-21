//! Readability-style extraction of fetched HTML into Markdown, per
//! `docs/ARCHITECTURE.md` ("Content extraction"). Outgoing hyperlinks found
//! in a page's content are preserved as Markdown links, not stripped, and
//! are also collected separately so a caller can decide whether to archive
//! one as its own Document later — one level deep, on request only. This
//! module never follows a link itself.

use std::fmt::Write as _;

use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

/// The result of extracting a fetched page's main content to Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Extracted {
    /// The page's title, from its `<title>` element, if present.
    pub title: Option<String>,
    /// The extracted content as Markdown, with outgoing hyperlinks
    /// preserved inline as `[text](url)`.
    pub markdown: String,
    /// Every outgoing hyperlink discovered in the content whose target is
    /// itself an archivable page (`http`/`https`), in the order they
    /// appear. Returned as data only — a caller can archive one as its own
    /// Document later, one level deep, never automatically (see
    /// `docs/ARCHITECTURE.md`, "Content extraction").
    pub outgoing_links: Vec<OutgoingLink>,
}

/// One hyperlink discovered in a page's content during extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingLink {
    /// The link's visible text.
    pub text: String,
    /// The link's target, resolved to an absolute URL against the page it
    /// was found on.
    pub url: String,
}

/// Element names dropped wherever they appear in the content, along with
/// all of their descendants — boilerplate a readability pass discards
/// rather than rendering literally.
const DROPPED_ELEMENTS: &[&str] = &[
    "script", "style", "nav", "header", "footer", "aside", "noscript", "form",
];

/// Extracts `html` (fetched from `page_url`) to Markdown.
///
/// Picks the main content root as the first of `<article>`, `<main>`, or
/// `<body>` present in the document, then walks it converting headings,
/// paragraphs, emphasis, and lists to Markdown while preserving hyperlinks
/// inline and collecting them into `outgoing_links`. `page_url` is used
/// only to resolve relative hrefs to absolute URLs; it is not fetched by
/// this function (see [`crate::fetch`] for that).
#[must_use]
pub fn extract(html: &str, page_url: &str) -> Extracted {
    let document = Html::parse_document(html);
    let base = Url::parse(page_url).ok();
    let title = title_text(&document);

    let mut markdown = String::new();
    let mut outgoing_links = Vec::new();
    if let Some(root) = main_content_root(&document) {
        for child in root.children() {
            render_node(child, base.as_ref(), &mut markdown, &mut outgoing_links);
        }
    }

    Extracted {
        title,
        markdown: normalize(&markdown),
        outgoing_links,
    }
}

/// Extracts the trimmed text of the document's `<title>` element, if any
/// and non-empty.
fn title_text(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").expect("\"title\" is a valid CSS selector");
    let text: String = document.select(&selector).next()?.text().collect();
    let trimmed = collapse_whitespace(&text);
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Picks the main content root: the first of `<article>`, `<main>`, or
/// `<body>` present in `document`.
fn main_content_root(document: &Html) -> Option<ElementRef<'_>> {
    ["article", "main", "body"].into_iter().find_map(|tag| {
        let selector = Selector::parse(tag).expect("tag name is a valid CSS selector");
        document.select(&selector).next()
    })
}

/// Renders one DOM node (and, for elements, its descendants) into `out` as
/// Markdown, collecting any outgoing hyperlinks found along the way.
fn render_node(
    node: ego_tree::NodeRef<'_, Node>,
    base: Option<&Url>,
    out: &mut String,
    links: &mut Vec<OutgoingLink>,
) {
    match node.value() {
        Node::Text(text) => out.push_str(text),
        Node::Element(_) => {
            let Some(element_ref) = ElementRef::wrap(node) else {
                return;
            };
            render_element(element_ref, base, out, links);
        }
        // Comments, doctypes, and processing instructions carry nothing
        // readability-relevant.
        _ => {}
    }
}

/// Renders every child of `node` in order. Used both for elements that are
/// transparent containers (e.g. `<div>`) and as the building block inline
/// renderers use to collect an element's text content.
fn render_children(
    node: ego_tree::NodeRef<'_, Node>,
    base: Option<&Url>,
    out: &mut String,
    links: &mut Vec<OutgoingLink>,
) {
    for child in node.children() {
        render_node(child, base, out, links);
    }
}

/// Renders `node`'s children into a fresh string, trimmed of surrounding
/// whitespace. Used to gather the text of an inline element (a link's
/// label, emphasis content) before deciding how to wrap it.
fn render_children_to_string(
    node: ElementRef<'_>,
    base: Option<&Url>,
    links: &mut Vec<OutgoingLink>,
) -> String {
    let mut inner = String::new();
    render_children(*node, base, &mut inner, links);
    collapse_whitespace(&inner)
}

/// Ensures `out` has a blank line separating whatever came before from the
/// block-level content about to be appended. A no-op at the very start of
/// the document.
fn start_block(out: &mut String) {
    if !out.is_empty() {
        out.push_str("\n\n");
    }
}

/// Dispatches on `element`'s tag name to render it (and its descendants)
/// as Markdown into `out`.
fn render_element(
    element: ElementRef<'_>,
    base: Option<&Url>,
    out: &mut String,
    links: &mut Vec<OutgoingLink>,
) {
    let name = element.value().name();

    if DROPPED_ELEMENTS.contains(&name) {
        return;
    }

    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            // Safe to unwrap: matched against the literal set "h1".."h6".
            let level: usize = name[1..].parse().unwrap_or(1);
            start_block(out);
            out.push_str(&"#".repeat(level));
            out.push(' ');
            render_children(*element, base, out, links);
        }
        "p" => {
            start_block(out);
            render_children(*element, base, out, links);
        }
        "br" => out.push('\n'),
        "hr" => {
            start_block(out);
            out.push_str("---");
        }
        "strong" | "b" => wrap_inline(element, base, "**", out, links),
        "em" | "i" => wrap_inline(element, base, "*", out, links),
        "code" => wrap_inline(element, base, "`", out, links),
        "ul" => {
            start_block(out);
            render_list(element, base, false, out, links);
        }
        "ol" => {
            start_block(out);
            render_list(element, base, true, out, links);
        }
        "a" => render_link(element, base, out, links),
        // Image bytes and local rewriting are a later roadmap task
        // ("image handling during extraction"); this pass just drops them.
        "img" => {}
        // Transparent containers (div, span, section, article, main, body,
        // blockquote, figure, table cells, ...): render their content
        // without adding markup of their own.
        _ => render_children(*element, base, out, links),
    }
}

/// Renders `element`'s children, then wraps the trimmed result in
/// `marker` on both sides (e.g. `**bold**`). Emits nothing if the content
/// is empty, so an empty `<strong></strong>` doesn't leave behind a stray
/// `****`.
fn wrap_inline(
    element: ElementRef<'_>,
    base: Option<&Url>,
    marker: &str,
    out: &mut String,
    links: &mut Vec<OutgoingLink>,
) {
    let inner = render_children_to_string(element, base, links);
    if inner.is_empty() {
        return;
    }
    out.push_str(marker);
    out.push_str(&inner);
    out.push_str(marker);
}

/// Renders an `<a>` element as a Markdown link, resolving its `href`
/// against `base`. Falls back to plain text if there's no `href` or it
/// can't be resolved to a URL. Also records the link in `links`, but only
/// when its target is an archivable `http`/`https` page — a `mailto:` or
/// `javascript:` href is still rendered inline, but isn't a page that could
/// be saved as its own Document.
fn render_link(
    element: ElementRef<'_>,
    base: Option<&Url>,
    out: &mut String,
    links: &mut Vec<OutgoingLink>,
) {
    let text = render_children_to_string(element, base, links);
    let resolved = element
        .value()
        .attr("href")
        .and_then(|href| resolve_url(base, href));

    let Some(url) = resolved else {
        out.push_str(&text);
        return;
    };

    let display_text = if text.is_empty() { url.clone() } else { text };
    if url.starts_with("http://") || url.starts_with("https://") {
        links.push(OutgoingLink {
            text: display_text.clone(),
            url: url.clone(),
        });
    }

    out.push('[');
    out.push_str(&display_text);
    out.push_str("](");
    out.push_str(&url);
    out.push(')');
}

/// Resolves `href` to an absolute URL string against `base`, if possible.
fn resolve_url(base: Option<&Url>, href: &str) -> Option<String> {
    let resolved = match base {
        Some(base) => base.join(href).ok()?,
        None => Url::parse(href).ok()?,
    };
    Some(resolved.to_string())
}

/// Renders a `<ul>`/`<ol>`'s direct `<li>` children as Markdown list items.
fn render_list(
    element: ElementRef<'_>,
    base: Option<&Url>,
    ordered: bool,
    out: &mut String,
    links: &mut Vec<OutgoingLink>,
) {
    let mut number = 1u32;
    for child in element.child_elements() {
        if child.value().name() != "li" {
            continue;
        }
        let item = render_children_to_string(child, base, links);
        if ordered {
            let _ = writeln!(out, "{number}. {item}");
            number += 1;
        } else {
            out.push_str("- ");
            out.push_str(&item);
            out.push('\n');
        }
    }
    // Drop the trailing newline after the last item — `start_block` adds
    // the blank-line separation before whatever comes next.
    if out.ends_with('\n') {
        out.pop();
    }
}

/// Collapses every run of whitespace (including newlines, as a browser
/// would when rendering HTML) to a single space, and trims the ends.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapses each line's internal whitespace, then squeezes any run of
/// blank lines down to exactly one, and trims the result. The rendering
/// pass above only ever inserts significant newlines at block boundaries
/// (via [`start_block`] and list rendering), so a "line" at this point is
/// always one paragraph/heading/list item's worth of inline content.
fn normalize(markdown: &str) -> String {
    let mut squeezed = String::with_capacity(markdown.len());
    let mut blank_run = 0u32;
    for line in markdown.split('\n') {
        let collapsed = collapse_whitespace(line);
        if collapsed.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                squeezed.push('\n');
            }
        } else {
            blank_run = 0;
            squeezed.push_str(&collapsed);
            squeezed.push('\n');
        }
    }
    squeezed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_converts_headings_and_paragraphs_to_markdown() {
        let html = "<html><body><article>\
            <h1>Title</h1>\
            <p>Some <strong>bold</strong> and <em>italic</em> text.</p>\
            </article></body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(
            extracted.markdown,
            "# Title\n\nSome **bold** and *italic* text."
        );
    }

    #[test]
    fn extract_reads_the_title_from_the_title_tag() {
        let html = "<html><head><title> My Page </title></head><body><article><p>Hi.</p></article></body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(extracted.title, Some("My Page".to_string()));
    }

    #[test]
    fn extract_preserves_outgoing_hyperlinks_inline_as_markdown_links() {
        let html = "<html><body><article><p>See \
            <a href=\"https://www.rust-lang.org/\">Rust</a> for more.</p></article></body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(
            extracted.markdown,
            "See [Rust](https://www.rust-lang.org/) for more."
        );
    }

    #[test]
    fn extract_resolves_relative_links_against_the_page_url() {
        let html =
            "<html><body><article><p><a href=\"/about\">About</a></p></article></body></html>";

        let extracted = extract(html, "https://example.com/blog/post");

        assert_eq!(extracted.markdown, "[About](https://example.com/about)");
    }

    #[test]
    fn extract_collects_outgoing_links_as_data_in_document_order() {
        let html = "<html><body><article>\
            <p><a href=\"https://one.example/\">One</a></p>\
            <p><a href=\"https://two.example/\">Two</a></p>\
            </article></body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(
            extracted.outgoing_links,
            vec![
                OutgoingLink {
                    text: "One".to_string(),
                    url: "https://one.example/".to_string(),
                },
                OutgoingLink {
                    text: "Two".to_string(),
                    url: "https://two.example/".to_string(),
                },
            ]
        );
    }

    #[test]
    fn extract_excludes_non_http_link_targets_from_outgoing_links() {
        let html = "<html><body><article>\
            <p><a href=\"mailto:someone@example.com\">Email</a></p>\
            <p><a href=\"https://real.example/\">Real link</a></p>\
            </article></body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(
            extracted.outgoing_links,
            vec![OutgoingLink {
                text: "Real link".to_string(),
                url: "https://real.example/".to_string(),
            }],
            "a mailto: link isn't an archivable page, so it should not be reported as an \
             outgoing link, even though it's still rendered inline as a Markdown link"
        );
        assert!(extracted.markdown.contains("[Email](mailto:someone@example.com)"));
    }

    #[test]
    fn extract_drops_boilerplate_elements_wherever_they_appear() {
        let html = "<html><body>\
            <nav>Home | About</nav>\
            <article>\
                <script>trackPageView();</script>\
                <p>Real content.</p>\
            </article>\
            <footer>Copyright 2026</footer>\
            </body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(extracted.markdown, "Real content.");
    }

    #[test]
    fn extract_prefers_article_over_surrounding_page_chrome() {
        let html = "<html><body>\
            <header><p>Site Header</p></header>\
            <article><p>Article body.</p></article>\
            <aside><p>Sidebar</p></aside>\
            </body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(extracted.markdown, "Article body.");
    }

    #[test]
    fn extract_falls_back_to_main_then_body_when_no_article_is_present() {
        let with_main = extract(
            "<html><body><header><p>Header</p></header><main><p>Main content.</p></main></body></html>",
            "https://example.com/post",
        );
        assert_eq!(with_main.markdown, "Main content.");

        let with_only_body =
            extract("<html><body><p>Just body content.</p></body></html>", "https://example.com/post");
        assert_eq!(with_only_body.markdown, "Just body content.");
    }

    #[test]
    fn extract_renders_unordered_and_ordered_lists_as_markdown() {
        let html = "<html><body><article>\
            <ul><li>First</li><li>Second</li></ul>\
            <ol><li>Step one</li><li>Step two</li></ol>\
            </article></body></html>";

        let extracted = extract(html, "https://example.com/post");

        assert_eq!(
            extracted.markdown,
            "- First\n- Second\n\n1. Step one\n2. Step two"
        );
    }
}
