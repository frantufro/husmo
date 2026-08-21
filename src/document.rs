//! The Document model: the one unified concept for anything saved into
//! husmo — a URL fetch, pasted/typed text, or an ingested local file. See
//! `docs/ARCHITECTURE.md` ("Storage model") and `CONTEXT.md`.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A saved unit of content.
///
/// Every Document is serialized as a Markdown file with YAML frontmatter
/// (see [`Document::to_markdown`] / [`Document::from_markdown`]), committed
/// to the data repo. There is no separate "Link" type — a saved web page is
/// just a Document whose `canonical_url` is set.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Document {
    /// Stable internal identifier, set once at creation. Never changes,
    /// even if `title` (and therefore `slug`) changes later.
    pub id: String,
    /// Filesystem-safe, human-browsable name derived from `title`. Not
    /// guaranteed unique by itself — see `dedupe_slug` for collision
    /// handling against a directory's already-taken slugs.
    pub slug: String,
    /// The URL this Document was fetched from, if any. Pasted/typed
    /// content has no canonical URL. At most one Document may share a
    /// given canonical URL — re-saving overwrites that Document in place.
    pub canonical_url: Option<String>,
    /// The Document's title.
    pub title: String,
    /// Free-form labels for organizing and filtering.
    pub tags: Vec<String>,
    /// When the Document was saved.
    pub saved_at: DateTime<Utc>,
    /// An optional short summary.
    pub summary: Option<String>,
    /// An optional author.
    pub author: Option<String>,
    /// The Document's Markdown body.
    pub content: String,
    /// Ids of other Documents this one is deliberately, symmetrically
    /// Related to. Distinct from outgoing hyperlinks discovered in
    /// `content` — see `CONTEXT.md`.
    pub related: Vec<String>,
}

impl Document {
    /// Creates a new Document with a freshly generated stable id and a slug
    /// derived from `title`. All optional fields start empty; `saved_at` is
    /// set to now.
    #[must_use]
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        let title = title.into();
        let slug = slug_from_title(&title);
        Document {
            id: uuid::Uuid::new_v4().to_string(),
            slug,
            canonical_url: None,
            title,
            tags: Vec::new(),
            saved_at: Utc::now(),
            summary: None,
            author: None,
            content: content.into(),
            related: Vec::new(),
        }
    }

    /// Serializes this Document to a Markdown file's contents: a YAML
    /// frontmatter block followed by `content`.
    ///
    /// # Panics
    ///
    /// Panics if the frontmatter fails to serialize to YAML, which isn't
    /// expected to happen for any value a `Document`'s fields can hold.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let frontmatter = Frontmatter {
            id: self.id.clone(),
            slug: self.slug.clone(),
            canonical_url: self.canonical_url.clone(),
            title: self.title.clone(),
            tags: self.tags.clone(),
            saved_at: self.saved_at,
            summary: self.summary.clone(),
            author: self.author.clone(),
            related: self.related.clone(),
        };
        let yaml =
            serde_norway::to_string(&frontmatter).expect("Frontmatter always serializes to YAML");
        format!("---\n{yaml}---\n\n{}", self.content)
    }

    /// Parses a Markdown file's contents (frontmatter + body) back into a
    /// Document.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentParseError`] if the frontmatter delimiters are
    /// missing or the frontmatter is not valid YAML matching the expected
    /// shape.
    pub fn from_markdown(input: &str) -> Result<Document, DocumentParseError> {
        let after_open = input
            .strip_prefix("---\n")
            .ok_or(DocumentParseError::MissingOpeningDelimiter)?;
        let (yaml, content) = after_open
            .split_once("\n---\n")
            .ok_or(DocumentParseError::MissingClosingDelimiter)?;
        let frontmatter: Frontmatter =
            serde_norway::from_str(yaml).map_err(DocumentParseError::Yaml)?;
        Ok(Document {
            id: frontmatter.id,
            slug: frontmatter.slug,
            canonical_url: frontmatter.canonical_url,
            title: frontmatter.title,
            tags: frontmatter.tags,
            saved_at: frontmatter.saved_at,
            summary: frontmatter.summary,
            author: frontmatter.author,
            content: content.strip_prefix('\n').unwrap_or(content).to_string(),
            related: frontmatter.related,
        })
    }
}

/// The frontmatter fields of a Document, as they appear in the YAML block —
/// everything except `content`, which is the Markdown body after it.
#[derive(Debug, Serialize, Deserialize)]
struct Frontmatter {
    id: String,
    slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_url: Option<String>,
    title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    saved_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    related: Vec<String>,
}

/// An error encountered while parsing a Document from Markdown.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DocumentParseError {
    /// The input doesn't start with the opening `---` frontmatter
    /// delimiter.
    #[error("document is missing the opening \"---\" frontmatter delimiter")]
    MissingOpeningDelimiter,
    /// The opening delimiter was found but no closing `---` follows it.
    #[error("document is missing the closing \"---\" frontmatter delimiter")]
    MissingClosingDelimiter,
    /// The frontmatter block isn't valid YAML matching the expected shape.
    #[error("document frontmatter is not valid YAML")]
    Yaml(#[source] serde_norway::Error),
}

/// Derives a filesystem-safe, human-browsable slug from a title: lowercased,
/// with runs of non-alphanumeric characters collapsed to a single `-`, and
/// leading/trailing `-` trimmed. Falls back to `"untitled"` if that leaves
/// nothing.
#[must_use]
pub fn slug_from_title(title: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for ch in title.chars() {
        if ch.is_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            pending_separator = false;
            slug.extend(ch.to_lowercase());
        } else {
            pending_separator = true;
        }
    }
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}

/// Returns a slug guaranteed not to collide with any of `existing`. If
/// `base` itself is free it's returned unchanged; otherwise `-2`, `-3`, ...
/// is appended until a free slug is found.
#[must_use]
pub fn dedupe_slug<S: std::hash::BuildHasher>(base: &str, existing: &HashSet<String, S>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    let mut counter = 2u64;
    loop {
        let candidate = format!("{base}-{counter}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_round_trips_through_a_document() {
        let mut original = Document::new("My Title", "# Heading\n\nSome body text.\n");
        original.canonical_url = Some("https://example.com/post".to_string());
        original.tags = vec!["rust".to_string(), "notes".to_string()];
        original.summary = Some("A short summary.".to_string());
        original.author = Some("Fran".to_string());
        original.related = vec!["some-other-id".to_string()];

        let markdown = original.to_markdown();
        let parsed = Document::from_markdown(&markdown).expect("markdown should parse back");

        assert_eq!(parsed, original);
    }

    #[test]
    fn slug_from_title_lowercases_and_hyphenates() {
        assert_eq!(slug_from_title("My Great Title!"), "my-great-title");
    }

    #[test]
    fn slug_from_title_collapses_runs_of_punctuation_and_trims_ends() {
        assert_eq!(slug_from_title("  --Hello,   World--  "), "hello-world");
    }

    #[test]
    fn slug_from_title_falls_back_to_untitled_when_nothing_alphanumeric_remains() {
        assert_eq!(slug_from_title("!!!"), "untitled");
    }

    #[test]
    fn from_markdown_rejects_missing_opening_delimiter() {
        let result = Document::from_markdown("no frontmatter here");

        assert!(matches!(
            result,
            Err(DocumentParseError::MissingOpeningDelimiter)
        ));
    }

    #[test]
    fn from_markdown_rejects_missing_closing_delimiter() {
        let result = Document::from_markdown("---\nid: abc\n");

        assert!(matches!(
            result,
            Err(DocumentParseError::MissingClosingDelimiter)
        ));
    }

    #[test]
    fn dedupe_slug_returns_base_when_it_is_free() {
        let existing = HashSet::new();

        assert_eq!(dedupe_slug("my-title", &existing), "my-title");
    }

    #[test]
    fn dedupe_slug_appends_a_counter_on_collision() {
        let existing: HashSet<String> = ["my-title", "my-title-2"]
            .into_iter()
            .map(str::to_string)
            .collect();

        assert_eq!(dedupe_slug("my-title", &existing), "my-title-3");
    }
}
