//! A page taken from the network, cut down to what the recovery's browser
//! shows: its title, the words on it, and the links to go on with. There is
//! no engine here: the markup is read once, the way a reader would take it,
//! so that a page's words show on a phone whose screen is a canvas.

use anyhow::{bail, Context, Result};
use std::io::Read;

/// How much of a page is read at most.
pub const MAX_BYTES: u64 = 2 << 20;
/// The most lines kept of a page's words.
pub const LINES: usize = 600;
/// The most characters kept of one line.
pub const LINE_CHARS: usize = 512;
/// The most links kept of a page.
pub const LINKS: usize = 60;

/// A link on a page: the words it shows, and where it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub text: String,
    pub url: String,
}

/// A page as the browser shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// The address it was read from.
    pub url: String,
    /// What its title says, where it says one.
    pub title: String,
    /// Its words, a line each.
    pub text: Vec<String>,
    pub links: Vec<Link>,
}

impl Page {
    /// What the page is called: its title, or the address where it says
    /// none.
    pub fn called(&self) -> &str {
        match self.title.trim() {
            "" => &self.url,
            title => title,
        }
    }
}

/// The address `input` asks for, as the page reads it: `https://` put in
/// front where no scheme is given, and nothing for anything but the web.
pub fn address(input: &str) -> Option<String> {
    let at = input.trim();
    if at.is_empty() {
        return None;
    }
    let lower = at.to_ascii_lowercase();
    let full = match lower.starts_with("https://") || lower.starts_with("http://") {
        true => at.to_string(),
        false if lower.contains("://") => return None,
        false => format!("https://{at}"),
    };
    let rest = full.split_once("://").map(|(_, rest)| rest)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host).split(':').next().unwrap_or("");
    match host.is_empty() || host.starts_with('.') || host.ends_with('.') {
        true => None,
        false => Some(full),
    }
}

/// Where `href` goes from the page at `base`, where it goes to the web:
/// root-relative and document-relative links are made whole, and anything
/// else — another scheme, a bare fragment — is nothing to go to.
pub fn absolute(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) {
        // Keep the address as written; only the scheme is checked.
        return match rest.is_empty() {
            true => None,
            false => Some(href.to_string()),
        };
    }
    if lower.starts_with("//") {
        let scheme = base.split_once("://").map(|(s, _)| s).unwrap_or("https");
        return Some(format!("{scheme}:{href}"));
    }
    if href.contains(':') && !href.starts_with('/') {
        // mailto:, javascript:, tel: and the rest are not pages.
        let scheme = href.split(':').next().unwrap_or("");
        if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return None;
        }
    }
    let (scheme, rest) = base.split_once("://")?;
    let (authority, _) = match rest.find(['/', '?', '#']) {
        Some(k) => (&rest[..k], &rest[k..]),
        None => (rest, ""),
    };
    let origin = format!("{scheme}://{authority}");
    let joined = match href.starts_with('/') {
        true => format!("{origin}{href}"),
        false => {
            let path = base.split(['?', '#']).next().unwrap_or(base);
            let dir = match path.rfind('/') {
                Some(k) if k + 1 > origin.len() => &path[..=k],
                _ => return Some(clean(&format!("{origin}/{href}"))),
            };
            format!("{dir}{href}")
        }
    };
    Some(clean(&joined))
}

/// `url` with its path resolved of the `.` and `..` steps it has, its query
/// and fragment kept.
fn clean(url: &str) -> String {
    let (head, tail) = match url.split_once("://") {
        Some((scheme, rest)) => match rest.find('/') {
            Some(k) => (format!("{scheme}://{}", &rest[..k]), rest[k..].to_string()),
            None => (format!("{scheme}://{rest}"), "/".to_string()),
        },
        None => (String::new(), url.to_string()),
    };
    let (path, rest) = match tail.find(['?', '#']) {
        Some(k) => (&tail[..k], &tail[k..]),
        None => (tail.as_str(), ""),
    };
    let mut steps: Vec<&str> = Vec::new();
    for (k, step) in path.split('/').enumerate() {
        match step {
            "." => {}
            ".." => {
                if steps.len() > 1 {
                    steps.pop();
                }
            }
            other => {
                if k == 0 {
                    steps.push("");
                } else {
                    steps.push(other);
                }
            }
        }
    }
    let path = steps.join("/");
    let path = match path.is_empty() {
        true => "/".to_string(),
        false => path,
    };
    format!("{head}{path}{rest}")
}

/// The page `body` holds, read from `url`, whose content type the server
/// gave as `kind`.
pub fn distill(url: &str, kind: Option<&str>, body: &str) -> Page {
    let markup = match kind {
        Some(kind) => kind.to_ascii_lowercase().contains("html") || kind.to_ascii_lowercase().contains("xml"),
        None => body.contains('<'),
    };
    let (title, text, links) = match markup {
        true => read_markup(url, body),
        false => (String::new(), lines_of(&plain(body)), Vec::new()),
    };
    Page { url: url.to_string(), title, text, links }
}

/// The words of a page that is not markup: its lines, as they are.
fn plain(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\r', "\n")
}

/// A page's title, words and links, read out of its markup.
fn read_markup(base: &str, body: &str) -> (String, Vec<String>, Vec<Link>) {
    let mut title = String::new();
    let mut words = String::new();
    let mut links: Vec<Link> = Vec::new();
    let mut open: Option<Link> = None;
    let (mut skipping, mut in_title, mut link_text) = (0usize, false, String::new());
    let bytes = body.as_bytes();
    let mut at = 0usize;
    // A page's own line breaks are spaces: the lines of the words are the
    // blocks' doing, as a reader sees them.
    let push = |words: &mut String, text: &str| {
        let text = decode(text);
        for c in text.chars() {
            match c {
                '\n' | '\r' | '\t' => words.push(' '),
                c => words.push(c),
            }
        }
    };
    while at < bytes.len() {
        if bytes[at] == b'<' {
            // A comment runs to its end, whatever it holds.
            if body[at..].starts_with("<!--") {
                match body[at..].find("-->") {
                    Some(k) => {
                        at += k + 3;
                        continue;
                    }
                    None => break,
                }
            }
            let Some(end) = body[at..].find('>') else { break };
            let inner = body[at + 1..at + end].trim();
            let closing = inner.starts_with('/');
            let name: String = inner
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            at += end + 1;
            match (closing, name.as_str()) {
                (false, "script") | (false, "style") | (false, "noscript") | (false, "template") => skipping += 1,
                (true, "script") | (true, "style") | (true, "noscript") | (true, "template") => skipping = skipping.saturating_sub(1),
                _ if skipping > 0 => {}
                (false, "title") => in_title = true,
                (true, "title") => in_title = false,
                (false, "a") => {
                    let href = attribute(inner, "href");
                    open = href.as_deref().and_then(|href| absolute(base, href)).map(|url| Link { text: String::new(), url });
                    link_text.clear();
                }
                (true, "a") => {
                    if let Some(mut link) = open.take() {
                        let text = squeeze(&decode(&link_text));
                        link.text = match text.is_empty() {
                            true => short(&link.url),
                            false => text,
                        };
                        if links.len() < LINKS && links.iter().all(|l| l.url != link.url) {
                            links.push(link);
                        }
                    }
                    link_text.clear();
                }
                _ if in_title => {}
                (_, name) if is_block(name) => words.push('\n'),
                _ => {}
            }
            continue;
        }
        let next = body[at..].find('<').map_or(bytes.len(), |k| at + k);
        let text = &body[at..next];
        if in_title {
            if title.len() + text.len() < 200 {
                title.push_str(text);
            }
        } else if skipping == 0 {
            if open.is_some() && link_text.len() < 200 {
                link_text.push_str(text);
            }
            push(&mut words, text);
        }
        at = next;
    }
    let title = squeeze(&decode(&title));
    (title, lines_of_markup(&words), links)
}

/// A markup page's words as lines: trimmed, without the blank lines the
/// blocks leave between them, cut to what the page keeps.
fn lines_of_markup(words: &str) -> Vec<String> {
    lines_of(words).into_iter().filter(|l| !l.is_empty()).collect()
}

/// The value of `name` in a tag's insides, as `href="..."`, `href='...'` or
/// `href=...`.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(k) = lower[from..].find(name) {
        let start = from + k;
        let after = &tag[start + name.len()..];
        let before_ok = tag[..start].chars().next_back().is_none_or(|c| c.is_whitespace());
        let after_trimmed = after.trim_start();
        if before_ok && after_trimmed.starts_with('=') {
            let value = after_trimmed[1..].trim_start();
            let value = value.trim_start_matches(['"', '\'']);
            let quote = after_trimmed[1..].trim_start().chars().next();
            let end = match quote {
                Some(q @ ('"' | '\'')) => value.find(q).unwrap_or(value.len()),
                _ => value.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(value.len()),
            };
            return Some(value[..end].to_string());
        }
        from = start + name.len();
        if from >= lower.len() {
            break;
        }
    }
    None
}

/// The tags a page's words are broken at.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "br"
            | "li"
            | "tr"
            | "td"
            | "th"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "aside"
            | "blockquote"
            | "pre"
            | "ul"
            | "ol"
            | "table"
            | "form"
            | "hr"
            | "figure"
            | "figcaption"
            | "dd"
            | "dt"
            | "dl"
    )
}

/// The words as lines: each trimmed of spaces, without runs of blank lines,
/// cut to what the page keeps.
fn lines_of(words: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in words.replace('\r', "\n").split('\n') {
        let line = squeeze(line);
        if line.is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) && out.len() < LINES {
                out.push(String::new());
            }
            continue;
        }
        if out.len() >= LINES {
            break;
        }
        let mut line = line;
        if line.chars().count() > LINE_CHARS {
            line = line.chars().take(LINE_CHARS).collect();
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// `text` with its spaces, tabs and newlines run together and trimmed.
fn squeeze(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        match c.is_whitespace() {
            true => space = !out.is_empty(),
            false => {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(c);
            }
        }
    }
    out
}

/// The address as a link or a page shows it where the words are the page's
/// own concern: its host and path, without the scheme, cut short.
pub fn short(url: &str) -> String {
    let short = url.split_once("://").map_or(url, |(_, rest)| rest);
    let short = short.strip_prefix("www.").unwrap_or(short);
    match short.chars().count() > 64 {
        true => short.chars().take(63).collect::<String>() + "…",
        false => short.to_string(),
    }
}

/// The characters `&...;` names, read as the page means them.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    while let Some(k) = text[at..].find('&') {
        out.push_str(&text[at..at + k]);
        let start = at + k;
        let Some(end) = text[start..].find(';').filter(|&end| end <= 12) else {
            out.push('&');
            at = start + 1;
            continue;
        };
        let name = &text[start + 1..start + end];
        let read = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "middot" => Some('·'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "trade" => Some('™'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            _ => {
                let code = match name.strip_prefix('#') {
                    Some(hex) => match hex.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => hex.parse::<u32>().ok(),
                    },
                    None => None,
                };
                code.and_then(char::from_u32)
            }
        };
        match read {
            Some(c) => {
                out.push(c);
                at = start + end + 1;
            }
            None => {
                out.push('&');
                at = start + 1;
            }
        }
    }
    out.push_str(&text[at.min(text.len())..]);
    out
}

/// Read the page at `url` over the network the phone is on.
pub fn get(servers: crate::http::Servers, url: &str) -> Result<Page> {
    let agent = crate::http::agent(servers);
    let mut response = agent.get(url).call().with_context(|| format!("open {url}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        bail!("the server answered {status}");
    }
    let kind = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut body = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .reader()
        .read_to_end(&mut body)
        .with_context(|| format!("read {url}"))?;
    Ok(distill(url, kind.as_deref(), &String::from_utf8_lossy(&body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_without_a_scheme_is_taken_as_the_web() {
        assert_eq!(address("example.com/a").as_deref(), Some("https://example.com/a"));
        assert_eq!(address("  http://example.com  ").as_deref(), Some("http://example.com"));
        assert_eq!(address("ftp://example.com/x"), None, "only the web");
        assert_eq!(address(""), None);
        assert_eq!(address("https://"), None, "no host");
    }

    #[test]
    fn links_are_made_whole_from_the_page_they_are_on() {
        let base = "https://example.com/a/b/page.html?x=1";
        assert_eq!(absolute(base, "/root"), Some("https://example.com/root".into()));
        assert_eq!(absolute(base, "other.html"), Some("https://example.com/a/b/other.html".into()));
        assert_eq!(absolute(base, "../up.html"), Some("https://example.com/a/up.html".into()));
        assert_eq!(absolute(base, "https://other.test/x"), Some("https://other.test/x".into()));
        assert_eq!(absolute(base, "//cdn.test/x"), Some("https://cdn.test/x".into()));
        assert_eq!(absolute(base, "mailto:a@b"), None);
        assert_eq!(absolute(base, "#top"), None);
        assert_eq!(absolute(base, "javascript:void(0)"), None);
    }

    #[test]
    fn a_pages_words_its_title_and_its_links_are_read_out_of_the_markup() {
        let body = r#"
            <html><head><title>  What &amp; Why </title>
            <style>body { color: red }</style>
            <script>var x = "<p>not words</p>";</script></head>
            <body><!-- a comment --><h1>Hello</h1>
            <p>First   line<br>second &lt;line&gt; &#65; &#x42;</p>
            <a href="/next">Next page</a> and <a href='https://other.test/go'>Other</a>
            <a href="/next">Again</a>
            <noscript>no scripts</noscript></body></html>"#;
        let page = distill("https://example.com/a/page", Some("text/html; charset=utf-8"), body);
        assert_eq!(page.title, "What & Why");
        assert_eq!(page.text, ["Hello", "First line", "second <line> A B", "Next page and Other Again"]);
        assert_eq!(
            page.links,
            [
                Link { text: "Next page".into(), url: "https://example.com/next".into() },
                Link { text: "Other".into(), url: "https://other.test/go".into() }
            ],
            "a link seen twice is one link, and what a script holds is no word of the page"
        );
        assert_eq!(page.called(), "What & Why");
    }

    #[test]
    fn a_page_that_is_not_markup_keeps_its_own_lines() {
        let page = distill("https://example.com/readme.txt", Some("text/plain"), "one\ntwo\n\nthree\n");
        assert_eq!((page.title.as_str(), page.text.clone()), ("", vec!["one".to_string(), "two".into(), "".into(), "three".into()]));
        assert!(page.links.is_empty());
        assert_eq!(page.called(), "https://example.com/readme.txt");
    }

    #[test]
    fn a_link_with_no_words_shows_where_it_goes() {
        let page = distill("https://example.com/", None, r#"<a href="https://example.com/file.zip"></a>"#);
        assert_eq!(page.links[0].text, "example.com/file.zip");
    }
}