//! Partial port of Mako's configuration and criteria engine.
//!
//! Preserves Mako's INI-like `key=value` format and ordered criteria rules.
//! This is deliberately not TOML (TOML is scheduled *after* Mako parity).

use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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
            Some(Regex::new(value).map_err(|e| format!("invalid regex: {e}"))?)
        } else { None };
        if ["actionable", "expiring", "grouped", "hidden"].contains(&key)
            && parse_bool(value).is_none() {
            return Err(format!("invalid boolean '{value}' for {key}"));
        }
        if key == "urgency" && !["low", "normal", "critical", "high"].contains(&value.to_ascii_lowercase().as_str()) {
            return Err(format!("invalid urgency '{value}'"));
        }
        if key == "group-index" && value.parse::<i32>().is_err() {
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
            "group-index" => return self.value.parse::<i32>().ok() == Some(n.group_index),
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
        }
    }
}

impl Config {
    /// Resolve root style and matching user rules. Grouped/hidden built-in
    /// selectors and cross-output layout are deliberately not claimed as ported.
    pub fn style_for(&self, n: &Traits<'_>) -> Options {
        let mut style = self.global.clone();
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
            }
        }
        style
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let mut conf = Self::default();
        conf.append(text, "<input>", None, 0, &mut HashSet::new())?;
        Ok(conf)
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
        for (lineno, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let location = format!("{source}:{}", lineno + 1);
            if line.starts_with('[') && line.ends_with(']') {
                let selector = &line[1..line.len()-1];
                let selectors = parse_selectors(selector).map_err(|e| err(&location, e))?;
                self.rules.push(Rule {
                    source: location, raw_selector: selector.into(), selectors,
                    options: Options::new(),
                });
                current = Some(self.rules.len() - 1);
                continue;
            }
            let (key, val) = line.split_once('=').ok_or_else(||err(&location,"expected key=value"))?;
            // Mako doesn't treat `#` after '=' as a comment: it is often a color.
            let key = key.trim_end();
            let value = val.trim_end();
            if key == "include" && current.is_none() {
                let path = resolve_include(value).map_err(|e|err(&location,e))?;
                self.load(&path, depth+1, seen)?;
            } else if current.is_none() && key == "max-history" {
                // Mako parses max-history as a signed int and treats
                // zero or negative values as history disabled.
                let signed=value.parse::<i32>()
                    .map_err(|_|err(&location,"invalid max-history"))?;
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
                match current {
                    Some(index) => { self.rules[index].options.insert(key.into(),value.into()); },
                    None => { self.global.insert(key.into(),value.into()); },
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
        if let Ok(number) = value.parse::<i32>() {
            if number >= (if key=="width"||key=="height"||key=="max-icon-size" { 1 } else { 0 }) { return Ok(()); }
        }
    } else if key == "max-visible" {
        if value.parse::<i32>().is_ok() { return Ok(()); }
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
        let items: Vec<_> = value.split(',').collect();
        if (1..=4).contains(&items.len()) && items.iter().all(|x|x.parse::<i32>().is_ok()) { return Ok(()); }
    } else if COLORS.contains(&key) {
        let raw = value.strip_prefix('#');
        if let Some(raw) = raw {
            if (raw.len()==6 || raw.len()==8) && raw.chars().all(|x|x.is_ascii_hexdigit()) { return Ok(()); }
        }
    } else if key=="progress-color" {
        let words:Vec<_>=value.split_whitespace().collect();
        let color=match words.as_slice(){
            [color]=>Some(*color),
            [mode,color] if mode.eq_ignore_ascii_case("over")
                || mode.eq_ignore_ascii_case("source")=>Some(*color),
            _=>None,
        };
        if color.is_some_and(|c|crate::raster::Color::parse_mako(c).is_ok()){
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
        for value in ["#5588AAFF","source #123456","OVER #55667788"] {
            assert!(Config::parse(&format!("progress-color={value}\n")).is_ok());
        }
        for value in ["purple","source","multiply #112233",
            "over #11223","source #FFFFFF trailing"] {
            assert!(Config::parse(&format!("progress-color={value}\n")).is_err());
        }
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
    fn signed_max_history_matches_mako_nonpositive_disable_semantics() {
        for setting in ["max-history=-1","max-history=-100","max-history=0"] {
            let config=Config::parse(&format!("{setting}\n")).unwrap();
            assert_eq!(config.max_history,0);
        }
        assert_eq!(Config::parse("max-history=7\n").unwrap().max_history,7);
        assert!(Config::parse("max-history=2147483648\n").is_err());
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
