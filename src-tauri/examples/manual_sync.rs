use std::{
    env,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use chrono::Local;
use toutiao_sync_desktop_lib::{
    app_state::AppState,
    build_info,
    db,
    error::AppError,
    models::SyncSession,
    sync,
};
use uuid::Uuid;

fn main() -> Result<(), AppError> {
    let source = env::args().nth(1).unwrap_or_else(|| "favorites".to_string());
    let mode = env::args()
        .nth(2)
        .unwrap_or_else(|| "incremental".to_string());

    println!("{}", build_info::command_banner("manual_sync"));

    let workspace_dir = env::current_dir()?;
    let config_dir = default_config_dir()?;
    let script_path = resolve_script_path(&workspace_dir)?;
    let state = AppState::new(config_dir, script_path)?;

    let session = SyncSession {
        id: Uuid::new_v4().to_string(),
        source,
        status: "running".into(),
        mode,
        total_candidates: 0,
        total_skipped: 0,
        total_discovered: 0,
        total_saved: 0,
        total_downloaded: 0,
        message: "等待同步脚本连接当前 Chrome".into(),
        started_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        finished_at: None,
    };

    let conn = db::connect(&state.db_path()?)?;
    db::insert_session(&conn, &session)?;
    state.mark_running(&session.id)?;
    sync::spawn_sync(state.clone(), session.clone())?;

    let started = Instant::now();
    loop {
        thread::sleep(Duration::from_secs(2));
        let conn = db::connect(&state.db_path()?)?;
        let sessions = db::list_sessions(&conn)?;
        if let Some(current) = sessions.iter().find(|item| item.id == session.id) {
            println!(
                "[{}] {} | 候选 {} 跳过 {} 新增 {} 入库 {} 下载 {}",
                current.status,
                current.message,
                current.total_candidates,
                current.total_skipped,
                current.total_discovered,
                current.total_saved,
                current.total_downloaded
            );

            if current.status != "running" {
                let events = db::list_sync_events(&conn, Some(&session.id))?;
                println!("会话结束: {}", current.status);
                for event in events.iter().take(10).rev() {
                    println!("{} [{}] {}", event.created_at, event.level, event.message);
                }
                break;
            }
        }

        if started.elapsed() > Duration::from_secs(60 * 30) {
            return Err(AppError::Message("manual_sync 超时（30 分钟）".into()));
        }
    }

    Ok(())
}

fn default_config_dir() -> Result<PathBuf, AppError> {
    let local_app_data = env::var("LOCALAPPDATA")
        .map_err(|_| AppError::Message("未找到 LOCALAPPDATA".into()))?;
    Ok(PathBuf::from(local_app_data).join("com.alex.toutiao-sync"))
}

fn resolve_script_path(workspace_dir: &Path) -> Result<PathBuf, AppError> {
    let candidates = [
        workspace_dir.join("src-tauri").join("target").join("debug").join("sync-runtime").join("scripts").join("toutiao_sync.js"),
        workspace_dir.join("sync-runtime").join("scripts").join("toutiao_sync.js"),
        workspace_dir.join("scripts").join("toutiao_sync.js"),
    ];

    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| AppError::Message("未找到可用的同步脚本".into()))
}
