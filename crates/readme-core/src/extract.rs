//! Extração do conteúdo principal de uma página HTML para o AST de leitura.
//!
//! Não é um algoritmo de legibilidade completo: remove ruído por tag e por
//! class/id/role, escolhe um container (article > main > pontuação > body) e
//! converte os blocos de texto. Nunca copia markup: só texto puro.

use std::collections::HashMap;

use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

use crate::ast::{Block, Document, Meta};

/// Tags que nunca fazem parte do conteúdo de leitura.
const NOISE_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "nav", "header", "footer", "aside", "form", "iframe", "svg",
];
/// Palavras de ruído, casadas por substring em cada token de class/id/role.
const NOISE_WORDS: &[&str] = &[
    "comment", "sidebar", "promo", "related", "share", "cookie", "newsletter", "popup", "menu",
];
/// Elementos que só agrupam blocos: o walker desce neles.
const CONTAINER_TAGS: &[&str] = &["article", "body", "details", "div", "dl", "html", "main", "section"];

pub fn extract(html: &str, base: &Url) -> Document {
    let doc = Html::parse_document(html);
    let meta = read_meta(&doc, base);
    let blocks = main_container(&doc)
        .map(|root| {
            let mut out = Vec::new();
            walk(&root, base, &mut out);
            out
        })
        .unwrap_or_default();
    Document { meta, blocks }
}

fn sel(css: &str) -> Selector {
    Selector::parse(css).expect("seletor CSS fixo e válido")
}

fn read_meta(doc: &Html, base: &Url) -> Meta {
    let title_tag = doc.select(&sel("title")).next().and_then(|t| non_empty(plain(&t)));
    let published = meta_content(doc, r#"meta[property="article:published_time"]"#).or_else(|| {
        doc.select(&sel("time[datetime]"))
            .filter_map(|t| t.value().attr("datetime"))
            .find_map(|d| non_empty(normalize(d)))
    });
    Meta {
        title: meta_content(doc, r#"meta[property="og:title"]"#).or(title_tag),
        author: meta_content(doc, r#"meta[name="author"]"#)
            .or_else(|| meta_content(doc, r#"meta[property="article:author"]"#)),
        published,
        site: meta_content(doc, r#"meta[property="og:site_name"]"#),
        lang: doc
            .root_element()
            .value()
            .attr("lang")
            .and_then(|l| non_empty(l.trim().to_string())),
        url: base.to_string(),
    }
}

fn meta_content(doc: &Html, css: &str) -> Option<String> {
    doc.select(&sel(css))
        .filter_map(|e| e.value().attr("content"))
        .map(normalize)
        .find(|s| !s.is_empty())
}

/// Primeiro <article> fora de ruído; senão <main>; senão o bloco mais pontuado; senão <body>.
fn main_container(doc: &Html) -> Option<ElementRef<'_>> {
    ["article", "main"]
        .iter()
        .find_map(|tag| doc.select(&sel(tag)).find(|el| !in_noise(el)))
        .or_else(|| scored_container(doc))
        .or_else(|| doc.select(&sel("body")).next())
}

/// Cada <p> com texto que não é link dá pontos ao pai (inteiro) e ao avô (metade).
fn scored_container(doc: &Html) -> Option<ElementRef<'_>> {
    let mut scores = HashMap::new();
    for p in doc.select(&sel("p")).filter(|p| !in_noise(p)) {
        let gain = plain_len(&p).saturating_sub(link_len(&p));
        let Some(parent) = p.parent().and_then(ElementRef::wrap) else {
            continue;
        };
        *scores.entry(parent.id()).or_insert(0) += gain;
        if let Some(grand) = parent.parent().and_then(ElementRef::wrap) {
            *scores.entry(grand.id()).or_insert(0) += gain / 2;
        }
    }
    // Ordem de documento: em empate vence o primeiro (o mais externo).
    let mut best: Option<(ElementRef, usize)> = None;
    for el in doc.root_element().descendants().filter_map(ElementRef::wrap) {
        if el.value().name() == "html" {
            continue;
        }
        let Some(&score) = scores.get(&el.id()) else {
            continue;
        };
        match best {
            Some((_, b)) if b >= score => {}
            _ => best = Some((el, score)),
        }
    }
    best.map(|(el, _)| el)
}

fn plain_len(el: &ElementRef) -> usize {
    plain(el).chars().count()
}

fn link_len(el: &ElementRef) -> usize {
    el.select(&sel("a")).map(|a| plain_len(&a)).sum()
}

fn walk(parent: &ElementRef, base: &Url, out: &mut Vec<Block>) {
    let mut loose = String::new();
    for child in parent.children() {
        match child.value() {
            Node::Text(text) => loose.push_str(text),
            Node::Element(_) => {
                let Some(el) = ElementRef::wrap(child) else {
                    continue;
                };
                if is_noise(&el) {
                    continue;
                }
                if !is_structural(&el) {
                    // Inline (a, span, strong...): vira texto do bloco corrente.
                    loose.push_str(&text_of(&el));
                    loose.push(' ');
                    if el.select(&sel("img")).next().is_some() {
                        flush(&mut loose, out);
                        push_images(&el, base, out);
                    }
                    continue;
                }
                flush(&mut loose, out);
                let name = el.value().name();
                match name {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => out.extend(
                        non_empty(plain(&el)).map(|text| Block::Heading {
                            level: name.as_bytes()[1] - b'0',
                            text,
                        }),
                    ),
                    "p" => {
                        out.extend(non_empty(plain(&el)).map(|text| Block::Paragraph { text }));
                        push_images(&el, base, out);
                    }
                    "ul" | "ol" => out.extend(list(&el, name == "ol")),
                    "blockquote" => out.extend(non_empty(plain(&el)).map(|text| Block::Quote { text })),
                    "pre" | "code" => out.extend(code_block(&el)),
                    "figure" => out.extend(figure(&el, base)),
                    "img" => out.extend(image(&el, base, None)),
                    "table" => out.extend(table(&el)),
                    _ => walk(&el, base, out),
                }
            }
            _ => {}
        }
    }
    flush(&mut loose, out);
}

/// Elementos que viram bloco próprio (ou containers); o resto é inline.
fn is_structural(el: &ElementRef) -> bool {
    match el.value().name() {
        "p" | "ul" | "ol" | "blockquote" | "pre" | "figure" | "img" | "table" | "h1" | "h2" | "h3" | "h4"
        | "h5" | "h6" => true,
        // <code> só é bloco quando é multilinha; inline fica no texto do parágrafo.
        "code" => text_of(el).contains('\n'),
        name => CONTAINER_TAGS.contains(&name),
    }
}

fn flush(loose: &mut String, out: &mut Vec<Block>) {
    out.extend(non_empty(normalize(&std::mem::take(loose))).map(|text| Block::Paragraph { text }));
}

fn list(el: &ElementRef, ordered: bool) -> Option<Block> {
    let items: Vec<String> = el
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|li| li.value().name() == "li" && !is_noise(li))
        .filter_map(|li| non_empty(plain(&li)))
        .collect();
    (!items.is_empty()).then_some(Block::List { ordered, items })
}

fn code_block(el: &ElementRef) -> Option<Block> {
    let raw = raw_text(el);
    let text = raw.trim_matches(|c: char| c == '\n' || c == '\r').trim_end().to_string();
    if text.trim().is_empty() {
        return None;
    }
    Some(Block::Code { lang: language(el), text })
}

/// Linguagem vem de `language-x` / `lang-x` no <pre> ou no <code> interno.
fn language(el: &ElementRef) -> Option<String> {
    let inner = el.select(&sel("code")).next();
    inner
        .iter()
        .chain(std::iter::once(el))
        .flat_map(|e| e.value().attr("class").unwrap_or("").split_whitespace())
        .find_map(|c| c.strip_prefix("language-").or_else(|| c.strip_prefix("lang-")))
        .map(str::to_string)
}

fn table(el: &ElementRef) -> Option<Block> {
    let rows: Vec<Vec<String>> = el
        .select(&sel("tr"))
        .map(|tr| {
            tr.children()
                .filter_map(ElementRef::wrap)
                .filter(|c| matches!(c.value().name(), "td" | "th"))
                .map(|c| plain(&c))
                .collect()
        })
        .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
        .collect();
    (!rows.is_empty()).then_some(Block::Table { rows })
}

fn figure(el: &ElementRef, base: &Url) -> Option<Block> {
    let img = el.select(&sel("img")).next()?;
    let caption = el.select(&sel("figcaption")).next().and_then(|c| non_empty(plain(&c)));
    image(&img, base, caption)
}

fn push_images(el: &ElementRef, base: &Url, out: &mut Vec<Block>) {
    for img in el.select(&sel("img")).filter(|img| !in_noise(img)) {
        out.extend(image(&img, base, None));
    }
}

fn image(el: &ElementRef, base: &Url, caption: Option<String>) -> Option<Block> {
    let src = image_src(el, base)?;
    let alt = el.value().attr("alt").and_then(|a| non_empty(normalize(a)));
    Some(Block::Image { src, alt, caption })
}

/// Ordem: `data-src` (lazy-load), maior candidato de `srcset`, `src`.
/// Candidatos inválidos (ex.: `javascript:`) são descartados um a um.
fn image_src(el: &ElementRef, base: &Url) -> Option<String> {
    let attr = |name: &str| el.value().attr(name).map(str::trim).filter(|v| !v.is_empty());
    [attr("data-src"), attr("srcset").and_then(best_srcset), attr("src")]
        .into_iter()
        .flatten()
        .find_map(|raw| resolve(base, raw))
}

fn best_srcset(srcset: &str) -> Option<&str> {
    srcset
        .split(',')
        .filter_map(|candidate| {
            let mut parts = candidate.split_whitespace();
            let url = parts.next()?;
            let weight = parts
                .next()
                .and_then(|d| d.trim_end_matches(['w', 'x']).parse::<f64>().ok())
                .unwrap_or(1.0);
            Some((url, weight))
        })
        .fold(None, |best: Option<(&str, f64)>, cur| match best {
            Some(b) if b.1 >= cur.1 => Some(b),
            _ => Some(cur),
        })
        .map(|(url, _)| url)
}

fn resolve(base: &Url, raw: &str) -> Option<String> {
    let url = base.join(raw).ok()?;
    matches!(url.scheme(), "http" | "https" | "data").then(|| url.to_string())
}

/// Texto normalizado (espaços colapsados), sem markup.
fn plain(el: &ElementRef) -> String {
    normalize(&text_of(el))
}

fn text_of(el: &ElementRef) -> String {
    let mut out = String::new();
    push_text(el, &mut out, false);
    out
}

/// Texto com quebras de linha preservadas (para código).
fn raw_text(el: &ElementRef) -> String {
    let mut out = String::new();
    push_text(el, &mut out, true);
    out
}

fn push_text(el: &ElementRef, out: &mut String, raw: bool) {
    for child in el.children() {
        match child.value() {
            Node::Text(text) => out.push_str(text),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                let name = child.value().name();
                if name == "br" {
                    out.push('\n');
                    continue;
                }
                if is_noise(&child) {
                    continue;
                }
                let brk = !raw && is_text_break(name);
                if brk {
                    out.push(' ');
                }
                push_text(&child, out, raw);
                if brk {
                    out.push(' ');
                }
            }
            _ => {}
        }
    }
}

fn is_text_break(name: &str) -> bool {
    matches!(
        name,
        "article" | "blockquote" | "dd" | "details" | "div" | "dl" | "dt" | "figcaption" | "figure" | "h1"
            | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "ol" | "p" | "pre" | "section" | "table" | "td"
            | "th" | "tr" | "ul"
    )
}

fn is_noise(el: &ElementRef) -> bool {
    let name = el.value().name();
    if NOISE_TAGS.contains(&name) {
        return true;
    }
    // Nunca se remove o conteúdo principal por causa de class/id/role.
    if matches!(name, "article" | "main" | "body" | "html") {
        return false;
    }
    ["class", "id", "role"].iter().any(|attr| {
        el.value()
            .attr(attr)
            .unwrap_or("")
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| is_noise_word(&tok.to_ascii_lowercase()))
    })
}

fn is_noise_word(tok: &str) -> bool {
    // "ad" vale só como palavra inteira: senão casaria header, heading, shadow...
    matches!(tok, "ad" | "ads") || tok.starts_with("advert") || NOISE_WORDS.iter().any(|w| tok.contains(*w))
}

/// O próprio elemento ou algum ancestral é ruído.
fn in_noise(el: &ElementRef) -> bool {
    is_noise(el) || el.ancestors().filter_map(ElementRef::wrap).any(|a| is_noise(&a))
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(html: &str) -> Document {
        extract(html, &Url::parse("https://ex.com/post/").unwrap())
    }

    fn para(text: &str) -> Block {
        Block::Paragraph { text: text.into() }
    }

    #[test]
    fn entrada_vazia() {
        let doc = run("");
        assert!(doc.blocks.is_empty());
        assert_eq!(doc.meta.url, "https://ex.com/post/");
    }

    #[test]
    fn artigo_simples_com_titulo() {
        let doc = run("<html lang='pt-BR'><head><title>Página</title></head>\
            <body><article><h1>Título</h1><p>Olá   mundo.</p></article></body></html>");
        assert_eq!(doc.meta.title.as_deref(), Some("Página"));
        assert_eq!(doc.meta.lang.as_deref(), Some("pt-BR"));
        assert_eq!(
            doc.blocks,
            vec![Block::Heading { level: 1, text: "Título".into() }, para("Olá mundo.")]
        );
    }

    #[test]
    fn remove_nav_footer_e_script() {
        let doc = run("<body><nav><a>Menu</a></nav><div><script>var x = 1;</script>\
            <p>Corpo do texto.</p></div><footer>Rodapé</footer></body>");
        assert_eq!(doc.blocks, vec![para("Corpo do texto.")]);
    }

    #[test]
    fn nao_remove_por_substring_enganosa() {
        let doc = run(r#"<div class="heading-shadow"><p>Conteúdo</p></div>"#);
        assert_eq!(doc.blocks, vec![para("Conteúdo")]);
    }

    #[test]
    fn lazy_load_resolve_src_relativo() {
        let doc = run(r#"<article><p>Veja:</p><figure>
            <img data-src="img/a.png" src="data:image/gif;base64,R0lGOD" alt="Gráfico">
            <figcaption>Figura 1</figcaption></figure></article>"#);
        assert_eq!(
            doc.blocks,
            vec![
                para("Veja:"),
                Block::Image {
                    src: "https://ex.com/post/img/a.png".into(),
                    alt: Some("Gráfico".into()),
                    caption: Some("Figura 1".into()),
                },
            ]
        );
    }

    #[test]
    fn lista_ordenada() {
        let doc = run("<article><ol><li>Primeiro</li><li> Segundo   item </li></ol></article>");
        assert_eq!(
            doc.blocks,
            vec![Block::List { ordered: true, items: vec!["Primeiro".into(), "Segundo item".into()] }]
        );
    }

    #[test]
    fn bloco_de_codigo_rust() {
        let doc = run(r#"<article><pre><code class="language-rust">fn main() {
    println!("oi");
}</code></pre></article>"#);
        assert_eq!(
            doc.blocks,
            vec![Block::Code {
                lang: Some("rust".into()),
                text: "fn main() {\n    println!(\"oi\");\n}".into(),
            }]
        );
    }

    #[test]
    fn imagem_javascript_descartada() {
        let doc = run(r#"<article><p>Texto</p><img src="javascript:alert(1)" alt="x"></article>"#);
        assert_eq!(doc.blocks, vec![para("Texto")]);
    }
}
