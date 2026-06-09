#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_state;
mod build_info;
mod commands;
mod db;
mod debug_server;
mod error;
mod models;
mod sync;

use app_state::{AppState, BrowserPanelState};
use std::{path::{Path, PathBuf}, sync::Arc};
use tauri::{
    path::BaseDirectory,
    webview::WebviewBuilder,
    LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, WindowEvent, Wry,
};

const TOP_TOOLBAR_HEIGHT: f64 = 88.0;

fn resize_browser_webview(browser: &Webview<Wry>, width: f64, height: f64, left_width: f64) {
    let left_width = left_width.max(320.0);
    let right_width = (width - left_width).max(320.0);
    let right_height = (height - TOP_TOOLBAR_HEIGHT).max(320.0);
    let _ = browser.set_position(LogicalPosition::new(left_width, TOP_TOOLBAR_HEIGHT));
    let _ = browser.set_size(LogicalSize::new(right_width, right_height));
}

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
            let browser_panel_state = Arc::new(BrowserPanelState::default());
            app.manage(browser_panel_state.clone());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(build_info::APP_TITLE);
                let size = window.inner_size()?;
                let logical_size = size.to_logical::<f64>(window.scale_factor()?);
                let left_width = (logical_size.width / 2.0).max(320.0);
                browser_panel_state.set_split_width(left_width);
                let browser_width = (logical_size.width - left_width).max(320.0);
                let browser = WebviewBuilder::new(
                    "toutiao_browser",
                    WebviewUrl::External("https://www.toutiao.com/".parse()?),
                );
                let browser = window.as_ref().window().add_child(
                    browser,
                    LogicalPosition::new(left_width, TOP_TOOLBAR_HEIGHT),
                    LogicalSize::new(
                        browser_width,
                        (logical_size.height - TOP_TOOLBAR_HEIGHT).max(320.0),
                    ),
                )?;
                browser_panel_state.set_webview(browser.clone());
                let window_for_resize = window.clone();
                let browser_for_resize = browser.clone();
                let panel_state_for_resize = browser_panel_state.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Resized(size) = event {
                        if let Ok(scale_factor) = window_for_resize.scale_factor() {
                            let logical_size = size.to_logical::<f64>(scale_factor);
                            let left_width = panel_state_for_resize
                                .split_width()
                                .unwrap_or(logical_size.width / 2.0);
                            resize_browser_webview(
                                &browser_for_resize,
                                logical_size.width,
                                logical_size.height,
                                left_width,
                            );
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap_app,
            commands::choose_data_directory,
            commands::migrate_data_directory,
            commands::launch_debug_chrome,
            commands::open_toutiao_in_panel,
            commands::resize_toutiao_panel,
            commands::check_toutiao_login,
            commands::diagnose_page,
            commands::start_sync,
            commands::stop_sync,
            commands::list_sync_sessions,
            commands::list_sync_events,
            commands::search_items,
            commands::get_user_profile,
            commands::open_download_dir,
            commands::open_item_dir,
            commands::open_item_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
