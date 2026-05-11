use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default)]
pub struct DevLuaHotReloadStatus {
    pub enabled: bool,
    pub active_generation: Option<u64>,
    pub active_hash: Option<String>,
    pub pending_generation: Option<u64>,
    pub pending_hash: Option<String>,
    pub pending_apply_tick: Option<u64>,
    pub last_reload_tick: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDevLuaReload {
    pub generation: u64,
    pub hash: String,
    pub apply_tick: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevLuaHotReloadEvent {
    Candidate(omoba_template_ids::runtime_content::RuntimeContentInfo),
    Scheduled(PendingDevLuaReload),
    Failed(String),
}

pub struct DevLuaHotReload {
    root: PathBuf,
    last_scan: Instant,
    scan_interval: Duration,
    debounce: Duration,
    baseline: ContentFingerprint,
    pending_fingerprint: Option<ContentFingerprint>,
    validated_fingerprint: Option<ContentFingerprint>,
    pending_since: Option<Instant>,
    pending_apply: Option<PendingDevLuaReload>,
    status: DevLuaHotReloadStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContentFingerprint {
    files: Vec<FileFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint {
    rel_path: String,
    len: u64,
    modified_ns: u128,
}

const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

impl DevLuaHotReload {
    pub fn from_env() -> Option<Self> {
        if !omoba_template_ids::lua_hot_reload_enabled() {
            return None;
        }

        match Self::new(DEFAULT_SCAN_INTERVAL, DEFAULT_DEBOUNCE) {
            Ok(manager) => Some(manager),
            Err(err) => {
                log::warn!("[dev-lua-hot-reload] disabled: {}", err);
                None
            }
        }
    }

    fn new(scan_interval: Duration, debounce: Duration) -> Result<Self, String> {
        let info = omoba_template_ids::runtime_content::runtime_lua_content_info()?;
        let Some(info) = info else {
            return Err("runtime Lua content is not active".into());
        };
        let baseline = ContentFingerprint::scan(&info.root)?;
        let status = DevLuaHotReloadStatus {
            enabled: true,
            active_generation: Some(info.generation),
            active_hash: Some(info.hash),
            ..Default::default()
        };

        log::info!(
            "[dev-lua-hot-reload] enabled root={} generation={:?}",
            info.root.display(),
            status.active_generation
        );

        Ok(Self {
            root: info.root,
            last_scan: Instant::now(),
            scan_interval,
            debounce,
            baseline,
            pending_fingerprint: None,
            validated_fingerprint: None,
            pending_since: None,
            pending_apply: None,
            status,
        })
    }

    pub fn poll(&mut self, local_tick: u64) -> Option<DevLuaHotReloadEvent> {
        self.clear_applied_pending(local_tick);

        let now = Instant::now();
        if now.duration_since(self.last_scan) < self.scan_interval {
            return None;
        }
        self.last_scan = now;

        let fingerprint = match ContentFingerprint::scan(&self.root) {
            Ok(fingerprint) => fingerprint,
            Err(err) => {
                self.status.last_error = Some(err.clone());
                return Some(DevLuaHotReloadEvent::Failed(err));
            }
        };

        if fingerprint == self.baseline {
            self.pending_fingerprint = None;
            self.pending_since = None;
            return None;
        }

        if self.pending_fingerprint.as_ref() != Some(&fingerprint) {
            self.pending_fingerprint = Some(fingerprint);
            self.pending_since = Some(now);
            return None;
        }

        if self
            .pending_since
            .map(|since| now.duration_since(since) < self.debounce)
            .unwrap_or(true)
        {
            return None;
        }

        let fingerprint = self.pending_fingerprint.take().unwrap_or(fingerprint);
        self.pending_since = None;
        match omoba_template_ids::validate_runtime_lua_content_dev() {
            Ok(Some(info)) => {
                self.validated_fingerprint = Some(fingerprint);
                Some(DevLuaHotReloadEvent::Candidate(info))
            }
            Ok(None) => None,
            Err(err) => {
                self.baseline = fingerprint;
                self.status.last_error = Some(err.clone());
                Some(DevLuaHotReloadEvent::Failed(err))
            }
        }
    }

    pub fn status(&self) -> DevLuaHotReloadStatus {
        self.status.clone()
    }

    pub fn complete_reload(
        &mut self,
        info: omoba_template_ids::runtime_content::RuntimeContentInfo,
        local_tick: u64,
    ) -> PendingDevLuaReload {
        if let Some(fingerprint) = self.validated_fingerprint.take() {
            self.baseline = fingerprint;
        }
        self.status.active_generation = Some(info.generation);
        self.status.active_hash = Some(info.hash.clone());
        self.status.last_reload_tick = Some(local_tick);
        self.status.last_error = None;
        let pending = PendingDevLuaReload {
            generation: info.generation,
            hash: info.hash,
            apply_tick: local_tick.saturating_add(1),
        };
        self.status.pending_generation = Some(pending.generation);
        self.status.pending_hash = Some(pending.hash.clone());
        self.status.pending_apply_tick = Some(pending.apply_tick);
        self.pending_apply = Some(pending.clone());
        pending
    }

    pub fn fail_reload(&mut self, err: String) {
        if let Some(fingerprint) = self.validated_fingerprint.take() {
            self.baseline = fingerprint;
        }
        self.status.last_error = Some(err);
    }

    fn clear_applied_pending(&mut self, local_tick: u64) {
        if self
            .pending_apply
            .as_ref()
            .map(|pending| local_tick >= pending.apply_tick)
            .unwrap_or(false)
        {
            self.pending_apply = None;
            self.status.pending_generation = None;
            self.status.pending_hash = None;
            self.status.pending_apply_tick = None;
        }
    }

    #[cfg(test)]
    fn new_for_tests(
        root: PathBuf,
        scan_interval: Duration,
        debounce: Duration,
    ) -> Result<Self, String> {
        let baseline = ContentFingerprint::scan(&root)?;
        Ok(Self {
            root,
            last_scan: Instant::now() - scan_interval,
            scan_interval,
            debounce,
            baseline,
            pending_fingerprint: None,
            validated_fingerprint: None,
            pending_since: None,
            pending_apply: None,
            status: DevLuaHotReloadStatus {
                enabled: true,
                ..Default::default()
            },
        })
    }
}

impl ContentFingerprint {
    fn scan(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|err| format!("canonicalize Lua content root {}: {}", root.display(), err))?;
        let mut files = Vec::new();
        scan_dir(&root, &root, &mut files)?;
        files.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        Ok(Self { files })
    }
}

fn scan_dir(root: &Path, dir: &Path, out: &mut Vec<FileFingerprint>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {}", dir.display(), err))? {
        let entry = entry.map_err(|err| format!("read {} entry: {}", dir.display(), err))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|err| format!("metadata {}: {}", path.display(), err))?;
        if meta.is_dir() {
            scan_dir(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let rel_path = path
            .strip_prefix(root)
            .map_err(|err| format!("strip root {}: {}", path.display(), err))?
            .to_string_lossy()
            .replace('\\', "/");
        let modified_ns = meta.modified().ok().and_then(system_time_ns).unwrap_or(0);
        out.push(FileFingerprint {
            rel_path,
            len: meta.len(),
            modified_ns,
        });
    }
    Ok(())
}

fn system_time_ns(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("omoba_dev_lua_reload_{name}_{stamp}"))
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn from_env_requires_explicit_hot_reload_env() {
        let _guard = test_lock();
        std::env::remove_var("OMB_LUA_CONTENT");
        std::env::remove_var("OMB_LUA_CONTENT_ROOT");
        std::env::remove_var("OMB_LUA_HOT_RELOAD");
        assert!(DevLuaHotReload::from_env().is_none());
        std::env::set_var("OMB_LUA_CONTENT", "1");
        assert!(DevLuaHotReload::from_env().is_none());
        std::env::remove_var("OMB_LUA_CONTENT");
    }

    #[test]
    fn fingerprint_changes_when_file_changes() {
        let _guard = test_lock();
        let root = temp_root("fingerprint");
        write(&root.join("templates.lua"), "a");
        let first = ContentFingerprint::scan(&root).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        write(&root.join("templates.lua"), "aa");
        let second = ContentFingerprint::scan(&root).unwrap();
        assert_ne!(first, second);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn poll_waits_for_debounce() {
        let _guard = test_lock();
        let root = temp_root("debounce");
        write(&root.join("templates.lua"), "a");
        let mut manager =
            DevLuaHotReload::new_for_tests(root.clone(), Duration::ZERO, Duration::from_secs(60))
                .unwrap();
        write(&root.join("templates.lua"), "aa");
        assert!(manager.poll(1).is_none());
        assert!(manager.poll(2).is_none());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn poll_reports_scan_failure_without_panic() {
        let _guard = test_lock();
        let root = temp_root("scan_failure");
        write(&root.join("templates.lua"), "a");
        let mut manager =
            DevLuaHotReload::new_for_tests(root.clone(), Duration::ZERO, Duration::ZERO).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let event = manager.poll(1).unwrap();
        match event {
            DevLuaHotReloadEvent::Failed(err) => {
                assert!(err.contains("canonicalize Lua content root"), "{err}")
            }
            other => panic!("expected failure, got {other:?}"),
        }
        assert!(manager.status().last_error.is_some());
    }
}
