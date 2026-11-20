//! Embedded documentation bundle.
//!
//! The Markdown and image files under `docs/` are compiled into the API binary, so the
//! handbook is readable from the control plane itself (including air-gapped installs).
//! Nothing is ever read from the filesystem at request time: only files that were
//! embedded, and that appear in the manifest built here, can be served.
//!
//! `docs/planning` (internal drafts) is excluded.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use pulldown_cmark::{html, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rust_embed::RustEmbed;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(RustEmbed)]
#[folder = "../../docs/"]
#[exclude = "planning/*"]
struct Embedded;

/// URL prefix of the documentation endpoints.
pub const DOCS_URL: &str = "/api/v1/docs";
const ASSETS_PREFIX: &str = "assets/";

/// Directory name → (display title, sort rank). Unknown directories sort last by name.
const CATEGORIES: &[(&str, &str)] = &[
    ("overview", "Overview"),
    ("guide", "User guide"),
    ("development", "Development"),
    ("architecture", "Architecture"),
    ("decisions", "Decisions"),
    ("security", "Security"),
    ("examples", "Examples"),
    ("assets", "Brand assets"),
];

#[derive(Debug, Clone)]
pub struct DocPage {
    /// URL slug, for example `architecture/overview`.
    pub slug: String,
    /// Path inside `docs/`, for example `architecture/overview.md`.
    pub path: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub order: i32,
    /// Markdown body with any front matter removed. Links are as authored.
    pub markdown: String,
    /// Heading anchors (GitHub-style slugs) available in this page.
    pub anchors: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct DocAsset {
    pub path: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct DocsBundle {
    pages: Vec<DocPage>,
    by_slug: HashMap<String, usize>,
    by_path: HashMap<String, usize>,
    assets: BTreeMap<String, DocAsset>,
}

/// Where a link inside a page points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Absolute URL, `mailto:`, server-absolute path: left untouched.
    External,
    /// `#anchor` inside the current page.
    SamePage {
        anchor: String,
    },
    Page {
        slug: String,
        anchor: Option<String>,
    },
    Asset {
        path: String,
    },
    /// A relative link that does not resolve inside the bundle.
    Broken(String),
}

// ------------------------------------------------------------------ serialisation

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocSummary {
    pub slug: String,
    pub title: String,
    pub description: String,
    /// API route of the page, e.g. `/api/v1/docs/architecture/overview`.
    pub url: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocCategory {
    pub id: String,
    pub title: String,
    pub pages: Vec<DocSummary>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocIndex {
    pub categories: Vec<DocCategory>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocPageView {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub url: String,
    /// Source path inside `docs/`.
    pub path: String,
    /// Markdown with relative links rewritten to documentation routes.
    pub markdown: String,
}

// ---------------------------------------------------------------------- the bundle

fn markdown_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// The process-wide bundle, built once from the embedded files.
pub fn bundle() -> &'static DocsBundle {
    static BUNDLE: OnceLock<DocsBundle> = OnceLock::new();
    BUNDLE.get_or_init(DocsBundle::load)
}

fn is_excluded(path: &str) -> bool {
    path.starts_with("planning/") || path.split('/').any(|part| part.starts_with('.'))
}

fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn slug_for(path: &str) -> String {
    let stem = path.strip_suffix(".md").unwrap_or(path);
    match stem.rsplit_once('/') {
        Some((dir, "README")) => dir.to_string(),
        None if stem == "README" => "index".to_string(),
        _ => stem.to_string(),
    }
}

fn category_for(path: &str) -> String {
    match path.split_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => "overview".to_string(),
    }
}

/// Minimal `key: value` front matter. Returns the metadata and the remaining body.
fn split_front_matter(source: &str) -> (HashMap<String, String>, &str) {
    let mut meta = HashMap::new();
    let Some(rest) = source.strip_prefix("---\n") else {
        return (meta, source);
    };
    let Some(end) = rest.find("\n---") else {
        return (meta, source);
    };
    let after = &rest[end + 4..];
    let body = after.strip_prefix('\n').unwrap_or(after);
    for line in rest[..end].lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            meta.insert(key.trim().to_ascii_lowercase(), value.to_string());
        }
    }
    (meta, body)
}

/// GitHub-compatible heading slug: lowercase, punctuation dropped, spaces to hyphens.
pub fn heading_slug(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                Some(c.to_lowercase().collect::<String>())
            } else if c == ' ' {
                Some("-".to_string())
            } else {
                None
            }
        })
        .collect()
}

/// One entry per heading, in document order, with GitHub-style de-duplicated ids.
fn heading_ids(events: &[Event<'_>]) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut current: Option<String> = None;
    for event in events {
        match event {
            Event::Start(Tag::Heading { .. }) => current = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(text) = current.take() {
                    let base = heading_slug(&text);
                    let count = seen.entry(base.clone()).or_insert(0);
                    let id = if *count == 0 {
                        base.clone()
                    } else {
                        format!("{base}-{count}")
                    };
                    *count += 1;
                    ids.push(id);
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(buffer) = current.as_mut() {
                    buffer.push_str(text);
                }
            }
            _ => {}
        }
    }
    ids
}

fn first_heading_and_description(markdown: &str) -> (Option<String>, String) {
    let mut title: Option<String> = None;
    let mut description = String::new();
    let mut in_h1: Option<String> = None;
    let mut in_paragraph = false;
    let mut quote_or_list_depth = 0usize;
    for event in Parser::new_ext(markdown, markdown_options()) {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) if title.is_none() => in_h1 = Some(String::new()),
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                if let Some(text) = in_h1.take() {
                    title = Some(text.trim().to_string());
                }
            }
            Event::Start(Tag::BlockQuote(_) | Tag::List(_) | Tag::Table(_)) => {
                quote_or_list_depth += 1
            }
            Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Table) => {
                quote_or_list_depth = quote_or_list_depth.saturating_sub(1)
            }
            Event::Start(Tag::Paragraph) if description.is_empty() && quote_or_list_depth == 0 => {
                in_paragraph = true
            }
            Event::End(TagEnd::Paragraph) => in_paragraph = false,
            Event::Text(text) | Event::Code(text) => {
                if let Some(buffer) = in_h1.as_mut() {
                    buffer.push_str(&text);
                } else if in_paragraph {
                    description.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if in_paragraph => description.push(' '),
            _ => {}
        }
        if title.is_some() && !description.is_empty() && !in_paragraph {
            break;
        }
    }
    (title, truncate(description.trim(), 200))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches(|c: char| !c.is_alphanumeric()))
}

fn category_rank(id: &str) -> (usize, String) {
    let rank = CATEGORIES
        .iter()
        .position(|(key, _)| *key == id)
        .unwrap_or(CATEGORIES.len());
    (rank, id.to_string())
}

pub fn category_title(id: &str) -> String {
    CATEGORIES
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, title)| (*title).to_string())
        .unwrap_or_else(|| {
            let mut chars = id.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
}

impl DocsBundle {
    fn load() -> Self {
        let mut pages = Vec::new();
        let mut assets = BTreeMap::new();
        for path in Embedded::iter() {
            let path = path.to_string();
            if is_excluded(&path) {
                continue;
            }
            let Some(file) = Embedded::get(&path) else {
                continue;
            };
            if path.ends_with(".md") {
                let Ok(source) = String::from_utf8(file.data.to_vec()) else {
                    continue;
                };
                pages.push(build_page(&path, &source));
            } else {
                assets.insert(
                    path.clone(),
                    DocAsset {
                        content_type: content_type(&path),
                        bytes: file.data.to_vec(),
                        path,
                    },
                );
            }
        }
        pages.sort_by(|a, b| {
            category_rank(&a.category)
                .cmp(&category_rank(&b.category))
                .then(a.order.cmp(&b.order))
                .then(a.path.cmp(&b.path))
        });
        let by_slug = pages
            .iter()
            .enumerate()
            .map(|(i, page)| (page.slug.clone(), i))
            .collect();
        let by_path = pages
            .iter()
            .enumerate()
            .map(|(i, page)| (page.path.clone(), i))
            .collect();
        Self {
            pages,
            by_slug,
            by_path,
            assets,
        }
    }

    pub fn pages(&self) -> &[DocPage] {
        &self.pages
    }

    pub fn page(&self, slug: &str) -> Option<&DocPage> {
        self.by_slug.get(slug).map(|i| &self.pages[*i])
    }

    pub fn asset(&self, path: &str) -> Option<&DocAsset> {
        self.assets.get(path)
    }

    pub fn assets(&self) -> impl Iterator<Item = &DocAsset> {
        self.assets.values()
    }

    pub fn index(&self) -> DocIndex {
        let mut categories: Vec<DocCategory> = Vec::new();
        for page in &self.pages {
            if categories.last().map(|c| c.id.as_str()) != Some(page.category.as_str()) {
                categories.push(DocCategory {
                    id: page.category.clone(),
                    title: category_title(&page.category),
                    pages: Vec::new(),
                });
            }
            if let Some(category) = categories.last_mut() {
                category.pages.push(DocSummary {
                    slug: page.slug.clone(),
                    title: page.title.clone(),
                    description: page.description.clone(),
                    url: page_url(&page.slug),
                });
            }
        }
        DocIndex { categories }
    }

    pub fn view(&self, page: &DocPage) -> DocPageView {
        DocPageView {
            slug: page.slug.clone(),
            title: page.title.clone(),
            description: page.description.clone(),
            category: page.category.clone(),
            url: page_url(&page.slug),
            path: page.path.clone(),
            markdown: self.rewrite_markdown(page),
        }
    }

    // ---------------------------------------------------------------- link handling

    /// Resolve `href`, written inside the page at `from_path`, against the bundle.
    pub fn resolve(&self, from_path: &str, href: &str) -> LinkTarget {
        let href = href.trim();
        if href.is_empty() {
            return LinkTarget::External;
        }
        if let Some(anchor) = href.strip_prefix('#') {
            return LinkTarget::SamePage {
                anchor: percent_decode(anchor),
            };
        }
        if href.starts_with('/') || has_scheme(href) {
            return LinkTarget::External;
        }
        let (path_part, anchor) = match href.split_once('#') {
            Some((p, a)) => (p, Some(percent_decode(a))),
            None => (href, None),
        };
        let path_part = path_part.split('?').next().unwrap_or("");
        let decoded = percent_decode(path_part);

        let mut segments: Vec<&str> = from_path.split('/').collect();
        segments.pop(); // the file name
        for part in decoded.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if segments.pop().is_none() {
                        return LinkTarget::Broken("points outside docs/".into());
                    }
                }
                other => segments.push(other),
            }
        }
        let joined = segments.join("/");

        if let Some(i) = self.by_path.get(&joined) {
            return LinkTarget::Page {
                slug: self.pages[*i].slug.clone(),
                anchor,
            };
        }
        if self.assets.contains_key(&joined) {
            return LinkTarget::Asset { path: joined };
        }
        // A directory link resolves to that directory's README.
        let readme = if joined.is_empty() {
            "README.md".to_string()
        } else {
            format!("{joined}/README.md")
        };
        if let Some(i) = self.by_path.get(&readme) {
            return LinkTarget::Page {
                slug: self.pages[*i].slug.clone(),
                anchor,
            };
        }
        LinkTarget::Broken(format!("no such page or asset: {joined}"))
    }

    /// The rewritten destination for `href`, or `None` when it should stay as written.
    fn rewrite(&self, from_path: &str, href: &str) -> Option<String> {
        match self.resolve(from_path, href) {
            LinkTarget::Page { slug, anchor } => Some(match anchor {
                Some(anchor) if !anchor.is_empty() => format!("{}#{anchor}", page_url(&slug)),
                _ => page_url(&slug),
            }),
            LinkTarget::Asset { path } => Some(asset_url(&path)),
            _ => None,
        }
    }

    /// Markdown with relative links and image paths rewritten to documentation routes.
    pub fn rewrite_markdown(&self, page: &DocPage) -> String {
        let source = page.markdown.as_str();
        let parser = Parser::new_ext(source, markdown_options());
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();

        for (name, def) in parser.reference_definitions().iter() {
            let _ = name;
            if let Some(new) = self.rewrite(&page.path, &def.dest) {
                if let Some(range) = replace_in(source, &def.span, &def.dest) {
                    edits.push((range, new));
                }
            }
        }
        for (event, range) in parser.into_offset_iter() {
            let dest = match &event {
                Event::Start(Tag::Link { dest_url, .. })
                | Event::Start(Tag::Image { dest_url, .. }) => dest_url,
                _ => continue,
            };
            let Some(new) = self.rewrite(&page.path, dest) else {
                continue;
            };
            // Inline links read `[text](dest "title")`; the destination is the last
            // `](dest` (or `](<dest`) in the node's source range.
            let slice = &source[range.clone()];
            for marker in [format!("]({dest}"), format!("](<{dest}")] {
                if let Some(at) = slice.rfind(&marker) {
                    let start = range.start + at + marker.len() - dest.len();
                    edits.push((start..start + dest.len(), new.clone()));
                    break;
                }
            }
        }

        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        edits.dedup_by_key(|(range, _)| range.start);
        let mut out = source.to_string();
        for (range, new) in edits {
            out.replace_range(range, &new);
        }
        out
    }

    /// Rendered HTML body (no document wrapper) with rewritten links and heading ids.
    /// Raw HTML in the source is escaped, never interpreted.
    pub fn render_html(&self, page: &DocPage) -> String {
        let events: Vec<Event> = Parser::new_ext(&page.markdown, markdown_options()).collect();
        let ids = heading_ids(&events);
        let mut ids = ids.into_iter();
        let mapped = events.into_iter().map(|event| match event {
            Event::Start(Tag::Heading {
                level,
                classes,
                attrs,
                ..
            }) => Event::Start(Tag::Heading {
                level,
                id: ids.next().map(CowStr::from),
                classes,
                attrs,
            }),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Link {
                link_type,
                dest_url: self.dest(&page.path, dest_url),
                title,
                id,
            }),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Image {
                link_type,
                dest_url: self.dest(&page.path, dest_url),
                title,
                id,
            }),
            Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
            other => other,
        });
        let mut out = String::new();
        html::push_html(&mut out, mapped);
        out
    }

    fn dest<'a>(&self, from: &str, dest: CowStr<'a>) -> CowStr<'a> {
        match self.rewrite(from, &dest) {
            Some(new) => CowStr::from(new),
            None => dest,
        }
    }

    // ----------------------------------------------------------------- link audit

    /// Every relative link and anchor in the bundle that does not resolve. Empty when
    /// the documentation is internally consistent.
    pub fn broken_links(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for page in &self.pages {
            for dest in link_destinations(&page.markdown) {
                let problem = match self.resolve(&page.path, &dest) {
                    LinkTarget::Broken(reason) => Some(reason),
                    LinkTarget::SamePage { anchor } if !page.anchors.contains(&anchor) => {
                        Some(format!("no heading for anchor #{anchor}"))
                    }
                    LinkTarget::Page {
                        slug,
                        anchor: Some(anchor),
                    } if !anchor.is_empty()
                        && self
                            .page(&slug)
                            .is_some_and(|target| !target.anchors.contains(&anchor)) =>
                    {
                        Some(format!("no heading for anchor #{anchor} in {slug}"))
                    }
                    _ => None,
                };
                if let Some(problem) = problem {
                    problems.push(format!("{}: `{dest}` {problem}", page.path));
                }
            }
        }
        problems
    }
}

/// Every link, image and reference-definition destination in a Markdown document.
fn link_destinations(markdown: &str) -> Vec<String> {
    let parser = Parser::new_ext(markdown, markdown_options());
    let mut out: Vec<String> = parser
        .reference_definitions()
        .iter()
        .map(|(_, def)| def.dest.to_string())
        .collect();
    for event in parser {
        match event {
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => out.push(dest_url.to_string()),
            _ => {}
        }
    }
    out
}

/// Byte range of `needle` within `source[span]`.
fn replace_in(source: &str, span: &Range<usize>, needle: &str) -> Option<Range<usize>> {
    let slice = source.get(span.clone())?;
    let at = slice.find(needle)?;
    Some(span.start + at..span.start + at + needle.len())
}

fn build_page(path: &str, source: &str) -> DocPage {
    let (meta, body) = split_front_matter(source);
    let (heading, paragraph) = first_heading_and_description(body);
    let slug = slug_for(path);
    let title = meta
        .get("title")
        .cloned()
        .or(heading)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| slug.rsplit('/').next().unwrap_or(&slug).to_string());
    let description = meta.get("description").cloned().unwrap_or(paragraph);
    let order = meta
        .get("order")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(if path.ends_with("README.md") { 0 } else { 100 });
    let events: Vec<Event> = Parser::new_ext(body, markdown_options()).collect();
    let anchors = heading_ids(&events).into_iter().collect();
    DocPage {
        category: category_for(path),
        slug,
        path: path.to_string(),
        title,
        description,
        order,
        markdown: body.to_string(),
        anchors,
    }
}

pub fn page_url(slug: &str) -> String {
    format!("{DOCS_URL}/{slug}")
}

pub fn asset_url(path: &str) -> String {
    format!("{DOCS_URL}/{ASSETS_PREFIX}{}", path.replace(' ', "%20"))
}

/// Asset path from a request tail such as `assets/architecture/orbit-overview.png`.
pub fn asset_path(tail: &str) -> Option<&str> {
    tail.strip_prefix(ASSETS_PREFIX)
}

fn has_scheme(href: &str) -> bool {
    let Some((scheme, _)) = href.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

// ------------------------------------------------------------------- HTML document

const STYLE: &str = r#"
:root{color-scheme:light dark;--bg:#fff;--fg:#1a1d23;--muted:#5b6472;--line:#e3e6eb;--sub:#f5f6f8;--link:#2a52be}
@media (prefers-color-scheme:dark){:root{--bg:#14161a;--fg:#e8eaee;--muted:#9aa3b2;--line:#2a2e36;--sub:#1c1f25;--link:#8fb0ff}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif}
header{border-bottom:1px solid var(--line);padding:.75rem 1rem;font-size:.9rem}
header a{color:var(--muted);text-decoration:none;font-weight:600}
main{max-width:46rem;margin:0 auto;padding:1.5rem 1rem 4rem;overflow-wrap:anywhere}
h1,h2,h3,h4{line-height:1.25;margin:1.8em 0 .5em}h1{font-size:1.8rem;margin-top:.5em}h2{font-size:1.35rem}h3{font-size:1.1rem}
a{color:var(--link)}img{max-width:100%;height:auto}
code{background:var(--sub);padding:.1em .3em;border-radius:4px;font:.88em ui-monospace,Menlo,Consolas,monospace}
pre{background:var(--sub);border:1px solid var(--line);border-radius:6px;padding:.8rem;overflow:auto}pre code{background:none;padding:0}
blockquote{margin:1em 0;padding-left:1em;border-left:3px solid var(--line);color:var(--muted)}
table{border-collapse:collapse;display:block;overflow-x:auto}th,td{border:1px solid var(--line);padding:.35rem .6rem;text-align:left}th{background:var(--sub)}
ul.index{list-style:none;padding:0}ul.index li{padding:.5rem 0;border-bottom:1px solid var(--line)}ul.index p{margin:.1rem 0 0;color:var(--muted);font-size:.9rem}
"#;

pub const HTML_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; base-uri 'none'; form-action 'none'";

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{} · ORBIT docs</title><style>{STYLE}</style></head><body>\
<header><a href=\"{DOCS_URL}?format=html\">ORBIT documentation</a></header><main>{body}</main></body></html>",
        esc(title)
    )
}

impl DocsBundle {
    pub fn page_html_document(&self, page: &DocPage) -> String {
        document(&page.title, &self.render_html(page))
    }

    pub fn index_html_document(&self) -> String {
        let mut body = String::from("<h1>Documentation</h1>");
        for category in self.index().categories {
            body.push_str(&format!(
                "<h2>{}</h2><ul class=\"index\">",
                esc(&category.title)
            ));
            for page in category.pages {
                body.push_str(&format!(
                    "<li><a href=\"{}?format=html\">{}</a>",
                    esc(&page.url),
                    esc(&page.title)
                ));
                if !page.description.is_empty() {
                    body.push_str(&format!("<p>{}</p>", esc(&page.description)));
                }
                body.push_str("</li>");
            }
            body.push_str("</ul>");
        }
        document("Documentation", &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_slugs_match_github() {
        assert_eq!(heading_slug("Hello, World! v1.2"), "hello-world-v12");
        assert_eq!(heading_slug("Provider & connector"), "provider--connector");
        assert_eq!(heading_slug("snake_case Title"), "snake_case-title");
    }

    #[test]
    fn manifest_is_built_and_excludes_planning() {
        let bundle = bundle();
        assert!(bundle.page("index").is_some());
        assert!(bundle.page("architecture/overview").is_some());
        assert!(bundle
            .pages()
            .iter()
            .all(|p| !p.path.starts_with("planning/")));
        assert!(bundle.assets().all(|a| !a.path.starts_with("planning/")));
        let slugs: HashSet<_> = bundle.pages().iter().map(|p| &p.slug).collect();
        assert_eq!(slugs.len(), bundle.pages().len(), "slugs must be unique");
        assert!(bundle.pages().iter().all(|p| !p.title.is_empty()));
    }

    #[test]
    fn links_resolve_and_rewrite() {
        let bundle = bundle();
        assert_eq!(
            bundle.resolve("architecture/api.md", "overview.md#layout"),
            LinkTarget::Page {
                slug: "architecture/overview".into(),
                anchor: Some("layout".into())
            }
        );
        assert_eq!(
            bundle.resolve("architecture/api.md", "https://example.com/x.md"),
            LinkTarget::External
        );
        assert!(matches!(
            bundle.resolve("architecture/api.md", "../../Cargo.toml"),
            LinkTarget::Broken(_)
        ));
        assert!(matches!(
            bundle.resolve("architecture/api.md", "../../../etc/passwd"),
            LinkTarget::Broken(_)
        ));
        assert_eq!(
            bundle
                .rewrite("README.md", "architecture/orbit-overview.png")
                .as_deref(),
            Some("/api/v1/docs/assets/architecture/orbit-overview.png")
        );
    }

    #[test]
    fn markdown_rewrite_handles_inline_and_reference_links() {
        let bundle = bundle();
        let page = DocPage {
            slug: "x".into(),
            path: "README.md".into(),
            title: "x".into(),
            description: String::new(),
            category: "overview".into(),
            order: 0,
            markdown: "See [a](architecture/overview.md#top \"t\"), [b](https://x.io/a.md), \
                       `[c](guide/agents.md)` and [d][ref].\n\n![i](architecture/orbit-overview.png)\n\n[ref]: guide/agents.md\n"
                .into(),
            anchors: HashSet::new(),
        };
        let out = bundle.rewrite_markdown(&page);
        assert!(
            out.contains("[a](/api/v1/docs/architecture/overview#top \"t\")"),
            "{out}"
        );
        assert!(out.contains("[b](https://x.io/a.md)"));
        assert!(out.contains("`[c](guide/agents.md)`"));
        assert!(out.contains("![i](/api/v1/docs/assets/architecture/orbit-overview.png)"));
        assert!(out.contains("[ref]: /api/v1/docs/guide/agents"));
    }

    #[test]
    fn html_has_heading_ids_and_escapes_raw_html() {
        let bundle = bundle();
        let page = DocPage {
            slug: "x".into(),
            path: "README.md".into(),
            title: "x".into(),
            description: String::new(),
            category: "overview".into(),
            order: 0,
            markdown: "# Title\n\n## Title\n\n<script>alert(1)</script>\n".into(),
            anchors: HashSet::new(),
        };
        let out = bundle.render_html(&page);
        assert!(out.contains("<h1 id=\"title\">"), "{out}");
        assert!(out.contains("<h2 id=\"title-1\">"), "{out}");
        assert!(!out.contains("<script>"), "{out}");
    }

    /// CI link check: every relative link, image and anchor in the embedded docs must
    /// resolve. Links that point outside `docs/` (repository files) do not work in-app
    /// and are reported too.
    #[test]
    fn embedded_docs_have_no_broken_links() {
        let problems = bundle().broken_links();
        assert!(
            problems.is_empty(),
            "broken documentation links:\n{}",
            problems.join("\n")
        );
    }
}
