//! Partial port of Mako's configuration and criteria engine.
//!
//! Preserves Mako's INI-like `key=value` format and ordered criteria rules.
//! This is deliberately not TOML (TOML is scheduled *after* Mako parity).

use regex::Regex;
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

/// In glibc's POSIX REG_EXTENDED, escaped ASCII letters are literals
/// except the GNU word/whitespace operators \b, \B, \w, \W, \s, \S.
/// Within bracket expressions *all* escaped ASCII letters are literals.
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
                normalized.push(ch);
                // POSIX [[:alpha:]], [[.ch.]] and [[=a=]] are nested
                // bracket elements, not the end of the surrounding class.
                if let Some(delimiter @ (':' | '.' | '=')) = chars.peek().copied() {
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
                }
            }
            ']' if in_class => {
                in_class = false;
                normalized.push(ch);
            }
            _ => normalized.push(ch),
        }
    }
    normalized
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
            // Unlike Rust regex's default, POSIX '.' matches a newline in
            // that mode. Dot-all must NOT also enable multiline anchors.
            if has_rust_only_group_extension(value) {
                return Err("POSIX ERE does not support (? group extensions".into());
            }
            let pattern = mako_posix_ere_escapes(value);
            Some(Regex::new(&format!("(?s:{pattern})"))
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
            "anchor" => n.anchor,
            "output" => n.output,
            "mode" => return n.modes.contains(&self.value.as_str()),
            "actionable" => return parse_bool(&self.value) == Some(n.actionable),
            "expiring" => return parse_bool(&self.value) == Some(n.requested_timeout_ms != 0),
            "grouped" => return parse_bool(&self.value) == Some(n.group_index >= 0),
            "hidden" => return parse_bool(&self.value) == Some(n.hidden),
            "group-index" => return parse_signed_mako(&self.value) == Some(n.group_index),
            _ => return false,
        };
        match &self.regex {
            Some(re) => re.is_match(observed),
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
        let default = [
            ("width", "300"), ("height", "100"), ("margin", "10"),
            ("padding", "5"), ("border-size", "2"),
            ("background-color", "#285577FF"), ("text-color", "#FFFFFFFF"),
            ("border-color", "#4C7899FF"), ("font", "monospace 10"),
            ("format", "<b>%s</b>\\n%b"), ("markup", "1"),
            ("actions", "1"), ("default-timeout", "0"),
            ("ignore-timeout", "0"), ("history", "1"),
            ("max-visible", "5"), ("anchor", "top-right"), ("layer", "top"),
            ("on-button-left", "invoke-default-action"),
            ("on-button-right", "dismiss"), ("on-button-middle", "none"),
            ("on-touch", "dismiss"),
        ];
        Self {
            global: default.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            rules: Vec::new(), max_history: 5, sort: vec!["-time".into()],
            root_padding_state:MakoPaddingState::root(),
        }
    }
}

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

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let mut conf = Self::default();
        conf.append(text, "<input>", None, 0, &mut HashSet::new())?;
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
        conf.load(path, 0, &mut seen)?;
        Ok(conf)
    }

    fn load(&mut self, path: &Path, depth: usize, seen: &mut HashSet<PathBuf>) -> Result<(), ConfigError> {
        if depth >= 16 { return Err(err(path.display().to_string(), "include nesting exceeded 16")); }
        let canonical = path.canonicalize().map_err(|e| err(path.display().to_string(), e.to_string()))?;
        if !seen.insert(canonical.clone()) {
            return Err(err(path.display().to_string(), "recursive include"));
        }
        let result = (|| {
            let text = fs::read_to_string(&canonical)
                .map_err(|e| err(path.display().to_string(), e.to_string()))?;
            self.append(&text, &canonical.display().to_string(), Some(&canonical), depth, seen)
        })();
        seen.remove(&canonical);
        result
    }

    fn append(&mut self, text: &str, source: &str, path: Option<&Path>, depth: usize,
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
                self.load(&path, depth+1, seen)?;
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

    // Mako excludes output, anchor and hidden from its "non-surface"
    // selector test. Only those selectors may control max-visible.
    let non_surface = rule.selectors.iter().any(|selector|
        !matches!(selector.key.as_str(), "output" | "anchor" | "hidden"));
    if non_surface && rule.options.contains_key("max-visible") {
        return Err("max-visible is only allowed with output and/or anchor criteria".into());
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
        let raw = value.strip_prefix('#');
        if let Some(raw) = raw {
            if (raw.len()==6 || raw.len()==8) && raw.chars().all(|x|x.is_ascii_hexdigit()) { return Ok(()); }
        }
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
            modes, requested_timeout_ms:-1, actionable:false, group_index:-1, hidden:false }
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
        assert!(!normal.contains_key("on-notify"));
        let quiet=c.style_for(&note("success", &["default","quiet"]));
        assert_eq!(quiet["background-color"],"#305747");
        assert_eq!(quiet["on-notify"],"none");
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
