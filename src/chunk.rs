//! Splits a Document's Markdown content into paragraph/section-sized
//! chunks ahead of embedding, per `docs/ARCHITECTURE.md` ("Retrieval"):
//! "Documents are split into chunks (paragraph/section-sized) before
//! embedding — not embedded as one whole-document vector. Better recall
//! against long content."

/// Splits `content` into chunks at blank-line paragraph boundaries and at
/// Markdown ATX heading lines (`#` through `######`).
///
/// A heading always starts a new chunk, so a heading and the paragraph
/// immediately following it (no blank line between them) are kept
/// together as one section-sized chunk; a heading followed by a blank
/// line before its first paragraph stands alone. Later paragraphs under
/// the same heading, separated by blank lines, each become their own
/// chunk. Runs of multiple blank lines collapse to a single boundary, and
/// leading/trailing blank lines produce no empty chunks.
#[must_use]
pub fn chunk(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            flush(&mut current, &mut chunks);
        } else if is_heading(line) && !current.is_empty() {
            flush(&mut current, &mut chunks);
            current.push(line);
        } else {
            current.push(line);
        }
    }
    flush(&mut current, &mut chunks);

    chunks
}

/// Pushes `current`'s joined, trimmed text onto `chunks` if it holds
/// anything, then clears it — the shared "end of chunk" step for both
/// blank-line and heading boundaries in [`chunk`].
fn flush(current: &mut Vec<&str>, chunks: &mut Vec<String>) {
    if !current.is_empty() {
        chunks.push(current.join("\n").trim().to_string());
        current.clear();
    }
}

/// True when `line` is a Markdown ATX heading: one to six `#` characters,
/// optionally indented up to three spaces, followed by a space.
fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hash_count) && trimmed.as_bytes().get(hash_count) == Some(&b' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_splits_paragraphs_separated_by_a_blank_line() {
        let content = "First paragraph.\n\nSecond paragraph.";

        assert_eq!(
            chunk(content),
            vec!["First paragraph.".to_string(), "Second paragraph.".to_string()]
        );
    }

    #[test]
    fn chunk_collapses_runs_of_multiple_blank_lines_to_one_boundary() {
        let content = "First paragraph.\n\n\n\nSecond paragraph.";

        assert_eq!(
            chunk(content),
            vec!["First paragraph.".to_string(), "Second paragraph.".to_string()]
        );
    }

    #[test]
    fn chunk_drops_leading_and_trailing_blank_lines_without_empty_chunks() {
        let content = "\n\nOnly paragraph.\n\n\n";

        assert_eq!(chunk(content), vec!["Only paragraph.".to_string()]);
    }

    #[test]
    fn chunk_returns_nothing_for_empty_or_blank_content() {
        assert_eq!(chunk(""), Vec::<String>::new());
        assert_eq!(chunk("\n\n   \n\n"), Vec::<String>::new());
    }

    #[test]
    fn chunk_keeps_a_multiline_paragraph_without_blank_lines_together() {
        let content = "Line one\nline two\nline three";

        assert_eq!(
            chunk(content),
            vec!["Line one\nline two\nline three".to_string()]
        );
    }

    #[test]
    fn chunk_starts_a_new_chunk_at_a_heading_even_without_a_blank_line_before_it() {
        let content = "Intro paragraph.\n# Section One\nSection body.";

        assert_eq!(
            chunk(content),
            vec![
                "Intro paragraph.".to_string(),
                "# Section One\nSection body.".to_string(),
            ]
        );
    }

    #[test]
    fn chunk_treats_a_heading_followed_by_a_blank_line_as_its_own_chunk() {
        let content = "# Section One\n\nFirst paragraph under it.\n\nSecond paragraph under it.";

        assert_eq!(
            chunk(content),
            vec![
                "# Section One".to_string(),
                "First paragraph under it.".to_string(),
                "Second paragraph under it.".to_string(),
            ]
        );
    }

    #[test]
    fn chunk_does_not_treat_a_hash_without_a_following_space_as_a_heading() {
        let content = "Intro paragraph.\n#hashtag-not-a-heading\nMore of the same paragraph.";

        assert_eq!(
            chunk(content),
            vec!["Intro paragraph.\n#hashtag-not-a-heading\nMore of the same paragraph.".to_string()]
        );
    }
}
