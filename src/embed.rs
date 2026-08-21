//! Local, in-process embedding generation — zero network calls, per
//! `docs/adr/0001-local-first-no-external-services.md`: husmo trades
//! embedding quality for privacy, offline use, and operational simplicity.
//!
//! Rather than loading a downloaded neural model (which would mean
//! fetching and storing model weights before the first save could ever
//! run), this computes a deterministic feature-hashing embedding: each
//! token in a chunk's text is hashed into one of [`EMBEDDING_DIM`] slots
//! with a sign, the slots are accumulated into a bag-of-words vector, and
//! the result is L2-normalized. It is small, dependency-free, and fully
//! in-process, and its cosine similarity still reflects shared vocabulary
//! well enough for the semantic-search flavor of retrieval described in
//! `docs/ARCHITECTURE.md` ("Retrieval").

/// Dimensionality of every embedding vector [`embed`] produces.
pub const EMBEDDING_DIM: usize = 256;

/// Computes a deterministic, fixed-[`EMBEDDING_DIM`]-dimension embedding
/// vector for `text`.
///
/// The same `text` always produces the same vector (no randomness, no
/// network calls, no external state). Text sharing more vocabulary with
/// another text tends to produce a higher cosine similarity between their
/// vectors. Text with no tokens at all (empty or entirely punctuation)
/// embeds to the zero vector.
///
/// # Panics
///
/// Never panics in practice: the only fallible conversion inside is a
/// remainder of a division by [`EMBEDDING_DIM`], which always fits in a
/// `usize` on any platform this crate supports.
#[must_use]
pub fn embed(text: &str) -> Vec<f32> {
    let mut vector = vec![0f32; EMBEDDING_DIM];

    for token in tokenize(text) {
        let hash = fnv1a_64(token.as_bytes());
        let slot = usize::try_from(hash % EMBEDDING_DIM as u64)
            .expect("remainder of a division by EMBEDDING_DIM always fits in a usize");
        let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
        vector[slot] += sign;
    }

    normalize(&mut vector);
    vector
}

/// Splits `text` into lowercased alphanumeric tokens, discarding any run
/// of non-alphanumeric characters as a separator.
fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
}

/// The 64-bit FNV-1a hash of `bytes` — a small, fast, deterministic,
/// non-cryptographic hash, used here only to spread tokens across
/// [`EMBEDDING_DIM`] slots (not for security).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Scales `vector` to unit length in place. Leaves an all-zero vector
/// untouched — there's no direction to normalize toward.
fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vector.iter_mut() {
            *v /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_is_deterministic_for_the_same_text() {
        let text = "Local, in-process embeddings with zero network calls.";

        assert_eq!(embed(text), embed(text));
    }

    #[test]
    fn embed_produces_a_vector_of_the_fixed_dimension() {
        for text in ["", "one word", "a longer sentence with several words in it"] {
            assert_eq!(embed(text).len(), EMBEDDING_DIM);
        }
    }

    #[test]
    fn embed_of_text_with_no_tokens_is_the_zero_vector() {
        assert_eq!(embed(""), vec![0f32; EMBEDDING_DIM]);
        assert_eq!(embed("!!! ... ---"), vec![0f32; EMBEDDING_DIM]);
    }

    #[test]
    fn embed_is_case_insensitive() {
        assert_eq!(embed("Rust Embeddings"), embed("rust embeddings"));
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn embed_makes_texts_sharing_vocabulary_more_similar_than_unrelated_texts() {
        let rust_one = embed("Rust is a systems programming language with strong typing.");
        let rust_two = embed("Systems programming in Rust favors strong static typing.");
        let unrelated = embed("Bake the sourdough loaf for forty minutes at high heat.");

        let related_similarity = cosine_similarity(&rust_one, &rust_two);
        let unrelated_similarity = cosine_similarity(&rust_one, &unrelated);

        assert!(
            related_similarity > unrelated_similarity,
            "expected related texts ({related_similarity}) to be more similar than unrelated \
             texts ({unrelated_similarity})"
        );
    }
}
