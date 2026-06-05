use std::{
    collections::HashSet,
    env,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

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
}

#[derive(Clone)]
pub struct AppState {
    pub config_path: PathBuf,
    pub script_path: PathBuf,
    pub node_bin: PathBuf,
    pub chrome_cdp_port: u16,
    paths: Arc<Mutex<AppPaths>>,
    active_sessions: Arc<Mutex<HashSet<String>>>,
}

impl AppState {
    pub fn new(config_dir: PathBuf, script_path: PathBuf) -> Result<Self, AppError> {
        fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("settings.json");
        let data_root = load_configured_root(&config_path)?.unwrap_or_else(default_data_root);
        let paths = build_paths(data_root);
        ensure_data_dirs(&paths)?;
        let conn = db::connect(&paths.db_path)?;
        drop(conn);

        Ok(Self {
            config_path,
            script_path,
            node_bin: resolve_node_bin()?,
            chrome_cdp_port: 19222,
            paths: Arc::new(Mutex::new(paths)),
            active_sessions: Arc::new(Mutex::new(HashSet::new())),
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
        save_config(&self.config_path, &new_paths.data_dir)?;

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

fn load_configured_root(config_path: &Path) -> Result<Option<PathBuf>, AppError> {
    if !config_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(config_path)?;
    let config: AppConfig = serde_json::from_str(&content)?;
    Ok(Some(PathBuf::from(config.data_root)))
}

fn save_config(config_path: &Path, data_root: &Path) -> Result<(), AppError> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let config = AppConfig {
        data_root: data_root.display().to_string(),
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
