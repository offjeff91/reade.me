//! Download de páginas com validação anti-SSRF, limite de tamanho e
//! detecção de encoding.

use crate::error::{ReadmeError, Result};
use encoding_rs::Encoding;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, USER_AGENT as USER_AGENT_HEADER};
use reqwest::redirect::Policy;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::{Host, Url};

/// User-Agent fixo, sem dados pessoais.
pub const USER_AGENT: &str = "read.me/0.1 (mobile reader)";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 5;
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const SNIFF_LEN: usize = 1024;

pub struct Page {
    pub html: String,
    pub final_url: Url,
}

/// Erro gerado dentro do closure de redirect (que só pode devolver `Action`),
/// guardado aqui para ser recuperado com a variante correta.
type ErrorSlot = Arc<Mutex<Option<ReadmeError>>>;

pub fn fetch(url: &str) -> Result<Page> {
    let parsed = Url::parse(url).map_err(|e| ReadmeError::InvalidUrl(e.to_string()))?;
    validate_url(&parsed)?;

    let slot: ErrorSlot = Arc::new(Mutex::new(None));
    let client = build_client(slot.clone())?;

    let resp = client
        .get(parsed)
        .header(USER_AGENT_HEADER, USER_AGENT)
        .send()
        .map_err(|e| map_send_error(&e, &slot))?;

    let status = resp.status();
    if status.is_client_error() || status.is_server_error() {
        return Err(ReadmeError::Http(status.as_u16()));
    }
    if status.is_redirection() {
        return Err(ReadmeError::Network("redirecionamento sem destino".into()));
    }

    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    check_content_type(content_type.as_deref())?;

    let final_url = resp.url().clone();
    if let Some(len) = resp.content_length() {
        if len > MAX_BODY_BYTES as u64 {
            return Err(ReadmeError::TooLarge(MAX_BODY_BYTES));
        }
    }

    let body = read_limited(resp)?;
    let html = decode_html(&body, content_type.as_deref());
    Ok(Page { html, final_url })
}

fn build_client(slot: ErrorSlot) -> Result<Client> {
    let policy = Policy::custom(move |attempt| {
        // previous() inclui a URL original; o redirect nº 6 é o primeiro recusado.
        if attempt.previous().len() > MAX_REDIRECTS {
            store_error(&slot, ReadmeError::Network("redirecionamentos demais".into()));
            return attempt.error("redirecionamentos demais");
        }
        match validate_url(attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(e) => {
                store_error(&slot, e);
                attempt.error("destino de redirecionamento bloqueado")
            }
        }
    });
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(TIMEOUT)
        .redirect(policy)
        .build()
        .map_err(|_| ReadmeError::Network("falha ao inicializar cliente HTTP".into()))
}

fn store_error(slot: &ErrorSlot, err: ReadmeError) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(err);
    }
}

/// Mapeia erros do reqwest sem incluir a URL (que pode conter credenciais).
fn map_send_error(e: &reqwest::Error, slot: &ErrorSlot) -> ReadmeError {
    if let Some(err) = slot.lock().ok().and_then(|mut g| g.take()) {
        return err;
    }
    if e.is_timeout() {
        ReadmeError::Network("tempo esgotado".into())
    } else if e.is_connect() {
        ReadmeError::Network("não foi possível conectar ao host".into())
    } else if e.is_redirect() {
        ReadmeError::Network("redirecionamento inválido".into())
    } else {
        ReadmeError::Network("erro de rede".into())
    }
}

/// Validação feita antes de cada requisição e em cada redirect.
fn validate_url(url: &Url) -> Result<()> {
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(ReadmeError::SchemeNotAllowed(other.to_string())),
    }
    match url.host() {
        None => Err(ReadmeError::InvalidUrl("host vazio".into())),
        Some(Host::Ipv4(ip)) => check_ip(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => check_ip(IpAddr::V6(ip)),
        Some(Host::Domain(domain)) => {
            if domain.is_empty() {
                return Err(ReadmeError::InvalidUrl("host vazio".into()));
            }
            let lower = domain.to_ascii_lowercase();
            if lower == "localhost" || lower.ends_with(".localhost") {
                return Err(ReadmeError::PrivateAddress(domain.to_string()));
            }
            let port = url.port_or_known_default().unwrap_or(80);
            let addrs = (domain, port)
                .to_socket_addrs()
                .map_err(|_| ReadmeError::Network("não foi possível resolver o host".into()))?;
            let mut any = false;
            for addr in addrs {
                any = true;
                check_ip(addr.ip())?;
            }
            if any {
                Ok(())
            } else {
                Err(ReadmeError::Network("host sem endereços".into()))
            }
        }
    }
}

fn check_ip(ip: IpAddr) -> Result<()> {
    if is_public_ip(ip) {
        Ok(())
    } else {
        Err(ReadmeError::PrivateAddress(ip.to_string()))
    }
}

/// Retorna `true` apenas para endereços roteáveis publicamente.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local() // inclui 169.254.169.254 (metadados de nuvem)
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || o[0] == 0 // "esta rede" 0.0.0.0/8
        || (o[0] == 100 && (o[1] & 0xC0) == 64) // CGNAT 100.64.0.0/10
        || o[0] >= 240) // reservado 240.0.0.0/4
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (s[0] & 0xffc0) == 0xfec0 // site-local (obsoleto) fec0::/10
        || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentação
}

fn check_content_type(ct: Option<&str>) -> Result<()> {
    let Some(raw) = ct else { return Ok(()) };
    let mime = raw.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match mime.as_str() {
        "" | "text/html" | "application/xhtml+xml" => Ok(()),
        other => Err(ReadmeError::Network(format!(
            "tipo de conteúdo não aceito: {other}"
        ))),
    }
}

fn read_limited<R: Read>(mut reader: R) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|_| ReadmeError::Network("erro ao ler a resposta".into()))?;
        if n == 0 {
            break;
        }
        if body.len() + n > MAX_BODY_BYTES {
            return Err(ReadmeError::TooLarge(MAX_BODY_BYTES));
        }
        body.extend_from_slice(&buf[..n]);
    }
    Ok(body)
}

/// Decodifica o corpo: BOM, depois charset do Content-Type, depois
/// `<meta charset>` / `http-equiv` nos primeiros 1024 bytes, default UTF-8.
pub fn decode_html(bytes: &[u8], content_type: Option<&str>) -> String {
    let encoding = Encoding::for_bom(bytes)
        .map(|(enc, _)| enc)
        .or_else(|| content_type.and_then(charset_from_content_type))
        .or_else(|| sniff_meta_charset(&bytes[..bytes.len().min(SNIFF_LEN)]))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

fn charset_from_content_type(ct: &str) -> Option<&'static Encoding> {
    let lower = ct.to_ascii_lowercase();
    let idx = lower.find("charset")?;
    parse_charset_value(&lower[idx + "charset".len()..])
}

fn sniff_meta_charset(prefix: &[u8]) -> Option<&'static Encoding> {
    let lower = String::from_utf8_lossy(prefix).to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("<meta") {
        let start = from + pos;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &lower[start..end];
        if let Some(idx) = tag.find("charset") {
            if let Some(enc) = parse_charset_value(&tag[idx + "charset".len()..]) {
                return Some(enc);
            }
        }
        from = end;
    }
    None
}

/// Lê `\s*=\s*["']?label` e resolve o label. UTF-16 declarado em meta é tratado como UTF-8.
fn parse_charset_value(rest: &str) -> Option<&'static Encoding> {
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let rest = rest.trim_start_matches(['"', '\'']);
    let label: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .collect();
    if label.is_empty() {
        return None;
    }
    let enc = Encoding::for_label(label.as_bytes())?;
    if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
        Some(encoding_rs::UTF_8)
    } else {
        Some(enc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ipv4_publico_e_permitido() {
        assert!(is_public_ip(ip("8.8.8.8")));
        assert!(is_public_ip(ip("1.1.1.1")));
        assert!(is_public_ip(ip("172.15.0.1")));
        assert!(is_public_ip(ip("172.32.0.1")));
    }

    #[test]
    fn ipv4_nao_publico_e_bloqueado() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.4",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "100.64.0.1",
            "192.0.2.1",
            "240.0.0.1",
        ] {
            assert!(!is_public_ip(ip(s)), "{s} deveria ser bloqueado");
        }
    }

    #[test]
    fn ipv6_publico_e_permitido() {
        assert!(is_public_ip(ip("2606:4700:4700::1111")));
        assert!(is_public_ip(ip("2001:4860:4860::8888")));
    }

    #[test]
    fn ipv6_nao_publico_e_bloqueado() {
        for s in [
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_public_ip(ip(s)), "{s} deveria ser bloqueado");
        }
    }

    #[test]
    fn rejeita_esquemas_nao_http() {
        assert!(matches!(
            fetch("file:///etc/passwd"),
            Err(ReadmeError::SchemeNotAllowed(_))
        ));
        assert!(matches!(
            fetch("ftp://example.com/a"),
            Err(ReadmeError::SchemeNotAllowed(_))
        ));
    }

    #[test]
    fn rejeita_ips_literais_privados_sem_rede() {
        assert!(matches!(
            fetch("http://127.0.0.1/"),
            Err(ReadmeError::PrivateAddress(_))
        ));
        assert!(matches!(
            fetch("http://169.254.169.254/latest/meta-data"),
            Err(ReadmeError::PrivateAddress(_))
        ));
        assert!(matches!(
            fetch("http://[::1]:8080/"),
            Err(ReadmeError::PrivateAddress(_))
        ));
    }

    #[test]
    fn rejeita_localhost_sem_rede() {
        assert!(matches!(
            fetch("http://localhost:3000/"),
            Err(ReadmeError::PrivateAddress(_))
        ));
    }

    #[test]
    fn content_type_aceitos_e_recusados() {
        assert!(check_content_type(None).is_ok());
        assert!(check_content_type(Some("text/html; charset=utf-8")).is_ok());
        assert!(check_content_type(Some("Application/XHTML+xml")).is_ok());
        assert!(check_content_type(Some("application/pdf")).is_err());
        assert!(check_content_type(Some("image/png")).is_err());
    }

    #[test]
    fn meta_charset_windows_1252() {
        // "Café" em windows-1252: 'é' = 0xE9
        let bytes = b"<html><head><meta charset=\"windows-1252\"></head><body>Caf\xE9</body></html>";
        assert!(decode_html(bytes, None).ends_with("Café</body></html>"));
    }

    #[test]
    fn meta_http_equiv_charset() {
        let bytes = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=ISO-8859-1\">Caf\xE9";
        assert!(decode_html(bytes, None).ends_with("Café"));
    }

    #[test]
    fn charset_do_content_type_tem_prioridade_sobre_meta() {
        let bytes = b"<meta charset=\"windows-1252\">Caf\xC3\xA9";
        assert_eq!(
            decode_html(bytes, Some("text/html; charset=utf-8")),
            "<meta charset=\"windows-1252\">Café"
        );
    }

    #[test]
    fn bom_tem_prioridade() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("Olá".as_bytes());
        assert_eq!(decode_html(&bytes, Some("text/html; charset=windows-1252")), "Olá");
    }

    #[test]
    fn default_utf8() {
        assert_eq!(decode_html("Olá".as_bytes(), None), "Olá");
    }

    #[test]
    fn leitura_respeita_limite() {
        let data = vec![b'a'; MAX_BODY_BYTES + 1];
        assert!(matches!(
            read_limited(&data[..]),
            Err(ReadmeError::TooLarge(_))
        ));
        let ok = vec![b'a'; 1000];
        assert_eq!(read_limited(&ok[..]).unwrap().len(), 1000);
    }
}
