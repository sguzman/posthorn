//! Partial port of Mako's configuration and criteria engine.
//!
//! Preserves Mako's INI-like `key=value` format and ordered criteria rules.
//! This is deliberately not TOML (TOML is scheduled *after* Mako parity).

use regex::bytes::Regex;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use crate::wayland::PopupMargins;

/// Mako adjusts a style's left/right padding only when the same style
/// explicitly contains both padding and border-radius. The root style has
/// both from built-in defaults; a criteria section begins with neither.
/// Recording each assignment is essential: later lowering border-radius
/// does not undo an earlier padding clamp.
#[derive(Clone, Debug, Default)]
struct MakoPaddingState {
    input_padding: Option<PopupMargins>,
    effective_padding: Option<PopupMargins>,
    border_radius: Option<PopupMargins>,
}

impl MakoPaddingState {
    fn root() -> Self {
        let padding=PopupMargins::uniform(5);
        Self {
            input_padding:Some(padding),
            effective_padding:Some(padding),
            border_radius:Some(PopupMargins::uniform(0)),
        }
    }

    fn assign(&mut self, key:&str, value:&str) {
        let Some(value)=PopupMargins::parse_mako(value) else{return};
        match key {
            "padding" => {
                self.input_padding=Some(value);
                self.effective_padding=Some(value);
            }
            "border-radius" => self.border_radius=Some(value),
            _ => return,
        }
        if let (Some(padding),Some(radius))=(&mut self.effective_padding,self.border_radius) {
            padding.left=padding.left.max(radius.left);
            padding.right=padding.right.max(radius.right);
        }
    }

    fn override_padding(&self) -> Option<String> {
        let (Some(original),Some(actual))=(self.input_padding,self.effective_padding)
            else{return None};
        if actual==original {return None}
        Some(format!("{},{},{},{}",actual.top,actual.right,actual.bottom,actual.left))
    }
}

/// Exactly what appeared in the Mako config, with no conversion to CSS or TOML.
pub type Options = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub location: String,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

impl std::error::Error for ConfigError {}

fn err(location: impl Into<String>, message: impl Into<String>) -> ConfigError {
    ConfigError { location: location.into(), message: message.into() }
}

/// Pinned Mako config.c:load_config_file() reads bytes with getline()
/// and skips lines beginning with '#' (after ASCII space/tab indentation)
/// before any option parsing. A comment is not required to be UTF-8.
/// Keep its physical newline, so diagnostics and section precedence retain
/// the original line numbering. Active settings must remain representable
/// as Rust UTF-8 strings; reject malformed active lines rather than replacing
/// their bytes with an unrelated selector or path.
fn decode_mako_file_bytes(bytes:&[u8],source:&str)->Result<String,ConfigError>{
    let mut text=String::new();
    for (index,line) in bytes.split(|byte|*byte==b'\n').enumerate(){
        if index>0{text.push('\n');}
        let uncommented=line.iter().copied()
            .skip_while(|byte|*byte==b' '||*byte==b'\t');
        if uncommented.into_iter().next()==Some(b'#'){
            continue;
        }
        let decoded=std::str::from_utf8(line).map_err(|_|err(
            format!("{source}:{}",index+1),
            "non-UTF-8 bytes in active config line",
        ))?;
        text.push_str(decoded);
    }
    Ok(text)
}

/// glibc's C-locale REG_EXTENDED treats '\' INSIDE a bracket class as
/// an ordinary literal character, not as an escape for the next byte.
/// Rust regex treats it as a metacharacter. Turn each source backslash
/// inside a class into a Rust literal-backslash token, without consuming
/// the following character: that next character may be a range endpoint,
/// an unescaped '-' operator, or even the class-closing ']'.
/// Outside bracket classes the GNU POSIX backslash operators retain
/// their separate semantics and remain untouched.
fn mako_posix_c_class_backslashes(pattern:&str)->String{
    let mut normalized=String::with_capacity(pattern.len());
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    let mut first=false;
    let mut can_negate=false;
    while let Some(ch)=chars.next(){
        if !in_class {
            match ch {
                '\\' => {
                    normalized.push('\\');
                    if let Some(next)=chars.next(){normalized.push(next);}
                }
                '[' => {
                    in_class=true;
                    first=true;
                    can_negate=true;
                    normalized.push('[');
                }
                _ => normalized.push(ch),
            }
            continue;
        }
        if first && ch=='^' && can_negate {
            normalized.push('^');
            can_negate=false;
            continue;
        }
        if first && ch==']' {
            normalized.push(']');
            first=false;
            continue;
        }
        first=false;
        if ch=='[' {
            normalized.push('[');
            // POSIX bracket elements have nested closing brackets, which
            // must not terminate the enclosing class's state machine.
            if let Some(delimiter @ (':'|'.'|'='))=chars.peek().copied(){
                normalized.push(chars.next().expect("peeked delimiter"));
                let mut previous='\0';
                for next in chars.by_ref(){
                    normalized.push(next);
                    if previous==delimiter && next==']'{break;}
                    previous=next;
                }
            }
        }else if ch=='\\'{
            normalized.push('\\');
            normalized.push('\\');
        }else{
            if ch==']'{in_class=false;}
            normalized.push(ch);
        }
    }
    normalized
}

/// In POSIX bracket syntax the very first ']' (or first after '^')
/// is a LITERAL class member. Rust regex requires it escaped. Normalize
/// before every other POSIX class pass so later scanners do not mistake
/// the leading ']' for the end of the bracket expression.
fn mako_posix_ere_literal_leading_closer(pattern:&str)->String{
    let mut normalized=String::with_capacity(pattern.len());
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    while let Some(ch)=chars.next(){
        match ch{
            '\\' => {
                normalized.push(ch);
                if let Some(next)=chars.next(){normalized.push(next);}
            }
            '[' if !in_class => {
                in_class=true;
                normalized.push('[');
                if chars.peek()==Some(&'^'){
                    normalized.push(chars.next().expect("peeked negation"));
                }
                if chars.peek()==Some(&']'){
                    normalized.push('\\');
                    normalized.push(chars.next().expect("peeked leading closer"));
                }
            }
            '[' if in_class => {
                normalized.push('[');
                // Don't let a nested POSIX class, collating element or
                // equivalence element's own ']' close its enclosing class.
                if let Some(delimiter @ (':'|'.'|'='))=chars.peek().copied(){
                    normalized.push(chars.next().expect("peeked delimiter"));
                    let mut previous='\0';
                    for next in chars.by_ref(){
                        normalized.push(next);
                        if previous==delimiter && next==']'{break;}
                        previous=next;
                    }
                }
            }
            ']' if in_class => {
                in_class=false;
                normalized.push(']');
            }
            _ => normalized.push(ch),
        }
    }
    normalized
}

/// In GNU glibc REG_EXTENDED, a closing ')', ']' or '}' with no
/// matching active opener is a literal atom rather than a syntax error.
/// Rust regex rejects an unmatched closing parenthesis. Escape only those
/// *unpaired* closers, preserving balanced capture groups, real repetition
/// intervals, POSIX bracket elements and escaped delimiters. This runs
/// before GNU word-boundary rewriting adds Rust-only \b{start}/\b{end}.
fn mako_posix_ere_literal_unmatched_closers(pattern:&str)->String {
    let mut out=String::with_capacity(pattern.len());
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    let mut group_depth=0usize;
    while let Some(ch)=chars.next() {
        match ch {
            '\\' => {
                out.push('\\');
                if let Some(next)=chars.next(){out.push(next);}
            }
            '[' if !in_class => {
                in_class=true;
                out.push('[');
            }
            '[' if in_class => {
                out.push('[');
                if let Some(delimiter @ (':'|'.'|'='))=chars.peek().copied(){
                    out.push(chars.next().expect("peeked POSIX bracket delimiter"));
                    let mut previous='\0';
                    for next in chars.by_ref(){
                        out.push(next);
                        if previous==delimiter && next==']'{break;}
                        previous=next;
                    }
                }
            }
            ']' if in_class => {
                in_class=false;
                out.push(']');
            }
            ']' => out.push_str(r"\]"),
            '(' if !in_class => {
                group_depth+=1;
                out.push('(');
            }
            ')' if !in_class && group_depth>0 => {
                group_depth-=1;
                out.push(')');
            }
            ')' if !in_class => out.push_str(r"\)"),
            '{' if !in_class => {
                // All unescaped opening braces were already checked by
                // the glibc interval validator. Copy through its matching
                // closing brace so it remains repetition syntax.
                out.push('{');
                for next in chars.by_ref(){
                    out.push(next);
                    if next=='}'{break;}
                }
            }
            '}' if !in_class => out.push_str(r"\}"),
            _ => out.push(ch),
        }
    }
    out
}

/// In glibc's POSIX REG_EXTENDED, escaped ASCII letters are literals
/// except the GNU word/whitespace operators \b, \B, \w, \W, \s, \S.
/// Within bracket expressions glibc treats the *backslash itself*
/// as a literal class member, separately normalized by the preceding pass.
/// Rust regex assigns extra semantics to \A, \d, \x, \u, \z, etc. Apply
/// the source-tested glibc rule before passing the expression to Rust regex.
/// Other POSIX-versus-Rust regex differences remain an MVP parity gap.
fn mako_posix_ere_escapes(pattern: &str) -> String {
    let mut normalized = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(letter) if letter.is_ascii_alphabetic() &&
                    (in_class || !matches!(letter, 'b'|'B'|'w'|'W'|'s'|'S')) =>
                        normalized.push(letter),
                // GNU glibc ERE uses \< and \> for the start and
                // end of a word. In bracket expressions these are
                // literal '<' and '>', not assertions.
                Some('<') if !in_class => normalized.push_str(r"\b{start}"),
                Some('>') if !in_class => normalized.push_str(r"\b{end}"),
                // GNU glibc escaped backtick and apostrophe delimit
                // the entire input, like Rust \A and \z.
                Some('`') if !in_class => normalized.push_str(r"\A"),
                Some('\'') if !in_class => normalized.push_str(r"\z"),
                Some('<') => normalized.push('<'),
                Some('>') => normalized.push('>'),
                Some('`') => normalized.push('`'),
                Some('\'') => normalized.push('\''),
                // glibc accepts backslashes before ordinary punctuation
                // (e.g. \#, \!, \@, \_, \/) and before the digit zero
                // as literal characters. Rust regex rejects some of
                // those escapes or gives them a different meaning.
                // Preserve Rust metacharacter escaping, but do not
                // consume \1..\9: glibc uses those for backreferences.
                Some(other) if !in_class && !other.is_ascii_alphabetic()
                    && !matches!(other, '1'..='9') =>
                {
                    normalized.push_str(&regex::escape(&other.to_string()));
                }
                Some(other) => {
                    normalized.push('\\');
                    normalized.push(other);
                }
                None => normalized.push('\\'),
            },
            '[' if !in_class => {
                in_class = true;
                normalized.push(ch);
            }
            '[' if in_class => {
                // A bare '[' in glibc's POSIX bracket class is a literal
                // member: [a[] matches 'a' or '['. Rust regex instead
                // starts a nested set, so escape it. Only actual POSIX
                // [[:class:]], [[.collating.]], [[=equivalence=]] elements
                // retain their recognized nested syntax.
                if let Some(delimiter @ (':' | '.' | '=')) = chars.peek().copied() {
                    normalized.push('[');
                    chars.next();
                    normalized.push(delimiter);
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        normalized.push(next);
                        if previous == delimiter && next == ']' {
                            break;
                        }
                        previous = next;
                    }
                }else{
                    normalized.push('\\');
                    normalized.push('[');
                }
            }
            ']' if in_class => {
                in_class = false;
                normalized.push(ch);
            }
            // glibc REG_EXTENDED treats '&', '~' and '|' as ordinary
            // bracket characters: [a&&b], [a~~b], [a||b] include their
            // punctuation. Rust regex reserves &&, ~~ and || for set
            // intersection, symmetric difference and union respectively.
            // Escape unescaped punctuation in bracket classes for parity.
            '&' | '~' | '|' if in_class => {
                normalized.push('\\');
                normalized.push(ch);
            }
            _ => normalized.push(ch),
        }
    }
    normalized
}

/// A C-locale POSIX ERE recognizes single-byte collation [.[char].]
/// and equivalence [=[char]=] elements ONLY inside character classes.
/// Rust regex instead treats those brackets as nested classes and can
/// unintentionally match the delimiter '.' or '='. Rewrite only valid
/// one-byte C-locale elements, keeping other POSIX named classes intact.
/// The C locale rejects multi-character elements with REG_ECOLLATE.
fn mako_posix_c_collating_elements(pattern:&str)->Result<String,String>{
    let mut out=String::with_capacity(pattern.len());
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    while let Some(ch)=chars.next(){
        match ch{
            '\\'=>{
                out.push('\\');
                if let Some(next)=chars.next(){out.push(next);}
            }
            '[' if !in_class=>{
                in_class=true;
                out.push('[');
            }
            '[' if in_class=>{
                if let Some(delimiter @ (':'|'.'|'='))=chars.peek().copied(){
                    chars.next();
                    let mut content=String::new();
                    let mut terminated=false;
                    while let Some(next)=chars.next(){
                        if next==delimiter && chars.peek()==Some(&']'){
                            chars.next();
                            terminated=true;
                            break;
                        }
                        content.push(next);
                    }
                    if !terminated{
                        return Err("unterminated POSIX bracket element".into());
                    }
                    if delimiter==':'{
                        out.push_str("[:");
                        out.push_str(&content);
                        out.push_str(":]");
                    }else{
                        let mut units=content.chars();
                        let one=units.next().filter(|c|c.is_ascii() && *c!='\0');
                        if one.is_none()||units.next().is_some(){
                            return Err("invalid C-locale POSIX collating element".into());
                        }
                        let value=one.unwrap();
                        // Within Rust character classes, escape literals
                        // with class syntax (and its set-algebra operators).
                        if matches!(value,'\\'|']'|'['|'^'|'-'|'&'|'~'){
                            out.push('\\');
                        }
                        out.push(value);
                    }
                }else{
                    out.push('[');
                }
            }
            ']' if in_class=>{
                in_class=false;
                out.push(']');
            }
            _=>out.push(ch),
        }
    }
    Ok(out)
}

/// glibc's GNU POSIX ERE accepts omitted lower repetition bounds:
/// a{,2} is a{0,2}, and a{,} is a{0,}. Rust regex's default parser
/// rejects those spellings. Rewrite only real unescaped repeat tokens,
/// never escaped literal braces or bracket-class members.
fn mako_posix_ere_empty_min_repetitions(pattern:&str)->String{
    let mut out=String::with_capacity(pattern.len());
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    while let Some(ch)=chars.next(){
        match ch {
            '\\' => {
                out.push(ch);
                if let Some(next)=chars.next(){out.push(next);}
            }
            '[' if !in_class => {
                in_class=true;
                out.push(ch);
            }
            '[' if in_class => {
                out.push(ch);
                // A POSIX class/collating/equivalence element owns its
                // nested closing bracket; do not close the outer class.
                if let Some(delimiter @ (':'|'.'|'='))=chars.peek().copied(){
                    chars.next();
                    out.push(delimiter);
                    let mut previous='\0';
                    for next in chars.by_ref(){
                        out.push(next);
                        if previous==delimiter && next==']'{break;}
                        previous=next;
                    }
                }
            }
            ']' if in_class => {
                in_class=false;
                out.push(ch);
            }
            '{' if !in_class => {
                let mut look=chars.clone();
                let omitted_min=if look.next()==Some(','){
                    while look.peek().is_some_and(char::is_ascii_digit){
                        look.next();
                    }
                    look.peek()==Some(&'}')
                }else{false};
                if omitted_min {out.push_str("{0");}
                else{out.push('{');}
            }
            _ => out.push(ch),
        }
    }
    out
}

/// glibc POSIX REG_EXTENDED treats every unescaped '{' outside a bracket
/// expression as a counted repetition opener, not a literal fallback.
/// Its numeric bounds must fit RE_DUP_MAX (32767) and descending ranges,
/// signs, alphabetic contents and missing '}' are compile errors. Rust
/// regex can instead treat malformed braces as literals, or compile a
/// repetition larger than glibc permits. Validate without allocating the
/// potentially enormous expanded automaton. Escaped braces and bracket
/// characters remain literal, including nested [:class:] expressions.
fn mako_posix_ere_validate_intervals(pattern:&str)->Result<(),String>{
    const RE_DUP_MAX:u32=32767;
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    let mut first_in_class=false;
    while let Some(ch)=chars.next(){
        if ch=='\\'{
            chars.next();
            continue;
        }
        if in_class{
            if ch=='[' && matches!(chars.peek().copied(),Some(':'|'.'|'=')){
                let delimiter=chars.next().expect("peeked bracket delimiter");
                let mut previous='\0';
                for part in chars.by_ref(){
                    if previous==delimiter && part==']'{break;}
                    previous=part;
                }
                first_in_class=false;
                continue;
            }
            if ch==']' && !first_in_class{in_class=false;}
            // A leading ] after [ or [^ is literal in glibc. The
            // preceding normalizer has already backslash-escaped it.
            first_in_class=false;
            continue;
        }
        if ch=='['{
            in_class=true;
            first_in_class=true;
            continue;
        }
        if ch!='{'{continue;}
        let mut contents=String::new();
        let mut closed=false;
        for next in chars.by_ref(){
            if next=='}'{closed=true;break;}
            contents.push(next);
            // Numeric repetitions above 32767 are invalid anyway. Avoid
            // building an unbounded allocation from adversarial config.
            if contents.len()>32{
                return Err("invalid or oversized POSIX repetition interval".into());
            }
        }
        if !closed{return Err("unmatched POSIX repetition '{'".into());}
        let (lower,upper)=if let Some((low,high))=contents.split_once(','){
            // GNU glibc accepts omitted lower and upper bounds, even {,}.
            (if low.is_empty(){Some(0)}else{low.parse::<u32>().ok()},
             if high.is_empty(){None}else{Some(high.parse::<u32>()
                 .map_err(|_|"invalid POSIX repetition upper bound".to_string())?)})
        }else{
            let parsed=contents.parse::<u32>().ok();
            (parsed,parsed)
        };
        let Some(lower)=lower else {
            return Err("invalid POSIX repetition lower bound".into());
        };
        if lower>RE_DUP_MAX || upper.is_some_and(|upper|
            upper>RE_DUP_MAX || upper<lower){
            return Err("POSIX repetition exceeds glibc bound or descends".into());
        }
        // {,} already normalized to {0,}, so an empty upper means
        // infinity, not an error; a bare {} has no lower and fails above.
    }
    Ok(())
}

/// GNU glibc REG_EXTENDED permits stacking postfix repetition operators:
/// a+? means (a+)?, a{2}+ means (a{2})+, and a{2}{3} means
/// (a{2}){3}. Rust regex interprets +?/*? as lazy quantifiers and
/// rejects nested counted repetitions. REG_NOSUB still observes different
/// *match languages*: ^a+?$ matches the empty string in glibc.
/// Insert noncapturing parentheses around the complete previously quantified
/// atom, rather than discarding either operator or interpreting laziness.
/// POSIX bracket sets, escaped punctuation and ordinary grouping are atoms.
fn mako_posix_ere_stacked_quantifiers(pattern:&str)->String{
    let mut out=String::with_capacity(pattern.len());
    let mut chars=pattern.chars().peekable();
    let mut groups=Vec::<usize>::new();
    let mut atom_start:Option<usize>=None;
    let mut previous_quantifier=false;
    while let Some(ch)=chars.next(){
        match ch{
            '\\'=>{
                let start=out.len();
                out.push('\\');
                if let Some(next)=chars.next(){out.push(next);}
                atom_start=Some(start);
                previous_quantifier=false;
            }
            '['=>{
                let start=out.len();
                out.push('[');
                let mut first=true;
                while let Some(next)=chars.next(){
                    out.push(next);
                    if next=='\\'{
                        if let Some(escaped)=chars.next(){out.push(escaped);}
                        first=false;
                    }else if next=='[' && matches!(chars.peek().copied(),Some(':'|'.'|'=')){
                        let delimiter=chars.next().expect("peeked delimiter");
                        out.push(delimiter);
                        let mut prev='\0';
                        for part in chars.by_ref(){
                            out.push(part);
                            if prev==delimiter && part==']'{break;}
                            prev=part;
                        }
                        first=false;
                    }else if next==']' && !first{
                        break;
                    }else if next!='^' || !first{
                        first=false;
                    }
                }
                atom_start=Some(start);
                previous_quantifier=false;
            }
            '('=>{
                groups.push(out.len());
                out.push('(');
                atom_start=None;
                previous_quantifier=false;
            }
            ')'=>{
                out.push(')');
                atom_start=groups.pop();
                previous_quantifier=false;
            }
            '|'=>{
                out.push('|');
                atom_start=None;
                previous_quantifier=false;
            }
            '*'|'+'|'?'=>{
                if let Some(start)=atom_start{
                    if previous_quantifier{
                        out.insert_str(start,"(?:");
                        out.push(')');
                    }
                    out.push(ch);
                    previous_quantifier=true;
                }else{
                    out.push(ch);
                    previous_quantifier=false;
                }
            }
            '{'=>{
                let mut look=chars.clone();
                let mut contents=String::new();
                let mut terminated=false;
                for _ in 0..32{
                    let Some(next)=look.next() else {break};
                    if next=='}'{terminated=true;break;}
                    contents.push(next);
                }
                let valid=if let Some((lower,upper))=contents.split_once(','){
                    !lower.is_empty() && lower.bytes().all(|b|b.is_ascii_digit())
                        && upper.bytes().all(|b|b.is_ascii_digit())
                }else{
                    !contents.is_empty() && contents.bytes().all(|b|b.is_ascii_digit())
                };
                if terminated && valid && atom_start.is_some(){
                    for _ in 0..contents.len()+1{chars.next();}
                    let start=atom_start.expect("checked atom");
                    if previous_quantifier{
                        out.insert_str(start,"(?:");
                        out.push(')');
                    }
                    out.push('{');
                    out.push_str(&contents);
                    out.push('}');
                    previous_quantifier=true;
                }else{
                    atom_start=Some(out.len());
                    out.push('{');
                    previous_quantifier=false;
                }
            }
            _=>{
                atom_start=Some(out.len());
                out.push(ch);
                previous_quantifier=false;
            }
        }
    }
    out
}

/// glibc regcomp(REG_EXTENDED) does not recognize the additional
/// Rust-regex ASCII classes [[:ascii:]] and [[:word:]], or Rust's
/// [[:^alpha:]]-style per-class negation extension. A Mako configuration
/// containing any of those fails to load.
/// Scan only real POSIX class elements inside brackets;
/// escaped opening brackets and quoted literals are not class openers.
fn has_rust_only_posix_class(pattern: &str) -> bool {
    let mut chars=pattern.chars().peekable();
    let mut in_class=false;
    while let Some(ch)=chars.next() {
        match ch {
            '\\' => { chars.next(); }
            '[' if !in_class => in_class=true,
            '[' if in_class && chars.peek()==Some(&':') => {
                chars.next(); // POSIX class-opening colon.
                let mut name=String::new();
                let mut terminated=false;
                while let Some(next)=chars.next() {
                    if next==':' && chars.peek()==Some(&']') {
                        chars.next();
                        terminated=true;
                        break;
                    }
                    name.push(next);
                }
                if terminated && (matches!(name.as_str(),"ascii"|"word")
                    || name.starts_with('^')) {
                    return true;
                }
            }
            ']' if in_class => in_class=false,
            _ => {}
        }
    }
    false
}

/// POSIX REG_EXTENDED rejects constructs beginning with unescaped "(?".
/// Rust regex accepts inline flags and non-capturing/lookaround-style groups.
/// A bare "(?" inside a bracket expression or an escaped "(" is not a group.
/// POSIX collating/equivalence/character-class elements may contain brackets.
fn has_rust_only_group_extension(pattern: &str) -> bool {
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '[' if !in_class => in_class = true,
            '[' if in_class => {
                if let Some(delimiter @ (':' | '.' | '=')) = chars.peek().copied() {
                    chars.next();
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if previous == delimiter && next == ']' {
                            break;
                        }
                        previous = next;
                    }
                }
            }
            ']' if in_class => in_class = false,
            '(' if !in_class && chars.peek() == Some(&'?') => return true,
            _ => {}
        }
    }
    false
}

#[derive(Clone, Debug)]
pub struct Selector {
    pub key: String,
    pub value: String,
    regex: Option<Regex>,
}

impl Selector {
    fn new(key: &str, value: &str) -> Result<Self, String> {
        const EXACT: &[&str] = &[
            "app-name", "app-icon", "urgency", "category", "desktop-entry", "summary",
            "body", "group-index", "anchor", "output", "mode", "actionable", "expiring",
            "grouped", "hidden",
        ];
        if key != "summary~" && key != "body~" && !EXACT.contains(&key) {
            return Err(format!("unknown criteria field '{key}'"));
        }
        let regex = if key == "summary~" || key == "body~" {
            // Mako uses POSIX REG_EXTENDED|REG_NOSUB without REG_NEWLINE.
            // Unlike Unicode-aware Rust regex, C-locale POSIX matching is
            // byte-oriented. Dot-all must also match newlines without
            // enabling multiline anchors; -u keeps ASCII class semantics.
            if has_rust_only_group_extension(value) {
                return Err("POSIX ERE does not support (? group extensions".into());
            }
            if has_rust_only_posix_class(value) {
                return Err("glibc POSIX ERE does not recognize this bracket class".into());
            }
            let backslashes=mako_posix_c_class_backslashes(value);
            let leading=mako_posix_ere_literal_leading_closer(&backslashes);
            let collated=mako_posix_c_collating_elements(&leading)?;
            // Validate the original POSIX syntax BEFORE GNU word-boundary
            // escapes become Rust \b{start}/\b{end} assertions. Those
            // Rust-only braces are not POSIX repetition intervals.
            mako_posix_ere_validate_intervals(&collated)?;
            let closers=mako_posix_ere_literal_unmatched_closers(&collated);
            let repeat=mako_posix_ere_empty_min_repetitions(
                &mako_posix_ere_escapes(&closers));
            let pattern=mako_posix_ere_stacked_quantifiers(&repeat);
            Some(Regex::new(&format!("(?s-u:{pattern})"))
                .map_err(|e| format!("invalid regex: {e}"))?)
        } else { None };
        if ["actionable", "expiring", "grouped", "hidden"].contains(&key)
            && parse_bool(value).is_none() {
            return Err(format!("invalid boolean '{value}' for {key}"));
        }
        if key == "urgency" && !["low", "normal", "critical", "high"].contains(&value.to_ascii_lowercase().as_str()) {
            return Err(format!("invalid urgency '{value}'"));
        }
        if key == "group-index" && parse_signed_mako(value).is_none() {
            return Err(format!("invalid group-index '{value}'"));
        }
        if key == "anchor" && ![
            "top-right","top-center","top-left","bottom-right",
            "bottom-center","bottom-left","center-right","center-left","center",
        ].contains(&value) {
            return Err(format!("invalid anchor '{value}'"));
        }
        Ok(Self { key: key.into(), value: value.into(), regex })
    }

    fn matches(&self, n: &Traits<'_>) -> bool {
        let observed = match self.key.as_str() {
            "app-name" => n.app_name,
            "app-icon" => n.app_icon,
            "urgency" => {
                // Mako accepts urgency aliases and compares enum values, not spellings.
                let expected = self.value.to_ascii_lowercase();
                let observed = n.urgency.to_ascii_lowercase();
                return match expected.as_str() {
                    "high" | "critical" => observed == "critical" || observed == "high",
                    _ => expected == observed,
                };
            },
            "category" => n.category,
            "desktop-entry" => n.desktop_entry,
            "summary" | "summary~" => n.summary,
            "body" | "body~" => n.body,
            // Mako criteria.c:match_criteria never matches anchor
            // without notif->surface, nor output without both the
            // surface and its compositor-observed surface_output.
            "anchor" => {
                if !n.surface_associated { return false; }
                n.anchor
            },
            "output" => {
                if !n.surface_associated || !n.output_known { return false; }
                n.output
            },
            "mode" => return n.modes.contains(&self.value.as_str()),
            "actionable" => return parse_bool(&self.value) == Some(n.actionable),
            "expiring" => return parse_bool(&self.value) == Some(n.requested_timeout_ms != 0),
            "grouped" => return parse_bool(&self.value) == Some(n.group_index >= 0),
            "hidden" => return parse_bool(&self.value) == Some(n.hidden),
            "group-index" => return parse_signed_mako(&self.value) == Some(n.group_index),
            _ => return false,
        };
        match &self.regex {
            Some(re) => re.is_match(observed.as_bytes()),
            None => observed == self.value,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub source: String,
    pub raw_selector: String,
    pub selectors: Vec<Selector>,
    pub options: Options,
    padding_state: MakoPaddingState,
}

impl Rule {
    /// Mako makes user criteria implicitly `hidden=false` unless specified.
    pub fn matches(&self, n: &Traits<'_>) -> bool {
        if !self.selectors.iter().any(|s| s.key == "hidden") && n.hidden {
            return false;
        }
        self.selectors.iter().all(|s| s.matches(n))
    }
}

/// Borrowed notification fields needed for matching Mako's criteria.
#[derive(Clone, Debug)]
pub struct Traits<'a> {
    pub app_name: &'a str,
    pub app_icon: &'a str,
    pub category: &'a str,
    pub desktop_entry: &'a str,
    pub summary: &'a str,
    pub body: &'a str,
    pub urgency: &'a str,
    pub anchor: &'a str,
    pub output: &'a str,
    /// Mako's notif->surface, which is NULL before initial Notify styling.
    pub surface_associated: bool,
    /// Mako's notif->surface->surface_output, filled by wl_surface.enter.
    /// An empty configured output name is not proof of an entered output.
    pub output_known: bool,
    pub modes: &'a [&'a str],
    pub requested_timeout_ms: i32,
    pub actionable: bool,
    pub group_index: i32,
    pub hidden: bool,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub global: Options,
    /// Mako criteria are applied in declaration order; later rules override.
    pub rules: Vec<Rule>,
    pub max_history: usize,
    pub sort: Vec<String>,
    root_padding_state: MakoPaddingState,
}

impl Default for Config {
    fn default() -> Self {
        // Mako v1.11.0 config.c:init_default_style marks every field
        // in its root style.spec explicitly specified. Represent the
        // complete root rather than relying on scattered UI fallbacks.
        let default = [
            ("width", "300"), ("height", "100"),
            ("outer-margin", "0"), ("margin", "10"), ("padding", "5"),
            ("border-radius", "0"), ("border-size", "2"),
            ("background-color", "#285577FF"), ("text-color", "#FFFFFFFF"),
            ("border-color", "#4C7899FF"), ("progress-color", "#5588AAFF"),
            ("icons", "1"), ("max-icon-size", "64"), ("icon-path", ""),
            ("icon-border-radius", "0"), ("icon-location", "left"),
            ("font", "monospace 10"),
            ("format", "<b>%s</b>\\n%b"), ("markup", "1"),
            ("text-alignment", "left"), ("actions", "1"),
            ("default-timeout", "0"), ("ignore-timeout", "0"),
            ("group-by", "none"), ("invisible", "0"), ("history", "1"),
            ("max-visible", "5"), ("output", ""),
            ("anchor", "top-right"), ("layer", "top"),
            ("on-button-left", "invoke-default-action"),
            ("on-button-right", "dismiss"), ("on-button-middle", "none"),
            ("on-touch", "dismiss"), ("on-notify", "none"),
        ];
        Self {
            global: default.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            rules: Vec::new(), max_history: 5, sort: vec!["-time".into()],
            root_padding_state:MakoPaddingState::root(),
        }
    }
}

/// Mako v1.11 config.c:parse_config_arguments() long_options.
/// This deliberately excludes on-notify, text-alignment and include:
/// those work in config files, not as Mako command-line switches.
pub const MAKO_CLI_STYLE_OPTIONS:&[&str]=&[
    "font","background-color","text-color","width","height",
    "outer-margin","margin","padding","border-size","border-color",
    "border-radius","progress-color","icons","icon-location",
    "icon-path","max-icon-size","icon-border-radius","markup",
    "actions","format","max-visible","max-history","history",
    "default-timeout","ignore-timeout","output","layer","anchor",
    "sort","group-by","on-button-left","on-button-right",
    "on-button-middle","on-touch",
];

impl Config {
    /// Resolve root style and matching user rules. Grouped/hidden built-in
    /// selectors and cross-output layout are deliberately not claimed as ported.
    pub fn style_for(&self, n: &Traits<'_>) -> Options {
        let mut style = self.global.clone();
        if let Some(padding)=self.root_padding_state.override_padding(){
            style.insert("padding".into(),padding);
        }
        // Mako installs a hidden-notification formatting rule before user
        // criteria. Later [hidden] rules may override this built-in format.
        if n.hidden {
            style.insert("format".into(),"(%h more)".into());
        } else if n.group_index>=0 {
            // Mako's ordered built-ins first hide all grouped members, then
            // make index zero visible. Explicit user rules run afterwards.
            style.insert("format".into(),"(%g) <b>%s</b>\\n%b".into());
            style.insert("invisible".into(),
                if n.group_index==0{"0"}else{"1"}.into());
        }
        for rule in &self.rules {
            if rule.matches(n) {
                style.extend(rule.options.clone());
                if let Some(padding)=rule.padding_state.override_padding(){
                    style.insert("padding".into(),padding);
                }
            }
        }
        style
    }

    /// Apply a Mako -style option *after* reading the config file, as
    /// upstream parse_config_arguments does on its second getopt pass.
    /// The private --check-config mode uses this without launching a
    /// notification daemon. Never parse the value as additional file
    /// lines: an argv value may contain newline characters.
    pub fn apply_cli_override(&mut self,key:&str,value:&str)
        -> Result<(),ConfigError>
    {
        if !MAKO_CLI_STYLE_OPTIONS.contains(&key) {
            return Err(err("<cli>",format!("unsupported Mako command-line option '{key}'")));
        }
        match key {
            "sort" => {
                if !["+priority","-priority","+time","-time"].contains(&value) {
                    return Err(err("<cli>",format!("invalid sort order '{value}'")));
                }
                self.sort.retain(|existing| &existing[1..]!=&value[1..]);
                self.sort.push(value.into());
            }
            "max-history" => {
                let value=parse_signed_mako(value)
                    .ok_or_else(||err("<cli>","invalid max-history"))?;
                self.max_history=value.max(0) as usize;
            }
            _ => {
                validate_option(key,value).map_err(|why|err("<cli>",why))?;
                // parse_directional with zero strtok_r tokens leaves the
                // existing default/root style unchanged in source Mako.
                if ["padding","margin","outer-margin","border-radius"].contains(&key)
                    && value.split(',').all(str::is_empty) {
                    return Ok(());
                }
                self.global.insert(key.into(),value.into());
                self.root_padding_state.assign(key,value);
            }
        }
        Ok(())
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let mut conf = Self::default();
        conf.append(text, "<input>", None, &mut HashSet::new())?;
        Ok(conf)
    }

    /// Mako v1.11 searches ~/.mako/config before XDG_CONFIG_HOME/mako/config
    /// (falling back to ~/.config/mako/config). Do not search XDG_CONFIG_DIRS:
    /// upstream Mako does not use it for configuration discovery.
    pub fn find_default_path(home:&Path,xdg_config_home:Option<&Path>)->Option<PathBuf>{
        let legacy=home.join(".mako/config");
        if fs::File::open(&legacy).is_ok(){return Some(legacy);}
        let fallback=home.join(".config");
        let config_home=xdg_config_home.unwrap_or(&fallback);
        let modern=config_home.join("mako/config");
        if fs::File::open(&modern).is_ok(){return Some(modern);}
        None
    }

    /// Read the same implicit config as Mako when available; otherwise use
    /// upstream defaults. No daemon startup or notification bus access occurs.
    pub fn from_mako_defaults()->Result<(Self,Option<PathBuf>),ConfigError>{
        let Some(home)=std::env::var_os("HOME") else {
            return Ok((Self::default(),None));
        };
        let xdg=std::env::var_os("XDG_CONFIG_HOME").filter(|p|!p.is_empty());
        let path=Self::find_default_path(Path::new(&home),xdg.as_deref().map(Path::new));
        match path {
            Some(path)=>{
                let config=Self::from_path(&path)?;
                Ok((config,Some(path)))
            }
            None=>Ok((Self::default(),None)),
        }
    }

    /// Includes are resolved with Mako's absolute or `~/` path convention.
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let mut conf = Self::default();
        let mut seen = HashSet::new();
        conf.load(path, &mut seen)?;
        Ok(conf)
    }

    fn load(&mut self, path: &Path, seen: &mut HashSet<PathBuf>) -> Result<(), ConfigError> {
        let canonical = path.canonicalize().map_err(|e| err(path.display().to_string(), e.to_string()))?;
        if !seen.insert(canonical.clone()) {
            return Err(err(path.display().to_string(), "recursive include"));
        }
        let result = (|| {
            let bytes = fs::read(&canonical)
                .map_err(|e| err(path.display().to_string(), e.to_string()))?;
            let text=decode_mako_file_bytes(&bytes,&canonical.display().to_string())?;
            self.append(&text, &canonical.display().to_string(), Some(&canonical), seen)
        })();
        seen.remove(&canonical);
        result
    }

    fn append(&mut self, text: &str, source: &str, path: Option<&Path>,
              seen: &mut HashSet<PathBuf>) -> Result<(), ConfigError> {
        let mut current: Option<usize> = None;
        // getline() in pinned Mako strips leading/trailing ASCII spaces
        // and tabs only. Rust lines() and trim() also discard CRLF and
        // Unicode whitespace, silently accepting files Mako rejects.
        for (lineno, raw) in text.split('\n').enumerate() {
            let line = raw.trim_matches([' ', '\t']);
            if line.is_empty() || line.starts_with('#') { continue; }
            let location = format!("{source}:{}", lineno + 1);
            if line.starts_with('[') && line.ends_with(']') {
                let selector = &line[1..line.len()-1];
                let selectors = parse_selectors(selector).map_err(|e| err(&location, e))?;
                self.rules.push(Rule {
                    source: location, raw_selector: selector.into(), selectors,
                    options: Options::new(),
                    padding_state:MakoPaddingState::default(),
                });
                current = Some(self.rules.len() - 1);
                continue;
            }
            let (key, val) = line.split_once('=').ok_or_else(||err(&location,"expected key=value"))?;
            // Mako doesn't treat `#` after '=' as a comment: it is often a color.
            // Mako strips outer line whitespace, but NOT whitespace before
            // '='. "width =400" is therefore an unknown option.
            // The whole line was already stripped of Mako's ASCII
            // surrounding space/tab characters; a second Unicode trim
            // would incorrectly eat CR and NBSP after '='.
            let value = val;
            if key == "include" && current.is_none() {
                let path = resolve_include(value).map_err(|e|err(&location,e))?;
                self.load(&path, seen)?;
            } else if current.is_none() && key == "max-history" {
                // Mako parses max-history as a signed int and treats
                // zero or negative values as history disabled.
                let signed=parse_signed_mako(value)
                    .ok_or_else(||err(&location,"invalid max-history"))?;
                self.max_history=signed.max(0) as usize;
            } else if current.is_none() && key == "sort" {
                if !["+priority", "-priority", "+time", "-time"].contains(&value) {
                    return Err(err(&location,"invalid sort order"));
                }
                // Mako sets a bit per sort dimension; the last direction wins.
                self.sort.retain(|existing| &existing[1..] != &value[1..]);
                self.sort.push(value.into());
            } else {
                validate_option(key, value).map_err(|e|err(&location,e))?;
                // An empty strtok_r result leaves the *current C style*
                // unchanged. The root already has built-in values, whereas
                // a newly created criteria section begins with zeroes.
                if ["margin","outer-margin","padding","border-radius"].contains(&key)
                    && value.split(',').all(str::is_empty) {
                    if let Some(index)=current {
                        let rule=&mut self.rules[index];
                        if !rule.options.contains_key(key){
                            rule.options.insert(key.into(),"0".into());
                            rule.padding_state.assign(key,"0");
                        }
                    }
                    continue;
                }
                match current {
                    Some(index) => {
                        let rule=&mut self.rules[index];
                        rule.options.insert(key.into(),value.into());
                        rule.padding_state.assign(key,value);
                    },
                    None => {
                        self.global.insert(key.into(),value.into());
                        self.root_padding_state.assign(key,value);
                    },
                }
            }
        }
        // Upstream Mako validates the *whole criteria section*, not only
        // individual options. Includes are validated independently, too.
        for rule in &self.rules {
            validate_rule(rule).map_err(|message| err(&rule.source, message))?;
        }
        let _ = path; // reserved for richer include diagnostics
        Ok(())
    }
}

/// Match upstream Mako 1.11 validate_criteria restrictions. A syntactically
/// valid option can still be invalid for the selectors of its section.
fn validate_rule(rule: &Rule) -> Result<(), String> {
    let has = |key: &str| rule.selectors.iter().any(|selector| selector.key == key);
    if has("summary") && has("summary~") {
        return Err("cannot combine summary and summary~ selectors".into());
    }
    if has("body") && has("body~") {
        return Err("cannot combine body and body~ selectors".into());
    }

    let surface_or_group = ["grouped", "group-index", "output", "anchor"]
        .iter().any(|key| has(key));
    if surface_or_group {
        for option in ["anchor", "output", "group-by"] {
            if rule.options.contains_key(option) {
                return Err(format!("'{option}' cannot be set under grouped, group-index, output or anchor criteria"));
            }
        }
    }

    // Mako 1.11's validate_criteria checks style.max_visible itself,
    // NOT style.spec.max_visible. A zero value is permitted under category,
    // app-name, etc., even when the option was explicitly assigned. C's
    // parse_int uses (int)strtol, so signed 32-bit narrowing also matters.
    let non_surface = rule.selectors.iter().any(|selector|
        !matches!(selector.key.as_str(), "output" | "anchor" | "hidden"));
    if non_surface && rule.options.get("max-visible")
        .and_then(|value| parse_signed_mako(value))
        .is_some_and(|value| value != 0) {
        return Err("nonzero max-visible is only allowed with output and/or anchor criteria".into());
    }
    if non_surface && rule.selectors.iter().any(|selector|
        selector.key == "hidden" && parse_bool(&selector.value) == Some(true)) {
        return Err("hidden=true cannot be combined with non-surface selectors".into());
    }
    Ok(())
}

fn resolve_include(value: &str) -> Result<PathBuf, String> {
    if value.starts_with('/') { return Ok(PathBuf::from(value)); }
    if let Some(relative) = value.strip_prefix("~/") {
        return std::env::var_os("HOME").map(|h| PathBuf::from(h).join(relative))
            .ok_or_else(|| "HOME is not set".into());
    }
    Err("Mako include must be absolute or start with ~/".into())
}

fn parse_selectors(text: &str) -> Result<Vec<Selector>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = false;
    let mut escape = false;
    for ch in text.chars() {
        if escape { current.push(ch); escape = false; continue; }
        match ch {
            '\\' => escape = true,
            '"' => quote = !quote,
            ' ' if !quote => {
                if !current.is_empty() { tokens.push(std::mem::take(&mut current)); }
            }
            _ => current.push(ch),
        }
    }
    if escape { return Err("trailing backslash in criteria".into()); }
    if quote { return Err("unmatched quote in criteria".into()); }
    if !current.is_empty() { tokens.push(current); }
    let parsed: Vec<Selector> = tokens.into_iter().map(|token| {
        if let Some((key, value)) = token.split_once('=') {
            Selector::new(key, value)
        } else if let Some(key) = token.strip_prefix('!') {
            if ["actionable","expiring","grouped","hidden"].contains(&key) {
                Selector::new(key,"false")
            } else {
                Err(format!("invalid boolean criteria field '{token}'"))
            }
        } else if ["actionable","expiring","grouped","hidden"].contains(&token.as_str()) {
            Selector::new(&token, "true")
        } else {
            Err(format!("invalid boolean criteria field '{token}'"))
        }
    }).collect::<Result<_, _>>()?;
    // Upstream parse_criteria writes fields to one criteria struct. Repeating
    // the same field replaces its prior value; matching every occurrence
    // independently would incorrectly make many valid sections impossible.
    let mut selectors: Vec<Selector> = Vec::new();
    for selector in parsed {
        if let Some(existing) = selectors.iter_mut().find(|s| s.key == selector.key) {
            *existing = selector;
        } else {
            selectors.push(selector);
        }
    }
    Ok(selectors)
}

/// Mako 1.11 parse_int calls (int)strtol(text, &end, 10), then accepts
/// errno==0 and *end=='\\0'. On 64-bit Linux this parses an i64 and NARROWS
/// to i32, so values in i64 but outside i32 wrap. Empty input is also
/// accepted as zero by strtol's end-pointer test; whitespace-only is not.
pub fn parse_signed_mako(text:&str)->Option<i32>{
    if text.is_empty(){return Some(0);}
    // C strtol skips all six C isspace bytes, including vertical tab.
    let trimmed=text.trim_start_matches([' ','\t','\n','\r','\x0b','\x0c']);
    let wide=trimmed.parse::<i64>().ok()?;
    Some(wide as i32)
}

/// Apply the same 64-bit strtol plus narrowing that config validation uses.
/// Negative narrowed values still fail Mako's parse_int_ge minimum check.
pub fn parse_nonnegative_mako(text:&str)->Option<u32>{
    u32::try_from(parse_signed_mako(text)?).ok()
}

fn parse_bool(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() { "1" | "true" => Some(true), "0" | "false" => Some(false), _ => None }
}

fn validate_option(key: &str, value: &str) -> Result<(), String> {
    const BOOL: &[&str] = &["markup", "actions", "icons", "ignore-timeout", "invisible", "history"];
    const NONNEG: &[&str] = &["width", "height", "border-size", "max-icon-size",
        "icon-border-radius", "default-timeout"];
    const STRINGS: &[&str] = &["font", "icon-path", "output"];
    const COLORS: &[&str] = &["background-color", "text-color", "border-color"];
    const DIRECTIONS: &[&str] = &["padding", "margin", "outer-margin", "border-radius"];
    const BINDS: &[&str] = &["on-button-left", "on-button-right", "on-button-middle", "on-touch", "on-notify"];
    if BOOL.contains(&key) {
        if parse_bool(value).is_some() { return Ok(()); }
    } else if NONNEG.contains(&key) {
        if let Some(number) = parse_signed_mako(value) {
            if number >= (if key=="width"||key=="height"||key=="max-icon-size" { 1 } else { 0 }) { return Ok(()); }
        }
    } else if key == "max-visible" {
        if parse_signed_mako(value).is_some() { return Ok(()); }
    } else if key == "format" { return crate::format::decode_format(value).map(|_| ()).map_err(|e| e.to_string()); }
    else if STRINGS.contains(&key) { return Ok(()); }
    else if key == "group-by" {
        const FIELDS: &[&str] = &[
            "app-name", "app-icon", "actionable", "expiring", "urgency",
            "category", "desktop-entry", "summary", "body", "grouped",
            "group-index", "anchor", "output", "none",
        ];
        // Mako accepts comma-separated criteria *field names*, not selector values.
        // Mako uses strtok_r here: repeated/trailing separators are ignored,
        // and 'none' wins even when combined with other valid fields.
        // Some fields parse successfully but are rejected by validate_criteria.
        let fields: Vec<_> = value.split(',').filter(|field| !field.is_empty()).collect();
        if fields.iter().all(|field| FIELDS.contains(field))
            && !fields.iter().any(|field| ["grouped", "group-index", "anchor", "output"].contains(field)) {
            return Ok(());
        }
    }
    else if DIRECTIONS.contains(&key) {
        // Mako uses strtok_r and reads at most four directional integers:
        // repeated separators are ignored and extra values are unexamined.
        let items:Vec<_>=value.split(',')
            .filter(|item|!item.is_empty()).take(4).collect();
        if items.iter().all(|item|parse_signed_mako(item).is_some()){
            return Ok(());
        }
    } else if COLORS.contains(&key) {
        // This must use the same glibc strtoul-compatible parser as the
        // renderer; strict per-digit validation would reject Mako config.
        if crate::raster::Color::parse_mako(value).is_ok() { return Ok(()); }
    } else if key=="progress-color" {
        if crate::raster::parse_progress_color_mako(value).is_ok(){
            return Ok(());
        }
    }
    else if key=="icon-location" { if ["left","right","top","bottom"].contains(&value) { return Ok(()); } }
    else if key=="text-alignment" { if ["left","right","center"].contains(&value) { return Ok(()); } }
    else if key=="layer" { if ["background","bottom","top","overlay"].contains(&value) { return Ok(()); } }
    else if key=="anchor" { if ["top-right","top-center","top-left","bottom-right","bottom-center","bottom-left","center-right","center-left","center"].contains(&value) { return Ok(()); } }
    else if BINDS.contains(&key) {
        if ["none", "dismiss", "dismiss --no-history", "dismiss-all", "dismiss-group", "invoke-default-action"].contains(&value)
            || value.starts_with("exec ") || value.starts_with("invoke-action ") { return Ok(()); }
    } else { return Err(format!("unsupported Mako option '{key}'")); }
    Err(format!("invalid value '{value}' for '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn note<'a>(category: &'a str, modes: &'a [&'a str]) -> Traits<'a> {
        Traits { app_name: "example", app_icon:"", category, desktop_entry:"", summary:"Saved",
            body:"All changes written", urgency:"normal", anchor:"top-right", output:"",
            surface_associated:true,output_known:true,modes, requested_timeout_ms:-1, actionable:false, group_index:-1, hidden:false }
    }

    #[test]
    fn surface_selectors_require_real_surface_and_entered_output() {
        // In source criteria.c, [output=] must not match NULL
        // surface_output even though both strings are represented as "".
        let config=Config::parse(
            "[output=]\nwidth=411\n[anchor=top-right]\nheight=211\n"
        ).unwrap();
        let mut fields=note("ordinary",&["default"]);
        fields.surface_associated=false;
        fields.output_known=false;
        assert_eq!(config.style_for(&fields)["width"],"300");
        assert_eq!(config.style_for(&fields)["height"],"100");
        fields.surface_associated=true;
        // A surface may be created before the compositor sends enter.
        assert_eq!(config.style_for(&fields)["width"],"300");
        assert_eq!(config.style_for(&fields)["height"],"211");
        fields.output_known=true;
        assert_eq!(config.style_for(&fields)["width"],"411");
        fields.output="eDP-1";
        assert_eq!(config.style_for(&fields)["width"],"300",
            "entered output must actually have the empty name");
    }

    #[test]
    fn default_root_style_explicitly_matches_mako_v111_source() {
        // config.c:init_default_style sets the complete style and marks
        // style.spec entirely specified (memset(..., true, ...)).
        let config=Config::default();
        let expected=[
            ("width","300"),("height","100"),("outer-margin","0"),
            ("margin","10"),("padding","5"),("border-radius","0"),
            ("border-size","2"),("icons","1"),("max-icon-size","64"),
            ("icon-path",""),("icon-border-radius","0"),
            ("icon-location","left"),("text-alignment","left"),
            ("progress-color","#5588AAFF"),("group-by","none"),
            ("invisible","0"),("output",""),("max-visible","5"),
            ("on-notify","none"),("anchor","top-right"),
        ];
        for (key,value) in expected {
            assert_eq!(config.global.get(key).map(String::as_str),Some(value),
                "Mako root default missing or mismatched: {key}");
        }
        for (key,value) in &config.global {
            validate_option(key,value).unwrap_or_else(|error|
                panic!("Mako root style option {key}={value:?} invalid: {error}"));
        }
        let style=config.style_for(&note("success",&["default"]));
        for (key,value) in expected {
            assert_eq!(style.get(key).map(String::as_str),Some(value));
        }
        assert_eq!(config.rules.len(),0,
            "built-in root style does not create user criteria sections");
    }

    #[test]
    fn cli_overrides_apply_after_config_without_mutating_scoped_rules() {
        let mut cfg=Config::parse(
            "width=200\nfont=monospace 10\nmax-history=4\n\
             sort=-priority\n[category=warning]\nwidth=340\n"
        ).unwrap();
        for (key,value) in [
            ("width","450"),("font","Luciole 14"),("max-history","3"),
            ("sort","+priority"),("sort","+time")
        ]{
            cfg.apply_cli_override(key,value).unwrap();
        }
        assert_eq!(cfg.global["width"],"450");
        assert_eq!(cfg.global["font"],"Luciole 14");
        assert_eq!(cfg.max_history,3);
        assert_eq!(cfg.sort,vec!["+priority","+time"]);
        assert_eq!(cfg.style_for(&note("warning",&["default"]))["width"],"340",
            "scoped criteria retain their source priority over CLI root style");
        assert_eq!(cfg.style_for(&note("success",&["default"]))["width"],"450");
        cfg.apply_cli_override("sort","-priority").unwrap();
        assert_eq!(cfg.sort,vec!["+time","-priority"]);
    }

    #[test]
    fn cli_overrides_validate_atomically_and_do_not_parse_injected_lines() {
        let mut cfg=Config::parse("width=210\nmax-history=5\n").unwrap();
        for (key,value) in [
            ("width","0"),("width","320\nhistory=0"),
            ("max-history","9223372036854775808"),("sort","random"),
            ("include","/tmp/other.conf"),("on-notify","dismiss"),
            ("text-alignment","center"),("width"," \tgarbage"),
        ]{
            assert!(cfg.apply_cli_override(key,value).is_err(),
                "invalid/unsupported CLI override accepted: {key}={value:?}");
            assert_eq!(cfg.global["width"],"210");
            assert_eq!(cfg.max_history,5);
            assert!(!cfg.global.contains_key("include"));
        }
        cfg.apply_cli_override("width"," 320").unwrap();
        assert_eq!(cfg.global["width"]," 320",
            "Mako strtol accepts leading space in numeric argument");
        cfg.apply_cli_override("margin",",,,").unwrap();
        assert_eq!(cfg.global["margin"],"10",
            "empty strtok_r directional option preserves current root value");
    }

    #[test]
    fn max_visible_criteria_validation_matches_upstream_numeric_value() {
        // Mako v1.11 criteria.c:validate_criteria tests the numeric
        // max_visible field, rather than its explicitly-set flag.
        // Its parse_int narrows 64-bit strtol to C int on Linux.
        for zero in ["0", "+0", "-0", "", "4294967296", "-4294967296"] {
            let source=format!("[category=urgent]\nmax-visible={zero}\n");
            let config=Config::parse(&source).unwrap_or_else(|e|
                panic!("zero-valued max-visible rejected: {zero:?}: {e}"));
            let style=config.style_for(&note("urgent",&["default"]));
            assert_eq!(parse_signed_mako(&style["max-visible"]),Some(0));
        }
        for nonzero in ["1", "-1", "4294967297", "-4294967297"] {
            assert!(Config::parse(&format!(
                "[category=urgent]\nmax-visible={nonzero}\n"
            )).is_err(),"nonzero max-visible accepted under category: {nonzero}");
        }
        // Mako validates a section at its end using the final value.
        assert!(Config::parse(
            "[category=urgent]\nmax-visible=3\nmax-visible=0\n"
        ).is_ok());
        assert!(Config::parse(
            "[category=urgent]\nmax-visible=0\nmax-visible=3\n"
        ).is_err());
        // Under a surface selector, any signed value is allowed by Mako.
        assert!(Config::parse("[output=DP-1]\nmax-visible=-1\n").is_ok());
    }

    #[test]
    fn border_radius_padding_clamp_follows_mako_style_scope_and_assignment_order(){
        let fields=note("success",&["default"]);
        let root=Config::parse("border-radius=20\n").unwrap();
        assert_eq!(root.global["padding"],"5",
            "the parsed user input remains available unchanged");
        assert_eq!(root.style_for(&fields)["padding"],"5,20,5,20",
            "root style includes default padding and explicit radius");
        let lowered=Config::parse("border-radius=20\nborder-radius=0\n").unwrap();
        assert_eq!(lowered.style_for(&fields)["padding"],"5,20,5,20",
            "Mako does not reverse a previous config-time clamp");
        let reset=Config::parse("border-radius=20\npadding=8\nborder-radius=0\n").unwrap();
        assert_eq!(reset.style_for(&fields)["padding"],"8,20,8,20",
            "a padding reassignment reclamps against the then-current radius");

        let radius_only=Config::parse(
            "[category=success]\nborder-radius=30\n"
        ).unwrap();
        assert_eq!(radius_only.style_for(&fields)["padding"],"5",
            "a scoped radius-only rule does not reclamp inherited root padding");
        let scoped=Config::parse(
            "[category=success]\npadding=2\nborder-radius=30\n"
        ).unwrap();
        assert_eq!(scoped.style_for(&fields)["padding"],"2,30,2,30");
        let different=note("failure",&["default"]);
        assert_eq!(scoped.style_for(&different)["padding"],"5");
        let order=Config::parse(
            "[category=success]\nborder-radius=30\npadding=2\nborder-radius=4\n"
        ).unwrap();
        assert_eq!(order.style_for(&fields)["padding"],"2,30,2,30");
        let later=Config::parse(
            "[category=success]\npadding=2\nborder-radius=30\n            [category=success]\npadding=7\n"
        ).unwrap();
        assert_eq!(later.style_for(&fields)["padding"],"7",
            "a later independent scoped padding override is not re-clamped");
    }

    #[test]
    fn grouped_secondary_members_are_hidden_by_default_but_can_be_unhidden(){
        let conf=Config::default();
        let mut fields=note("ordinary",&["default"]);
        fields.group_index=1;
        assert_eq!(conf.style_for(&fields).get("invisible").map(String::as_str),Some("1"));
        fields.group_index=0;
        assert_eq!(conf.style_for(&fields).get("invisible").map(String::as_str),Some("0"));
        let explicit=Config::parse("[grouped]\ninvisible=0\n").unwrap();
        fields.group_index=2;
        assert_eq!(explicit.style_for(&fields).get("invisible").map(String::as_str),
            Some("0"));
    }

    #[test]
    fn grouped_first_notification_uses_mako_count_format() {
        let defaults=Config::default();
        let mut fields=note("normal",&["default"]);
        fields.group_index=0;
        assert_eq!(defaults.style_for(&fields).get("format").map(String::as_str),
            Some("(%g) <b>%s</b>\\n%b"));
        let custom=Config::parse("[group-index=0]\nformat=[%g] %s\n").unwrap();
        assert_eq!(custom.style_for(&fields).get("format").map(String::as_str),
            Some("[%g] %s"));
    }

    #[test]
    fn hidden_format_defaults_are_overridable_by_explicit_hidden_criteria() {
        let ordinary=Config::parse("format=%s\n").unwrap();
        let mut fields=note("system",&["default"]);
        fields.hidden=true;
        assert_eq!(ordinary.style_for(&fields).get("format").map(String::as_str),
            Some("(%h more)"));
        let custom=Config::parse("[hidden]\nformat=<b>%h of %t unseen</b>\n").unwrap();
        assert_eq!(custom.style_for(&fields).get("format").map(String::as_str),
            Some("<b>%h of %t unseen</b>"));
    }

    #[test]
    fn glibc_color_spellings_validate_and_reach_effective_styles() {
        let conf=Config::parse(
            "background-color=#+fffff\nborder-color=#0x1234\n            [category=warning]\ntext-color=#-fffff\n"
        ).unwrap();
        let style=conf.style_for(&note("warning",&["default"]));
        assert_eq!(style["background-color"],"#+fffff");
        assert_eq!(style["border-color"],"#0x1234");
        assert_eq!(style["text-color"],"#-fffff");
        for bad in ["#12345G","#abcdef0","#  " ] {
            assert!(Config::parse(&format!("background-color={bad}\n")).is_err());
        }
    }

    #[test]
    fn progress_color_parses_mako_source_and_over_syntax() {
        for value in ["#5588AAFF","source #123456","OVER #55667788",
            "source #FFFFFF trailing"," #112233 extra"] {
            assert!(Config::parse(&format!("progress-color={value}\n")).is_ok());
        }
        for value in ["purple","source","multiply #112233",
            "over #11223","source\u{a0}#FFFFFF"] {
            assert!(Config::parse(&format!("progress-color={value}\n")).is_err());
        }
    }

    #[test]
    fn mako_directional_tokenization_ignores_empty_and_fifth_values() {
        let c=Config::parse(
            "margin=10,,20,30,40,unused\n"
        ).unwrap();
        assert_eq!(c.global["margin"],"10,,20,30,40,unused");
        let no_change=Config::parse(
            "margin=11,22\n[category=warning]\nmargin=,,,\n"
        ).unwrap();
        assert_eq!(no_change.global["margin"],"11,22");
        assert_eq!(no_change.rules[0].options.get("margin").map(String::as_str),
            Some("0"),"empty section-local margin inherits empty style zero");
        let earlier=Config::parse(
            "[category=warning]\nmargin=7\nmargin=,,,\n"
        ).unwrap();
        assert_eq!(earlier.rules[0].options["margin"],"7");
        let root=Config::parse("margin=,,\n").unwrap();
        assert_eq!(root.global["margin"],"10");
        assert!(Config::parse("margin=1 ,2\n").is_err());
        assert!(Config::parse("margin=1,nope,3\n").is_err());
    }

    #[test]
    fn config_line_whitespace_matches_mako_ascii_only() {
        let parsed=Config::parse("\twidth=215  \n\t[category=warning] \n\tbackground-color=#715536 \n").unwrap();
        assert_eq!(parsed.global["width"],"215");
        assert_eq!(parsed.rules.len(),1);
        assert_eq!(parsed.rules[0].options["background-color"],"#715536");
        assert!(Config::parse("width=215\r\n").is_err(),
            "Mako getline does not remove the carriage return");
        assert!(Config::parse("\u{a0}width=215\n").is_err(),
            "Unicode NBSP is not a Mako leading whitespace character");
        assert!(Config::parse("width=215\u{a0}\n").is_err(),
            "Unicode NBSP is not stripped from Mako values");
    }

    #[test]
    fn numeric_runtime_conversion_keeps_mako_valid_leading_whitespace(){
        assert_eq!(parse_signed_mako("  -42"),Some(-42));
        assert_eq!(parse_signed_mako("\t+42"),Some(42));
        assert_eq!(parse_signed_mako("\u{b}-7"),Some(-7));
        assert_eq!(parse_signed_mako("42 "),None);
        assert_eq!(parse_signed_mako("2147483648"),Some(i32::MIN));
        assert_eq!(parse_signed_mako("4294967297"),Some(1));
        assert_eq!(parse_signed_mako("-2147483649"),Some(i32::MAX));
        assert_eq!(parse_signed_mako(""),Some(0));
        assert_eq!(parse_signed_mako(" "),None);
        assert_eq!(parse_signed_mako("9223372036854775808"),None);
        assert_eq!(parse_nonnegative_mako("4294967297"),Some(1));
        assert_eq!(parse_nonnegative_mako(" \t+440"),Some(440));
        assert_eq!(parse_nonnegative_mako(" -4"),None);
    }

    #[test]
    fn empty_and_wrapped_mako_config_integers_follow_c_strtol(){
        let config=Config::parse(
            "default-timeout=\nborder-size=\nmax-history=4294967298\nwidth=4294967297\n"
        ).unwrap();
        assert_eq!(config.max_history,2);
        assert_eq!(parse_nonnegative_mako(&config.global["default-timeout"]),Some(0));
        assert_eq!(parse_nonnegative_mako(&config.global["width"]),Some(1));
        assert_eq!(parse_nonnegative_mako(&config.global["border-size"]),Some(0));
        assert!(Config::parse("width=2147483648\n").is_err(),
            "strtol accepted it, but narrowing to negative fails min=1");
        assert!(Config::parse("max-history=9223372036854775808\n").is_err(),
            "LP64 strtol overflows before narrowing");
        assert!(Config::parse("default-timeout= \n").is_ok(),
            "getline strips the single trailing ASCII space");
    }

    #[test]
    fn mako_numeric_values_allow_leading_space_but_option_keys_do_not() {
        let c=Config::parse("width= 320\nheight= 75\nmax-history= 4\nmargin=2, 4, 6, 8\n").unwrap();
        assert_eq!(c.global["width"]," 320");
        assert_eq!(c.max_history,4);
        assert_eq!(c.global["margin"],"2, 4, 6, 8");
        assert!(Config::parse("width =320\n").is_err());
        assert!(Config::parse("width=320 garbage\n").is_err());
        assert!(Config::parse("max-history=4 garbage\n").is_err());
    }

    #[test]
    fn mako_nested_includes_do_not_hit_an_artificial_depth_limit() {
        let temporary=tempfile::tempdir().unwrap();
        let file=|i:usize|temporary.path().join(format!("part-{i:02}.conf"));
        // Upstream Mako recursively calls load_config_file without a
        // fixed depth cap; a valid modular config can exceed 16 includes.
        for i in 0..22 {
            let content=if i==21 {
                "width=440\nmax-history=3\n".to_string()
            } else {
                format!("include={}\n",file(i+1).display())
            };
            std::fs::write(file(i),content).unwrap();
        }
        let config=Config::from_path(&file(0)).unwrap();
        assert_eq!(config.global["width"],"440");
        assert_eq!(config.max_history,3);
        // Keep the existing safety guard on actually recursive includes.
        std::fs::write(file(21),format!("include={}\n",file(0).display())).unwrap();
        let result=Config::from_path(&file(0)).unwrap_err();
        assert!(result.message.contains("recursive include"));
    }

    #[test]
    fn upstream_byte_comments_do_not_block_valid_utf8_configuration(){
        let dir=tempfile::tempdir().unwrap();
        let root=dir.path().join("root.conf");
        let child=dir.path().join("child.conf");
        let mut nested=b"  # legacy Latin-1 comment: ".to_vec();
        nested.extend_from_slice(&[0xE9,0xFF,b'\n']);
        nested.extend_from_slice(b"width=420\n[category=success]\ntext-color=#AA3344\n");
        std::fs::write(&child,&nested).unwrap();
        let mut parent=b"# binary comment ".to_vec();
        parent.extend_from_slice(&[0x80,b'\n']);
        parent.extend_from_slice(format!(
            "include={}\nheight=111\n",child.display()).as_bytes());
        std::fs::write(&root,&parent).unwrap();

        let config=Config::from_path(&root).unwrap();
        assert_eq!(config.global["width"],"420");
        assert_eq!(config.global["height"],"111");
        assert_eq!(config.rules.len(),1);
        assert_eq!(config.rules[0].source,format!("{}:3",child.display()));
        assert_eq!(config.rules[0].options["text-color"],"#AA3344");
    }

    #[test]
    fn invalid_utf8_in_active_file_line_fails_without_lossy_replacement(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("config");
        std::fs::write(&path,b"# allowed \xFF\nwidth=330\nfont=bad\xFFname\n").unwrap();
        let err=Config::from_path(&path).unwrap_err();
        assert_eq!(err.location,format!("{}:3",path.display()));
        assert!(err.message.contains("non-UTF-8"));
        // In-memory already-UTF-8 configs still parse directly.
        assert_eq!(Config::parse("# caf\u{e9}\nwidth=331\n")
            .unwrap().global["width"],"331");
    }

    #[test]
    fn nested_include_precedence_and_criteria_remain_in_source_order(){
        let dir=tempfile::tempdir().unwrap();
        let root=dir.path().join("root.conf");
        let first=dir.path().join("first.conf");
        let second=dir.path().join("second.conf");
        std::fs::write(&second,
            "width=380\n[category=warning]\nbackground-color=#334455\n"
        ).unwrap();
        std::fs::write(&first,format!(
            "include={}\nwidth=420\n[category=success]\nbackground-color=#111111\n",
            second.display()
        )).unwrap();
        std::fs::write(&root,format!(
            "width=310\ninclude={}\nwidth=460\n\
             [category=success]\nbackground-color=#222222\n",
            first.display()
        )).unwrap();
        let config=Config::from_path(&root).unwrap();
        assert_eq!(config.global["width"],"460",
            "outer root styles after include override included defaults");
        assert_eq!(config.rules.len(),3,
            "inlined nested criteria retain their original positions");
        assert!(config.rules[0].source.starts_with(&second.display().to_string()));
        assert!(config.rules[1].source.starts_with(&first.display().to_string()));
        assert!(config.rules[2].source.starts_with(&root.display().to_string()));
        let warn=config.style_for(&note("warning",&["default"]));
        assert_eq!(warn["background-color"],"#334455");
        assert_eq!(warn["width"],"460");
        let success=config.style_for(&note("success",&["default"]));
        assert_eq!(success["background-color"],"#222222",
            "later outer criteria win over the same selector in included file");
    }

    #[test]
    fn mako_config_lookup_prefers_legacy_then_xdg_then_home_fallback() {
        let root=tempfile::tempdir().unwrap();
        let home=root.path().join("home");
        let xdg=root.path().join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(Config::find_default_path(&home,Some(&xdg)),None);

        std::fs::create_dir_all(xdg.join("mako")).unwrap();
        std::fs::write(xdg.join("mako/config"),"width=320\n").unwrap();
        assert_eq!(Config::find_default_path(&home,Some(&xdg)),
            Some(xdg.join("mako/config")));

        std::fs::create_dir_all(home.join(".mako")).unwrap();
        std::fs::write(home.join(".mako/config"),"width=420\n").unwrap();
        assert_eq!(Config::find_default_path(&home,Some(&xdg)),
            Some(home.join(".mako/config")));

        std::fs::remove_file(home.join(".mako/config")).unwrap();
        std::fs::create_dir_all(home.join(".config/mako")).unwrap();
        std::fs::write(home.join(".config/mako/config"),"width=300\n").unwrap();
        assert_eq!(Config::find_default_path(&home,None),
            Some(home.join(".config/mako/config")));
    }

    #[test]
    fn defaults_follow_mako_1_11() {
        let c=Config::default();
        assert_eq!(c.max_history,5);
        assert_eq!(c.global["width"],"300");
        assert_eq!(c.global["default-timeout"],"0");
    }
    #[test]
    fn category_colors_and_sounds() {
        let c=Config::parse("default-timeout=7000\nignore-timeout=1\nbackground-color=#3B4858\n[category=success]\nbackground-color=#305747\non-notify=exec /usr/bin/pw-play success.wav\n[category=failure]\nbackground-color=#684148\n[category=warning]\nbackground-color=#715536\n").unwrap();
        assert_eq!(c.style_for(&note("success", &["default"]))["background-color"],"#305747");
        assert_eq!(c.style_for(&note("failure", &["default"]))["background-color"],"#684148");
        assert_eq!(c.style_for(&note("warning", &["default"]))["background-color"],"#715536");
        assert_eq!(c.style_for(&note("other", &["default"]))["background-color"],"#3B4858");
        assert_eq!(c.style_for(&note("success", &["default"]))["default-timeout"],"7000");
    }
    #[test]
    fn modes_override_without_suppressing_colors() {
        let c=Config::parse("[category=success]\nbackground-color=#305747\n[mode=quiet]\non-notify=none\n").unwrap();
        let normal=c.style_for(&note("success", &["default"]));
        assert_eq!(normal["on-notify"],"none",
            "Mako root explicitly defaults on-notify to a no-op");
        let quiet=c.style_for(&note("success", &["default","quiet"]));
        assert_eq!(quiet["background-color"],"#305747");
        assert_eq!(quiet["on-notify"],"none");
    }
    #[test]
    fn glibc_bare_opening_bracket_is_literal_in_ere_character_class(){
        // Native glibc REG_EXTENDED in the C locale accepts a lone '['
        // as a member of an existing bracket class. Rust regex requires
        // escaping to avoid interpreting it as nested set algebra.
        for (pattern,yes,no) in [
            (r"^[[]$",vec!["["],vec!["a","]","1"]),
            (r"^[a[]$",vec!["[","a"],vec!["b","]"]),
            (r"^[a[b]$",vec!["a","[","b"],vec!["z","]"]),
            (r"^[^[]$",vec!["a","]","1"],vec!["["]),
            (r"^[a[[:digit:]]+$",vec!["a","[","1","a1["],vec!["z","]"]),
            (r"^[[[:digit:]]+$",vec!["[","1"],vec!["a","]"]),
        ]{
            let sel=Selector::new("summary~",pattern).unwrap_or_else(|e|
                panic!("glibc accepts {pattern:?}: {e}"));
            let mut fields=note("normal",&["default"]);
            for value in yes{
                fields.summary=value;
                assert!(sel.matches(&fields),
                    "{pattern:?} must match {value:?} with glibc");
            }
            for value in no{
                fields.summary=value;
                assert!(!sel.matches(&fields),
                    "{pattern:?} must reject {value:?} with glibc");
            }
        }
        assert_eq!(mako_posix_ere_escapes(r"^[a[]$"),r"^[a\[]$");
        assert_eq!(mako_posix_ere_escapes(r"^[[:digit:]]$"),r"^[[:digit:]]$");
        let cfg=Config::parse("[body~=\"^[a[]$\"]\nwidth=472\n").unwrap();
        let mut fields=note("normal",&["default"]);
        fields.body="[";
        assert_eq!(cfg.style_for(&fields)["width"],"472");
        fields.body="a";
        assert_eq!(cfg.style_for(&fields)["width"],"472");
        fields.body="z";
        assert_eq!(cfg.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_posix_interval_syntax_and_dup_max_are_enforced_before_regex_compilation(){
        // C locale regcomp(REG_EXTENDED|REG_NOSUB) rejects all these
        // spellings. Rust regex can accept malformed {..} as literal,
        // and glibc RE_DUP_MAX is 32767 rather than Rust's large default.
        for malformed in [
            r"a{foo}",r"a{+2}",r"a{-2}",r"a{2,+3}",
            r"a{2,-3}",r"a{ 2 }",r"a{2.3}",r"a{2,1}",
            r"a{}",r"a{",r"a{2",r"a{2,3,4}",
            r"a{32768}",r"a{0,32768}",r"a{32768,}",
            r"a{9999999999999}",r"a{00032768}",
            r"a{2,32768}",r"a{,32768}",
        ]{
            let rejected=Selector::new("summary~",malformed);
            assert!(rejected.is_err(),
                "glibc rejects malformed/out-of-range interval {malformed:?}");
            assert!(Selector::new("body~",malformed).is_err(),
                "both Mako regex criteria enforce the interval grammar");
        }
        // Escaped braces and class-member braces are NOT intervals.
        let mut fields=note("normal",&["default"]);
        let escaped=Selector::new("summary~",r"^a\{foo\}$").unwrap();
        fields.summary="a{foo}";
        assert!(escaped.matches(&fields));
        fields.summary="afoo";
        assert!(!escaped.matches(&fields));
        let bracket=Selector::new("summary~",r"^[a{}[:digit:]]+$").unwrap();
        for literal in ["{","}","a","1","{a}1"]{
            fields.summary=literal;
            assert!(bracket.matches(&fields),
                "bracket-class braces stay literal: {literal:?}");
        }
        for (pattern,yes,no) in [
            (r"^a{,2}$",vec!["","a","aa"],vec!["aaa","b"]),
            (r"^a{,}$",vec!["","a","aaaa"],vec!["b","ab"]),
            (r"^a{2,}$",vec!["aa","aaa","aaaa"],vec!["","a","ab"]),
            (r"^a{02}$",vec!["aa"],vec!["","a","aaa"]),
            (r"^a{2,3}$",vec!["aa","aaa"],vec!["","a","aaaa"]),
        ]{
            let rule=Selector::new("summary~",pattern).unwrap();
            for word in yes{
                fields.summary=word;
                assert!(rule.matches(&fields),"{pattern:?} should match {word:?}");
            }
            for word in no{
                fields.summary=word;
                assert!(!rule.matches(&fields),"{pattern:?} must reject {word:?}");
            }
        }
        // This is precisely glibc's numerical limit, without allocating
        // the enormous automaton that compiling the top bound can require.
        assert!(mako_posix_ere_validate_intervals("a{32767}").is_ok());
        assert!(mako_posix_ere_validate_intervals("a{0,32767}").is_ok());
        assert!(mako_posix_ere_validate_intervals("a{32768}").is_err());
        // Native GNU POSIX \< and \> are converted into Rust \b{start}
        // and \b{end}, which are NOT POSIX repetitions and must not be
        // inspected after that transformation.
        let boundary=Selector::new("summary~",r"\<word\>").unwrap();
        fields.summary="a word!";
        assert!(boundary.matches(&fields));
        fields.summary="sword";
        assert!(!boundary.matches(&fields));
        let cfg=Config::parse("[body~=\"^a{foo}$\"]\nwidth=430\n");
        assert!(cfg.is_err(),"Mako rejects malformed body~ sections");
        let cfg=Config::parse("[body~=\"^a{2,3}$\"]\nwidth=430\n").unwrap();
        fields.body="aaa";
        assert_eq!(cfg.style_for(&fields)["width"],"430");
        fields.body="aaaa";
        assert_eq!(cfg.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_stacked_posix_ere_quantifiers_are_repeated_not_rust_lazy(){
        // Positive/negative matches checked with native C-locale glibc
        // regcomp(REG_EXTENDED|REG_NOSUB) and regexec. In particular,
        // ^a+?$ accepts empty while Rust regex's lazy a+? does not.
        for (pattern,yes,no) in [
            (r"^a+?$",vec!["","a","aa","aaaa"],vec!["b","ab"]),
            (r"^a?+$",vec!["","a","aa","aaaa"],vec!["b","ab"]),
            (r"^a{2}?$",vec!["","aa"],vec!["a","aaa","aaaa"]),
            (r"^a{2}+$",vec!["aa","aaaa","aaaaaa"],vec!["","a","aaa"]),
            (r"^a+{2}$",vec!["aa","aaa","aaaa"],vec!["","a","b"]),
            (r"^a+*$",vec!["","a","aa","aaaa"],vec!["b","ab"]),
            (r"^a?*$",vec!["","a","aa","aaaa"],vec!["b","ab"]),
            (r"^a{2}*$",vec!["","aa","aaaa","aaaaaa"],vec!["a","aaa"]),
            (r"^(ab)+?$",vec!["","ab","abab","ababab"],vec!["a","aba","b"]),
            (r"^[ab]+?$",vec!["","a","b","abab","baa"],vec!["c","abc"]),
            (r"^a++$",vec!["a","aa","aaaa"],vec!["","b","ab"]),
            (r"^a*+$",vec!["","a","aa","aaaa"],vec!["b","ab"]),
            (r"^a{2}{3}$",vec!["aaaaaa"],vec!["","aa","aaaa","aaaaa"]),
            (r"^a{2}??$",vec!["","aa"],vec!["a","aaa"]),
            (r"^a{,2}+$",vec!["","a","aa","aaa"],vec!["b","ab"]),
        ]{
            let matcher=Selector::new("summary~",pattern)
                .unwrap_or_else(|e|panic!("glibc accepts {pattern:?}: {e}"));
            let mut fields=note("normal",&["default"]);
            for value in yes{
                fields.summary=value;
                assert!(matcher.matches(&fields),
                    "glibc {pattern:?} must match {value:?}");
            }
            for value in no{
                fields.summary=value;
                assert!(!matcher.matches(&fields),
                    "glibc {pattern:?} must reject {value:?}");
            }
        }
        assert_eq!(mako_posix_ere_stacked_quantifiers(r"^a+?$"),
            r"^(?:a+)?$");
        assert_eq!(mako_posix_ere_stacked_quantifiers(r"^a{2}{3}$"),
            r"^(?:a{2}){3}$");
        assert_eq!(mako_posix_ere_stacked_quantifiers(r"^[a+?]{2}$"),
            r"^[a+?]{2}$","bracket contents must not be altered");
        assert_eq!(mako_posix_ere_stacked_quantifiers(r"^a\+\?$"),
            r"^a\+\?$","escaped operators remain literal");
        // Verify normal criteria-driven configuration after parsing.
        let cfg=Config::parse("[summary~=\"^a+?$\"]\nwidth=463\n").unwrap();
        let mut fields=note("normal",&["default"]);
        fields.summary="";
        assert_eq!(cfg.style_for(&fields)["width"],"463");
        fields.summary="aaaa";
        assert_eq!(cfg.style_for(&fields)["width"],"463");
        fields.summary="bad";
        assert_eq!(cfg.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_c_locale_class_backslashes_are_literal_range_endpoints(){
        // Compared with native C-locale glibc regcomp(REG_EXTENDED) and
        // regexec. These results differ from Rust regex escaped characters.
        for (pattern,yes,no) in [
            (r"^[a\-b]$", vec!["a","b","\\","]","^","_"],vec!["-","c","&"]),
            (r"^[a\^b]$", vec!["a","b","^","\\"],vec!["-","&"]),
            (r"^[a\&b]$", vec!["a","b","&","\\"],vec!["-","c"]),
            (r"^[a\~b]$", vec!["a","b","~","\\"],vec!["-","c"]),
            (r"^[\w]$", vec!["w","\\"],vec!["s","1","-"]),
            (r"^[\d]$", vec!["d","\\"],vec!["1","w"]),
            (r"^[\-]$", vec!["\\","-"],vec!["a","b"]),
            (r"^[\^]$", vec!["\\","^"],vec!["a","b"]),
            (r"^[a\-z]$", vec!["a","b","z","\\","]","^","_"],vec!["-","&"]),
            (r"^[a\d-z]$", vec!["a","\\","d","z","x"],vec!["b","-","]"]),
            (r"^[a\[:digit:]]$", vec!["a","\\","1"],vec!["&","x","-"]),
        ]{
            let sel=Selector::new("summary~",pattern).unwrap_or_else(|e|
                panic!("glibc accepted {pattern:?}: {e}"));
            let mut fields=note("normal",&["default"]);
            for value in yes{
                fields.summary=value;
                assert!(sel.matches(&fields),
                    "{pattern:?} should match {value:?} with glibc");
            }
            for value in no{
                fields.summary=value;
                assert!(!sel.matches(&fields),
                    "{pattern:?} should reject {value:?} with glibc");
            }
        }
        assert_eq!(mako_posix_c_class_backslashes(r"^[\w]$"),
            r"^[\\w]$");
        assert_eq!(mako_posix_c_class_backslashes(r"^[a\-b]$"),
            r"^[a\\-b]$");
        assert_eq!(mako_posix_c_class_backslashes(r"\w"),
            r"\w","GNU word character outside a class stays special");
        // Full configuration and body selector; a backslash in the
        // received D-Bus body must not be stripped by the rule parser.
        let cfg=Config::parse("[body~=\"^[\\\\w]+$\"]\nwidth=461\n").unwrap();
        let mut fields=note("normal",&["default"]);
        fields.body="\\";
        assert_eq!(cfg.style_for(&fields)["width"],"461");
        fields.body="w";
        assert_eq!(cfg.style_for(&fields)["width"],"461");
        fields.body="x";
        assert_eq!(cfg.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_unmatched_closing_delimiters_are_literal_regex_atoms(){
        // Direct native glibc REG_EXTENDED|REG_NOSUB fixtures. A closing
        // delimiter without a matching opener is a literal, and repeated
        // unmatched closers can themselves take postfix quantifiers.
        let mut fields=note("normal",&["default"]);
        for (pattern,yes,no) in [
            (r"^)$",")","("),
            (r"^]$","]","["),
            (r"^}$","}","{"),
            (r"^(a))$","a)","a"),
            (r"^a)*$","a)))","a("),
            (r"^a]*$","a]]","a["),
            (r"^a}*$","a}}","a{"),
            (r"^a{2}}*$","aa}}","a}"),
            (r"^[]]$","]","["),
            (r"^\)$",")","("),
            (r"^\]$","]","["),
            (r"^\}$","}","{"),
        ] {
            let selector=Selector::new("summary~",pattern).unwrap_or_else(|e|
                panic!("glibc accepts {pattern:?}: {e}"));
            fields.summary=yes;
            assert!(selector.matches(&fields),
                "{pattern:?} must match glibc literal {yes:?}");
            fields.summary=no;
            assert!(!selector.matches(&fields),
                "{pattern:?} must reject {no:?}");
        }
        assert_eq!(mako_posix_ere_literal_unmatched_closers(r"^(a))$"),
            r"^(a)\)$");
        assert_eq!(mako_posix_ere_literal_unmatched_closers(r"^a{2}}*$"),
            r"^a{2}\}*$");
        assert_eq!(mako_posix_ere_literal_unmatched_closers(r"^\)$"),
            r"^\)$","explicit escaping must not be doubled");
        // The full Mako config path must accept literal closing parens.
        let config=Config::parse("[summary~=^a)*$]\nwidth=463\n").unwrap();
        fields.summary="a)))";
        assert_eq!(config.style_for(&fields)["width"],"463");
        fields.summary="a(";
        assert_eq!(config.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_leading_right_bracket_is_a_literal_not_a_class_terminator(){
        // Checked directly with glibc regcomp/regexec in the C locale.
        // The first ']' after '[' or '[^' is a member of the bracket set.
        for (pattern,matches,rejects) in [
            (r"^[]]$",vec!["]"],vec!["[","a"]),
            (r"^[^]]$",vec!["a","[","w"],vec!["]"]),
            (r"^[]a]+$",vec!["]","a","a]"],vec!["b","w"]),
            (r"^[]\w]+$",vec!["]","w","w]"],vec!["z","1"]),
            (r"^[^]\w]+$",vec!["a","z","[","1"],vec!["]","w"]),
            (r"^[]&&a]+$",vec!["]","&","a","a&"],vec!["b","~"]),
            (r"^[]~~a]+$",vec!["]","~","a","a~"],vec!["b","&"]),
        ] {
            let rule=Selector::new("summary~",pattern).unwrap_or_else(|e|
                panic!("glibc accepts {pattern:?}: {e}"));
            let mut fields=note("normal",&["default"]);
            for candidate in matches{
                fields.summary=candidate;
                assert!(rule.matches(&fields),
                    "{pattern:?} must match {candidate:?} on glibc");
            }
            for candidate in rejects{
                fields.summary=candidate;
                assert!(!rule.matches(&fields),
                    "{pattern:?} must reject {candidate:?} on glibc");
            }
        }
        assert_eq!(mako_posix_ere_literal_leading_closer(r"^[]\w]$"),
            r"^[\]\w]$");
        assert_eq!(mako_posix_ere_literal_leading_closer(r"^[^]\w]$"),
            r"^[^\]\w]$");
        assert_eq!(mako_posix_ere_literal_leading_closer(r"^\[literal\]$"),
            r"^\[literal\]$");
        // Exercise the loader and body selector, not just local selectors.
        let config=Config::parse("[body~=\"^[]\\\\w]+$\"]\nwidth=462\n").unwrap();
        let mut fields=note("normal",&["default"]);
        fields.body="]";
        assert_eq!(config.style_for(&fields)["width"],"462");
        fields.body="w";
        assert_eq!(config.style_for(&fields)["width"],"462");
        fields.body="z";
        assert_eq!(config.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_bracket_ampersand_and_tilde_are_literals_not_rust_set_operators(){
        // Independently probed with glibc regcomp(REG_EXTENDED|REG_NOSUB)
        // in the default C locale. GNU POSIX ERE has no bracket set
        // intersection or symmetric difference operators.
        for (pattern,matches,rejects) in [
            (r"^[a&&b]+$",vec!["a","b","&","a&b"],vec!["c","1"]),
            (r"^[a~~b]+$",vec!["a","b","~","a~b"],vec!["c","&"]),
            (r"^[a&&[:digit:]]+$",vec!["a","&","7","a7&"],vec!["b","~"]),
            (r"^[a~~[:digit:]]+$",vec!["a","~","7","a7~"],vec!["b","&"]),
            (r"^[a||b]+$",vec!["a","b","|","a|b"],vec!["c","&"]),
            (r"^[||]+$",vec!["|","||"],vec!["&","a"]),
            (r"^[&&]+$",vec!["&","&&"],vec!["~","a"]),
            (r"^[~~]+$",vec!["~","~~"],vec!["&","a"]),
            (r"^[a&~b]+$",vec!["&","~","a","b"],vec!["c","1"]),
        ] {
            let rule=Selector::new("summary~",pattern).unwrap_or_else(|e|
                panic!("glibc accepts {pattern:?}: {e}"));
            let mut fields=note("normal",&["default"]);
            for candidate in matches{
                fields.summary=candidate;
                assert!(rule.matches(&fields),
                    "{pattern:?} should match {candidate:?} under glibc POSIX ERE");
            }
            for candidate in rejects {
                fields.summary=candidate;
                assert!(!rule.matches(&fields),
                    "{pattern:?} should reject {candidate:?} under glibc POSIX ERE");
            }
        }
        // The same translator powers body regexes through the entire config
        // loader; explicitly preserve escaped punctuation as well.
        let cfg=Config::parse(
            "[body~=\"^[a&&b]+$\"]\nwidth=438\n"
        ).unwrap();
        let mut fields=note("normal",&["default"]);
        fields.body="&";
        assert_eq!(cfg.style_for(&fields)["width"],"438");
        fields.body="c";
        assert_eq!(cfg.style_for(&fields)["width"],"300");
        assert_eq!(mako_posix_ere_escapes(r"[a\&\~b]"),r"[a\&\~b]");
        // Outside brackets these punctuation characters stay unchanged.
        assert_eq!(mako_posix_ere_escapes("a&&b~~c"),"a&&b~~c");
    }

    #[test]
    fn posix_ere_digit_spelling_is_literal_not_rust_digit_class() {
        let mut fields = note("normal", &["default"]);
        for pattern in [r"\d+", r"[\d]+"] {
            let rule = Selector::new("summary~", pattern).unwrap();
            fields.summary = "ddd";
            assert!(rule.matches(&fields), "{pattern} must match literal d");
            fields.summary = "123";
            assert!(!rule.matches(&fields), "{pattern} must not match digits");
        }
        for pattern in [r"\D+", r"[\D]+"] {
            let rule = Selector::new("summary~", pattern).unwrap();
            fields.summary = "DDD";
            assert!(rule.matches(&fields), "{pattern} must match literal D");
            fields.summary = "abc";
            assert!(!rule.matches(&fields), "{pattern} is not a non-digit class");
        }
        // A doubled escape must remain a literal backslash, not be
        // normalized into the POSIX escaped-letter behavior.
        assert_eq!(mako_posix_ere_escapes(r"\\d+"), r"\\d+");
        let rule = Config::parse("[summary~=\\\\d+]\nwidth=441\n").unwrap();
        fields.summary = "dd";
        assert!(rule.rules[0].selectors[0].matches(&fields));
        fields.summary = "123";
        assert!(!rule.rules[0].selectors[0].matches(&fields));
    }

    #[test]
    fn glibc_posix_escaped_punctuation_and_zero_remain_literal() {
        // Native glibc regcomp(REG_EXTENDED | REG_NOSUB), C locale:
        // escaped ordinary punctuation is a literal, even when Rust's
        // regex parser rejects the escape. GNU \0 is also literal '0',
        // unlike a NUL-byte escape in other regex dialects.
        let mut fields=note("normal",&["default"]);
        for (pattern,yes,no) in [
            (r"^\!$","!","?"),
            (r"^\#$","#","x"),
            (r"^\@$","@","x"),
            (r"^\_$","_","x"),
            (r"^\=$","=","x"),
            (r"^\,$",",","x"),
            (r"^\/$","/","x"),
            (r"^\:$",":","x"),
            (r"^\ $"," ","x"),
            (r"^\0$","0","\0"),
            (r"^\.$",".","x"),
            (r"^\+$","+","x"),
            (r"^\($","(","x"),
            (r"^\{$","{","x"),
            (r"^a\#b$","a#b","a\\#b"),
        ]{
            let selector=Selector::new("summary~",pattern).unwrap_or_else(|e|
                panic!("glibc accepts escaped literal {pattern:?}: {e}"));
            fields.summary=yes;
            assert!(selector.matches(&fields),
                "{pattern:?} must match escaped ordinary literal {yes:?}");
            fields.summary=no;
            assert!(!selector.matches(&fields),
                "{pattern:?} must not match {no:?}");
        }
        assert_eq!(mako_posix_ere_escapes(r"\! \# \0 \+"),r"! \# 0 \+");
        // Inside POSIX bracket expressions the backslash is a separate
        // literal member; do not apply the outside-class conversion.
        assert_eq!(mako_posix_ere_escapes(r"[\!]"),r"[\!]");
        // A parsed Mako file uses the same regex path, including its
        // one layer of criteria backslash decoding.
        let cfg=Config::parse("[body~=^a\\\\#b$]\nwidth=419\n").unwrap();
        fields.body="a#b";
        assert_eq!(cfg.style_for(&fields)["width"],"419");
        fields.body="a\\#b";
        assert_eq!(cfg.style_for(&fields)["width"],"300");
    }

    #[test]
    fn glibc_posix_literal_escape_letters_do_not_become_rust_regex_operators() {
        let mut fields = note("normal", &["default"]);
        for (pattern, matching, not_matching) in [
            (r"\Afoo", "Afoo", "foo"),
            (r"\x41", "x41", "A"),
            (r"\u0041", "u0041", "A"),
            (r"foo\z", "fooz", "foo"),
            (r"\Qab\E", "QabE", "ab"),
            (r"\pL", "pL", "abc"),
            (r"\v+", "vvv", "\u{000b}"),
        ] {
            let selector=Selector::new("summary~",pattern).unwrap();
            fields.summary=matching;
            assert!(selector.matches(&fields), "glibc {pattern} must match {matching:?}");
            fields.summary=not_matching;
            assert!(!selector.matches(&fields), "glibc {pattern} must not match {not_matching:?}");
        }
        // GNU POSIX shortcuts recognized by glibc still keep their meaning.
        let word=Selector::new("summary~",r"\w+").unwrap();
        fields.summary="hello_123";
        assert!(word.matches(&fields));
        let digit=Selector::new("summary~",r"[[:digit:]]+").unwrap();
        fields.summary="123";
        assert!(digit.matches(&fields));
        // Escaped backslashes represent literal backslashes, not an escape.
        assert_eq!(mako_posix_ere_escapes(r"\\x41"), r"\\x41");
    }

    #[test]
    fn posix_c_locale_regex_uses_ascii_classes_and_byte_width() {
        // Mako v1.11 uses regcomp(REG_EXTENDED|REG_NOSUB) without
        // setlocale(). glibc's initial C locale matches *bytes*:
        // [[:alpha:]] and GNU \w do not include non-ASCII letters,
        // and . consumes one UTF-8 byte rather than one Unicode scalar.
        let mut traits=note("normal",&["default"]);
        for pattern in [r"^[[:alpha:]]+$", r"^\w+$", r"^[[:alnum:]]+$"] {
            let selector=Selector::new("summary~",pattern).unwrap();
            traits.summary=if pattern.contains("alpha") {"Ascii"}
                else if pattern.contains("alnum") {"Ascii123"}
                else {"ASCII_123"};
            assert!(selector.matches(&traits),"{pattern} must match ASCII");
            traits.summary="café";
            assert!(!selector.matches(&traits),
                "glibc C-locale {pattern} must not match accented Unicode");
            traits.summary="東京";
            assert!(!selector.matches(&traits),
                "glibc C-locale {pattern} must not match CJK Unicode");
        }
        traits.summary="é";
        assert!(!Selector::new("summary~",r"^.$").unwrap().matches(&traits),
            "one POSIX dot cannot consume a two-byte UTF-8 character");
        assert!(Selector::new("summary~",r"^..$").unwrap().matches(&traits),
            "two POSIX dots consume two UTF-8 bytes in the C locale");
        traits.summary="A";
        assert!(Selector::new("summary~",r"^.$").unwrap().matches(&traits));
    }

    #[test]
    fn glibc_gnu_buffer_begin_end_anchors_are_not_literal_punctuation(){
        // Checked against glibc REG_EXTENDED|REG_NOSUB in the C locale.
        // A backslash-backtick anchors input start; backslash-apostrophe
        // anchors input end. Both are literals inside character classes.
        assert_eq!(mako_posix_ere_escapes(r"\`cat\'"),r"\Acat\z");
        assert_eq!(mako_posix_ere_escapes(r"[\`][\']"),"[`][']");
        let mut fields=note("normal",&["default"]);
        for (pattern,yes,no) in [
            (r"\`cat\'","cat","cat!"),
            (r"\`cat","cat!","bobcat"),
            (r"cat\'","bobcat","catnip"),
            (r"^[\`]$","`","cat"),
            (r"^[\']$","'","cat"),
        ]{
            let selector=Selector::new("summary~",pattern).unwrap();
            fields.summary=yes;
            assert!(selector.matches(&fields),
                "glibc GNU buffer anchor {pattern:?} failed {yes:?}");
            fields.summary=no;
            assert!(!selector.matches(&fields),
                "glibc GNU buffer anchor {pattern:?} matched {no:?}");
        }
        assert_eq!(mako_posix_ere_escapes(r"\\`"),r"\\`",
            "doubled backslash is a literal backslash");
    }

    #[test]
    fn glibc_gnu_word_start_and_end_anchors_match_c_locale(){
        // Independently checked with glibc regcomp(REG_EXTENDED|REG_NOSUB)
        // in the initial C locale, not with REG_NEWLINE.
        assert_eq!(mako_posix_ere_escapes(r"\<cat\>"),
            r"\b{start}cat\b{end}");
        assert_eq!(mako_posix_ere_escapes(r"[\<\>]"),"[<>]");
        assert_eq!(mako_posix_ere_escapes(r"\\<"),r"\\<",
            "escaped backslash must remain an actual backslash");
        let mut fields=note("normal",&["default"]);
        for (pattern,matching,rejected) in [
            (r"\<cat\>","a cat!","bobcat"),
            (r"\<cat","catnip","bobcat"),
            (r"cat\>","bobcat!","catnip"),
            (r"\<a\>","a-b","ab"),
            (r"^[\<]$","<","a"),
            (r"^[\>]$",">","a"),
            (r"^[\<\>]$",">","a"),
            (r"a\<b","a<b","ab"),
        ]{
            let selector=Selector::new("summary~",pattern).unwrap();
            fields.summary=matching;
            // a\<b is valid but can never match in ordinary ASCII:
            // the literal 'a' is itself a word char, so no word-start
            // boundary can immediately follow it.
            if pattern!=r"a\<b"{
                assert!(selector.matches(&fields),
                    "glibc GNU word boundary {pattern:?} must match {matching:?}");
            }else{
                assert!(!selector.matches(&fields));
            }
            fields.summary=rejected;
            assert!(!selector.matches(&fields),
                "glibc GNU word boundary {pattern:?} matched {rejected:?}");
        }
        for (pattern,yes,no) in [
            (r"^\<cat\>$","cat","catnip"),
            (r"\<", "a","!!!"),
            (r"\>", "a","!!!"),
        ]{
            let sel=Selector::new("summary~",pattern).unwrap();
            fields.summary=yes;
            assert!(sel.matches(&fields));
            fields.summary=no;
            assert!(!sel.matches(&fields));
        }
    }

    #[test]
    fn posix_ere_letter_escapes_are_literal_except_gnu_operators() {
        let mut fields = note("normal", &["default"]);
        for (pattern, yes, no) in [
            (r"\n+", "nn", "\n"),
            (r"\t+", "tt", "\t"),
            (r"\Afoo", "Afoo", "foo"),
            (r"\Zfoo", "Zfoo", "foo"),
            (r"\d+", "ddd", "123"),
            (r"\x41", "x41", "A"),
            (r"\u0041", "u0041", "A"),
            (r"[\w]+", "www", "123"),
            (r"[\s]+", "sss", " "),
            (r"[\b]+", "bbb", "aaa"),
            (r"[\d]+", "ddd", "123"),
            (r"[[:digit:]\w]+", "www", "abc"),
        ] {
            let rule=Selector::new("summary~",pattern).unwrap();
            fields.summary=yes;
            assert!(rule.matches(&fields), "glibc {pattern} did not match {yes:?}");
            fields.summary=no;
            assert!(!rule.matches(&fields), "glibc {pattern} matched {no:?}");
        }
        // Outside classes glibc's GNU ERE shortcuts remain active.
        for (pattern, yes, no) in [
            (r"\w+", "alpha_1", "#"),
            (r"\s+", "\t ", "alpha"),
            (r"\bfoo\b", "foo!", "seafood"),
        ] {
            let rule=Selector::new("summary~",pattern).unwrap();
            fields.summary=yes;
            assert!(rule.matches(&fields), "{pattern} failed to match {yes:?}");
            fields.summary=no;
            assert!(!rule.matches(&fields), "{pattern} incorrectly matched {no:?}");
        }
    }

    #[test]
    fn posix_c_locale_collating_and_equivalence_elements_are_single_bytes(){
        // Checked using C-locale glibc regcomp(REG_EXTENDED|REG_NOSUB):
        // nested [.[char].] and [=[char]=] are ONE collating character,
        // not Rust-style nested sets admitting '.' or '='.
        for (source,normalized) in [
            ("^[[.a.]]$","^[a]$"),
            ("^[[=a=]]$","^[a]$"),
            ("^[a[.b.]c]$","^[abc]$"),
            ("^[a[=b=]c]$","^[abc]$"),
            ("^[[.a.]-[.z.]]$","^[a-z]$"),
            ("^[[.].]]$",r"^[\]]$"),
            (r"^[[.\.]]$",r"^[\\]$"),
            ("^[[.&.]]$",r"^[\&]$"),
            ("^[[.-.]]$",r"^[\-]$"),
            ("^[[.^.]]$",r"^[\^]$"),
            ("^[[:alpha:][.0.]]$","^[[:alpha:]0]$"),
            ("^[.a.]$","^[.a.]$"),
        ] {
            assert_eq!(mako_posix_c_collating_elements(source).unwrap(),normalized,
                "incorrect C-locale POSIX bracket lowering for {source:?}");
        }
        let mut fields=note("normal",&["default"]);
        for pattern in ["^[[.a.]]$","^[[=a=]]$"]{
            let selector=Selector::new("summary~",pattern).unwrap();
            fields.summary="a";
            assert!(selector.matches(&fields));
            for wrong in [".","=","b"]{
                fields.summary=wrong;
                assert!(!selector.matches(&fields),
                    "POSIX collation {pattern:?} also matched {wrong:?}");
            }
        }
        for pattern in ["^[[.ab.]]$","^[[=ab=]]$","^[[.newline.]]$"] {
            assert!(Selector::new("summary~",pattern).is_err(),
                "glibc rejects multi-byte/named collating element {pattern:?}");
            assert!(Config::parse(&format!(
                "[summary~={pattern}]\nwidth=400\n"
            )).is_err());
        }
        // Source literal UTF-8 letters occupy multiple bytes in the C
        // locale, so equivalence elements cannot span their encoding.
        assert!(Selector::new("summary~","^[[=é=]]$").is_err());
    }

    #[test]
    fn glibc_gnu_repetition_accepts_omitted_minimum_outside_brackets(){
        // glibc REG_EXTENDED in the C locale accepts these spellings;
        // Rust regex's default parser requires a numeric lower bound.
        for (source, normalized) in [
            (r"^a{,2}$",r"^a{0,2}$"),
            (r"^a{,}$",r"^a{0,}$"),
            (r"^a{,0}$",r"^a{0,0}$"),
            (r"^[{,2}]$",r"^[{,2}]$"),
            (r"^\{,2}$",r"^\{,2}$"),
            (r"^[[:alpha:]]{,2}$",r"^[[:alpha:]]{0,2}$"),
        ]{
            assert_eq!(mako_posix_ere_empty_min_repetitions(source),normalized);
        }
        let mut traits=note("normal",&["default"]);
        let bounded=Selector::new("summary~",r"^a{,2}$").unwrap();
        for summary in ["","a","aa"]{
            traits.summary=summary;
            assert!(bounded.matches(&traits),"{summary:?} must match 0..=2");
        }
        traits.summary="aaa";
        assert!(!bounded.matches(&traits));
        let unbounded=Selector::new("summary~",r"^a{,}$").unwrap();
        assert!(unbounded.matches(&traits));
        traits.summary="bbb";
        assert!(!unbounded.matches(&traits));
        let literal=Selector::new("summary~",r"^\{,2}$").unwrap();
        traits.summary="{,2}";
        assert!(literal.matches(&traits),"escaped literal brace was rewritten");
        assert!(Config::parse("[summary~=^a{,2}$]\nwidth=402\n").is_ok());
    }

    #[test]
    fn glibc_rejects_rust_only_ascii_and_word_posix_bracket_classes() {
        // Verified against glibc regcomp(REG_EXTENDED | REG_NOSUB) in the
        // C locale: these Rust extras produce REG_ECTYPE upstream.
        for unsupported in [
            "[[:ascii:]]", "[[:word:]]", "[^[:ascii:]]",
            "[[:alpha:][:word:]]",
            "[[:^alpha:]]", "[[:^digit:]]", "[[:^word:]]",
        ] {
            assert!(has_rust_only_posix_class(unsupported));
            assert!(Selector::new("summary~",unsupported).is_err(),
                "glibc must reject non-POSIX class {unsupported:?}");
            assert!(Config::parse(&format!(
                "[summary~={unsupported}]\nwidth=400\n"
            )).is_err());
        }
        // The standard POSIX classes continue to compile, as does the
        // glibc GNU word operator outside of bracket expressions.
        for supported in [
            "[[:alpha:]]", "[[:digit:]]", "[[:xdigit:]]",
            "[[:blank:]]", r"\w+", "[[:alpha:][:digit:]]",
        ] {
            assert!(!has_rust_only_posix_class(supported));
            assert!(Selector::new("summary~",supported).is_ok(),
                "valid Mako POSIX/GNU class rejected: {supported:?}");
        }
        // Literal escaped '[' is not a POSIX bracket expression, so this
        // guard must not reject it merely by scanning raw substrings.
        assert!(!has_rust_only_posix_class(r"\[[:word:]]"));
        assert!(!has_rust_only_posix_class(r"\[[:ascii:]]"));
        assert!(!has_rust_only_posix_class(r"\[[:^alpha:]]"));
    }

    #[test]
    fn posix_ere_rejects_rust_only_inline_groups_outside_character_classes() {
        for invalid in ["(?i)hello", "(?:hello)", "(?P<word>hello)", "a(?s:.)"] {
            assert!(has_rust_only_group_extension(invalid));
            assert!(Selector::new("summary~",invalid).is_err(),
                "glibc regcomp rejects {invalid:?}");
        }
        assert!(Config::parse("[summary~=(?i)hello]\nwidth=440\n").is_err());
        for legal in ["[(?]", r"\(?i)", "[(?]a", r"abc\(?",
            "[[:digit:](?i)]"] {
            assert!(!has_rust_only_group_extension(legal),
                "group detection cannot reject literal {legal:?}");
        }
        assert!(!has_rust_only_group_extension("[[:digit:]]"));
        assert!(has_rust_only_group_extension("[[:digit:]](?i)"));
        assert!(Selector::new("summary~","[(?]").is_ok());
    }

    #[test]
    fn quoted_criteria_and_regex() {
        let c=Config::parse("[app-name=\"Example App\" summary~=^Hello]\nwidth=440\n").unwrap();
        let mut n=note("general", &["default"]);n.app_name="Example App";n.summary="Hello there";
        assert_eq!(c.style_for(&n)["width"],"440");
    }
    #[test]
    fn rejects_unknown_settings_instead_of_faking_parity() {
        assert!(Config::parse("magic-feature=1\n").is_err());
        assert!(Config::parse("[category=success]\nwidth=0\n").is_err());
        assert!(Config::parse("[summary~=\"(\"]\nwidth=440\n").is_err());
    }
    #[test]
    fn validates_format_and_group_spec_instead_of_accepting_any_string() {
        assert!(Config::parse("format=%z\n").is_err());
        assert!(Config::parse("format=<b>%s</b>\\\\n%b\n").is_ok());
        assert!(Config::parse("group-by=app-name,summary,category\n").is_ok());
        assert!(Config::parse("group-by=magic-field\n").is_err());
        assert!(Config::parse("group-by=app-name,,summary\n").is_ok());
    }
    #[test]
    fn urgency_matches_upstream_aliases_case_insensitively() {
        let c = Config::parse("[urgency=HIGH]\nwidth=440\n").unwrap();
        let mut n = note("general", &["default"]);
        n.urgency = "critical";
        assert_eq!(c.style_for(&n)["width"], "440");
    }
    #[test]
    fn posix_ere_dot_matches_newline_but_anchors_remain_whole_string(){
        let config=Config::parse(
            "[summary~=^Line.One$]\nwidth=441\n[body~=^Top.Bottom$]\nheight=111\n"
        ).unwrap();
        let mut fields=note("general",&["default"]);
        fields.summary="Line\nOne";
        fields.body="Top\nBottom";
        let style=config.style_for(&fields);
        assert_eq!(style["width"],"441");
        assert_eq!(style["height"],"111");
        fields.summary="prefix\nLine\nOne";
        fields.body="prefix\nTop\nBottom";
        let style=config.style_for(&fields);
        assert_eq!(style["width"],"300",
            "REG_NEWLINE was not set: ^ cannot match after a newline");
        assert_eq!(style["height"],"100",
            "REG_NEWLINE was not set: ^ cannot match after a newline");

        let literal=Config::parse("[summary~=^Line\\\\.One$]\nwidth=442\n").unwrap();
        fields.summary="Line\nOne";
        assert_eq!(literal.style_for(&fields)["width"],"300",
            "escaped dot remains literal under POSIX ERE");
    }

    #[test]
    fn signed_max_history_matches_mako_nonpositive_disable_semantics() {
        for setting in ["max-history=-1","max-history=-100","max-history=0"] {
            let config=Config::parse(&format!("{setting}\n")).unwrap();
            assert_eq!(config.max_history,0);
        }
        assert_eq!(Config::parse("max-history=7\n").unwrap().max_history,7);
        assert_eq!(Config::parse("max-history=2147483648\n")
            .unwrap().max_history,0,
            "LP64 strtol accepts the number, then Mako narrows it to i32::MIN");
        assert!(Config::parse("max-history=9223372036854775808\n").is_err(),
            "true 64-bit strtol range overflow is rejected");
    }

    #[test]
    fn bare_nonboolean_selectors_and_invalid_anchor_are_rejected_like_mako() {
        for raw in ["[app-name]","[category]","[summary]","[body]",
            "[mode]","[output]","[urgency]","[group-index]",
            "[!app-name]","[anchor=not-an-anchor]"] {
            assert!(Config::parse(&format!("{raw}\nbackground-color=#123456\n")).is_err(),
                "invalid selector unexpectedly accepted: {raw}");
        }
        for raw in ["[actionable]","[!actionable]","[grouped]",
            "[hidden]","[expiring]","[anchor=bottom-center]"] {
            assert!(Config::parse(&format!("{raw}\nbackground-color=#123456\n")).is_ok(),
                "valid selector rejected: {raw}");
        }
    }

    #[test]
    fn mako_rejects_conflicting_exact_and_regex_selectors() {
        for criteria in [
            "summary=hello summary~=^hello", "body=world body~=world",
        ] {
            assert!(Config::parse(&format!("[{criteria}]\nwidth=400\n")).is_err(),
                "conflicting criteria unexpectedly accepted: {criteria}");
        }
        assert!(Config::parse("[summary~=^hello body=world]\nwidth=400\n").is_ok());
    }

    #[test]
    fn pinned_mako_group_by_whitelist_distinguishes_selectors_from_fields(){
        // Exact types.c:parse_criteria_spec() field names (v1.11.0).
        // A syntax-valid [hidden] or [mode=quiet] selector does NOT imply
        // the corresponding group-by field is recognized.
        for field in ["app-name","app-icon","actionable","expiring",
            "urgency","category","desktop-entry","summary","body","none"]{
            assert!(Config::parse(&format!("group-by={field}\n")).is_ok(),
                "upstream-recognized field {field} was rejected");
        }
        for field in ["hidden","mode","summary~","body~",
            "app-name,hidden","none,hidden","none,mode"]{
            assert!(Config::parse(&format!("group-by={field}\n")).is_err(),
                "upstream-rejected group field {field} was accepted");
        }
        assert!(Config::parse("[hidden]\nformat=%h\n").is_ok(),
            "[hidden] remains a valid configuration criterion selector");
        assert!(Config::parse("[mode=quiet]\nmarkup=0\n").is_ok(),
            "[mode=quiet] remains a valid configuration criterion selector");
    }

    #[test]
    fn mako_restricts_surface_and_group_settings_by_selector() {
        for source in [
            "[grouped]\nanchor=bottom-right\n",
            "[group-index=0]\noutput=DP-1\n",
            "[output=DP-1]\ngroup-by=app-name\n",
            "[anchor=top-right]\noutput=DP-1\n",
            "[category=urgent]\nmax-visible=3\n",
            "[category=urgent hidden]\nformat=%s\n",
            "group-by=group-index\n",
            "group-by=app-name,output\n",
            "group-by=hidden\n",
            "group-by=app-name,hidden\n",
            "group-by=none,hidden\n",
        ] {
            assert!(Config::parse(source).is_err(), "Mako-invalid config accepted: {source}");
        }
        for source in [
            "[output=DP-1]\nmax-visible=3\n",
            "[anchor=bottom-right]\nmax-visible=2\n",
            "[hidden]\nformat=%h\n",
            "[category=urgent hidden=false]\nformat=%s\n",
            "group-by=app-name,category\n",
            "group-by=none,app-name\n",
            "group-by=app-name,,category\n",
            "group-by=\n",
        ] {
            assert!(Config::parse(source).is_ok(), "Mako-valid config rejected: {source}");
        }
    }

    #[test]
    fn mako_last_repeated_selector_wins_without_affecting_other_fields() {
        let c = Config::parse(
            "[category=wrong category=success app-name=example]\nwidth=440\n"
        ).unwrap();
        assert_eq!(c.style_for(&note("success", &["default"]))["width"], "440");
        assert_eq!(c.style_for(&note("wrong", &["default"]))["width"], "300");

        let c = Config::parse(
            "[summary~=^Wrong summary~=^Saved]\nheight=140\n"
        ).unwrap();
        assert_eq!(c.style_for(&note("general", &["default"]))["height"], "140");

        let c = Config::parse(
            "[mode=quiet mode=default !actionable]\nwidth=420\n"
        ).unwrap();
        assert_eq!(c.style_for(&note("general", &["default"]))["width"], "420");

        // Different selector types are *not* duplicates and remain forbidden.
        assert!(Config::parse(
            "[summary=Saved summary~=^Saved]\nwidth=420\n"
        ).is_err());
    }

    #[test]
    fn repeated_sort_direction_replaces_previous_bit() {
        let c = Config::parse("sort=+time\nsort=-time\nsort=+priority\n").unwrap();
        assert_eq!(c.sort, vec!["-time", "+priority"]);
    }
    #[test]
    fn latest_matching_rule_wins() {
        let c=Config::parse("width=300\n[category=success]\nwidth=400\n[app-name=example]\nwidth=440\n").unwrap();
        assert_eq!(c.style_for(&note("success", &["default"]))["width"],"440");
    }
}
