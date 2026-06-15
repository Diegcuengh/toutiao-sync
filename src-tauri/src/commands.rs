use std::{path::{Path, PathBuf}, process::Command, sync::Arc};

use chrono::Local;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use tauri::{LogicalPosition, LogicalSize, State, Webview, Wry};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    build_info,
    db,
    error::AppError,
    models::{AppBootstrap, DiagnosePageRequest, LoginStatus, PageDiagnosis, PagedContentItems, SyncEvent, SyncSession, SyncStartRequest, TagOption, UserProfile},
    sync,
};

fn log_command(name: &str) {
    println!("{}", build_info::command_banner(name));
}

fn is_debug_port_ready(port: u16) -> bool {
    let Ok(addr) = format!("127.0.0.1:{port}").parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn resolve_chrome_exe() -> Result<&'static str, AppError> {
    const CANDIDATES: [&str; 2] = [
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    ];
    for candidate in CANDIDATES {
        if Path::new(candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(AppError::Message("未找到 Chrome，请先安装 Chrome。".into()))
}

fn make_bootstrap(state: &AppState) -> Result<AppBootstrap, AppError> {
    let paths = state.current_paths()?;
    Ok(AppBootstrap {
        db_path: paths.db_path.display().to_string(),
        data_dir: paths.data_dir.display().to_string(),
        download_dir: paths.download_dir.display().to_string(),
        active_session_ids: state.active_session_ids(),
        version: build_info::VERSION.to_string(),
        build_date: build_info::BUILD_DATE.to_string(),
        build_time: build_info::BUILD_TIME.to_string(),
        build_label: build_info::BUILD_LABEL.to_string(),
        app_title: build_info::APP_TITLE.to_string(),
    })
}

fn choose_directory() -> Result<Option<PathBuf>, AppError> {
    let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '选择今日头条同步数据目录'
$dialog.ShowNewFolderButton = $true
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
  Write-Output $dialog.SelectedPath
}
"#;

    let output = Command::new("powershell.exe")
        .arg("-STA")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(script)
        .output()?;

    if !output.status.success() {
        return Err(AppError::Message(
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        ));
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(value)))
    }
}

#[tauri::command]
pub fn bootstrap_app(state: State<'_, AppState>) -> Result<AppBootstrap, AppError> {
    log_command("bootstrap_app");
    make_bootstrap(state.inner())
}

#[tauri::command]
pub fn choose_data_directory(state: State<'_, AppState>) -> Result<Option<AppBootstrap>, AppError> {
    log_command("choose_data_directory");
    let Some(path) = choose_directory()? else {
        return Ok(None);
    };
    state.set_data_root(path, false)?;
    Ok(Some(make_bootstrap(state.inner())?))
}

#[tauri::command]
pub fn migrate_data_directory(state: State<'_, AppState>) -> Result<Option<AppBootstrap>, AppError> {
    log_command("migrate_data_directory");
    let Some(path) = choose_directory()? else {
        return Ok(None);
    };
    state.set_data_root(path, true)?;
    Ok(Some(make_bootstrap(state.inner())?))
}

#[tauri::command]
pub fn list_sync_sessions(state: State<'_, AppState>) -> Result<Vec<SyncSession>, AppError> {
    log_command("list_sync_sessions");
    let conn = db::connect(&state.db_path()?)?;
    db::list_sessions(&conn)
}

#[tauri::command]
pub fn list_sync_events(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<Vec<SyncEvent>, AppError> {
    log_command("list_sync_events");
    let conn = db::connect(&state.db_path()?)?;
    db::list_sync_events(&conn, session_id.as_deref())
}

#[tauri::command]
pub fn search_items(
    state: State<'_, AppState>,
    query: String,
    source: Option<String>,
    content_type: Option<String>,
    tag_filters: Option<Vec<String>>,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Result<PagedContentItems, AppError> {
    log_command("search_items");
    let conn = db::connect(&state.db_path()?)?;
    db::search_items_page(
        &conn,
        &query,
        source.as_deref(),
        content_type.as_deref(),
        tag_filters.as_deref().unwrap_or(&[]),
        page.unwrap_or(1),
        page_size.unwrap_or(50),
    )
}

#[tauri::command]
pub fn get_user_profile(state: State<'_, AppState>) -> Result<Option<UserProfile>, AppError> {
    log_command("get_user_profile");
    let conn = db::connect(&state.db_path()?)?;
    db::get_user_profile(&conn)
}

#[tauri::command]
pub fn list_tags(state: State<'_, AppState>, source: Option<String>) -> Result<Vec<TagOption>, AppError> {
    log_command("list_tags");
    let conn = db::connect(&state.db_path()?)?;
    db::list_tag_options(&conn, source.as_deref())
}

#[tauri::command]
pub fn add_item_tag(state: State<'_, AppState>, item_id: i64, tag: String) -> Result<Vec<String>, AppError> {
    log_command("add_item_tag");
    let conn = db::connect(&state.db_path()?)?;
    db::add_item_tag(&conn, item_id, &tag)
}

#[tauri::command]
pub fn remove_item_tag(state: State<'_, AppState>, item_id: i64, tag: String) -> Result<Vec<String>, AppError> {
    log_command("remove_item_tag");
    let conn = db::connect(&state.db_path()?)?;
    db::remove_item_tag(&conn, item_id, &tag)
}

#[tauri::command]
pub fn diagnose_page(
    state: State<'_, AppState>,
    request: DiagnosePageRequest,
) -> Result<PageDiagnosis, AppError> {
    log_command("diagnose_page");
    sync::diagnose_page(state.inner(), &request.source)
}

#[tauri::command]
pub fn check_toutiao_login(
    state: State<'_, AppState>,
    request: DiagnosePageRequest,
) -> Result<LoginStatus, AppError> {
    log_command("check_toutiao_login");
    sync::check_login_status(state.inner(), &request.source)
}

#[tauri::command]
pub fn launch_debug_chrome(state: State<'_, AppState>) -> Result<(), AppError> {
    log_command("launch_debug_chrome");
    if is_debug_port_ready(state.chrome_cdp_port) {
        println!("[chrome] reuse debug port {}", state.chrome_cdp_port);
        return Ok(());
    }
    let chrome_exe = resolve_chrome_exe()?;
    let chrome_user_data_dir = state.chrome_user_data_dir()?;
    Command::new(chrome_exe)
        .arg(format!("--remote-debugging-port={}", state.chrome_cdp_port))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--remote-allow-origins=*")
        .arg(format!(
            "--user-data-dir={}",
            chrome_user_data_dir.display()
        ))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .spawn()?;
    Ok(())
}

#[tauri::command]
pub fn start_sync(state: State<'_, AppState>, request: SyncStartRequest) -> Result<SyncSession, AppError> {
    log_command("start_sync");
    let effective_mode = if request.mode == "verify" {
        let limit = request.max_items.unwrap_or(3).max(1);
        format!("verify:{limit}")
    } else {
        request.mode.clone()
    };
    let initial_message = match request.mode.as_str() {
        "verify" => format!(
            "等待同步脚本连接当前 Chrome（验证模式，最多抓取 {} 条）",
            request.max_items.unwrap_or(3).max(1)
        ),
        "list" => "等待同步脚本连接当前 Chrome（仅同步列表）".to_string(),
        "download" => "等待同步脚本连接当前 Chrome（下载未下载内容）".to_string(),
        _ => "等待同步脚本连接当前 Chrome".to_string(),
    };
    let session = SyncSession {
        id: Uuid::new_v4().to_string(),
        source: request.source,
        status: "running".into(),
        mode: effective_mode,
        total_candidates: 0,
        total_skipped: 0,
        total_discovered: 0,
        total_saved: 0,
        total_downloaded: 0,
        message: initial_message,
        started_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        finished_at: None,
    };

    let conn = db::connect(&state.db_path()?)?;
    db::insert_session(&conn, &session)?;
    state.mark_running(&session.id)?;
    sync::spawn_sync(state.inner().clone(), session.clone())?;
    Ok(session)
}

#[tauri::command]
pub fn stop_sync(state: State<'_, AppState>, session_id: String) -> Result<(), AppError> {
    log_command("stop_sync");
    if let Some(process_id) = state.process_id(&session_id) {
        let status = Command::new("taskkill.exe")
            .arg("/PID")
            .arg(process_id.to_string())
            .arg("/T")
            .arg("/F")
            .status()?;
        if !status.success() {
            return Err(AppError::Message(format!(
                "停止同步进程失败: PID {}，退出码 {:?}",
                process_id,
                status.code()
            )));
        }
    }

    let conn = db::connect(&state.db_path()?)?;
    let Some(session) = db::list_sessions(&conn)?
        .into_iter()
        .find(|item| item.id == session_id) else {
        state.mark_finished(&session_id);
        return Ok(());
    };
    let finished_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    db::update_session_progress(
        &conn,
        &session_id,
        session.total_candidates,
        session.total_skipped,
        session.total_discovered,
        session.total_saved,
        session.total_downloaded,
        "stopped",
        "用户已停止同步",
        Some(&finished_at),
    )?;
    db::insert_sync_event(&conn, &session_id, "info", "用户已停止同步")?;
    state.mark_finished(&session_id);
    Ok(())
}

#[tauri::command]
pub fn open_download_dir(state: State<'_, AppState>) -> Result<(), AppError> {
    log_command("open_download_dir");
    Command::new("explorer")
        .arg(state.download_dir()?)
        .spawn()?;
    Ok(())
}

#[tauri::command]
pub fn open_item_dir(state: State<'_, AppState>, item_id: i64) -> Result<(), AppError> {
    log_command("open_item_dir");
    let conn = db::connect(&state.db_path()?)?;
    let Some(local_dir) = db::get_item_dir(&conn, item_id)? else {
        return Err(AppError::Message("该内容没有本地目录".into()));
    };
    Command::new("explorer").arg(local_dir).spawn()?;
    Ok(())
}

#[tauri::command]
pub fn open_item_file(
    state: State<'_, AppState>,
    item_id: i64,
    kind: String,
) -> Result<(), AppError> {
    log_command("open_item_file");
    let conn = db::connect(&state.db_path()?)?;
    let Some(file_path) = db::get_item_file(&conn, item_id, &kind)? else {
        return Err(AppError::Message(format!("该内容没有可打开的{}文件", kind)));
    };
    Command::new("explorer").arg(file_path).spawn()?;
    Ok(())
}
