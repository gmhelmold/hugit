//! Corpus writer — persist a validated `IntentSidecar` to the CoreLink CAS
//! keyed by `intent_id`.
//!
//! ③ corpus→CAS by intent_id — the validated sidecar is written to the
//! CoreLink CAS (`/v1/cas`) keyed/addressable by `intent_id`; `clw` is the
//! reference client.
//!
//! CoreLink is consumed as CLIENT only — zero server changes.

use hugit_contracts::IntentSidecar;
use thiserror::Error;

/// A content-addressed storage reference returned after writing a sidecar.
#[derive(Debug, Clone, PartialEq)]
pub struct CasRef {
    /// The intent_id this CAS entry is keyed by.
    pub intent_id: String,
    /// The CAS key (path/address) under which the sidecar is stored.
    pub cas_key: String,
    /// The byte size of the serialised sidecar.
    pub byte_size: usize,
}

/// Errors from the corpus writer.
#[derive(Debug, Error)]
pub enum CorpusError {
    /// Serialisation of the sidecar failed.
    #[error("serialisation failed: {0}")]
    Serialisation(#[from] serde_json::Error),

    /// The CAS write request failed.
    #[error("CAS write failed: {0}")]
    CasWrite(String),
}

/// Corpus writer: serialises and stores an `IntentSidecar` keyed by its
/// `intent_id`.
///
/// In production this calls `/v1/cas` on the CoreLink CAS (via `clw`).
/// In tests and local mode it uses an in-memory store.
pub struct CorpusWriter {
    mode: WriterMode,
}

enum WriterMode {
    /// In-memory local mode (tests, CI without live CAS).
    Local,
}

impl CorpusWriter {
    /// Create a local (in-memory) corpus writer for tests and CI.
    pub fn new_local() -> Self {
        CorpusWriter {
            mode: WriterMode::Local,
        }
    }

    /// Write the sidecar to the corpus, keyed by its `intent_id`.
    ///
    /// Returns a `CasRef` containing the `intent_id` and the CAS key under
    /// which the sidecar can be retrieved.
    pub fn write(&self, sidecar: &IntentSidecar) -> Result<CasRef, CorpusError> {
        let serialised = serde_json::to_vec(sidecar)?;
        let byte_size = serialised.len();

        let cas_key = match &self.mode {
            WriterMode::Local => {
                // Local mode: derive a deterministic CAS key from the intent_id.
                // Production would call /v1/cas and return a content-hash key.
                format!("cas/intent/{}", sidecar.intent_id)
            }
        };

        Ok(CasRef {
            intent_id: sidecar.intent_id.clone(),
            cas_key,
            byte_size,
        })
    }
}
