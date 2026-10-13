//! First-party line coverage gate: native + instrumented Rust/WASM + union.
//!
//! Inputs are LCOV exports (`cargo llvm-cov report --lcov` for native, and
//! `llvm-cov export -format=lcov` per instrumented WASM build). The denominator
//! is the full module-tree inventory of `crates/*/src`; the only lines removed
//! are itemised `cfg(test)`/`#[test]` and `test-harness`-feature items.
use crate::rust_source::{self, SourceFile};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type LineMap = BTreeMap<String, BTreeMap<usize, u64>>;

/// Line coverage floors in percent: the single source of truth for the gate.
pub const NATIVE_FLOOR: f64 = 95.0;
pub const WASM_FLOOR: f64 = 95.0;
pub const UNION_FLOOR: f64 = 95.0;

/// Crates measured by the native gate (shared format/contracts included).
const NATIVE_GATE: &[&str] = &[
    "lytronaut-contracts",
    "lytronaut-format",
    "lytronaut-service",
    "lytronaut-storage",
];
/// Crate measured by the WASM gate: all browser-only first-party code.
const WASM_GATE: &[&str] = &["lytronaut-client"];
/// First-party tooling crates: reported separately, never in a denominator.
const TOOLING: &[&str] = &["xtask", "lytronaut-coverage"];

struct Args {
    native: PathBuf,
    wasm: Vec<(String, PathBuf)>,
    meta: Vec<PathBuf>,
    out: PathBuf,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let usage = "usage: xtask coverage --native LCOV --wasm NAME=DIR [--wasm NAME=DIR ...] [--meta FILE] --out DIR";
    let (mut native, mut wasm, mut meta, mut out) = (None, Vec::new(), Vec::new(), None);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let value = iter.next().ok_or(usage)?;
        match flag.as_str() {
            "--native" => native = Some(PathBuf::from(value)),
            "--wasm" => {
                let (name, dir) = value.split_once('=').ok_or(usage)?;
                wasm.push((name.to_owned(), PathBuf::from(dir)));
            }
            "--meta" => meta.push(PathBuf::from(value)),
            "--out" => out = Some(PathBuf::from(value)),
            _ => return Err(usage.into()),
        }
    }
    if wasm.is_empty() {
        return Err(usage.into());
    }
    Ok(Args {
        native: native.ok_or(usage)?,
        wasm,
        meta,
        out: out.ok_or(usage)?,
    })
}

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        return Err(format!("git {args:?} failed").into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Parse LCOV `SF`/`DA` records, keeping only files inside `root`.
fn parse_lcov(path: &Path, root: &Path) -> Result<(LineMap, usize)> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut map, mut external) = (LineMap::new(), BTreeSet::new());
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(file) = line.strip_prefix("SF:") {
            let file = Path::new(file);
            let absolute = if file.is_absolute() {
                file.to_path_buf()
            } else {
                root.join(file)
            };
            current = match absolute.strip_prefix(root) {
                Ok(relative) => Some(relative.to_string_lossy().replace('\\', "/")),
                Err(_) => {
                    external.insert(absolute);
                    None
                }
            };
        } else if let (Some(file), Some(record)) = (&current, line.strip_prefix("DA:")) {
            let mut fields = record.split(',');
            let line_no: usize = fields.next().ok_or("bad DA")?.parse()?;
            let count: u64 = fields.next().ok_or("bad DA")?.parse()?;
            let entry = map
                .entry(file.clone())
                .or_default()
                .entry(line_no)
                .or_insert(0);
            *entry = entry.saturating_add(count);
        } else if line == "end_of_record" {
            current = None;
        }
    }
    Ok((map, external.len()))
}

#[derive(Default, Clone, Copy)]
struct Tally {
    lines: usize,
    covered: usize,
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.lines += other.lines;
        self.covered += other.covered;
    }
    fn percent(self) -> Option<f64> {
        (self.lines > 0).then(|| self.covered as f64 * 100.0 / self.lines as f64)
    }
    fn json(self) -> Value {
        json!({"lines": self.lines, "covered": self.covered, "percent": self.percent().map(|p| (p * 100.0).round() / 100.0)})
    }
    fn cell(self) -> String {
        match self.percent() {
            Some(p) => format!("{}/{} ({p:.2}%)", self.covered, self.lines),
            None => "-".into(),
        }
    }
}

fn tally(lines: &BTreeMap<usize, u64>) -> Tally {
    Tally {
        lines: lines.len(),
        covered: lines.values().filter(|c| **c > 0).count(),
    }
}

fn keep(lines: Option<&BTreeMap<usize, u64>>, excluded: &BTreeSet<usize>) -> BTreeMap<usize, u64> {
    lines
        .map(|l| {
            l.iter()
                .filter(|(n, _)| !excluded.contains(n))
                .map(|(n, c)| (*n, *c))
                .collect()
        })
        .unwrap_or_default()
}

fn merge_into(target: &mut BTreeMap<usize, u64>, other: &BTreeMap<usize, u64>) {
    for (line, count) in other {
        let entry = target.entry(*line).or_insert(0);
        *entry = entry.saturating_add(*count);
    }
}

fn excluded_lines(file: &SourceFile) -> BTreeSet<usize> {
    if file.whole_file.is_some() {
        return (1..=file.lines.max(1)).collect();
    }
    file.exclusions
        .iter()
        .flat_map(|e| e.start..=e.end)
        .collect()
}

struct FileResult {
    path: String,
    krate: String,
    native: Tally,
    wasm: Tally,
    union: BTreeMap<usize, u64>,
}

pub fn run(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;
    let root = PathBuf::from(git(&["rev-parse", "--show-toplevel"])?).canonicalize()?;
    let inventory = rust_source::inventory(&root)?;
    let (native_map, native_external) = parse_lcov(&args.native, &root)?;
    let mut wasm_builds = Vec::new();
    let mut wasm_map = LineMap::new();
    for (name, dir) in &args.wasm {
        let (map, external) = parse_lcov(&dir.join("coverage.lcov"), &root)?;
        for (file, lines) in &map {
            merge_into(wasm_map.entry(file.clone()).or_default(), lines);
        }
        wasm_builds.push((name.clone(), dir.clone(), map, external));
    }

    let mut failures: Vec<String> = Vec::new();
    let known: BTreeSet<&str> = inventory.iter().map(|f| f.path.as_str()).collect();
    let mut tooling_lcov = Vec::new();
    for (target, map) in [("native", &native_map), ("wasm", &wasm_map)] {
        for (path, lines) in map.iter() {
            let first_party_src = path.starts_with("crates/") && path.contains("/src/");
            if first_party_src && !known.contains(path.as_str()) {
                failures.push(format!(
                    "{path}: in the {target} report but not in the source inventory"
                ));
            } else if !first_party_src {
                tooling_lcov
                    .push(json!({"path": path, "target": target, "coverage": tally(lines).json()}));
            }
        }
    }

    let mut results = Vec::new();
    let mut exclusions = Vec::new();
    let mut tooling = Vec::new();
    let mut declarations_only = Vec::new();
    for file in &inventory {
        let excluded = excluded_lines(file);
        let native_lines = keep(native_map.get(&file.path), &excluded);
        let wasm_lines = keep(wasm_map.get(&file.path), &excluded);
        let removed = |map: &LineMap| {
            map.get(&file.path)
                .map(|l| l.keys().filter(|n| excluded.contains(n)).count())
                .unwrap_or(0)
        };
        if let Some(reason) = &file.whole_file {
            exclusions.push(json!({"path": file.path, "start": 1, "end": file.lines, "reason": reason, "whole_file": true,
                "removed_instrumented": {"native": removed(&native_map), "wasm": removed(&wasm_map)}}));
        } else {
            for range in &file.exclusions {
                let in_range = |map: &LineMap| {
                    map.get(&file.path)
                        .map(|l| {
                            l.keys()
                                .filter(|n| (range.start..=range.end).contains(*n))
                                .count()
                        })
                        .unwrap_or(0)
                };
                exclusions.push(json!({"path": file.path, "start": range.start, "end": range.end, "reason": range.reason, "whole_file": false,
                    "removed_instrumented": {"native": in_range(&native_map), "wasm": in_range(&wasm_map)}}));
            }
        }
        if TOOLING.contains(&file.krate.as_str()) {
            tooling.push(json!({"path": file.path, "native": tally(&native_lines).json(), "wasm": tally(&wasm_lines).json()}));
            continue;
        }
        if file.whole_file.is_some() {
            continue;
        }
        if !file.native && !file.wasm {
            failures.push(format!("{}: compiled by no measured target", file.path));
        }
        if file.has_code {
            if file.native && NATIVE_GATE.contains(&file.krate.as_str()) && native_lines.is_empty()
            {
                failures.push(format!(
                    "{}: native build compiles it but the native report has no lines for it",
                    file.path
                ));
            }
            if file.wasm && WASM_GATE.contains(&file.krate.as_str()) && wasm_lines.is_empty() {
                failures.push(format!(
                    "{}: WASM build compiles it but no WASM report has lines for it",
                    file.path
                ));
            }
        } else if native_lines.is_empty() && wasm_lines.is_empty() {
            declarations_only.push(file.path.clone());
        }
        if !file.native && !native_lines.is_empty() {
            failures.push(format!(
                "{}: native lines reported for a file classified WASM-only",
                file.path
            ));
        }
        if !file.wasm && !wasm_lines.is_empty() {
            failures.push(format!(
                "{}: WASM lines reported for a file classified native-only",
                file.path
            ));
        }
        let mut union = native_lines.clone();
        merge_into(&mut union, &wasm_lines);
        results.push(FileResult {
            path: file.path.clone(),
            krate: file.krate.clone(),
            native: tally(&native_lines),
            wasm: tally(&wasm_lines),
            union,
        });
    }

    let mut native_total = Tally::default();
    let mut wasm_total = Tally::default();
    let mut union_total = Tally::default();
    let mut client_native = Tally::default();
    let mut shared_wasm = Tally::default();
    let mut crates: BTreeMap<String, (Tally, Tally, Tally)> = BTreeMap::new();
    for result in &results {
        let union = tally(&result.union);
        union_total.add(union);
        if NATIVE_GATE.contains(&result.krate.as_str()) {
            native_total.add(result.native);
        } else {
            client_native.add(result.native);
        }
        if WASM_GATE.contains(&result.krate.as_str()) {
            wasm_total.add(result.wasm);
        } else {
            shared_wasm.add(result.wasm);
        }
        let entry = crates.entry(result.krate.clone()).or_default();
        entry.0.add(result.native);
        entry.1.add(result.wasm);
        entry.2.add(union);
    }
    let gates = [
        ("native", native_total, NATIVE_FLOOR),
        ("wasm (lytronaut-client)", wasm_total, WASM_FLOOR),
        ("union", union_total, UNION_FLOOR),
    ];
    for (name, total, floor) in gates {
        match total.percent() {
            Some(p) if p >= floor => {}
            Some(p) => failures.push(format!(
                "{name} line coverage {p:.2}% is below the {floor}% floor"
            )),
            None => failures.push(format!("{name}: no instrumented lines")),
        }
    }

    // Largest uncovered union regions (consecutive uncovered instrumented lines).
    let mut regions = Vec::new();
    for result in &results {
        let mut current: Option<(usize, usize, usize)> = None;
        for (line, count) in &result.union {
            if *count == 0 {
                current = Some(match current {
                    Some((start, _, n)) => (start, *line, n + 1),
                    None => (*line, *line, 1),
                });
            } else if let Some(region) = current.take() {
                regions.push((result.path.clone(), region));
            }
        }
        if let Some(region) = current {
            regions.push((result.path.clone(), region));
        }
    }
    regions.sort_by(|a, b| b.1.2.cmp(&a.1.2).then(a.0.cmp(&b.0)));
    let mut by_file: Vec<&FileResult> = results.iter().collect();
    by_file.sort_by_key(|r| std::cmp::Reverse(tally(&r.union).lines - tally(&r.union).covered));

    let mut meta = BTreeMap::new();
    for file in &args.meta {
        for line in fs::read_to_string(file)?.lines() {
            if let Some((key, value)) = line.split_once('=') {
                meta.insert(key.trim().to_owned(), value.trim().to_owned());
            }
        }
    }
    let mut builds = Vec::new();
    for (name, dir, map, external) in &wasm_builds {
        let mut client = Tally::default();
        for (path, lines) in map {
            if let Some(file) = inventory
                .iter()
                .find(|f| &f.path == path && WASM_GATE.contains(&f.krate.as_str()))
            {
                client.add(tally(&keep(Some(lines), &excluded_lines(file))));
            }
        }
        builds.push(
            json!({"name": name, "application_wasm_sha256": sha256(&dir.join("application.wasm"))?,
            "external_files_dropped": external, "lytronaut_client": client.json()}),
        );
    }
    let head = git(&["rev-parse", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty();
    let report = json!({
        "schema": 1,
        "passed": failures.is_empty(),
        "failures": failures.clone(),
        "source": {"git_head": head, "worktree_dirty": dirty, "cargo_lock_sha256": sha256(&root.join("Cargo.lock"))?},
        "floors": {"native": NATIVE_FLOOR, "wasm": WASM_FLOOR, "union": UNION_FLOOR},
        "totals": {"native": native_total.json(), "wasm_client": wasm_total.json(), "union": union_total.json(),
            "informational": {"client_native": client_native.json(), "shared_crates_wasm": shared_wasm.json()}},
        "crates": crates.iter().map(|(k, (n, w, u))| json!({"crate": k, "native": n.json(), "wasm": w.json(), "union": u.json()})).collect::<Vec<_>>(),
        "files": results.iter().map(|r| json!({"path": r.path, "crate": r.krate, "native": r.native.json(), "wasm": r.wasm.json(), "union": tally(&r.union).json(),
            "uncovered_union_lines": r.union.iter().filter(|(_, c)| **c == 0).map(|(n, _)| *n).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "largest_uncovered_regions": regions.iter().take(60).map(|(p, (s, e, n))| json!({"path": p, "start": s, "end": e, "uncovered_lines": n})).collect::<Vec<_>>(),
        "exclusions": exclusions,
        "declarations_only": declarations_only,
        "tooling": {"inventory": tooling, "tests_examples_from_reports": tooling_lcov},
        "wasm_builds": builds,
        "native_external_files_dropped": native_external,
        "meta": meta,
    });

    let mut md = String::new();
    md.push_str(&format!(
        "# Lytronaut first-party line coverage\n\nResult: **{}**\n\nSource `{head}`{} · Cargo.lock sha256 `{}`\n\n",
        if report["passed"] == true { "PASS" } else { "FAIL" },
        if dirty { " (dirty worktree)" } else { "" },
        report["source"]["cargo_lock_sha256"].as_str().unwrap_or("")
    ));
    md.push_str("| Gate | Covered/instrumented lines | Floor |\n|---|---|---|\n");
    for (name, total, floor) in gates {
        md.push_str(&format!("| {name} | {} | {floor}% |\n", total.cell()));
    }
    md.push_str(&format!(
        "\nInformational (not gated): lytronaut-client native {} · format/contracts in WASM {}\n",
        client_native.cell(),
        shared_wasm.cell()
    ));
    if !failures.is_empty() {
        md.push_str("\n## Failures\n\n");
        for failure in report["failures"].as_array().into_iter().flatten() {
            md.push_str(&format!("- {}\n", failure.as_str().unwrap_or("")));
        }
    }
    md.push_str("\n## Per crate\n\n| Crate | Native | WASM | Union |\n|---|---|---|---|\n");
    for (name, (n, w, u)) in &crates {
        md.push_str(&format!(
            "| {name} | {} | {} | {} |\n",
            n.cell(),
            w.cell(),
            u.cell()
        ));
    }
    md.push_str("\n## Per file (largest union gap first)\n\n| File | Native | WASM | Union | Uncovered |\n|---|---|---|---|---|\n");
    for r in &by_file {
        let u = tally(&r.union);
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            r.path,
            r.native.cell(),
            r.wasm.cell(),
            u.cell(),
            u.lines - u.covered
        ));
    }
    md.push_str(
        "\n## Largest uncovered regions (union)\n\n| File | Lines | Uncovered |\n|---|---|---|\n",
    );
    for (path, (start, end, n)) in regions.iter().take(40) {
        md.push_str(&format!("| {path} | {start}-{end} | {n} |\n"));
    }
    md.push_str("\n## Itemised exclusions (only cfg(test)/#[test] and test-harness feature code)\n\n| File | Lines | Reason | Instrumented removed (native/WASM) |\n|---|---|---|---|\n");
    for e in report["exclusions"].as_array().into_iter().flatten() {
        md.push_str(&format!(
            "| {} | {}-{} | {} | {}/{} |\n",
            e["path"].as_str().unwrap_or(""),
            e["start"],
            e["end"],
            e["reason"].as_str().unwrap_or("").replace('|', "\\|"),
            e["removed_instrumented"]["native"],
            e["removed_instrumented"]["wasm"]
        ));
    }
    md.push_str("\n## Declarations-only files (no function bodies, no instrumented lines)\n\n");
    for path in &report["declarations_only"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        md.push_str(&format!("- {}\n", path.as_str().unwrap_or("")));
    }
    md.push_str("\n## First-party tooling (outside every denominator)\n\nIntegration tests, examples and benches live outside `crates/*/src`; cargo-llvm-cov's default filename filter omits them from the native LCOV, and any that a report does contain are listed below.\n\n");
    for t in report["tooling"]["inventory"]
        .as_array()
        .into_iter()
        .flatten()
    {
        md.push_str(&format!(
            "- {} (crate source; native {} lines)\n",
            t["path"].as_str().unwrap_or(""),
            t["native"]["lines"]
        ));
    }
    for t in report["tooling"]["tests_examples_from_reports"]
        .as_array()
        .into_iter()
        .flatten()
    {
        md.push_str(&format!(
            "- {} ({}; {} lines)\n",
            t["path"].as_str().unwrap_or(""),
            t["target"].as_str().unwrap_or(""),
            t["coverage"]["lines"]
        ));
    }
    md.push_str("\n## Builds and tools\n\n");
    for b in &builds {
        md.push_str(&format!(
            "- WASM `{}`: application.wasm sha256 `{}`; lytronaut-client lines {}/{}\n",
            b["name"].as_str().unwrap_or(""),
            b["application_wasm_sha256"].as_str().unwrap_or(""),
            b["lytronaut_client"]["covered"],
            b["lytronaut_client"]["lines"]
        ));
    }
    for (key, value) in &meta {
        md.push_str(&format!("- {key}: {value}\n"));
    }

    fs::create_dir_all(&args.out)?;
    fs::write(
        args.out.join("coverage.json"),
        serde_json::to_string_pretty(&report)? + "\n",
    )?;
    fs::write(args.out.join("coverage.md"), md)?;
    println!(
        "native {} · wasm(client) {} · union {}",
        native_total.cell(),
        wasm_total.cell(),
        union_total.cell()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        for failure in &failures {
            eprintln!("coverage gate: {failure}");
        }
        Err(format!(
            "coverage gate failed ({} issue(s)); see {}",
            failures.len(),
            args.out.join("coverage.md").display()
        )
        .into())
    }
}
