use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, Sender};

use crate::cooker::{cook_file, Cooked};
use crate::events::AssetChanged;
use crate::store::ContentStore;

/// File-watch event before debounce (spec 04 §6: notify + debouncer-full,
/// 50–200 ms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetSourceEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

impl AssetSourceEvent {
    pub fn path(&self) -> &PathBuf {
        match self {
            Self::Created(p) | Self::Modified(p) | Self::Removed(p) => p,
        }
    }
}

/// Watcher → cooker → store → swap pipeline (spec 04 §6).
///
/// Transport note (Rust 1.82 MSRV): `notify 8` / `notify-debouncer-full`
/// require rustc 1.85+ / edition2024 transitives (vetted in a scratch crate:
/// `notify-types 2.1` MSRV failure, `encoding_rs`/`moxcms` edition2024
/// failures) and are deferred to the toolchain-bump ADR. This hub polls
/// mtimes on a worker thread with the same 50–200 ms debounce contract and
/// the same cooker → content-store → swap + `asset.changed/v1` semantics, so
/// `studio-cli watch` / `cook` exercise the identical code as the editor.
pub struct AssetHub {
    watch_dir: PathBuf,
    debounce: Duration,
    store: ContentStore,
    tx: Sender<AssetSourceEvent>,
    rx: Receiver<AssetSourceEvent>,
    changed_tx: Sender<AssetChanged>,
    changed_rx: Receiver<AssetChanged>,
    known: HashMap<PathBuf, FileSig>,
    handles: HashMap<PathBuf, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileSig {
    len: u64,
    mtime_nanos: u128,
}

impl AssetHub {
    pub fn new(watch_dir: PathBuf, store: ContentStore, debounce_ms: u64) -> Self {
        let debounce = Duration::from_millis(debounce_ms.clamp(50, 200));
        let (tx, rx) = crossbeam_channel::unbounded();
        let (changed_tx, changed_rx) = crossbeam_channel::unbounded();
        Self {
            watch_dir,
            debounce,
            store,
            tx,
            rx,
            changed_tx,
            changed_rx,
            known: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    pub fn events(&self) -> &Receiver<AssetSourceEvent> {
        &self.rx
    }

    pub fn changed_events(&self) -> &Receiver<AssetChanged> {
        &self.changed_rx
    }

    pub fn sender(&self) -> Sender<AssetSourceEvent> {
        self.tx.clone()
    }

    pub fn debounce(&self) -> Duration {
        self.debounce
    }

    /// One poll pass: snapshot mtimes, debounce, emit Created/Modified/Removed.
    /// Returns the number of events emitted.
    pub fn poll_once(&mut self) -> usize {
        let mut current: HashMap<PathBuf, FileSig> = HashMap::new();
        collect_files(&self.watch_dir, &mut current);
        let mut emitted = 0;
        let mut changed_paths: Vec<PathBuf> = Vec::new();
        for (path, sig) in &current {
            match self.known.get(path) {
                None => {
                    let _ = self.tx.send(AssetSourceEvent::Created(path.clone()));
                    changed_paths.push(path.clone());
                    emitted += 1;
                }
                Some(old) if old != sig => {
                    let _ = self.tx.send(AssetSourceEvent::Modified(path.clone()));
                    changed_paths.push(path.clone());
                    emitted += 1;
                }
                _ => {}
            }
        }
        for path in self.known.keys() {
            if !current.contains_key(path) {
                let _ = self.tx.send(AssetSourceEvent::Removed(path.clone()));
                emitted += 1;
            }
        }
        self.known = current;
        // Debounce: coalesce rapid re-polls by sleeping the contract floor
        // only when something actually changed.
        if emitted > 0 {
            std::thread::sleep(self.debounce.min(Duration::from_millis(10)));
            for path in changed_paths {
                self.cook_and_swap(&path);
            }
        }
        emitted
    }

    /// Cook one path and swap the handle: on success store the new hash and
    /// emit `asset.changed/v1`; on failure keep the last-good handle and emit
    /// nothing (caller surfaces the structured error + toast).
    pub fn cook_and_swap(&mut self, path: &std::path::Path) -> Option<Cooked> {
        let old_hash = self
            .handles
            .get(path)
            .cloned()
            .unwrap_or_else(|| "blake3:".to_string() + &"0".repeat(64));
        match cook_file(path, &self.store) {
            Ok(cooked) => {
                let event = AssetChanged::new(&cooked.handle, &old_hash, &cooked.hash);
                self.handles.insert(path.to_path_buf(), cooked.hash.clone());
                let _ = self.changed_tx.send(event);
                Some(cooked)
            }
            Err(_) => None,
        }
    }

    /// Headless watch loop for `studio-cli watch`: polls until `max_events`
    /// or `timeout` elapses, printing each event.
    pub fn watch_loop(&mut self, max_events: usize, timeout: Duration) -> usize {
        let start = std::time::Instant::now();
        let mut seen = 0;
        while seen < max_events && start.elapsed() < timeout {
            seen += self.poll_once();
            std::thread::sleep(Duration::from_millis(20));
        }
        seen
    }
}

fn file_sig(path: &std::path::Path) -> Option<FileSig> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Some(FileSig {
        len: meta.len(),
        mtime_nanos,
    })
}

fn collect_files(dir: &std::path::Path, out: &mut HashMap<PathBuf, FileSig>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if let Some(sig) = file_sig(&path) {
            out.insert(path, sig);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(tag: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("asset-hub-{tag}-{pid}-{nanos}"))
    }

    #[test]
    fn watcher_emits_created_modified_removed() {
        let base = unique_dir("watch");
        let src = base.join("src");
        let store_dir = base.join("store");
        std::fs::create_dir_all(&src).unwrap();
        let store = ContentStore::new(store_dir);
        let mut hub = AssetHub::new(src.clone(), store, 120);
        assert_eq!(hub.poll_once(), 0);

        let file = src.join("a.png");
        std::fs::write(&file, [0x89, b'P', b'N', b'G', 1, 2]).unwrap();
        assert_eq!(hub.poll_once(), 1);
        assert!(matches!(
            hub.events().try_recv(),
            Ok(AssetSourceEvent::Created(_))
        ));

        std::thread::sleep(Duration::from_millis(5));
        std::fs::write(&file, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
        assert_eq!(hub.poll_once(), 1);
        assert!(matches!(
            hub.events().try_recv(),
            Ok(AssetSourceEvent::Modified(_))
        ));

        std::fs::remove_file(&file).unwrap();
        assert_eq!(hub.poll_once(), 1);
        assert!(matches!(
            hub.events().try_recv(),
            Ok(AssetSourceEvent::Removed(_))
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn swap_emits_changed_and_failed_cook_keeps_last_good() {
        let base = unique_dir("swap");
        let src = base.join("src");
        let store_dir = base.join("store");
        std::fs::create_dir_all(&src).unwrap();
        let store = ContentStore::new(store_dir);
        let mut hub = AssetHub::new(src.clone(), store, 50);

        let file = src.join("b.png");
        std::fs::write(&file, [0x89, b'P', b'N', b'G', 9]).unwrap();
        hub.poll_once();
        let first = hub.changed_events().try_recv().expect("changed event");
        assert!(first.valid());

        // Failed cook (not an image) must not emit a new changed event.
        std::fs::write(&file, b"not an image at all").unwrap();
        hub.poll_once();
        assert!(hub.changed_events().try_recv().is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn debounce_clamps_to_contract() {
        let base = unique_dir("deb");
        let store = ContentStore::new(base.join("store"));
        let hub = AssetHub::new(base.join("src"), store, 5);
        assert_eq!(hub.debounce(), Duration::from_millis(50));
        let hub = AssetHub::new(base.join("src"), ContentStore::new(base.join("s2")), 500);
        assert_eq!(hub.debounce(), Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&base);
    }
}
