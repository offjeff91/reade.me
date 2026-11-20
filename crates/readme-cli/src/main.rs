use readme_core::Block;

fn main() {
    let Some(url) = std::env::args().nth(1) else {
        eprintln!("uso: readme <url>");
        std::process::exit(2);
    };
    match readme_core::read_url(&url) {
        Ok(doc) => {
            if let Some(t) = &doc.meta.title { println!("# {t}\n"); }
            for b in &doc.blocks {
                match b {
                    Block::Heading { level, text } => println!("{} {text}\n", "#".repeat(*level as usize)),
                    Block::Paragraph { text } => println!("{text}\n"),
                    Block::Quote { text } => println!("> {text}\n"),
                    Block::List { ordered, items } => {
                        for (i, it) in items.iter().enumerate() {
                            if *ordered { println!("{}. {it}", i + 1) } else { println!("- {it}") }
                        }
                        println!();
                    }
                    Block::Code { text, .. } => println!("```\n{text}\n```\n"),
                    Block::Image { src, alt, .. } => println!("[imagem: {}] {src}\n", alt.clone().unwrap_or_default()),
                    Block::Table { rows } => for r in rows { println!("| {} |", r.join(" | ")) },
                }
            }
        }
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(1);
        }
    }
}
