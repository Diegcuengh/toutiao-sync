use std::{
    collections::{HashMap, HashSet},
    env,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{Webview, Wry};

use crate::{db, error::AppError};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub download_dir: PathBuf,
    pub jobs_dir: PathBuf,
    pub chrome_user_data_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct AppConfig {
    data_root: String,
    #[serde(default = "default_download_threads")]
    download_threads: usize,
}

#[derive(Default)]
pub struct BrowserPanelState {
    webview: Mutex<Option<Webview<Wry>>>,
    split_width: Mutex<Option<f64>>,
}

impl BrowserPanelState {
    pub fn set_webview(&self, webview: Webview<Wry>) {
        if let Ok(mut guard) = self.webview.lock() {
            *guard = Some(webview);
        }
    }

    pub fn webview(&self) -> Option<Webview<Wry>> {
        self.webview.lock().ok().and_then(|guard| guard.clone())
    }

    pub fn set_split_width(&self, width: f64) {
        if let Ok(mut guard) = self.split_width.lock() {
            *guard = Some(width);
        }
    }

    pub fn split_width(&self) -> Option<f64> {
        self.split_width.lock().ok().and_then(|guard| *guard)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config_path: PathBuf,
    pub script_path: PathBuf,
    pub node_bin: PathBuf,
    pub chrome_cdp_port: u16,
    paths: Arc<Mutex<AppPaths>>,
    active_sessions: Arc<Mutex<HashSet<String>>>,
    active_processes: Arc<Mutex<HashMap<String, u32>>>,
}

impl AppState {
    pub fn new(config_dir: PathBuf, script_path: PathBuf) -> Result<Self, AppError> {
        fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("settings.json");
        let data_root = load_configured_root(&config_path)?.unwrap_or_else(default_data_root);
        let paths = build_paths(data_root);
        ensure_data_dirs(&paths)?;
        let conn = db::connect(&paths.db_path)?;
        let finished_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        db::reset_running_sessions(&conn, "程序已重新启动，旧同步任务已停止", &finished_at)?;
        drop(conn);

        Ok(Self {
            config_path,
            script_path,
            node_bin: resolve_node_bin()?,
            chrome_cdp_port: 19222,
            paths: Arc::new(Mutex::new(paths)),
            active_sessions: Arc::new(Mutex::new(HashSet::new())),
            active_processes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn current_paths(&self) -> Result<AppPaths, AppError> {
        self.paths
            .lock()
            .map(|value| value.clone())
            .map_err(|_| AppError::Message("path state lock poisoned".into()))
    }

    pub fn db_path(&self) -> Result<PathBuf, AppError> {
        Ok(self.current_paths()?.db_path)
    }

    pub fn data_dir(&self) -> Result<PathBuf, AppError> {
        Ok(self.current_paths()?.data_dir)
    }

    pub fn download_dir(&self) -> Result<PathBuf, AppError> {
        Ok(self.current_paths()?.download_dir)
    }

    pub fn download_threads(&self) -> Result<usize, AppError> {
        let config = load_config(&self.config_path)?;
        Ok(normalize_download_threads(config.download_threads))
    }

    pub fn jobs_dir(&self) -> Result<PathBuf, AppError> {
        Ok(self.current_paths()?.jobs_dir)
    }

    pub fn chrome_user_data_dir(&self) -> Result<PathBuf, AppError> {
        Ok(self.current_paths()?.chrome_user_data_dir)
    }

    pub fn active_session_ids(&self) -> Vec<String> {
        self.active_sessions
            .lock()
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn has_active_sessions(&self) -> Result<bool, AppError> {
        let guard = self
            .active_sessions
            .lock()
            .map_err(|_| AppError::Message("active session lock poisoned".into()))?;
        Ok(!guard.is_empty())
    }

    pub fn mark_running(&self, session_id: &str) -> Result<(), AppError> {
        let mut guard = self
            .active_sessions
            .lock()
            .map_err(|_| AppError::Message("active session lock poisoned".into()))?;
        guard.insert(session_id.to_string());
        Ok(())
    }

    pub fn mark_finished(&self, session_id: &str) {
        if let Ok(mut guard) = self.active_sessions.lock() {
            guard.remove(session_id);
        }
        if let Ok(mut guard) = self.active_processes.lock() {
            guard.remove(session_id);
        }
    }

    pub fn mark_process(&self, session_id: &str, process_id: u32) -> Result<(), AppError> {
        let mut guard = self
            .active_processes
            .lock()
            .map_err(|_| AppError::Message("active process lock poisoned".into()))?;
        guard.insert(session_id.to_string(), process_id);
        Ok(())
    }

    pub fn process_id(&self, session_id: &str) -> Option<u32> {
        self.active_processes
            .lock()
            .ok()
            .and_then(|items| items.get(session_id).copied())
    }

    pub fn set_data_root(&self, new_root: PathBuf, migrate: bool) -> Result<(), AppError> {
        if self.has_active_sessions()? {
            return Err(AppError::Message("存在正在运行的同步任务，不能切换目录。".into()));
        }

        let current = self.current_paths()?;
        let new_paths = build_paths(new_root);
        ensure_data_dirs(&new_paths)?;

        if migrate && current.data_dir != new_paths.data_dir {
            migrate_data(&current, &new_paths)?;
        }

        let conn = db::connect(&new_paths.db_path)?;
        drop(conn);
        let current_config = load_config(&self.config_path)?;
        save_config(&self.config_path, &new_paths.data_dir, normalize_download_threads(current_config.download_threads))?;

        let mut guard = self
            .paths
            .lock()
            .map_err(|_| AppError::Message("path state lock poisoned".into()))?;
        *guard = new_paths;
        Ok(())
    }

    pub fn ensure_parent(path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    pub fn set_download_threads(&self, value: usize) -> Result<usize, AppError> {
        let download_threads = normalize_download_threads(value);
        let paths = self.current_paths()?;
        save_config(&self.config_path, &paths.data_dir, download_threads)?;
        Ok(download_threads)
    }
}

fn default_download_threads() -> usize {
    2
}

fn normalize_download_threads(value: usize) -> usize {
    value.clamp(1, 8)
}

fn default_data_root() -> PathBuf {
    let drive_d = PathBuf::from(r"D:\");
    if drive_d.exists() {
        PathBuf::from(r"D:\toutiao-sync")
    } else {
        env::temp_dir().join("toutiao-sync")
    }
}

fn build_paths(data_dir: PathBuf) -> AppPaths {
    AppPaths {
        db_path: data_dir.join("app.db"),
        download_dir: data_dir.join("downloads"),
        jobs_dir: data_dir.join("jobs"),
        chrome_user_data_dir: data_dir.join("chrome-debug-profile"),
        data_dir,
    }
}

fn ensure_data_dirs(paths: &AppPaths) -> Result<(), AppError> {
    for directory in [
        &paths.data_dir,
        &paths.download_dir,
        &paths.jobs_dir,
        &paths.chrome_user_data_dir,
    ] {
        fs::create_dir_all(directory)?;
    }
    Ok(())
}

fn load_config(config_path: &Path) -> Result<AppConfig, AppError> {
    if !config_path.exists() {
        return Ok(AppConfig {
            data_root: default_data_root().display().to_string(),
            download_threads: default_download_threads(),
        });
    }
    let content = fs::read_to_string(config_path)?;
    let config: AppConfig = serde_json::from_str(&content)?;
    Ok(AppConfig {
        data_root: config.data_root,
        download_threads: normalize_download_threads(config.download_threads),
    })
}

fn load_configured_root(config_path: &Path) -> Result<Option<PathBuf>, AppError> {
    if !config_path.exists() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(load_config(config_path)?.data_root)))
}

fn save_config(config_path: &Path, data_root: &Path, download_threads: usize) -> Result<(), AppError> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let config = AppConfig {
        data_root: data_root.display().to_string(),
        download_threads: normalize_download_threads(download_threads),
    };
    fs::write(config_path, serde_json::to_vec_pretty(&config)?)?;
    Ok(())
}

fn migrate_data(current: &AppPaths, target: &AppPaths) -> Result<(), AppError> {
    if current.data_dir == target.data_dir {
        return Ok(());
    }

    move_if_exists(&current.db_path, &target.db_path)?;
    move_if_exists(&current.download_dir, &target.download_dir)?;
    move_if_exists(&current.jobs_dir, &target.jobs_dir)?;
    move_if_exists(&current.chrome_user_data_dir, &target.chrome_user_data_dir)?;
    Ok(())
}

fn move_if_exists(source: &Path, target: &Path) -> Result<(), AppError> {
    if !source.exists() || source == target {
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            if source.is_dir() {
                copy_dir_recursive(source, target)?;
                fs::remove_dir_all(source)?;
            } else {
                fs::copy(source, target)?;
                fs::remove_file(source)?;
            }
            Ok(())
        }
    }
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), AppError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if metadata.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &target_path)?;
        } else {
            return Err(AppError::Message(format!("不支持迁移的文件类型: {}", source_path.display())));
        }
    }
    Ok(())
}

fn resolve_node_bin() -> Result<PathBuf, AppError> {
    if let Ok(node_path) = env::var("NODE_BINARY") {
        let candidate = PathBuf::from(node_path);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    for name in ["node.exe", "node"] {
        if let Some(candidate) = find_in_path(name) {
            return Ok(candidate);
        }
    }

    Err(AppError::Message("未找到 Node.js。请先安装 Node.js，并确保 `node -v` 可用。".into()))
}

fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    let path_value = env::var_os("PATH")?;
    for directory in env::split_paths(&path_value) {
        let candidate = directory.join(binary_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
