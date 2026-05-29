//! Most of these utils are taken from [Robrix](https://github.com/project-robius/robrix).
use std::borrow::Cow;

use matrix_sdk::ruma::events::room::message::{FormattedBody, MessageFormat};
use tracing::warn;
use url::Url;

use crate::room::frontend_events::msg_like::FrontendTextMessage;

pub fn linkify_text_message(body: String, formatted: Option<FormattedBody>) -> FrontendTextMessage {
    formatted
        .as_ref()
        .and_then(|fb| {
            (fb.format == MessageFormat::Html).then(|| {
                let mut matched_urls = Some(Vec::new());
                let filtered_and_trimmed = trim_start_html_whitespace(remove_mx_reply(&fb.body));
                FrontendTextMessage {
                    body: body.clone(),
                    formatted: Some(FormattedBody::html(
                        linkify_get_urls(filtered_and_trimmed, true, matched_urls.as_mut())
                            .to_string(),
                    )),
                    matched_urls,
                }
            })
        })
        .unwrap_or_else(|| {
            let mut matched_urls = Some(Vec::new());
            match linkify_get_urls(&body, false, matched_urls.as_mut()) {
                Cow::Borrowed(plaintext) => FrontendTextMessage {
                    body: htmlize::escape_text(plaintext).to_string(),
                    formatted: None,
                    matched_urls: None,
                },
                Cow::Owned(linkified) => FrontendTextMessage {
                    body,
                    formatted: Some(FormattedBody::html(linkified)),
                    matched_urls,
                },
            }
        })
}

/// Looks for bare links in the given `text` and converts them into proper HTML links.
///
/// If `links_found` is provided, it will be populated with the list of URLs found in the text.
pub fn linkify_get_urls<'t>(
    text: &'t str,
    is_html: bool,
    mut links_found: Option<&mut Vec<Url>>,
) -> Cow<'t, str> {
    const MAILTO: &str = "mailto:";

    use linkify::{Link, LinkFinder, LinkKind};
    let mut links = LinkFinder::new().links(text).peekable();
    if links.peek().is_none() {
        return Cow::Borrowed(text);
    }

    // A closure to escape text if it's not HTML.
    let escaped = |text| {
        if is_html {
            Cow::from(text)
        } else {
            htmlize::escape_text(text)
        }
    };

    let mut linkified_text = String::new();
    let mut last_end_index = 0;
    for link in links {
        let link_txt = link.as_str();

        // Only linkify the URL if it's not already part of an HTML or mailto href attribute.
        let is_link_within_href_attr = text.get(..link.start()).is_some_and(ends_with_href);
        let is_link_within_html_tag = |link: &Link<'_>| {
            text.get(link.end()..)
                .is_some_and(|after| after.trim_end().starts_with("</a>"))
        };
        let is_mailto_link_within_href_attr = |link: &Link<'_>| {
            if !matches!(link.kind(), LinkKind::Email) {
                return false;
            }
            let mailto_start = link.start().saturating_sub(MAILTO.len());
            text.get(mailto_start..link.start())
                .is_some_and(|t| t == MAILTO)
                .then(|| text.get(..mailto_start))
                .flatten()
                .is_some_and(ends_with_href)
        };

        let is_href = is_link_within_href_attr;
        let is_mailto_href = is_mailto_link_within_href_attr(&link);
        let is_html_tag = is_link_within_html_tag(&link);

        if is_href || is_mailto_href || is_html_tag {
            if is_href || is_mailto_href {
                // Get the text slice from our last position up to the start of the current URL
                let before_link = text.get(last_end_index..link.start()).unwrap_or_default();

                // Backtrack to find the start of the opening <a> tag container
                if let Some(a_index) = before_link.rfind("<a").or_else(|| before_link.rfind("<A")) {
                    let (prefix, tag_body) = before_link.split_at(a_index);
                    let is_matrix = link_txt.starts_with("matrix:")
                        || link_txt.starts_with("https://matrix.to");

                    linkified_text.push_str(prefix);

                    if is_matrix {
                        // Exception rule: inject or append "mx-pill"
                        if tag_body.contains("class=") {
                            if let Some(class_idx) = tag_body.find("class=\"") {
                                let (t_prefix, t_suffix) = tag_body.split_at(class_idx + 7);
                                linkified_text.push_str(t_prefix);
                                linkified_text.push_str("mx-pill ");
                                linkified_text.push_str(t_suffix);
                            } else if let Some(class_idx) = tag_body.find("class='") {
                                let (t_prefix, t_suffix) = tag_body.split_at(class_idx + 7);
                                linkified_text.push_str(t_prefix);
                                linkified_text.push_str("mx-pill ");
                                linkified_text.push_str(t_suffix);
                            } else {
                                linkified_text.push_str(tag_body);
                            }
                        } else {
                            let (a_lit, rest) = tag_body.split_at(2); // split right after "<a"
                            linkified_text.push_str(a_lit);
                            linkified_text.push_str(" class=\"mx-pill\"");
                            linkified_text.push_str(rest);
                        }
                    } else {
                        // Standard rule: inject target="_blank" and rel attributes safely
                        let mut injection = String::new();
                        if !tag_body.contains("target=") {
                            injection.push_str(" target=\"_blank\"");
                        }
                        if !tag_body.contains("rel=") {
                            injection.push_str(" rel=\"noopener noreferrer\"");
                        }

                        let (a_lit, rest) = tag_body.split_at(2);
                        linkified_text.push_str(a_lit);
                        linkified_text.push_str(&injection);
                        linkified_text.push_str(rest);
                    }
                } else {
                    linkified_text.push_str(before_link);
                }
                // Append the URL itself
                linkified_text.push_str(text.get(link.start()..link.end()).unwrap_or_default());
            } else {
                // `is_html_tag` handles inner tag text (e.g., <a>this text</a>); pass it through unchanged
                linkified_text.push_str(text.get(last_end_index..link.end()).unwrap_or_default());
            }

            if let Some(links_found) = links_found.as_mut()
                && let Ok(url) = Url::parse(link_txt)
            {
                links_found.push(url);
            }
        } else {
            // Processing bare text links
            match link.kind() {
                LinkKind::Url => {
                    let is_matrix = link_txt.starts_with("matrix:")
                        || link_txt.starts_with("https://matrix.to");
                    let attrs = if is_matrix {
                        "class=\"mx-pill\""
                    } else {
                        "target=\"_blank\" rel=\"noopener noreferrer\""
                    };

                    linkified_text = format!(
                        "{linkified_text}{}<a href=\"{}\" {}>{}</a>",
                        escaped(text.get(last_end_index..link.start()).unwrap_or_default()),
                        htmlize::escape_attribute(link_txt),
                        attrs,
                        htmlize::escape_text(link_txt),
                    );
                    if let Some(links_found) = links_found.as_mut()
                        && let Ok(url) = Url::parse(link_txt)
                    {
                        links_found.push(url);
                    }
                }
                LinkKind::Email => {
                    linkified_text = format!(
                        "{linkified_text}{}<a href=\"mailto:{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                        escaped(text.get(last_end_index..link.start()).unwrap_or_default()),
                        htmlize::escape_attribute(link_txt),
                        htmlize::escape_text(link_txt),
                    );
                }
                _ => return Cow::Borrowed(text),
            }
        }
        last_end_index = link.end();
    }
    linkified_text.push_str(&escaped(text.get(last_end_index..).unwrap_or_default()));
    Cow::Owned(linkified_text)
}

/// Returns true if the given `text` string ends with a valid href attribute opener.
///
/// An href attribute looks like this: `href="http://example.com"`,.
/// so we look for `href="` at the end of the given string.
///
/// Spaces are allowed to exist in between the `href`, `=`, and `"`.
/// In addition, the quotation mark is optional, and can be either a single or double quote,
/// so this function takes those into account as well.
pub fn ends_with_href(text: &str) -> bool {
    // let mut idx = text.len().saturating_sub(1);
    let mut substr = text.trim_end();
    // Search backwards for a single quote, double quote, or an equals sign.
    match substr.as_bytes().last() {
        Some(b'\'' | b'"')
            if substr
                .get(..substr.len().saturating_sub(1))
                .map(|s| {
                    substr = s.trim_end();
                    substr.as_bytes().last() == Some(&b'=')
                })
                .unwrap_or(false) =>
        {
            substr = &substr[..substr.len().saturating_sub(1)];
        }
        Some(b'=') => {
            substr = &substr[..substr.len().saturating_sub(1)];
        }
        _ => return false,
    }

    // Now we have found the equals sign, so search backwards for the `href` attribute.
    substr.trim_end().ends_with("href")
}

/// Looks for and removes the `<mx-reply>` element from the given HTML message body, if it exists.
///
/// Follows this behavior defined in the Matrix spec:
/// <https://spec.matrix.org/v1.13/client-server-api/#rich-replies>
pub fn remove_mx_reply(html_message_body: &str) -> &str {
    const MX_REPLY_START: &str = "<mx-reply>";
    const MX_REPLY_END: &str = "</mx-reply>";
    if html_message_body.trim().starts_with(MX_REPLY_START)
        && let Some(end) = html_message_body.find(MX_REPLY_END)
        && let Some(after) = html_message_body.get(end + MX_REPLY_END.len()..)
    {
        return after;
    }
    html_message_body
}

/// Removes leading whitespace and HTML whitespace tags (`<p>` and `<br>`) from the given `text`.
pub fn trim_start_html_whitespace(mut text: &str) -> &str {
    let mut prev_text_len = text.len();
    loop {
        text = text
            .trim_start_matches("<p>")
            .trim_start_matches("<br>")
            .trim_start_matches("<br/>")
            .trim_start_matches("<br />")
            .trim_start();

        if text.len() == prev_text_len {
            break;
        }
        prev_text_len = text.len();
    }
    text
}
