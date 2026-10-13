//! Minimal Rust source scanner for the coverage gate: resolves each crate's
//! module tree (so no source file can silently drop out of the denominator),
//! evaluates `cfg` predicates per build target and itemises the only permitted
//! line exclusions: items gated on `cfg(test)` / `#[test]` or on the opt-in
//! `test-harness` feature.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Punct(char),
    Str(String),
    Other,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    line: usize,
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let (mut i, mut line) = (0, 1);
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            loop {
                if i >= chars.len() {
                    return Err(format!("unterminated block comment (line {line})"));
                }
                if chars[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == '"' {
            let start = line;
            let (text, next, lines) = quoted(&chars, i + 1)?;
            tokens.push(Token {
                tok: Tok::Str(text),
                line: start,
            });
            line += lines;
            i = next;
        } else if c == '\'' {
            // Char literal or lifetime/label.
            if at(i + 1) == '\\' {
                // Skip the escaped character itself, e.g. '\''.
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '\'' {
                    j += 1;
                }
                tokens.push(Token {
                    tok: Tok::Other,
                    line,
                });
                i = j + 1;
            } else if at(i + 2) == '\'' {
                tokens.push(Token {
                    tok: Tok::Other,
                    line,
                });
                i += 3;
            } else {
                tokens.push(Token {
                    tok: Tok::Punct('\''),
                    line,
                });
                i += 1;
            }
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let raw_capable = matches!(word.as_str(), "r" | "br" | "cr");
            let prefix = raw_capable || matches!(word.as_str(), "b" | "c");
            if prefix && (at(i) == '"' || (raw_capable && at(i) == '#')) {
                let mut hashes = 0;
                while at(i) == '#' {
                    hashes += 1;
                    i += 1;
                }
                if at(i) != '"' {
                    // Raw identifier such as r#match.
                    let start = i;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    tokens.push(Token {
                        tok: Tok::Ident(chars[start..i].iter().collect()),
                        line,
                    });
                    continue;
                }
                let start = line;
                let (text, next, lines) = if raw_capable {
                    raw_quoted(&chars, i + 1, hashes)?
                } else {
                    quoted(&chars, i + 1)?
                };
                tokens.push(Token {
                    tok: Tok::Str(text),
                    line: start,
                });
                line += lines;
                i = next;
            } else if word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                tokens.push(Token {
                    tok: Tok::Other,
                    line,
                });
            } else {
                tokens.push(Token {
                    tok: Tok::Ident(word),
                    line,
                });
            }
        } else {
            tokens.push(Token {
                tok: Tok::Punct(c),
                line,
            });
            i += 1;
        }
    }
    Ok(tokens)
}

/// Returns (contents, index after closing quote, newlines consumed).
fn quoted(chars: &[char], mut i: usize) -> Result<(String, usize, usize)> {
    let (mut text, mut lines) = (String::new(), 0);
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if chars.get(i + 1) == Some(&'\n') {
                    lines += 1;
                }
                i += 2;
            }
            '"' => return Ok((text, i + 1, lines)),
            c => {
                if c == '\n' {
                    lines += 1;
                }
                text.push(c);
                i += 1;
            }
        }
    }
    Err("unterminated string literal".into())
}

fn raw_quoted(chars: &[char], mut i: usize, hashes: usize) -> Result<(String, usize, usize)> {
    let (mut text, mut lines) = (String::new(), 0);
    while i < chars.len() {
        if chars[i] == '"' && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#')) {
            return Ok((text, i + 1 + hashes, lines));
        }
        if chars[i] == '\n' {
            lines += 1;
        }
        text.push(chars[i]);
        i += 1;
    }
    Err("unterminated raw string literal".into())
}

/// A parsed `cfg(...)` predicate.
#[derive(Clone, Debug, PartialEq)]
pub enum Cfg {
    Name(String),
    Value(String, String),
    All(Vec<Cfg>),
    Any(Vec<Cfg>),
    Not(Box<Cfg>),
}

impl std::fmt::Display for Cfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |f: &mut std::fmt::Formatter<'_>, name: &str, items: &[Cfg]| {
            let inner: Vec<String> = items.iter().map(ToString::to_string).collect();
            write!(f, "{name}({})", inner.join(", "))
        };
        match self {
            Cfg::Name(name) => f.write_str(name),
            Cfg::Value(key, value) => write!(f, "{key} = \"{value}\""),
            Cfg::All(items) => list(f, "all", items),
            Cfg::Any(items) => list(f, "any", items),
            Cfg::Not(inner) => write!(f, "not({inner})"),
        }
    }
}

/// The only line exclusions the coverage gate permits.
pub const TEST_HARNESS_FEATURE: &str = "test-harness";

impl Cfg {
    /// True when the predicate can only hold in a unit-test build or an
    /// opt-in test-harness build, i.e. the gated code is test code.
    pub fn test_only(&self) -> bool {
        match self {
            Cfg::Name(name) => name == "test",
            Cfg::Value(key, value) => key == "feature" && value == TEST_HARNESS_FEATURE,
            Cfg::All(items) => items.iter().any(Cfg::test_only),
            Cfg::Any(items) => !items.is_empty() && items.iter().all(Cfg::test_only),
            Cfg::Not(_) => false,
        }
    }

    pub fn eval(&self, target: Target) -> Result<bool> {
        Ok(match self {
            Cfg::Name(name) => match (name.as_str(), target) {
                ("test" | "unix" | "debug_assertions" | "coverage", Target::Native) => true,
                ("debug_assertions" | "coverage", Target::Wasm) => true,
                ("test" | "unix" | "windows", Target::Wasm) | ("windows", Target::Native) => false,
                _ => return Err(format!("unsupported cfg predicate `{self}`")),
            },
            Cfg::Value(key, value) => match (key.as_str(), value.as_str()) {
                ("target_arch", "wasm32") | ("target_family", "wasm") => target == Target::Wasm,
                ("target_family", "unix") => target == Target::Native,
                // Every feature is enabled in some measured build: native uses
                // --all-features; WASM builds `coverage` and `coverage,test-harness`.
                ("feature", _) => true,
                _ => return Err(format!("unsupported cfg predicate `{self}`")),
            },
            Cfg::All(items) => {
                for item in items {
                    if !item.eval(target)? {
                        return Ok(false);
                    }
                }
                true
            }
            Cfg::Any(items) => {
                for item in items {
                    if item.eval(target)? {
                        return Ok(true);
                    }
                }
                false
            }
            Cfg::Not(inner) => !inner.eval(target)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Native,
    Wasm,
}

fn parse_cfg(tokens: &[Tok], pos: &mut usize) -> Result<Cfg> {
    let Some(Tok::Ident(name)) = tokens.get(*pos) else {
        return Err("expected cfg predicate".into());
    };
    *pos += 1;
    match tokens.get(*pos) {
        Some(Tok::Punct('=')) => {
            *pos += 1;
            let Some(Tok::Str(value)) = tokens.get(*pos) else {
                return Err(format!("expected string after `{name} =`"));
            };
            *pos += 1;
            Ok(Cfg::Value(name.clone(), value.clone()))
        }
        Some(Tok::Punct('(')) => {
            *pos += 1;
            let mut items = Vec::new();
            while tokens.get(*pos) != Some(&Tok::Punct(')')) {
                items.push(parse_cfg(tokens, pos)?);
                if tokens.get(*pos) == Some(&Tok::Punct(',')) {
                    *pos += 1;
                }
            }
            *pos += 1;
            match name.as_str() {
                "all" => Ok(Cfg::All(items)),
                "any" => Ok(Cfg::Any(items)),
                "not" if items.len() == 1 => Ok(Cfg::Not(Box::new(items.remove(0)))),
                _ => Err(format!("unsupported cfg combinator `{name}`")),
            }
        }
        _ => Ok(Cfg::Name(name.clone())),
    }
}

#[derive(Default)]
struct Attrs {
    first_line: Option<usize>,
    cfgs: Vec<Cfg>,
    test: bool,
    path: Option<String>,
    keyword_at: Option<usize>,
}

impl Attrs {
    fn exclusion(&self) -> Option<String> {
        let mut reasons: Vec<String> = self
            .cfgs
            .iter()
            .filter(|cfg| cfg.test_only())
            .map(|cfg| format!("#[cfg({cfg})]"))
            .collect();
        if self.test {
            reasons.push("#[test] (implicitly cfg(test))".into());
        }
        (!reasons.is_empty()).then(|| reasons.join(" "))
    }
}

/// An inclusive line range removed from every coverage denominator.
#[derive(Clone, Debug, PartialEq)]
pub struct Exclusion {
    pub start: usize,
    pub end: usize,
    pub reason: String,
}

#[derive(Debug)]
struct ModDecl {
    name: String,
    inline_path: Vec<String>,
    path_attr: Option<String>,
    cfgs: Vec<Cfg>,
    excluded: Option<String>,
}

#[derive(Debug, Default)]
struct Scan {
    inner_cfgs: Vec<Cfg>,
    exclusions: Vec<Exclusion>,
    mods: Vec<ModDecl>,
    /// A `fn` body exists outside excluded ranges (the file has executable code).
    has_code: bool,
}

const ITEM_KEYWORDS: &[&str] = &[
    "fn",
    "impl",
    "mod",
    "trait",
    "enum",
    "union",
    "struct",
    "extern",
    "macro_rules",
];
const STATEMENT_KEYWORDS: &[&str] = &["const", "static", "let", "use", "type"];

/// Index of the last token of the item starting at `start`.
fn item_end(tokens: &[Token], start: usize) -> usize {
    let mut i = start;
    let mut kind = None;
    while let Some(Token {
        tok: Tok::Ident(word),
        ..
    }) = tokens.get(i)
    {
        match word.as_str() {
            "pub" | "async" | "unsafe" | "default" => {
                i += 1;
                if tokens.get(i).map(|t| &t.tok) == Some(&Tok::Punct('(')) {
                    while tokens.get(i).is_some_and(|t| t.tok != Tok::Punct(')')) {
                        i += 1;
                    }
                    i += 1;
                }
            }
            "const" if matches!(tokens.get(i + 1).map(|t| &t.tok), Some(Tok::Ident(next)) if next == "fn" || next == "unsafe" || next == "async") => {
                i += 1
            }
            w if ITEM_KEYWORDS.contains(&w) => {
                kind = Some(true);
                break;
            }
            w if STATEMENT_KEYWORDS.contains(&w) => {
                kind = Some(false);
                break;
            }
            _ => break,
        }
    }
    let block_item = kind == Some(true);
    let expression = kind.is_none();
    let mut depth: i64 = 0;
    let mut j = start;
    while j < tokens.len() {
        match tokens[j].tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth -= 1;
                if depth < 0 {
                    return j.saturating_sub(1).max(start);
                }
                if depth == 0 && tokens[j].tok == Tok::Punct('}') {
                    let next = tokens.get(j + 1).map(|t| &t.tok);
                    if block_item
                        || (expression && !matches!(next, Some(Tok::Punct(';' | '.' | '?'))))
                    {
                        return j;
                    }
                }
            }
            Tok::Punct(';') if depth == 0 => return j,
            Tok::Punct(',') if depth == 0 && expression => return j,
            _ => {}
        }
        j += 1;
    }
    tokens.len() - 1
}

fn scan(source: &str) -> Result<Scan> {
    let tokens = tokenize(source)?;
    let toks: Vec<Tok> = tokens.iter().map(|t| t.tok.clone()).collect();
    let mut out = Scan::default();
    let mut attrs = Attrs::default();
    // Brace stack: Some((module name, its cfgs)) for inline `mod name { }` blocks.
    let mut braces: Vec<Option<(String, Vec<Cfg>)>> = Vec::new();
    let mut pending_mod: Option<(String, Vec<Cfg>)> = None;
    // (last excluded token index, reason) for enclosing excluded items.
    let mut excluded: Vec<(usize, String)> = Vec::new();
    let mut seen_item = false;
    let mut i = 0;
    while i < tokens.len() {
        while excluded.last().is_some_and(|(end, _)| *end < i) {
            excluded.pop();
        }
        let tok = &toks[i];
        if *tok == Tok::Punct('#') {
            let inner = toks.get(i + 1) == Some(&Tok::Punct('!'));
            let open = if inner { i + 2 } else { i + 1 };
            if toks.get(open) == Some(&Tok::Punct('[')) {
                let mut depth = 0;
                let mut close = open;
                while close < toks.len() {
                    match toks[close] {
                        Tok::Punct('[') => depth += 1,
                        Tok::Punct(']') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    close += 1;
                }
                let body = &toks[open + 1..close];
                let mut cfg = None;
                match body.first() {
                    Some(Tok::Ident(name)) if name == "cfg" => {
                        let mut pos = 2;
                        cfg = Some(
                            parse_cfg(body, &mut pos)
                                .map_err(|e| format!("line {}: {e}", tokens[i].line))?,
                        );
                    }
                    Some(Tok::Ident(name)) if name == "test" && body.len() == 1 => {
                        attrs.test = true;
                    }
                    Some(Tok::Ident(name)) if name == "path" => {
                        if let Some(Tok::Str(path)) = body.get(2) {
                            attrs.path = Some(path.clone());
                        }
                    }
                    _ => {}
                }
                if inner {
                    if let Some(cfg) = cfg {
                        if seen_item || !braces.is_empty() {
                            return Err(format!(
                                "line {}: inner #![cfg] is only supported at file level",
                                tokens[i].line
                            ));
                        }
                        out.inner_cfgs.push(cfg);
                    }
                } else {
                    attrs.first_line.get_or_insert(tokens[i].line);
                    if let Some(cfg) = cfg {
                        attrs.cfgs.push(cfg);
                    }
                }
                i = close + 1;
                continue;
            }
        }
        // Start of an item/statement following outer attributes. Attributes stay
        // attached across a leading visibility such as `pub(crate)`.
        if attrs.first_line.is_some() && attrs.keyword_at.is_none() {
            let mut keyword = i;
            if toks[keyword] == Tok::Ident("pub".into()) {
                keyword += 1;
                if toks.get(keyword) == Some(&Tok::Punct('(')) {
                    while toks.get(keyword).is_some_and(|t| *t != Tok::Punct(')')) {
                        keyword += 1;
                    }
                    keyword += 1;
                }
            }
            attrs.keyword_at = Some(keyword);
            if let Some(reason) = attrs.exclusion() {
                let end = item_end(&tokens, i);
                out.exclusions.push(Exclusion {
                    start: attrs.first_line.unwrap_or(tokens[i].line),
                    end: tokens[end].line,
                    reason: reason.clone(),
                });
                excluded.push((end, reason));
            }
        }
        seen_item = true;
        if *tok == Tok::Ident("fn".into()) && excluded.is_empty() {
            let end = item_end(&tokens, i);
            if toks[end] == Tok::Punct('}') {
                out.has_code = true;
            }
        }
        if *tok == Tok::Ident("mod".into())
            && let Some(Tok::Ident(name)) = toks.get(i + 1)
        {
            match toks.get(i + 2) {
                Some(Tok::Punct(';')) => {
                    let inline_path = braces.iter().flatten().map(|(n, _)| n.clone()).collect();
                    let mut cfgs: Vec<Cfg> = braces
                        .iter()
                        .flatten()
                        .flat_map(|(_, c)| c.clone())
                        .collect();
                    cfgs.extend(attrs.cfgs.iter().cloned());
                    out.mods.push(ModDecl {
                        name: name.clone(),
                        inline_path,
                        path_attr: attrs.path.take(),
                        cfgs,
                        excluded: excluded.last().map(|(_, r)| r.clone()),
                    });
                }
                Some(Tok::Punct('{')) => {
                    pending_mod = Some((name.clone(), attrs.cfgs.clone()));
                }
                _ => {}
            }
        }
        if attrs.keyword_at.is_none_or(|keyword| i >= keyword) {
            attrs = Attrs::default();
        }
        match tok {
            Tok::Punct('{') => braces.push(pending_mod.take()),
            Tok::Punct('}') => {
                braces.pop();
            }
            _ => {}
        }
        i += 1;
    }
    out.exclusions = merge(out.exclusions);
    Ok(out)
}

/// Drop ranges nested inside an earlier range.
fn merge(mut ranges: Vec<Exclusion>) -> Vec<Exclusion> {
    ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut merged: Vec<Exclusion> = Vec::new();
    for range in ranges {
        if merged.last().is_some_and(|last| range.end <= last.end) {
            continue;
        }
        merged.push(range);
    }
    merged
}

/// Inclusive line ranges of every `fn name` item in `source`.
pub fn function_lines(source: &str, name: &str) -> Result<Vec<(usize, usize)>> {
    let tokens = tokenize(source)?;
    let mut ranges = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.tok == Tok::Ident("fn".into())
            && tokens.get(i + 1).map(|t| &t.tok) == Some(&Tok::Ident(name.into()))
        {
            let end = item_end(&tokens, i);
            ranges.push((token.line, tokens[end].line));
        }
    }
    Ok(ranges)
}

/// Inclusive lines of the first brace-delimited block that starts on or after
/// `start_line` (for example an `if ... { }` statement).
pub fn block_lines(source: &str, start_line: usize) -> Result<(usize, usize)> {
    let tokens = tokenize(source)?;
    let open = tokens
        .iter()
        .position(|t| t.line >= start_line && t.tok == Tok::Punct('{'))
        .ok_or("no block after the marker")?;
    let mut depth = 0usize;
    for token in &tokens[open..] {
        match token.tok {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => {
                depth -= 1;
                if depth == 0 {
                    return Ok((start_line, token.line));
                }
            }
            _ => {}
        }
    }
    Err("unterminated block".into())
}

/// One first-party source file and how the measured builds compile it.
#[derive(Debug)]
pub struct SourceFile {
    pub path: String,
    pub krate: String,
    pub lines: usize,
    pub native: bool,
    pub wasm: bool,
    /// Whole file is test/harness-only (reason), via its module declaration chain.
    pub whole_file: Option<String>,
    pub exclusions: Vec<Exclusion>,
    pub has_code: bool,
}

/// Crates linked into the browser client artifact.
pub const WASM_CRATES: &[&str] = &[
    "lytronaut-client",
    "lytronaut-format",
    "lytronaut-contracts",
];

/// Resolve every crate's module tree under `root/crates/*/src`. Any `.rs` file
/// that is not reachable from a crate root is an error, never a silent drop.
pub fn inventory(root: &Path) -> Result<Vec<SourceFile>> {
    let mut files = Vec::new();
    let mut crates: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("src").is_dir())
        .collect();
    crates.sort();
    for dir in crates {
        let krate = dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("bad crate dir")?
            .to_owned();
        let mut all = Vec::new();
        collect_rs(&dir.join("src"), &mut all)?;
        let mut reached: BTreeMap<PathBuf, (Vec<Cfg>, Option<String>)> = BTreeMap::new();
        let mut roots = vec![dir.join("src/lib.rs"), dir.join("src/main.rs")];
        if let Ok(bins) = fs::read_dir(dir.join("src/bin")) {
            roots.extend(bins.filter_map(|e| e.ok().map(|e| e.path())));
        }
        let mut queue: Vec<(PathBuf, Vec<Cfg>, Option<String>, bool)> = roots
            .into_iter()
            .filter(|p| p.is_file())
            .map(|p| (p, Vec::new(), None, true))
            .collect();
        let mut scans = BTreeMap::new();
        while let Some((file, chain, excluded, mod_rs)) = queue.pop() {
            if reached.contains_key(&file) {
                return Err(format!("{} is included more than once", file.display()));
            }
            let source =
                fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let scan = scan(&source).map_err(|e| format!("{}: {e}", file.display()))?;
            let mut chain = chain;
            chain.extend(scan.inner_cfgs.iter().cloned());
            let excluded = excluded.or_else(|| {
                scan.inner_cfgs
                    .iter()
                    .find(|c| c.test_only())
                    .map(|c| format!("#![cfg({c})] file attribute"))
            });
            let stem_dir = if mod_rs || file.file_name().is_some_and(|n| n == "mod.rs") {
                file.parent().ok_or("no parent")?.to_path_buf()
            } else {
                file.with_extension("")
            };
            for decl in &scan.mods {
                let child = if let Some(path) = &decl.path_attr {
                    if !decl.inline_path.is_empty() {
                        return Err(format!(
                            "{}: #[path] inside an inline module",
                            file.display()
                        ));
                    }
                    file.parent().ok_or("no parent")?.join(path)
                } else {
                    let mut base = stem_dir.clone();
                    base.extend(&decl.inline_path);
                    let flat = base.join(format!("{}.rs", decl.name));
                    if flat.is_file() {
                        flat
                    } else {
                        base.join(&decl.name).join("mod.rs")
                    }
                };
                if !child.is_file() {
                    return Err(format!(
                        "{}: module `{}` not found at {}",
                        file.display(),
                        decl.name,
                        child.display()
                    ));
                }
                let mut child_chain = chain.clone();
                child_chain.extend(decl.cfgs.iter().cloned());
                let child_excluded =
                    excluded
                        .clone()
                        .or_else(|| decl.excluded.clone())
                        .or_else(|| {
                            decl.cfgs
                                .iter()
                                .find(|c| c.test_only())
                                .map(|c| format!("#[cfg({c})] module declaration"))
                        });
                queue.push((child, child_chain, child_excluded, false));
            }
            reached.insert(file.clone(), (chain, excluded));
            scans.insert(file, (scan, source.lines().count()));
        }
        for file in &all {
            if !reached.contains_key(file) {
                return Err(format!(
                    "{} is not reachable from any crate root module tree; it would vanish from coverage",
                    file.display()
                ));
            }
        }
        for (file, (chain, excluded)) in reached {
            let (scan, lines) = scans.remove(&file).ok_or("scan missing")?;
            let mut native = true;
            let mut wasm = WASM_CRATES.contains(&krate.as_str());
            for cfg in &chain {
                native &= cfg
                    .eval(Target::Native)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                wasm &= cfg
                    .eval(Target::Wasm)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
            }
            // A crate's binary root is native-only.
            if file.ends_with("src/main.rs")
                || file.parent().is_some_and(|p| p.ends_with("src/bin"))
            {
                wasm = false;
            }
            let path = file
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(SourceFile {
                path,
                krate: krate.clone(),
                lines,
                native,
                wasm,
                whole_file: excluded,
                exclusions: scan.exclusions,
                has_code: scan.has_code,
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_rs(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_test_and_harness_items_but_not_production_cfgs() {
        let source = r###"
fn production() { let s = "#[cfg(test)] { not an attribute"; let c = '{'; }
#[cfg(test)]
mod tests {
    #[test]
    fn inner() {}
}
#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
pub fn harness() -> u8 {
    1
}
#[cfg(not(feature = "test-harness"))]
fn kept() {}
#[cfg(any(target_arch = "wasm32", test))]
fn shared<'a>(x: &'a str) -> &'a str { x }
struct S {
    a: u8,
    #[cfg(feature = "test-harness")]
    observed: u8,
}
fn body() {
    #[cfg(feature = "test-harness")]
    observe(r##"}"##);
    work();
}
"###;
        let scan = scan(source).unwrap();
        let ranges: Vec<(usize, usize)> =
            scan.exclusions.iter().map(|e| (e.start, e.end)).collect();
        assert_eq!(ranges, vec![(3, 7), (8, 11), (18, 19), (22, 23)]);
        assert!(scan.exclusions[1].reason.contains("test-harness"));
        assert!(scan.has_code);
    }

    #[test]
    fn module_declarations_carry_cfg_and_path() {
        let source = "#![cfg(not(target_arch = \"wasm32\"))]\n#[cfg(test)]\n#[path = \"x_tests.rs\"]\npub(crate) mod x_tests;\nmod a { mod b; }\npub const ONLY: &str = \"data\";\n";
        let scan = scan(source).unwrap();
        assert_eq!(scan.inner_cfgs.len(), 1);
        assert_eq!(scan.mods.len(), 2);
        assert_eq!(scan.mods[0].path_attr.as_deref(), Some("x_tests.rs"));
        assert!(scan.mods[0].excluded.is_some());
        assert_eq!(scan.mods[1].inline_path, vec!["a".to_owned()]);
        assert!(!scan.has_code);
    }

    #[test]
    fn function_lines_cover_the_whole_body() {
        let source = "impl A {\n    async fn present_gpu(&self) {\n        if x { y(); }\n    }\n    fn other() {}\n}\n";
        assert_eq!(function_lines(source, "present_gpu").unwrap(), vec![(2, 4)]);
        assert!(function_lines(source, "absent").unwrap().is_empty());
        assert_eq!(block_lines(source, 3).unwrap(), (3, 3));
    }

    #[test]
    fn cfg_evaluation_and_test_only_classification() {
        let parse = |s: &str| {
            let toks: Vec<Tok> = tokenize(s).unwrap().into_iter().map(|t| t.tok).collect();
            parse_cfg(&toks, &mut 0).unwrap()
        };
        let wasm_only = parse("target_arch = \"wasm32\"");
        assert!(wasm_only.eval(Target::Wasm).unwrap());
        assert!(!wasm_only.eval(Target::Native).unwrap());
        assert!(
            parse("all(feature = \"test-harness\", any(target_arch = \"wasm32\", test))")
                .test_only()
        );
        assert!(!parse("any(target_arch = \"wasm32\", test)").test_only());
        assert!(!parse("not(feature = \"test-harness\")").test_only());
        assert!(parse("target_os = \"linux\"").eval(Target::Native).is_err());
    }
}
