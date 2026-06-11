use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppBootstrap {
    pub db_path: String,
    pub data_dir: String,
    pub download_dir: String,
    pub active_session_ids: Vec<String>,
    pub version: String,
    pub build_date: String,
    pub build_time: String,
    pub build_label: String,
    pub app_title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStartRequest {
    pub source: String,
    pub mode: String,
    pub max_items: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosePageRequest {
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDiagnosis {
    pub ok: bool,
    pub source: String,
    pub message: String,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStatus {
    pub logged_in: bool,
    pub login_required: bool,
    pub source: String,
    pub message: String,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub name: String,
    pub avatar_url: Option<String>,
    pub likes: String,
    pub followers: String,
    pub following: String,
    pub bio: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSession {
    pub id: String,
    pub source: String,
    pub status: String,
    pub mode: String,
    pub total_candidates: i64,
    pub total_skipped: i64,
    pub total_discovered: i64,
    pub total_saved: i64,
    pub total_downloaded: i64,
    pub message: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    pub id: i64,
    pub session_id: String,
    pub level: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentItem {
    pub id: i64,
    pub remote_id: String,
    pub source: String,
    pub title: String,
    pub summary: String,
    pub content_text: String,
    pub author: String,
    pub content_type: String,
    pub source_url: String,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub article_path: Option<String>,
    pub video_path: Option<String>,
    pub local_dir: Option<String>,
    pub synced_at: String,
    pub downloaded: bool,
    pub raw_json: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptItem {
    pub remote_id: String,
    #[serde(default)]
    pub list_order: Option<i64>,
    #[serde(default)]
    pub list_run_id: Option<String>,
    pub title: String,
    pub summary: String,
    pub content_text: String,
    pub author: String,
    pub content_type: String,
    pub source_url: String,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub article_path: Option<String>,
    pub video_path: Option<String>,
    pub local_dir: Option<String>,
    pub downloaded: bool,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ScriptEvent {
    #[serde(rename = "progress")]
    Progress {
        message: String,
        candidates: Option<i64>,
        skipped: Option<i64>,
        discovered: Option<i64>,
        processed: Option<i64>,
        saved: Option<i64>,
        downloaded: Option<i64>,
        #[serde(alias = "pageUrl")]
        page_url: Option<String>,
        #[serde(alias = "pageTitle")]
        page_title: Option<String>,
    },
    #[serde(rename = "item")]
    Item { item: ScriptItem },
    #[serde(rename = "profile")]
    Profile { profile: UserProfile },
    #[serde(rename = "item_error")]
    ItemError {
        #[serde(alias = "sourceUrl")]
        source_url: String,
        message: String,
    },
    #[serde(rename = "done")]
    Done { summary: ScriptSummary },
    #[serde(rename = "error")]
    Error { message: String, stack: Option<String> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptSummary {
    pub candidates: i64,
    pub skipped: i64,
    pub discovered: i64,
    pub saved: i64,
    pub downloaded: i64,
}
