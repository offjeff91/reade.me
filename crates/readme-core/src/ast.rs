//! Modelo de documento de leitura (AST). Nunca carrega HTML bruto:
//! o shell nativo desenha cada bloco, então não há JS nem markup executável.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Meta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub published: Option<String>,
    pub site: Option<String>,
    pub lang: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph { text: String },
    List { ordered: bool, items: Vec<String> },
    Quote { text: String },
    Code { lang: Option<String>, text: String },
    Image { src: String, alt: Option<String>, caption: Option<String> },
    Table { rows: Vec<Vec<String>> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Document {
    pub meta: Meta,
    pub blocks: Vec<Block>,
}

impl Document {
    /// Palavras aproximadas, para o tempo estimado de leitura.
    pub fn word_count(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| match b {
                Block::Heading { text, .. }
                | Block::Paragraph { text }
                | Block::Quote { text }
                | Block::Code { text, .. } => text.split_whitespace().count(),
                Block::List { items, .. } => items.iter().map(|i| i.split_whitespace().count()).sum(),
                Block::Table { rows } => rows.iter().flatten().map(|c| c.split_whitespace().count()).sum(),
                Block::Image { .. } => 0,
            })
            .sum()
    }
}
