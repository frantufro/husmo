//! Local, in-process embedding generation — zero network calls per
//! save/search operation, per
//! `docs/adr/0001-local-first-no-external-services.md`.
//!
//! [`embed`] runs real inference, via `candle`, of a small pre-trained
//! sentence-embedding model ([`MODEL_ID`], `all-MiniLM-L6-v2`), so a
//! chunk's vector reflects its actual meaning rather than only its
//! vocabulary. The model's weights, config, and tokenizer are fetched
//! through the Hugging Face Hub API the first time any process on this
//! machine needs them, and cached locally after that (see [`load_model`]);
//! `embed` itself never makes a network call. This supersedes an earlier
//! deterministic feature-hashing bag-of-words placeholder that reasoned a
//! one-time weights download broke "zero network calls" — that reasoning
//! was rejected (see `docs/ARCHITECTURE.md`, "Retrieval"): the constraint
//! is zero network calls *per operation*, and a one-time local fetch at
//! setup/first-run is within it.

use std::sync::OnceLock;

use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
use hf_hub::api::sync::Api;
use tokenizers::{Tokenizer, TruncationParams};

/// Hugging Face Hub id of the pre-trained sentence-embedding model this
/// module runs locally.
const MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Dimensionality of every embedding vector [`embed`] produces — the
/// hidden size of [`MODEL_ID`].
pub const EMBEDDING_DIM: usize = 384;

/// The model and tokenizer [`embed`] runs, built once per process (see
/// [`model`]) and reused by every call after that.
struct Model {
    bert: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

/// The process-wide [`Model`] instance, built lazily on the first call to
/// [`embed`].
static MODEL: OnceLock<Model> = OnceLock::new();

/// Returns the process-wide [`Model`], building it via [`load_model`] on
/// the first call and reusing it on every later one.
fn model() -> &'static Model {
    MODEL.get_or_init(load_model)
}

/// Fetches (or reuses a local cache of) [`MODEL_ID`]'s config, tokenizer,
/// and weights, and builds the [`Model`] they describe.
///
/// The Hugging Face Hub API this calls into caches every file it fetches
/// under the OS's standard cache directory (respecting `HF_HOME`), so only
/// the very first call across every process on a machine performs any
/// network I/O; every later one, including in future runs, reads that
/// local cache instead.
///
/// # Panics
///
/// Panics if the model's files can't be fetched or cached, or if their
/// contents don't parse as the expected BERT config, tokenizer, or
/// safetensors weights. There is no expected-to-happen failure mode here
/// to recover from: without this model, `embed` cannot do its job.
fn load_model() -> Model {
    let repo = Api::new()
        .expect("failed to create the Hugging Face Hub API client")
        .model(MODEL_ID.to_string());

    let config_path = repo
        .get("config.json")
        .expect("failed to fetch/cache the embedding model's config.json");
    let tokenizer_path = repo
        .get("tokenizer.json")
        .expect("failed to fetch/cache the embedding model's tokenizer.json");
    let weights_path = repo
        .get("model.safetensors")
        .expect("failed to fetch/cache the embedding model's weights");

    let config: BertConfig = serde_json::from_str(
        &std::fs::read_to_string(&config_path)
            .expect("failed to read the embedding model's config.json"),
    )
    .expect("the embedding model's config.json did not parse as a BERT config");

    let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
        .expect("failed to load the embedding model's tokenizer.json");
    // Caps sequence length at the model's own limit (`TruncationParams`
    // defaults to 512, matching `config.max_position_embeddings`), so a
    // chunk longer than that gets truncated instead of overrunning the
    // model's position embeddings.
    tokenizer
        .with_truncation(Some(TruncationParams::default()))
        .expect("failed to configure the embedding model's tokenizer truncation");

    let device = Device::Cpu;
    // Safe: `weights_path` is a local file this process just fetched (or
    // found already cached) through the Hub API, not attacker-controlled
    // input.
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)
            .expect("failed to memory-map the embedding model's weights")
    };
    let bert = BertModel::load(vb, &config).expect("failed to build the BERT model from weights");

    Model { bert, tokenizer, device }
}

/// Computes a fixed-[`EMBEDDING_DIM`]-dimension embedding vector for
/// `text`, via local, in-process inference of [`MODEL_ID`].
///
/// The same `text` always produces the same vector: model inference here
/// runs no dropout and holds no other randomness. The vector is
/// mean-pooled over every token's final hidden state (following the
/// model's own pooling configuration) and L2-normalized, so cosine
/// similarity between two `embed` outputs reduces to a plain dot product.
/// Text sharing meaning with another text — including via synonyms or
/// paraphrasing, not just shared vocabulary — tends to produce a higher
/// cosine similarity between their vectors.
///
/// # Panics
///
/// Panics if the model can't be loaded (see [`load_model`]), or if
/// tokenizing `text` or running it through the model fails. Neither is
/// expected to happen for any `text` this is called with in practice.
#[must_use]
pub fn embed(text: &str) -> Vec<f32> {
    let model = model();

    let encoding = model
        .tokenizer
        .encode(text, true)
        .expect("failed to tokenize text for embedding");
    let input_ids = Tensor::new(encoding.get_ids(), &model.device)
        .expect("failed to build an input-ids tensor")
        .unsqueeze(0)
        .expect("failed to add a batch dimension to the input-ids tensor");
    let token_type_ids = input_ids
        .zeros_like()
        .expect("failed to build a token-type-ids tensor");

    let hidden_states = model
        .bert
        .forward(&input_ids, &token_type_ids, None)
        .expect("BERT forward pass failed");
    // Mean-pools over the token dimension (dim 1 of [batch, tokens,
    // hidden]) into one vector per input, following the model's own
    // pooling configuration.
    let pooled = hidden_states
        .mean(1)
        .expect("failed to average token hidden states into a mean-pooled vector");

    let mut vector = pooled
        .squeeze(0)
        .expect("failed to drop the pooled vector's batch dimension")
        .to_vec1::<f32>()
        .expect("failed to read the pooled embedding vector");
    normalize(&mut vector);
    vector
}

/// Scales `vector` to unit length in place. Leaves an all-zero vector
/// untouched — there's no direction to normalize toward. In practice
/// [`embed`]'s pooled vectors are never all-zero, but this stays safe
/// either way.
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
    fn embed_is_case_insensitive() {
        assert_eq!(embed("Rust Embeddings"), embed("rust embeddings"));
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn embed_makes_a_paraphrase_more_similar_than_a_coincidentally_wordy_unrelated_sentence() {
        let base = embed("The company's profits increased significantly this quarter.");
        let paraphrase = embed("The firm's earnings rose substantially in this period.");
        // Shares the content word "quarter" with `base`, unlike `paraphrase` — a
        // bag-of-words hash embedding latches onto that shared token, but a real
        // sentence-embedding model should still rank the meaning-preserving
        // paraphrase above it.
        let unrelated = embed("The weather was cold and rainy this quarter of the year.");

        let paraphrase_similarity = cosine_similarity(&base, &paraphrase);
        let unrelated_similarity = cosine_similarity(&base, &unrelated);

        assert!(
            paraphrase_similarity > unrelated_similarity,
            "expected the paraphrase ({paraphrase_similarity}) to score higher than the \
             coincidentally-wordy unrelated sentence ({unrelated_similarity})"
        );
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
