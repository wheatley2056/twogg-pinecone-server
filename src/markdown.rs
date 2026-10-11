//! Markdown → HTML für Beschreibungen und Neuigkeiten. Rohes HTML im Text wird nicht
//! übernommen, sondern als Text ausgegeben (kein Einschleusen von Skripten).

use pulldown_cmark::{Event, Options, Parser, html};

pub fn to_html(source: &str) -> String {
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    )
    .map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    // Links: nur http(s) und mailto zulassen.
    neutralize_links(&out)
}

fn neutralize_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("href=\"") {
        let (head, tail) = rest.split_at(i + 6);
        out.push_str(head);
        let end = tail.find('"').unwrap_or(tail.len());
        let url = &tail[..end];
        let lower = url.trim().to_ascii_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:")
            || lower.starts_with('#')
        {
            out.push_str(url);
        } else {
            out.push('#');
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_escapes() {
        let html = to_html(
            "# Hallo\n\n**fett** <script>alert(1)</script> [x](javascript:alert(1)) [y](https://twogg.de)",
        );
        assert!(html.contains("<h1>Hallo</h1>"));
        assert!(html.contains("<strong>fett</strong>"));
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(html.contains("href=\"#\""), "{html}");
        assert!(html.contains("href=\"https://twogg.de\""));
    }
}
