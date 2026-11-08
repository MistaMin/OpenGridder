//! VST3 and CLAP discovery for the whole system, AudioGridder style: bundles are loaded
//! in scan worker processes (`--scan-worker`), never in the server, so a plugin that crashes
//! or hangs while it is scanned only costs that worker. The bundle the worker was on when
//! it died, or that ran past the timeout, is blacklisted (the H9 deadman).
//!
//! VST3 bundles that ship a `moduleinfo.json` are listed from it without loading any code,
//! as the old OpenGridder server did. Results are cached by bundle modification time, so
//! only new or changed bundles are scanned on later starts. `wipe` (Rescan All) clears the
//! cache and the blacklist.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{c_char, c_int, CString};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::host::CatalogPlugin;

/// Print `SCAN_PROGRESS` lines for the server app's splash while bundles are scanned.
pub static PROGRESS: AtomicBool = AtomicBool::new(false);
/// Only the server's own startup scan loads bundles in workers (main.rs sets this). Any
/// other scan (sandbox children, tests, a catalog request) uses the cache and
/// moduleinfo.json only.
pub static WORKERS: AtomicBool = AtomicBool::new(false);

/// A bundle that has not reported back within this time is treated as hung.
const BUNDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// Workers are replaced after this many bundles: modules stay mapped (some plugins crash
/// in dlclose), so a long-lived worker would keep growing.
const BUNDLES_PER_WORKER: usize = 20;
const MAX_WORKERS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct AgCatalogItem {
    name: [u8; 128],
    company: [u8; 128],
    id: [u8; 768],
    format: [u8; 16],
    is_instrument: c_int,
}

unsafe extern "C" {
    fn ag_native_scan_bundle(path: *const c_char, out: *mut AgCatalogItem, cap: c_int) -> c_int;
}

fn support_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Application Support/OpenGridder")
}

fn cache_path() -> PathBuf {
    support_dir().join("server-bundle-cache.json")
}

fn blacklist_path() -> PathBuf {
    support_dir().join("server-bundle-blacklist.txt")
}

/// Rescan All: forget every cached result and give blacklisted bundles another chance.
pub fn wipe() {
    if cfg!(test) {
        return; // unit tests must not erase the user's catalog
    }
    let _ = std::fs::remove_file(cache_path());
    let _ = std::fs::remove_file(blacklist_path());
}

fn skipped(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.contains("audiogridder") || name.contains("opengridder")
}

/// Every .vst3 and .clap bundle in the system and user plug-in folders, including vendor
/// subfolders.
fn find_bundles() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3"), PathBuf::from("/Library/Audio/Plug-Ins/CLAP")];
    if let Some(home) = home {
        roots.push(home.join("Library/Audio/Plug-Ins/VST3"));
        roots.push(home.join("Library/Audio/Plug-Ins/CLAP"));
    }
    let mut found = Vec::new();
    for root in roots {
        walk(&root, 0, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("vst3") | Some("clap") => {
                if !skipped(&path) {
                    found.push(path);
                }
            }
            _ if depth < 3 => walk(&path, depth + 1, found),
            _ => {}
        }
    }
}

fn format_of(path: &Path) -> &'static str {
    if path.extension().and_then(|e| e.to_str()) == Some("clap") { "CLAP" } else { "VST3" }
}

/// Changes when the bundle is reinstalled or updated.
fn stamp(bundle: &Path) -> u64 {
    let mtime = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs());
    mtime(bundle.join("Contents/Info.plist"))
        .or_else(|| mtime(bundle.join("Contents/MacOS")))
        .or_else(|| mtime(bundle.to_path_buf()))
        .unwrap_or(0)
}

fn plugin(name: &str, company: &str, id: String, format: &str, instrument: bool) -> CatalogPlugin {
    CatalogPlugin {
        name: name.to_string(),
        company: company.to_string(),
        id,
        format: format.to_string(),
        is_instrument: instrument,
        category: if instrument { "Instrument".to_string() } else { "Effect".to_string() },
    }
}

/// VST 3.7 `moduleinfo.json`: the effect classes without loading the module. `None` when
/// the file is missing or unreadable (then the bundle is loaded in a worker).
fn from_moduleinfo(bundle: &Path) -> Option<Vec<CatalogPlugin>> {
    let text = std::fs::read_to_string(bundle.join("Contents/Resources/moduleinfo.json")).ok()?;
    let info: Value = serde_json::from_str(&text).or_else(|_| serde_json::from_str(&relaxed_json(&text))).ok()?;
    let factory_vendor = info.pointer("/Factory Info/Vendor").and_then(Value::as_str).unwrap_or("");
    let classes = info.get("Classes")?.as_array()?;
    let mut found = Vec::new();
    for class in classes {
        if class.get("Category").and_then(Value::as_str) != Some("Audio Module Class") {
            continue;
        }
        let (Some(cid), Some(name)) = (class.get("CID").and_then(Value::as_str), class.get("Name").and_then(Value::as_str)) else { continue };
        let vendor = class.get("Vendor").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(factory_vendor);
        let instrument = class
            .get("Sub Categories")
            .and_then(Value::as_array)
            .is_some_and(|subs| subs.iter().any(|s| s.as_str().is_some_and(|s| s.contains("Instrument"))));
        let id = format!("vst3:{}#{}", bundle.display(), cid.to_ascii_uppercase());
        found.push(plugin(name, vendor, id, "VST3", instrument));
    }
    Some(found)
}

/// moduleinfo.json files are JSON5-ish: drop // comments and trailing commas.
fn relaxed_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ',' => {
                let rest: String = chars.clone().take_while(|ch| ch.is_whitespace()).collect();
                let after = chars.clone().nth(rest.chars().count());
                if !matches!(after, Some(']') | Some('}')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn load_cache() -> HashMap<String, (u64, Vec<CatalogPlugin>)> {
    let mut cache = HashMap::new();
    let Ok(text) = std::fs::read_to_string(cache_path()) else { return cache };
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return cache };
    let Some(bundles) = value.get("bundles").and_then(Value::as_object) else { return cache };
    for (path, entry) in bundles {
        let stamp = entry.get("stamp").and_then(Value::as_u64).unwrap_or(0);
        let items = entry.get("items").and_then(Value::as_array).map(|items| items.iter().filter_map(item_from_json).collect()).unwrap_or_default();
        cache.insert(path.clone(), (stamp, items));
    }
    cache
}

fn save_cache(cache: &HashMap<String, (u64, Vec<CatalogPlugin>)>) {
    let bundles: serde_json::Map<String, Value> = cache
        .iter()
        .map(|(path, (stamp, items))| (path.clone(), json!({"stamp": stamp, "items": items.iter().map(item_to_json).collect::<Vec<_>>()})))
        .collect();
    let _ = std::fs::create_dir_all(support_dir());
    let tmp = cache_path().with_extension("json.tmp");
    if std::fs::write(&tmp, json!({"version": 1, "bundles": bundles}).to_string()).is_ok() {
        let _ = std::fs::rename(tmp, cache_path());
    }
}

fn item_to_json(item: &CatalogPlugin) -> Value {
    json!({"name": item.name, "company": item.company, "id": item.id, "format": item.format, "instrument": item.is_instrument})
}

fn item_from_json(value: &Value) -> Option<CatalogPlugin> {
    Some(plugin(
        value.get("name")?.as_str()?,
        value.get("company").and_then(Value::as_str).unwrap_or(""),
        value.get("id")?.as_str()?.to_string(),
        value.get("format").and_then(Value::as_str).unwrap_or("VST3"),
        value.get("instrument").and_then(Value::as_bool).unwrap_or(false),
    ))
}

fn load_blacklist() -> HashSet<String> {
    std::fs::read_to_string(blacklist_path())
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

fn add_to_blacklist(path: &str, reason: &str) {
    eprintln!("ag-server: scan {reason}, blacklisted: {path}");
    let _ = std::fs::create_dir_all(support_dir());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(blacklist_path()) {
        let _ = writeln!(file, "{path}");
    }
}

fn progress(current: usize, total: usize, name: &str, format: &str) {
    if PROGRESS.load(Ordering::Relaxed) {
        println!("SCAN_PROGRESS:{}", json!({"current": current, "total": total, "name": name, "format": format}));
        let _ = std::io::stdout().flush();
    }
}

/// All VST3 and CLAP plugins on the system, in bundle order.
pub fn scan() -> Vec<CatalogPlugin> {
    let bundles = find_bundles();
    let mut cache = load_cache();
    let blacklist = load_blacklist();
    let mut todo = Vec::new();
    let mut changed = false;
    for bundle in &bundles {
        let key = bundle.to_string_lossy().to_string();
        if blacklist.contains(&key) {
            continue;
        }
        let current = stamp(bundle);
        if cache.get(&key).is_some_and(|(cached, _)| *cached == current) {
            continue;
        }
        if format_of(bundle) == "VST3" {
            if let Some(items) = from_moduleinfo(bundle) {
                cache.insert(key, (current, items));
                changed = true;
                continue;
            }
        }
        todo.push(bundle.clone());
    }
    if !todo.is_empty() && (!WORKERS.load(Ordering::Relaxed) || !can_spawn_workers()) {
        // Not the server's startup scan (or a test binary without `--scan-worker`).
        todo.clear();
    }
    if !todo.is_empty() {
        eprintln!("ag-server: scanning {} plugin bundles in worker processes", todo.len());
        scan_in_workers(todo, &mut cache);
        changed = true;
    }
    // Forget bundles that were removed.
    let present: HashSet<String> = bundles.iter().map(|b| b.to_string_lossy().to_string()).collect();
    let before = cache.len();
    cache.retain(|path, _| present.contains(path));
    if changed || cache.len() != before {
        save_cache(&cache);
    }
    let blacklist = load_blacklist();
    let mut found = Vec::new();
    for bundle in &bundles {
        let key = bundle.to_string_lossy().to_string();
        if blacklist.contains(&key) {
            continue;
        }
        if let Some((_, items)) = cache.get(&key) {
            found.extend(items.iter().cloned());
        }
    }
    found
}

/// Workers are this executable started with `--scan-worker`, which only the server has.
fn can_spawn_workers() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().to_string()))
        .is_some_and(|name| name.starts_with("ag-server") || name.starts_with("OpenGridderServer"))
}

enum Outcome {
    Done(Vec<CatalogPlugin>),
    Failed,
}

fn scan_in_workers(todo: Vec<PathBuf>, cache: &mut HashMap<String, (u64, Vec<CatalogPlugin>)>) {
    let total = todo.len();
    let queue = Arc::new(Mutex::new(todo.into_iter().collect::<VecDeque<_>>()));
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, MAX_WORKERS).min(total);
    let (results, received) = mpsc::channel::<(PathBuf, Outcome)>();
    let mut threads = Vec::new();
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let results = results.clone();
        threads.push(std::thread::spawn(move || worker_thread(queue, results)));
    }
    drop(results);
    let mut done = 0;
    for (bundle, outcome) in received {
        done += 1;
        let name = bundle.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        progress(done, total, &name, format_of(&bundle));
        if let Outcome::Done(items) = outcome {
            cache.insert(bundle.to_string_lossy().to_string(), (stamp(&bundle), items));
            if done % 50 == 0 {
                save_cache(cache); // keep progress if the server is stopped mid-scan
            }
        }
    }
    for thread in threads {
        let _ = thread.join();
    }
}

struct Worker {
    child: Child,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<String>,
    scanned: usize,
}

fn spawn_worker() -> Option<Worker> {
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(exe).arg("--scan-worker").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Some(Worker { child, stdin, lines, scanned: 0 })
}

fn stop(mut worker: Worker) {
    drop(worker.stdin);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = worker.child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = worker.child.kill();
    let _ = worker.child.wait();
}

fn worker_thread(queue: Arc<Mutex<VecDeque<PathBuf>>>, results: mpsc::Sender<(PathBuf, Outcome)>) {
    let mut worker: Option<Worker> = None;
    loop {
        let Some(bundle) = queue.lock().ok().and_then(|mut q| q.pop_front()) else { break };
        if worker.is_none() {
            worker = spawn_worker();
        }
        let Some(mut current) = worker.take() else {
            let _ = results.send((bundle, Outcome::Failed));
            continue;
        };
        let path = bundle.to_string_lossy().to_string();
        let outcome = scan_one(&mut current, &path);
        match outcome {
            Some(items) => {
                current.scanned += 1;
                let _ = results.send((bundle, Outcome::Done(items)));
                if current.scanned >= BUNDLES_PER_WORKER {
                    stop(current);
                } else {
                    worker = Some(current);
                }
            }
            None => {
                let crashed = matches!(current.child.try_wait(), Ok(Some(_)));
                add_to_blacklist(&path, if crashed { "crashed the scanner" } else { "timed out" });
                let _ = current.child.kill();
                let _ = current.child.wait();
                let _ = results.send((bundle, Outcome::Failed));
            }
        }
    }
    if let Some(worker) = worker {
        stop(worker);
    }
}

/// Send one bundle to the worker and collect its entries. `None`: it crashed or hung.
fn scan_one(worker: &mut Worker, path: &str) -> Option<Vec<CatalogPlugin>> {
    if writeln!(worker.stdin, "{path}").and_then(|_| worker.stdin.flush()).is_err() {
        return None;
    }
    let mut items = Vec::new();
    loop {
        let line = worker.lines.recv_timeout(BUNDLE_TIMEOUT).ok()?;
        if let Some(json) = line.strip_prefix("ITEM\t") {
            if let Some(item) = serde_json::from_str::<Value>(json).ok().as_ref().and_then(item_from_json) {
                items.push(item);
            }
        } else if line.strip_prefix("END\t") == Some(path) {
            return Some(items);
        }
    }
}

unsafe extern "C" {
    fn dup(fd: c_int) -> c_int;
    fn dup2(from: c_int, to: c_int) -> c_int;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
}

/// `--scan-worker`: read bundle paths from stdin, report each one's entries on stdout.
/// Plugins print to stdout too, so the protocol gets a private copy of stdout and fd 1
/// goes to /dev/null.
pub fn worker_main() {
    use std::os::fd::FromRawFd;
    let report = unsafe {
        let private = dup(1);
        let null = CString::new("/dev/null").unwrap();
        let devnull = open(null.as_ptr(), 1 /* O_WRONLY */);
        if devnull >= 0 {
            dup2(devnull, 1);
        }
        std::fs::File::from_raw_fd(private)
    };
    let mut report = std::io::LineWriter::new(report);
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(path) = line else { break };
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        let _ = writeln!(report, "BEGIN\t{path}");
        for item in scan_bundle_here(&path) {
            let _ = writeln!(report, "ITEM\t{}", item_to_json(&item));
        }
        let _ = writeln!(report, "END\t{path}");
        let _ = report.flush();
    }
}

fn scan_bundle_here(path: &str) -> Vec<CatalogPlugin> {
    let Ok(c_path) = CString::new(path) else { return Vec::new() };
    let total = unsafe { ag_native_scan_bundle(c_path.as_ptr(), std::ptr::null_mut(), 0) }.max(0) as usize;
    if total == 0 {
        return Vec::new();
    }
    let mut items = vec![AgCatalogItem { name: [0; 128], company: [0; 128], id: [0; 768], format: [0; 16], is_instrument: 0 }; total];
    let count = unsafe { ag_native_scan_bundle(c_path.as_ptr(), items.as_mut_ptr(), items.len() as c_int) }.max(0) as usize;
    items.truncate(count);
    items
        .iter()
        .map(|item| plugin(&text(&item.name), &text(&item.company), text(&item.id), &text(&item.format), item.is_instrument != 0))
        .filter(|item| !item.id.is_empty())
        .collect()
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaxed_json_drops_comments_and_trailing_commas() {
        let text = "{\n // comment\n \"a\": [1, 2,],\n \"b\": \"x, // not a comment\",\n}";
        let value: Value = serde_json::from_str(&relaxed_json(text)).expect("parses");
        assert_eq!(value["a"], json!([1, 2]));
        assert_eq!(value["b"], "x, // not a comment");
    }

    #[test]
    fn moduleinfo_lists_audio_module_classes() {
        let dir = std::env::temp_dir().join(format!("ag-moduleinfo-{}", std::process::id()));
        let bundle = dir.join("Test.vst3");
        std::fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
        std::fs::write(
            bundle.join("Contents/Resources/moduleinfo.json"),
            r#"{"Factory Info": {"Vendor": "Acme"}, "Classes": [
                {"CID": "0123456789abcdef0123456789ABCDEF", "Category": "Audio Module Class", "Name": "Comp", "Sub Categories": ["Fx", "Dynamics"]},
                {"CID": "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF", "Category": "Component Controller Class", "Name": "Comp Controller"},
                {"CID": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "Category": "Audio Module Class", "Name": "Synth", "Vendor": "Other", "Sub Categories": ["Instrument"]},
            ]}"#,
        )
        .unwrap();
        let items = from_moduleinfo(&bundle).expect("read");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "Comp");
        assert_eq!(items[0].company, "Acme");
        assert!(items[0].id.ends_with("#0123456789ABCDEF0123456789ABCDEF"));
        assert!(!items[0].is_instrument);
        assert_eq!(items[1].company, "Other");
        assert!(items[1].is_instrument);
    }
}
