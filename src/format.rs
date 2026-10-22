//! Port of Mako's format placeholders and markup escaping.
//!
//! The Cairo/Pango renderer handles body-markup validation and falls back
//! to escaped text when sender-provided markup is invalid.

use std::fmt;

const SPECIFIERS: &str = "%asbhtgi";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError(pub String);
impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for FormatError {}

/// Decode Mako's format grammar: \n becomes a newline and \\ a literal
/// backslash. Unrecognized backslash escapes retain the backslash. Mako's
/// parse_format accepts a terminal '%' and a terminal backslash, too.
pub fn decode_format(value: &str) -> Result<String, FormatError> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '%' => match chars.next() {
                Some(next) => {
                    if !SPECIFIERS.contains(next) {
                        return Err(FormatError(format!("unknown format specifier %{next}")));
                    }
                    output.push('%');
                    output.push(next);
                }
                // Upstream parse_format sees the terminating NUL through
                // strchr(), then format_text emits the final '%' literally.
                None => output.push('%'),
            },
            '\\' => match chars.next() {
                Some('n') => output.push('\n'),
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            },
            _ => output.push(ch),
        }
    }
    Ok(output)
}

pub fn escape_markup(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct FormatNotification<'a> {
    pub id: u32,
    pub app_name: &'a str,
    pub summary: &'a str,
    pub body: &'a str,
    pub group_count: usize,
}

pub fn render_notification_format(template: &str, n: &FormatNotification<'_>,
                                  allow_valid_body_markup: bool) -> String {
    render(template, |key| {
        let (raw, trusted) = match key {
            'a' => (n.app_name.to_string(),false),
            's' => (n.summary.to_string(),false),
            'b' => (n.body.to_string(),allow_valid_body_markup),
            'i' => (n.id.to_string(),false),
            'g' => (n.group_count.to_string(),false),
            _ => (String::new(),false),
        };
        if trusted { raw } else { escape_markup(&raw) }
    })
}

pub fn render_hidden_format(template: &str, hidden: usize, total: usize) -> String {
    render(template, |key| match key {
        'h' => hidden.to_string(),
        't' => total.to_string(),
        _ => String::new(),
    })
}

fn render(template: &str, mut subst: impl FnMut(char) -> String) -> String {
    let mut chars = template.chars().peekable();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        if ch != '%' { output.push(ch); continue; }
        match chars.next() {
            Some('%') => output.push('%'),
            Some(key) => output.push_str(&subst(key)),
            None => output.push('%'),
        }
    }
    // Mako's trim_space uses C isspace on UTF-8 bytes. It strips ASCII
    // whitespace, not Unicode NBSP/EM SPACE text supplied by applications.
    output.trim_matches(|ch:char|ch.is_ascii_whitespace()).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_mako_placeholders_and_escapes() {
        assert_eq!(decode_format("<b>%s</b>\\n%b\\\\%%").unwrap(),"<b>%s</b>\n%b\\%%");
        assert!(decode_format("%z").is_err());
        assert_eq!(decode_format("%").unwrap(),"%");
        assert_eq!(decode_format(r"prefix\q\s").unwrap(),r"prefix\q\s");
        assert_eq!(decode_format(r"ending\").unwrap(),r"ending\");
        assert_eq!(decode_format(r"\%").unwrap(),r"\%");
        assert_eq!(decode_format("a\\nb").unwrap(),"a\nb");
    }
    #[test]
    fn escapes_untrusted_summary_but_retains_intentional_template_markup() {
        let n=FormatNotification {id:4,app_name:"app",summary:"<alert> &",body:"hello",group_count:1};
        assert_eq!(render_notification_format("<b>%s</b> %i",&n,false),"<b>&lt;alert&gt; &amp;</b> 4");
    }
    #[test]
    fn body_markup_is_fallback_safe() {
        let n=FormatNotification {body:"<b>Hello</b>",..FormatNotification::default()};
        assert_eq!(render_notification_format("%b",&n,false),"&lt;b&gt;Hello&lt;/b&gt;");
        assert_eq!(render_notification_format("%b",&n,true),"<b>Hello</b>");
    }
    #[test]
    fn preserves_unicode_spaces_that_mako_c_isspace_does_not_trim() {
        let n=FormatNotification{
            summary:"\u{a0}Hello\u{a0}",..FormatNotification::default()
        };
        assert_eq!(render_notification_format(" \t%s \n",&n,false),
            "\u{a0}Hello\u{a0}");
        assert_eq!(render_hidden_format(" \t(%h of %t)\n",2,5),
            "(2 of 5)");
    }

    #[test]
    fn hidden_and_total_placeholders() {
        assert_eq!(render_hidden_format("(%h more of %t)",3,8),"(3 more of 8)");
    }
}
