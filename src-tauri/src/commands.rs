use std::{path::{Path, PathBuf}, process::Command, sync::Arc};

use chrono::Local;
use std::{sync::mpsc, time::Duration};
use tauri::{LogicalPosition, LogicalSize, State, Webview, Wry};
use uuid::Uuid;
#[cfg(windows)]
use webview2_com::{CoTaskMemPWSTR, ExecuteScriptCompletedHandler};

use crate::{
    app_state::{AppState, BrowserPanelState},
    build_info,
    db,
    error::AppError,
    models::{AppBootstrap, ContentItem, DiagnosePageRequest, LoginStatus, PageDiagnosis, SyncEvent, SyncSession, SyncStartRequest, UserProfile},
    sync,
};

fn log_command(name: &str) {
    println!("{}", build_info::command_banner(name));
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

fn apply_browser_panel_layout(webview: &Webview<Wry>, left_width: f64) -> Result<(), AppError> {
    const TOP_TOOLBAR_HEIGHT: f64 = 88.0;
    let window = webview.window();
    let size = window
        .inner_size()
        .map_err(|error| AppError::Message(error.to_string()))?;
    let scale_factor = window
        .scale_factor()
        .map_err(|error| AppError::Message(error.to_string()))?;
    let logical_size = size.to_logical::<f64>(scale_factor);
    let left_width = left_width.max(320.0);
    let right_width = (logical_size.width - left_width).max(320.0);
    let right_height = (logical_size.height - TOP_TOOLBAR_HEIGHT).max(320.0);
    webview
        .set_position(LogicalPosition::new(left_width, TOP_TOOLBAR_HEIGHT))
        .map_err(|error| AppError::Message(error.to_string()))?;
    webview
        .set_size(LogicalSize::new(right_width, right_height))
        .map_err(|error| AppError::Message(error.to_string()))?;
    Ok(())
}

#[cfg(windows)]
fn detect_login_from_browser_panel(
    browser: &BrowserPanelState,
    source: &str,
) -> Result<Option<(LoginStatus, Option<UserProfile>)>, AppError> {
    let Some(webview) = browser.webview() else {
        return Ok(None);
    };

    let current_url = webview
        .url()
        .map_err(|error| AppError::Message(error.to_string()))?
        .to_string();
    if !current_url.contains("toutiao.com") {
        return Ok(Some((
            LoginStatus {
                logged_in: false,
                login_required: true,
                source: source.to_string(),
                message: "未登录：请先在右侧浏览器打开并登录今日头条，再点击“刷新”".into(),
                page_url: Some(current_url),
                page_title: None,
            },
            None,
        )));
    }

    let script = r#"
(() => {
  const text = (selector) => document.querySelector(selector)?.textContent?.replace(/\s+/g, " ").trim() || "";
  const bodyText = (document.body?.innerText || "").replace(/\s+/g, " ");
  const anchors = Array.from(document.querySelectorAll("a[href]")).map((anchor) => ({
    href: anchor.href || "",
    text: (anchor.textContent || anchor.getAttribute("title") || "").replace(/\s+/g, " ").trim(),
  }));
  const hasProfileEntry = anchors.some((item) => item.href.includes("/c/user/token/"));
  const hasSourceEntry = anchors.some((item) => (
    item.href.includes("/c/user/token/") &&
    (item.text.includes("我的收藏") || item.href.includes("tab=fav") || item.href.includes("source=mine_profile"))
  ));
  const visibleLoginButtons = Array.from(document.querySelectorAll("a, button, [role='button'], div, span"))
    .map((element) => {
      const text = (element.textContent || "").replace(/\s+/g, " ").trim();
      const rect = element.getBoundingClientRect();
      return { text, top: rect.top, width: rect.width, height: rect.height };
    })
    .filter((item) => item.top >= 0 && item.top < 260 && item.width > 0 && item.height > 0)
    .some((item) => ["登录", "立即登录"].includes(item.text) || item.text.includes("请登录"));
  const loginUrl = /login|passport|sso/i.test(location.href);
  const loginText = /请登录|立即登录|手机号登录|验证码登录|扫码登录/.test(bodyText);
  const loggedIn = !loginUrl && !visibleLoginButtons && !loginText && (hasSourceEntry || hasProfileEntry);
  const avatar =
    document.querySelector("img[src*='avatar']") ||
    Array.from(document.querySelectorAll("img")).find((img) => {
      const rect = img.getBoundingClientRect();
      return rect.width >= 80 && rect.height >= 80 && rect.top < 320;
    });
  const name =
    text("[class*='name']") ||
    Array.from(document.querySelectorAll("h1, h2, strong, span"))
      .map((node) => node.textContent?.trim() || "")
      .find((value) => value && value.length <= 12 && !/^\d/.test(value)) ||
    "";
  const readMetric = (label) => {
    const match = bodyText.match(new RegExp(`([\\d.万]+)\\s*${label}`));
    return match?.[1] || "";
  };
  const bioMatch = bodyText.match(/简介[:：]\s*([^\n\r]+)/);
  return {
    loggedIn,
    pageUrl: location.href,
    pageTitle: document.title || "",
    message: loggedIn ? "已登录今日头条，可以同步" : "未登录：请在右侧浏览器中登录今日头条，登录后点击“刷新”",
    profile: loggedIn ? {
      name,
      avatarUrl: avatar?.currentSrc || avatar?.getAttribute?.("src") || null,
      likes: readMetric("获赞"),
      followers: readMetric("粉丝"),
      following: readMetric("关注"),
      bio: bioMatch?.[1]?.replace(/\s*更多信息.*/, "").trim() || "",
      updatedAt: new Date().toISOString(),
    } : null,
  };
})()
"#;

    let (sender, receiver) = mpsc::channel::<Result<String, String>>();
    webview
        .with_webview(move |platform_webview| unsafe {
            let controller = platform_webview.controller();
            let core = match controller.CoreWebView2() {
                Ok(core) => core,
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                    return;
                }
            };
            let js = CoTaskMemPWSTR::from(script);
            let _ = core.ExecuteScript(
                *js.as_ref().as_pcwstr(),
                &ExecuteScriptCompletedHandler::create(Box::new(move |_, result| {
                    let _ = sender.send(Ok(result));
                    Ok(())
                })),
            );
        })
        .map_err(|error| AppError::Message(error.to_string()))?;

    let payload = receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| AppError::Message("右侧页面登录检测超时".into()))?
        .map_err(AppError::Message)?;

    let value: serde_json::Value =
        serde_json::from_str(&payload).map_err(|error| AppError::Message(error.to_string()))?;
    let page_url = value.get("pageUrl").and_then(|item| item.as_str()).map(|item| item.to_string());
    let page_title = value.get("pageTitle").and_then(|item| item.as_str()).map(|item| item.to_string());
    let logged_in = value.get("loggedIn").and_then(|item| item.as_bool()).unwrap_or(false);
    let message = value
        .get("message")
        .and_then(|item| item.as_str())
        .unwrap_or("登录状态未知")
        .to_string();
    let profile = value
        .get("profile")
        .cloned()
        .filter(|item| !item.is_null())
        .map(serde_json::from_value::<UserProfile>)
        .transpose()
        .map_err(|error| AppError::Message(error.to_string()))?;

    Ok(Some((
        LoginStatus {
            logged_in,
            login_required: !logged_in,
            source: source.to_string(),
            message,
            page_url,
            page_title,
        },
        profile,
    )))
}

#[cfg(not(windows))]
fn detect_login_from_browser_panel(
    _browser: &BrowserPanelState,
    _source: &str,
) -> Result<Option<(LoginStatus, Option<UserProfile>)>, AppError> {
    Ok(None)
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
) -> Result<Vec<ContentItem>, AppError> {
    log_command("search_items");
    let conn = db::connect(&state.db_path()?)?;
    db::search_items(&conn, &query, source.as_deref(), content_type.as_deref())
}

#[tauri::command]
pub fn get_user_profile(state: State<'_, AppState>) -> Result<Option<UserProfile>, AppError> {
    log_command("get_user_profile");
    let conn = db::connect(&state.db_path()?)?;
    db::get_user_profile(&conn)
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
    browser: State<'_, Arc<BrowserPanelState>>,
    request: DiagnosePageRequest,
) -> Result<LoginStatus, AppError> {
    log_command("check_toutiao_login");
    if let Some((login_status, profile)) = detect_login_from_browser_panel(browser.inner(), &request.source)? {
        if login_status.logged_in {
            if let Some(profile) = profile {
                let conn = db::connect(&state.db_path()?)?;
                db::upsert_user_profile(&conn, &profile)?;
            }
        }
        return Ok(login_status);
    }
    sync::check_login_status(state.inner(), &request.source)
}

#[tauri::command]
pub fn launch_debug_chrome(state: State<'_, AppState>) -> Result<(), AppError> {
    log_command("launch_debug_chrome");
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
        .arg("https://www.toutiao.com/")
        .spawn()?;
    Ok(())
}

#[tauri::command]
pub fn open_toutiao_in_panel(browser: State<'_, Arc<BrowserPanelState>>) -> Result<(), AppError> {
    log_command("open_toutiao_in_panel");
    let Some(webview) = browser.webview() else {
        return Err(AppError::Message("右侧内置浏览器未就绪".into()));
    };
    webview
        .eval("window.location.href = 'https://www.toutiao.com/';")
        .map_err(|error| AppError::Message(error.to_string()))?;
    webview
        .set_focus()
        .map_err(|error| AppError::Message(error.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn resize_toutiao_panel(
    browser: State<'_, Arc<BrowserPanelState>>,
    left_width: f64,
) -> Result<(), AppError> {
    log_command("resize_toutiao_panel");
    let Some(webview) = browser.webview() else {
        return Err(AppError::Message("右侧内置浏览器未就绪".into()));
    };
    browser.set_split_width(left_width);
    apply_browser_panel_layout(&webview, left_width)
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
