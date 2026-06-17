#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_state;
mod build_info;
mod commands;
mod db;
mod debug_server;
mod error;
mod models;
mod sync;

use app_state::AppState;
use std::path::{Path, PathBuf};
use tauri::{path::BaseDirectory, Manager};

fn first_existing_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.exists()).cloned()
}

fn resolve_debug_script_path() -> std::io::Result<PathBuf> {
    let current_dir = std::env::current_dir()?;
    let exe_dir = std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(current_dir.clone());

    let candidates = vec![
        current_dir.join("scripts").join("toutiao_sync.js"),
        exe_dir.join("scripts").join("toutiao_sync.js"),
        exe_dir
            .parent()
            .and_then(|path| path.parent())
            .map(|path| path.join("scripts").join("toutiao_sync.js"))
            .unwrap_or_else(|| exe_dir.join("scripts").join("toutiao_sync.js")),
        exe_dir
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .map(|path| path.join("scripts").join("toutiao_sync.js"))
            .unwrap_or_else(|| exe_dir.join("scripts").join("toutiao_sync.js")),
        current_dir
            .join("sync-runtime")
            .join("scripts")
            .join("toutiao_sync.js"),
        exe_dir
            .join("sync-runtime")
            .join("scripts")
            .join("toutiao_sync.js"),
    ];

    first_existing_path(&candidates).ok_or_else(|| {
        let checked_paths = candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("未找到同步脚本，已检查: {checked_paths}"),
        )
    })
}

fn main() {
    if let Err(error) = debug_server::ensure_debug_frontend_server() {
        panic!("内置前端服务启动失败: {error}");
    }

    tauri::Builder::default()
        .setup(|app| {
            println!("{}", build_info::command_banner("app_start"));
            let resolver = app.path();
            let data_dir = resolver
                .app_data_dir()
                .unwrap_or(std::env::current_dir()?.join("data"));
            let script_path = if cfg!(debug_assertions) {
                resolve_debug_script_path()?
            } else {
                resolver.resolve("sync-runtime/scripts/toutiao_sync.js", BaseDirectory::Resource)?
            };
            let state = AppState::new(data_dir, script_path)?;
            app.manage(state);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(build_info::APP_TITLE);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap_app,
            commands::choose_data_directory,
            commands::migrate_data_directory,
            commands::set_download_threads,
            commands::launch_debug_chrome,
            commands::check_toutiao_login,
            commands::diagnose_page,
            commands::start_sync,
            commands::stop_sync,
            commands::list_sync_sessions,
            commands::list_sync_events,
            commands::search_items,
            commands::list_tags,
            commands::add_item_tag,
            commands::remove_item_tag,
            commands::get_user_profile,
            commands::open_download_dir,
            commands::open_item_dir,
            commands::open_item_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
