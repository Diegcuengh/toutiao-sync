use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use chrono::Local;

use crate::{
    app_state::AppState,
    db,
    error::AppError,
    models::{LoginStatus, PageDiagnosis, ScriptEvent, SyncSession},
};

#[derive(serde::Serialize)]
struct ScriptJob<'a> {
    session_id: &'a str,
    source: &'a str,
    mode: &'a str,
    max_items: Option<i64>,
    data_dir: String,
    download_dir: String,
    chrome_user_data_dir: String,
    cdp_port: u16,
    known_remote_ids: Vec<String>,
    download_items: Vec<serde_json::Value>,
}

pub fn spawn_sync(state: AppState, session: SyncSession) -> Result<(), AppError> {
    let jobs_dir = state.jobs_dir()?;
    let db_path = state.db_path()?;
    let data_dir = state.data_dir()?;
    let download_dir = state.download_dir()?;
    let chrome_user_data_dir = state.chrome_user_data_dir()?;

    let job_path = jobs_dir.join(format!("{}.json", session.id));
    AppState::ensure_parent(&job_path)?;
    let (known_remote_ids, download_items) = {
        let conn = db::connect(&db_path)?;
        if session.mode == "download" {
            (
                db::list_remote_ids_requiring_download(&conn, &session.source)?,
                db::list_items_requiring_download(&conn, &session.source)?,
            )
        } else {
            (db::list_known_remote_ids(&conn, &session.source)?, Vec::new())
        }
    };
    let job = ScriptJob {
        session_id: &session.id,
        source: &session.source,
        mode: &session.mode,
        max_items: session.mode.strip_prefix("verify:").and_then(|value| value.parse::<i64>().ok()),
        data_dir: data_dir.display().to_string(),
        download_dir: download_dir.display().to_string(),
        chrome_user_data_dir: chrome_user_data_dir.display().to_string(),
        cdp_port: state.chrome_cdp_port,
        known_remote_ids,
        download_items,
    };
    fs::write(&job_path, serde_json::to_vec_pretty(&job)?)?;

    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_sync(state.clone(), session.clone(), job_path) {
            let finished_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            if let Ok(db_path) = state.db_path() {
                if let Ok(conn) = db::connect(&db_path) {
                    if db::list_sessions(&conn)
                        .map(|items| items.iter().any(|item| item.id == session.id && item.status == "stopped"))
                        .unwrap_or(false)
                    {
                        state.mark_finished(&session.id);
                        return;
                    }
                    let _ = db::update_session_progress(
                        &conn,
                        &session.id,
                        session.total_candidates,
                        session.total_skipped,
                        session.total_discovered,
                        session.total_saved,
                        session.total_downloaded,
                        "failed",
                        &error.to_string(),
                        Some(&finished_at),
                    );
                    let _ = db::insert_sync_event(&conn, &session.id, "error", &error.to_string());
                }
            }
            state.mark_finished(&session.id);
        }
    });
    Ok(())
}

pub fn diagnose_page(state: &AppState, source: &str) -> Result<PageDiagnosis, AppError> {
    let jobs_dir = state.jobs_dir()?;
    let data_dir = state.data_dir()?;
    let download_dir = state.download_dir()?;
    let chrome_user_data_dir = state.chrome_user_data_dir()?;

    let job_path = jobs_dir.join(format!("diagnose-{}.json", source));
    AppState::ensure_parent(&job_path)?;
    let job = ScriptJob {
        session_id: "diagnose",
        source,
        mode: "diagnose",
        max_items: None,
        data_dir: data_dir.display().to_string(),
        download_dir: download_dir.display().to_string(),
        chrome_user_data_dir: chrome_user_data_dir.display().to_string(),
        cdp_port: state.chrome_cdp_port,
        known_remote_ids: Vec::new(),
        download_items: Vec::new(),
    };
    fs::write(&job_path, serde_json::to_vec_pretty(&job)?)?;

    let result = run_diagnose_script(state, source, &job_path);
    let _ = fs::remove_file(&job_path);
    result
}

pub fn check_login_status(state: &AppState, source: &str) -> Result<LoginStatus, AppError> {
    let jobs_dir = state.jobs_dir()?;
    let data_dir = state.data_dir()?;
    let download_dir = state.download_dir()?;
    let chrome_user_data_dir = state.chrome_user_data_dir()?;

    let job_path = jobs_dir.join(format!("login-status-{}.json", source));
    AppState::ensure_parent(&job_path)?;
    let job = ScriptJob {
        session_id: "login-status",
        source,
        mode: "login-status",
        max_items: None,
        data_dir: data_dir.display().to_string(),
        download_dir: download_dir.display().to_string(),
        chrome_user_data_dir: chrome_user_data_dir.display().to_string(),
        cdp_port: state.chrome_cdp_port,
        known_remote_ids: Vec::new(),
        download_items: Vec::new(),
    };
    fs::write(&job_path, serde_json::to_vec_pretty(&job)?)?;

    let result = run_login_status_script(state, source, &job_path);
    let _ = fs::remove_file(&job_path);
    result
}

fn run_sync(state: AppState, session: SyncSession, job_path: PathBuf) -> Result<(), AppError> {
    let script_path = state.script_path.clone();
    if !script_path.exists() {
        return Err(AppError::Message(format!(
            "同步脚本不存在: {}",
            script_path.display()
        )));
    }

    let working_dir = resolve_script_working_dir(&script_path)?;
    let mut child = Command::new(&state.node_bin)
        .current_dir(working_dir)
        .arg(&script_path)
        .arg("--job")
        .arg(job_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    state.mark_process(&session.id, child.id())?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Message("无法读取同步脚本输出".into()))?;

    let reader = BufReader::new(stdout);

    let mut candidates = 0_i64;
    let mut skipped = 0_i64;
    let mut discovered = 0_i64;
    let mut saved = 0_i64;
    let mut downloaded = 0_i64;

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        let conn = db::connect(&state.db_path()?)?;
        let event: ScriptEvent = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(error) => {
                let message = format!("脚本输出解析失败，已跳过当前输出行并继续：{error}");
                println!("[sync] {}", message);
                db::insert_sync_event(&conn, &session.id, "error", &message)?;
                continue;
            }
        };

        match event {
            ScriptEvent::Progress {
                message,
                candidates: candidates_value,
                skipped: skipped_value,
                discovered: discovered_value,
                saved: saved_value,
                downloaded: downloaded_value,
                ..
            } => {
                if let Some(value) = candidates_value {
                    candidates = value;
                }
                if let Some(value) = skipped_value {
                    skipped = value;
                }
                if let Some(value) = discovered_value {
                    discovered = value;
                }
                if let Some(value) = saved_value {
                    saved = value;
                }
                if let Some(value) = downloaded_value {
                    downloaded = value;
                }
                println!("[sync] {}", message);
                db::update_session_progress(
                    &conn,
                    &session.id,
                    candidates,
                    skipped,
                    discovered,
                    saved,
                    downloaded,
                    "running",
                    &message,
                    None,
                )?;
                db::insert_sync_event(&conn, &session.id, "info", &message)?;
            }
            ScriptEvent::Item { item } => {
                let synced_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                //println!("[item] {} {}", item.title, item.source_url);
                db::upsert_item(&conn, &session.source, &item, &synced_at)?;
                saved += 1;
                if item.downloaded {
                    downloaded += 1;
                }
                let message = format!("已入库: {}", item.title);
                db::update_session_progress(
                    &conn,
                    &session.id,
                    candidates,
                    skipped,
                    discovered,
                    saved,
                    downloaded,
                    "running",
                    &message,
                    None,
                )?;
                db::insert_sync_event(&conn, &session.id, "info", &message)?;
            }
            ScriptEvent::Profile { profile } => {
                println!("[profile] {}", profile.name);
                db::upsert_user_profile(&conn, &profile)?;
                db::insert_sync_event(&conn, &session.id, "info", "已更新用户资料")?;
            }
            ScriptEvent::ItemError { source_url, message } => {
                let full_message = format!("抓取失败: {} ({})", message, source_url);
                db::update_session_progress(
                    &conn,
                    &session.id,
                    candidates,
                    skipped,
                    discovered,
                    saved,
                    downloaded,
                    "running",
                    &full_message,
                    None,
                )?;
                db::insert_sync_event(&conn, &session.id, "error", &full_message)?;
            }
            ScriptEvent::Done { summary } => {
                candidates = summary.candidates;
                skipped = summary.skipped;
                discovered = summary.discovered;
                saved = summary.saved;
                downloaded = summary.downloaded;
                let finished_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let completion_message = format!(
                    "同步完成：候选 {}，跳过 {}，新增 {}，入库 {}，下载 {}",
                    candidates, skipped, discovered, saved, downloaded
                );
                db::update_session_progress(
                    &conn,
                    &session.id,
                    candidates,
                    skipped,
                    discovered,
                    saved,
                    downloaded,
                    "success",
                    &completion_message,
                    Some(&finished_at),
                )?;
                db::insert_sync_event(&conn, &session.id, "info", &completion_message)?;
            }
            ScriptEvent::Error { message, .. } => {
                let finished_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                db::update_session_progress(
                    &conn,
                    &session.id,
                    candidates,
                    skipped,
                    discovered,
                    saved,
                    downloaded,
                    "failed",
                    &message,
                    Some(&finished_at),
                )?;
                db::insert_sync_event(&conn, &session.id, "error", &message)?;
                state.mark_finished(&session.id);
                return Err(AppError::Message(message));
            }
        }
    }

    let status = child.wait()?;
    state.mark_finished(&session.id);
    if !status.success() {
        let conn = db::connect(&state.db_path()?)?;
        let sessions = db::list_sessions(&conn)?;
        if sessions
            .iter()
            .any(|item| item.id == session.id && item.status == "stopped")
        {
            return Ok(());
        }
        return Err(AppError::Message(format!(
            "同步脚本退出码异常: {:?}",
            status.code()
        )));
    }
    Ok(())
}

fn run_login_status_script(state: &AppState, source: &str, job_path: &Path) -> Result<LoginStatus, AppError> {
    let script_path = state.script_path.clone();
    if !script_path.exists() {
        return Err(AppError::Message(format!(
            "同步脚本不存在: {}",
            script_path.display()
        )));
    }

    let working_dir = resolve_script_working_dir(&script_path)?;
    let mut child = Command::new(&state.node_bin)
        .current_dir(working_dir)
        .arg(&script_path)
        .arg("--login-status")
        .arg("--job")
        .arg(job_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Message("无法读取登录检查脚本输出".into()))?;

    let reader = BufReader::new(stdout);
    let mut login_status = None;

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line)?;
        match value.get("type").and_then(|item| item.as_str()) {
            Some("login_status") => {
                login_status = Some(serde_json::from_value::<LoginStatus>(value)?);
            }
            Some("profile") => {
                if let Some(profile_value) = value.get("profile") {
                    let profile = serde_json::from_value(profile_value.clone())?;
                    let conn = db::connect(&state.db_path()?)?;
                    db::upsert_user_profile(&conn, &profile)?;
                }
            }
            _ => {}
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(AppError::Message(format!(
            "登录检查脚本退出码异常: {:?}",
            status.code()
        )));
    }

    login_status.ok_or_else(|| {
        AppError::Message(format!(
            "未读取到登录状态，请确认 Chrome 调试端口可用并打开今日头条。来源: {source}"
        ))
    })
}

fn run_diagnose_script(state: &AppState, source: &str, job_path: &Path) -> Result<PageDiagnosis, AppError> {
    let script_path = state.script_path.clone();
    if !script_path.exists() {
        return Err(AppError::Message(format!(
            "同步脚本不存在: {}",
            script_path.display()
        )));
    }

    let working_dir = resolve_script_working_dir(&script_path)?;
    let mut child = Command::new(&state.node_bin)
        .current_dir(working_dir)
        .arg(&script_path)
        .arg("--job")
        .arg(job_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Message("无法读取同步脚本输出".into()))?;

    let reader = BufReader::new(stdout);
    let mut logs = Vec::new();
    let mut page_url = None;
    let mut page_title = None;
    let mut ok = true;
    let mut message = String::from("页面诊断完成");

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }
        let event: ScriptEvent = serde_json::from_str(&line)?;
        match event {
            ScriptEvent::Progress {
                message: progress_message,
                page_url: progress_page_url,
                page_title: progress_page_title,
                ..
            } => {
                logs.push(progress_message.clone());
                if page_url.is_none() {
                    page_url = progress_page_url;
                }
                if page_title.is_none() {
                    page_title = progress_page_title;
                }
                message = progress_message;
            }
            ScriptEvent::Done { .. } => {
                if logs.is_empty() {
                    logs.push(String::from("页面诊断完成"));
                }
                message = String::from("页面诊断完成，可以开始同步");
            }
            ScriptEvent::Error { message: error_message, .. } => {
                ok = false;
                logs.push(error_message.clone());
                message = error_message;
            }
            ScriptEvent::Item { .. } | ScriptEvent::ItemError { .. } | ScriptEvent::Profile { .. } => {}
        }
    }

    let status = child.wait()?;
    if !status.success() && ok {
        ok = false;
        message = format!("诊断脚本退出码异常: {:?}", status.code());
        logs.push(message.clone());
    }

    Ok(PageDiagnosis {
        ok,
        source: source.to_string(),
        message,
        page_url,
        page_title,
        logs,
    })
}

fn resolve_script_working_dir(script_path: &Path) -> Result<PathBuf, AppError> {
    let Some(script_dir) = script_path.parent() else {
        return Err(AppError::Message(format!(
            "无法解析同步脚本目录: {}",
            script_path.display()
        )));
    };
    let Some(work_dir) = script_dir.parent() else {
        return Err(AppError::Message(format!(
            "无法解析同步脚本工作目录: {}",
            script_path.display()
        )));
    };
    Ok(work_dir.to_path_buf())
}
