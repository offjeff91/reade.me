//! Ponte UniFFI: expõe o núcleo para Kotlin (Android) e Swift (iOS).
//! O documento viaja como JSON; o shell nativo desserializa e desenha.

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiError {
    #[error("{0}")]
    Failed(String),
}

/// Baixa a URL e devolve o documento de leitura em JSON.
#[uniffi::export]
pub fn read_url_json(url: String) -> Result<String, FfiError> {
    let doc = readme_core::read_url(&url).map_err(|e| FfiError::Failed(e.to_string()))?;
    serde_json::to_string(&doc).map_err(|e| FfiError::Failed(e.to_string()))
}
