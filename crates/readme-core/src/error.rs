use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReadmeError {
    #[error("URL inválida: {0}")]
    InvalidUrl(String),
    #[error("esquema não permitido: {0}")]
    SchemeNotAllowed(String),
    #[error("endereço local/privado bloqueado: {0}")]
    PrivateAddress(String),
    #[error("falha de rede: {0}")]
    Network(String),
    #[error("resposta HTTP {0}")]
    Http(u16),
    #[error("resposta maior que o limite de {0} bytes")]
    TooLarge(usize),
}

pub type Result<T> = std::result::Result<T, ReadmeError>;
