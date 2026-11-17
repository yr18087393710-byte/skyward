//! Skyward's own releases on GitHub: which is the newest, and the zip in
//! it for one phone, as the release workflow names each phone's.

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

/// Where GitHub answers which release of Skyward is the newest.
/// Our own releases, not upstream's: upstream's newest is ahead of this fork
/// in its numbering and carries builds this phone does not run, and the page
/// offered it as the newest. Our zips are named `skyward-v<tag>-pudding-…`, so
/// the codename match below finds them.
pub const LATEST: &str = "https://api.github.com/repos/yr18087393710-byte/skyward/releases/latest";

/// The version of GitHub's API the answer is read as.
const API_VERSION: &str = "2022-11-28";

/// The newest release, as the build of Skyward that asked sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newest {
    /// Its version, such as 0.1.4.
    pub version: String,
    /// It is newer than the build that asked.
    pub newer: bool,
    /// Its zip for the phone; none when it has none.
    pub build: Option<Build>,
    /// What the release says it changed, as the few lines worth reading on the
    /// phone's own screen: the release's words cut down, not all of them.
    pub notes: Vec<String>,
}

/// A phone's zip in a release, which holds Skyward's image for the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// The zip's own name, such as
    /// skyward-v0.1.4-ishtar-OS3.0.308.0.WMACNXM.zip.
    pub name: String,
    /// Where GitHub's API hands it out, which [`locate`] asks for where the
    /// file itself is.
    pub url: String,
    /// The release's own link to it on github.com.
    pub link: String,
    pub size: u64,
    /// Its SHA-256 as GitHub gives it, in hex.
    pub sha256: String,
}

/// Mirror prefixes that hand a github.com file over themselves. A build's zip
/// is a release asset, so these pass the whole link through; a mirror that
/// cannot reach it is dropped by measuring before the download starts.
const MIRRORS: [&str; 8] = [
    "https://ghfast.top/",
    "https://gh-proxy.com/",
    "https://ghproxy.net/",
    "https://gh.llkk.cc/",
    "https://gh.zwy.one/",
    "https://github.moeyy.xyz/",
    "https://ghproxy.homeboyc.cn/",
    "https://cdn.crashmc.com/",
];

impl Build {
    /// Where to download the zip from, in the order to try: the file's own
    /// short-lived address, where GitHub's API gave one, then the release's
    /// link on github.com, then the mirrors that pass that link through,
    /// which some networks reach where github.com itself is not.
    pub fn sources(&self, located: Option<String>) -> Vec<String> {
        let mut out: Vec<String> = located.into_iter().collect();
        out.push(self.link.clone());
        out.extend(MIRRORS.iter().map(|m| format!("{m}{}", self.link)));
        out
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
    /// What the release says of itself, as it was written.
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct Asset {
    url: String,
    name: String,
    size: u64,
    /// The digest GitHub took of the file as it was uploaded, such as
    /// `sha256:` and the SHA-256 in hex.
    digest: Option<String>,
    browser_download_url: String,
}

/// Why GitHub turned a question down, as it says.
#[derive(Deserialize)]
struct Message {
    message: String,
}

/// The newest release that `answer`, GitHub's answer at [`LATEST`], tells
/// of, and its zip for the phone `codename` names, as the build of version
/// `running` sees it.
pub fn newest(answer: &str, codename: &str, running: &str) -> Result<Newest> {
    let release: Release = serde_json::from_str(answer).context("read GitHub's answer")?;
    let tag = release.tag_name;
    let version = tag.strip_prefix('v').with_context(|| format!("the newest release is tagged {tag}, which is not v and a version"))?;
    let newest = semver::Version::parse(version).with_context(|| format!("the newest release is tagged {tag}"))?;
    // A build may name itself with the release's `v` in front, as a tag has it:
    // the version behind it is what is compared (2026-11-15, a build made with
    // SKYWARD_RELEASE=v0.3.1 could not ask GitHub at all).
    let this_version = running.strip_prefix('v').unwrap_or(running);
    let this = semver::Version::parse(this_version).with_context(|| format!("this build's version is {running}"))?;
    // Each zip is named skyward-, the tag, the phone's codename, and the
    // stock build the image is made on.
    let start = format!("skyward-{tag}-{codename}-");
    let build = match release.assets.into_iter().find(|a| a.name.starts_with(&start) && a.name.ends_with(".zip")) {
        None => None,
        Some(a) => {
            let sha256 = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).with_context(|| format!("GitHub gives no SHA-256 of {}", a.name))?;
            ensure!(sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()), "GitHub gives {} the SHA-256 {sha256}", a.name);
            Some(Build { sha256: sha256.to_lowercase(), name: a.name, url: a.url, link: a.browser_download_url, size: a.size })
        }
    };
    Ok(Newest { version: version.to_string(), newer: newest > this, build, notes: brief(release.body.as_deref()) })
}

/// The few lines of a release's own words worth showing on a phone's screen:
/// the bullet points written before the first heading, which are the ones
/// meant for here, or the whole of it where it has none.
fn brief(body: Option<&str>) -> Vec<String> {
    let Some(body) = body else { return Vec::new() };
    let head = match body.find("\n## ") {
        Some(at) => &body[..at],
        None => body,
    };
    let lines = both(head, body);
    lines
}

/// The points written at the head of `head`, or those of `body` where the head
/// says none.
fn both<'a>(head: &'a str, body: &'a str) -> Vec<String> {
    let from_head = points(head);
    if from_head.is_empty() { points(body) } else { from_head }
}

/// The bullet points of `text`, their marks taken off, each cut to a length
/// that reads in a line or two, and only the first handful of them.
fn points(text: &str) -> Vec<String> {
    const MOST: usize = 4;
    const LONGEST: usize = 64;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        // Bullets only: the headings and the prose around them do not fit. A
        // line of bold words is a heading, and `**` is not a bullet's mark.
        let rest = match line.strip_prefix("**") {
            Some(_) => continue,
            None => match line.strip_prefix(['-', '*', '·', '•']) {
                Some(rest) => rest,
                None => continue,
            },
        };
        let plain = plain_words(rest);
        if plain.is_empty() {
            continue;
        }
        let cut = match plain.chars().count() > LONGEST {
            true => {
                let mut short: String = plain.chars().take(LONGEST).collect();
                short.push('…');
                short
            }
            false => plain,
        };
        out.push(cut);
        if out.len() == MOST {
            break;
        }
    }
    out
}

/// `text` with the marks a release's words are written in taken off: bold and
/// italic stars, code ticks, and links kept by their own words.
fn plain_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('[') {
        out.push_str(&rest[..at]);
        let Some(close) = rest[at..].find(']') else { break };
        let Some(after) = rest[at + close..].strip_prefix("](") else { break };
        let Some(end) = after.find(')') else { break };
        out.push_str(&rest[at + 1..at + close]);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out.chars().filter(|c| !matches!(c, '*' | '`' | '#')).collect::<String>().trim().to_string()
}

/// Ask GitHub for Skyward's newest release, and its zip for the phone
/// `codename` names, as the build of version `running`.
pub fn ask(agent: &ureq::Agent, codename: &str, running: &str) -> Result<Newest> {
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("User-Agent", format!("Skyward/{running}"))
        .call()
        .with_context(|| format!("ask {LATEST}"))?;
    let status = response.status();
    let body = response.body_mut().read_to_string().with_context(|| format!("read the answer of {LATEST}"))?;
    if !status.is_success() {
        bail!(refusal(status, &body));
    }
    newest(&body, codename, running)
}

/// Where the file a build's `url` hands out is now, as GitHub's API sends
/// the build of version `running` that asks for it on: a short-lived
/// address of the file itself, away from github.com.
pub fn locate(agent: &ureq::Agent, url: &str, running: &str) -> Result<String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/octet-stream")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("User-Agent", format!("Skyward/{running}"))
        .config()
        .max_redirects(0)
        .build()
        .call()
        .with_context(|| format!("ask {url}"))?;
    let status = response.status();
    if status.is_redirection() {
        let location = response.headers().get(ureq::http::header::LOCATION).with_context(|| format!("GitHub answered {status}, and names no address"))?;
        return Ok(location.to_str().context("GitHub names an address that is not text")?.to_string());
    }
    ensure!(!status.is_success(), "GitHub sends the file itself in its answer, at no address of its own");
    let body = response.body_mut().read_to_string().with_context(|| format!("read the answer of {url}"))?;
    bail!(refusal(status, &body))
}

/// Why GitHub turned a question down, as it says, as when too many questions
/// came from the address that asked.
fn refusal(status: ureq::http::StatusCode, body: &str) -> String {
    match serde_json::from_str::<Message>(body) {
        Ok(m) => format!("GitHub answered {status}: {}", m.message),
        Err(_) => format!("GitHub answered {status}"),
    }
}

#[cfg(test)]
mod brief_tests {
    use super::brief;

    #[test]
    fn a_release_is_cut_to_the_few_lines_that_fit_a_phone() {
        let body = "## 新功能\n\n**U 盘文件管理**\n· 文件管理页顶部与「内部存储」并排显示「U 盘」，点一下切换，可浏览全部文件。\n· 高级页也有「U 盘管理」入口。\n· 第三条要点。\n· 第四条要点。\n· 第五条应被略过。\n\n## 修复\n· 修复项。\n";
        let notes = brief(Some(body));
        assert_eq!(notes.len(), 4, "four at most: {notes:?}");
        assert!(notes[0].starts_with("文件管理页顶部"), "marks off: {notes:?}");
        assert!(!notes.iter().any(|n| n.contains("第五条")), "the rest is left out");
        assert!(!notes.iter().any(|n| n.contains('*')), "no stars: {notes:?}");
    }

    #[test]
    fn a_line_too_long_for_the_screen_is_cut_short() {
        let long = "· ".to_string() + &"字".repeat(200);
        let notes = brief(Some(&long));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].chars().count() <= 65, "cut to a line: {}", notes[0].chars().count());
        assert!(notes[0].ends_with('…'));
    }

    #[test]
    fn a_release_with_nothing_to_say_gives_nothing() {
        assert!(brief(None).is_empty());
        assert!(brief(Some("## 新功能\n\n**只有标题**\n")).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What GitHub answered of the newest release on 2026-11-07, with the
    /// parts that do not matter here left out.
    const ANSWERED_013: &str = r#"{"url":"https://api.github.com/repos/kirklin/skyward/releases/437365758","tag_name":"v0.1.3","name":"v0.1.3","draft":false,"prerelease":false,"assets":[{"url":"https://api.github.com/repos/kirklin/skyward/releases/assets/719751468","id":719751468,"name":"SHA256SUMS","label":"","content_type":"application/octet-stream","state":"uploaded","size":2337,"digest":"sha256:052f9ca757baeb2eac78c8e546bcc54178f6bb9bf9e5bde53fa81b3f48448957","browser_download_url":"https://github.com/kirklin/skyward/releases/download/v0.1.3/SHA256SUMS"},{"url":"https://api.github.com/repos/kirklin/skyward/releases/assets/719751598","id":719751598,"name":"skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip","label":"Xiaomi 13 Ultra (ishtar)","content_type":"application/zip","state":"uploaded","size":42739608,"digest":"sha256:62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534","browser_download_url":"https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip"},{"url":"https://api.github.com/repos/kirklin/skyward/releases/assets/719751747","id":719751747,"name":"skyward-v0.1.3-nezha-OS4.0.4.0.XPACNXM.zip","label":"Xiaomi 17 Ultra (nezha)","content_type":"application/zip","state":"uploaded","size":43145196,"digest":"sha256:9f6165079529418d8f6635beddfdd9abf9a70c89d69eb467a9922b0db69bfaf1","browser_download_url":"https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-nezha-OS4.0.4.0.XPACNXM.zip"}]}"#;

    #[test]
    fn the_newest_release_gives_the_zip_for_the_phone() {
        let n = newest(ANSWERED_013, "ishtar", "0.1.2").unwrap();
        assert_eq!(n.version, "0.1.3");
        assert!(n.newer);
        assert_eq!(
            n.build,
            Some(Build {
                name: "skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip".into(),
                url: "https://api.github.com/repos/kirklin/skyward/releases/assets/719751598".into(),
                link: "https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip".into(),
                size: 42739608,
                sha256: "62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534".into(),
            })
        );
        assert_eq!(newest(ANSWERED_013, "nezha", "0.1.2").unwrap().build.unwrap().size, 43145196);
    }

    #[test]
    fn a_zip_is_downloaded_from_where_github_sends_it_then_from_its_link() {
        let build = newest(ANSWERED_013, "ishtar", "0.1.2").unwrap().build.unwrap();
        let link = "https://github.com/kirklin/skyward/releases/download/v0.1.3/skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip";
        let sources = build.sources(Some("https://release-assets.githubusercontent.com/a?sig=1".into()));
        assert_eq!(sources[0], "https://release-assets.githubusercontent.com/a?sig=1");
        assert_eq!(sources[1], link, "the link on github.com comes after the file's own address");
        assert!(sources.iter().any(|s| s == &format!("https://ghfast.top/{link}")), "the mirrors hand the link over themselves");
        let sources = build.sources(None);
        assert_eq!(sources[0], link, "where GitHub's API gave no address, the link comes first");
        assert!(sources.len() > 1, "the mirrors are tried after it");
    }

    /// A server on this computer that answers one request with `answer`,
    /// and gives the request it had.
    fn server(answer: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/repos/kirklin/skyward/releases/assets/719751598", listener.local_addr().unwrap());
        let held = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 512];
            while !request.ends_with(b"\r\n\r\n") {
                let n = s.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..n]);
            }
            s.write_all(answer.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, held)
    }

    #[test]
    fn github_s_api_sends_a_zip_on_to_where_the_file_is() {
        let agent = crate::http::agent(Default::default());
        let sent_on = "HTTP/1.1 302 Found\r\nLocation: https://release-assets.githubusercontent.com/a?sig=1\r\nContent-Length: 0\r\n\r\n";
        let (url, held) = server(sent_on.into());
        assert_eq!(locate(&agent, &url, "0.1.2").unwrap(), "https://release-assets.githubusercontent.com/a?sig=1", "the address, not followed");
        let request = held.join().unwrap().to_lowercase();
        assert!(request.contains("accept: application/octet-stream\r\n"), "asked for the file itself: {request}");
        assert!(request.contains("user-agent: skyward/0.1.2\r\n"), "{request}");
        let why = r#"{"message":"API rate limit exceeded for 203.0.113.7"}"#;
        let (url, _) = server(format!("HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{why}", why.len()));
        assert_eq!(locate(&agent, &url, "0.1.2").unwrap_err().to_string(), "GitHub answered 403 Forbidden: API rate limit exceeded for 203.0.113.7");
        let (url, _) = server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nPK".into());
        assert!(locate(&agent, &url, "0.1.2").is_err(), "the file itself, with no address to download it from again");
    }

    #[test]
    fn a_release_is_newer_only_than_an_older_build() {
        assert!(!newest(ANSWERED_013, "ishtar", "0.1.3").unwrap().newer, "the same version");
        assert!(!newest(ANSWERED_013, "ishtar", "0.2.0").unwrap().newer, "a build ahead of every release");
        assert!(newest(ANSWERED_013, "ishtar", "0.1.3-rc.1").unwrap().newer, "a build before the release it led to");
        assert!(newest(ANSWERED_013, "ishtar", "0.1").is_err(), "a version this build cannot have");
    }

    #[test]
    fn a_phone_is_found_by_its_whole_codename() {
        assert_eq!(newest(ANSWERED_013, "fuxi", "0.1.2").unwrap().build, None, "a phone the release has no zip for");
        assert_eq!(newest(ANSWERED_013, "ish", "0.1.2").unwrap().build, None, "the start of another phone's codename");
        assert_eq!(newest(ANSWERED_013, "SHA256SUMS", "0.1.2").unwrap().build, None);
    }

    #[test]
    fn a_zip_needs_its_sha256_and_a_release_its_version() {
        let unsummed = ANSWERED_013.replace("\"digest\":\"sha256:62b66f7d13a0eb6c9af3359b5ec6f9c223fd9a86b1f8abab3e9d2646cb2ce534\"", "\"digest\":null");
        let e = newest(&unsummed, "ishtar", "0.1.2").unwrap_err();
        assert_eq!(e.to_string(), "GitHub gives no SHA-256 of skyward-v0.1.3-ishtar-OS3.0.308.0.WMACNXM.zip");
        assert!(newest(&unsummed, "nezha", "0.1.2").is_ok(), "another phone's zip has its own");
        let untagged = ANSWERED_013.replace("\"tag_name\":\"v0.1.3\"", "\"tag_name\":\"nightly\"");
        assert!(newest(&untagged, "ishtar", "0.1.2").is_err());
        assert!(newest("{}", "ishtar", "0.1.2").is_err());
    }
}
