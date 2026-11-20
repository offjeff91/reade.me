pub mod ast;
pub mod error;
pub mod extract;
pub mod fetch;

pub use ast::{Block, Document, Meta};
pub use error::{ReadmeError, Result};

/// Pipeline completo: baixa a URL e devolve o documento de leitura.
pub fn read_url(url: &str) -> Result<Document> {
    let page = fetch::fetch(url)?;
    Ok(extract::extract(&page.html, &page.final_url))
}
