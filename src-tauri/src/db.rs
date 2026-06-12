use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    error::AppError,
    models::{ContentItem, ScriptItem, SyncEvent, SyncSession, TagOption, UserProfile},
};

pub fn connect(db_path: &Path) -> Result<Connection, AppError> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        CREATE TABLE IF NOT EXISTS sync_sessions (
          id TEXT PRIMARY KEY,
          source TEXT NOT NULL,
          mode TEXT NOT NULL,
          status TEXT NOT NULL,
          total_candidates INTEGER NOT NULL DEFAULT 0,
          total_skipped INTEGER NOT NULL DEFAULT 0,
          total_discovered INTEGER NOT NULL DEFAULT 0,
          total_saved INTEGER NOT NULL DEFAULT 0,
          total_downloaded INTEGER NOT NULL DEFAULT 0,
          message TEXT NOT NULL DEFAULT '',
          started_at TEXT NOT NULL,
          finished_at TEXT
        );

        CREATE TABLE IF NOT EXISTS sync_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          level TEXT NOT NULL,
          message TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
        );

        CREATE TABLE IF NOT EXISTS content_items (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          remote_id TEXT NOT NULL,
          source TEXT NOT NULL,
          title TEXT NOT NULL,
          summary TEXT NOT NULL DEFAULT '',
          content_text TEXT NOT NULL DEFAULT '',
          author TEXT NOT NULL DEFAULT '',
          content_type TEXT NOT NULL,
          source_url TEXT NOT NULL,
          cover_url TEXT,
          cover_path TEXT,
          article_path TEXT,
          video_path TEXT,
          local_dir TEXT,
          list_order INTEGER,
          list_run_id TEXT,
          raw_json TEXT NOT NULL DEFAULT '{}',
          synced_at TEXT NOT NULL,
          UNIQUE(source, remote_id)
        );

        CREATE TABLE IF NOT EXISTS user_profile (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          name TEXT NOT NULL DEFAULT '',
          avatar_url TEXT,
          likes TEXT NOT NULL DEFAULT '',
          followers TEXT NOT NULL DEFAULT '',
          following TEXT NOT NULL DEFAULT '',
          bio TEXT NOT NULL DEFAULT '',
          updated_at TEXT NOT NULL DEFAULT ''
        );

        CREATE INDEX IF NOT EXISTS idx_content_items_title ON content_items(title);
        CREATE INDEX IF NOT EXISTS idx_content_items_synced_at ON content_items(synced_at DESC);
        CREATE INDEX IF NOT EXISTS idx_sync_events_session_id ON sync_events(session_id);

        CREATE TABLE IF NOT EXISTS content_item_tags (
          item_id INTEGER NOT NULL,
          tag TEXT NOT NULL,
          source TEXT NOT NULL DEFAULT 'auto',
          created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
          UNIQUE(item_id, tag)
        );

        CREATE TABLE IF NOT EXISTS content_item_tag_blocks (
          item_id INTEGER NOT NULL,
          tag TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
          UNIQUE(item_id, tag)
        );

        CREATE INDEX IF NOT EXISTS idx_content_item_tags_item_id ON content_item_tags(item_id);
        CREATE INDEX IF NOT EXISTS idx_content_item_tags_tag ON content_item_tags(tag);
        "#,
    )?;

    ensure_column(&conn, "sync_sessions", "total_candidates", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(&conn, "sync_sessions", "total_skipped", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(&conn, "content_items", "content_text", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(&conn, "content_items", "cover_path", "TEXT")?;
    ensure_column(&conn, "content_items", "list_order", "INTEGER")?;
    ensure_column(&conn, "content_items", "list_run_id", "TEXT")?;
    migrate_legacy_remote_ids(&conn)?;
    classify_existing_items(&conn)?;
    Ok(conn)
}

const DEFAULT_TAGS: [&str; 9] = ["IT", "编程", "运动", "医学", "文化", "壮族", "语言", "汉族", "基因"];

fn tag_rules(tag: &str) -> &'static [&'static str] {
    match tag {
        "IT" => &[
            "it", "ai", "codex", "github", "redis", "pdf", "html", "markdown", "wps", "软件", "开源", "工具",
            "电脑", "微软", "程序", "token", "数据库", "vue", "api",
        ],
        "编程" => &[
            "编程", "代码", "程序员", "rust", "python", "javascript", "redis", "github", "开源", "源码", "api",
            "开发", "框架",
        ],
        "运动" => &["运动", "训练", "膝盖", "肌肉", "康复", "腿", "跑", "健身", "臀", "动作"],
        "医学" => &["医学", "医生", "治疗", "健康", "康复", "疾病", "血管", "膝", "药", "医院"],
        "文化" => &["文化", "历史", "民族", "语言", "汉语", "壮族", "汉族", "民俗", "传统"],
        "壮族" => &["壮族", "壮话", "壮语"],
        "语言" => &["语言", "汉语", "英语", "口语", "普通话", "方言", "词汇", "词汇量"],
        "汉族" => &["汉族", "皇汉", "汉人"],
        "基因" => &["基因", "dna", "血统", "染色体"],
        _ => &[],
    }
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_matches('#').trim().to_string()
}

fn classify_text_tags(text: &str) -> Vec<String> {
    let haystack = text.to_lowercase();
    DEFAULT_TAGS
        .iter()
        .filter(|tag| tag_rules(tag).iter().any(|keyword| haystack.contains(&keyword.to_lowercase())))
        .map(|tag| (*tag).to_string())
        .collect()
}

fn insert_auto_tags(conn: &Connection, item_id: i64, tags: &[String]) -> Result<(), AppError> {
    for tag in tags {
        let tag = normalize_tag(tag);
        if tag.is_empty() {
            continue;
        }
        conn.execute(
            r#"
            INSERT OR IGNORE INTO content_item_tags (item_id, tag, source)
            SELECT ?1, ?2, 'auto'
            WHERE NOT EXISTS (
              SELECT 1 FROM content_item_tag_blocks WHERE item_id = ?1 AND tag = ?2
            )
            "#,
            params![item_id, tag],
        )?;
    }
    Ok(())
}

fn classify_item_by_id(conn: &Connection, item_id: i64) -> Result<(), AppError> {
    let Some((title, summary, content_text, author, source_url, raw_json)) = conn
        .query_row(
            r#"
            SELECT title, summary, content_text, author, source_url, raw_json
            FROM content_items
            WHERE id = ?1
            "#,
            [item_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()? else {
        return Ok(());
    };

    let text = format!("{title}\n{summary}\n{content_text}\n{author}\n{source_url}\n{raw_json}");
    insert_auto_tags(conn, item_id, &classify_text_tags(&text))
}

fn classify_existing_items(conn: &Connection) -> Result<(), AppError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id
        FROM content_items
        WHERE NOT EXISTS (
          SELECT 1 FROM content_item_tags WHERE item_id = content_items.id
        )
        "#,
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    let mut item_ids = Vec::new();
    for row in rows {
        item_ids.push(row?);
    }
    drop(stmt);

    for item_id in item_ids {
        classify_item_by_id(conn, item_id)?;
    }
    Ok(())
}

fn canonical_remote_id_from_url(source_url: &str, fallback_type: &str, remote_id: &str) -> Option<String> {
    for kind in ["article", "video", "w"] {
        let marker = format!("/{kind}/");
        if let Some(start) = source_url.find(&marker) {
            let rest = &source_url[start + marker.len()..];
            let id = rest
                .split(|character| character == '/' || character == '?' || character == '#')
                .next()
                .unwrap_or("")
                .trim();
            if !id.is_empty() {
                return Some(format!("{kind}:{id}"));
            }
        }
    }

    if remote_id.chars().all(|character| character.is_ascii_digit()) {
        let kind = if fallback_type == "video" { "video" } else { "article" };
        return Some(format!("{kind}:{remote_id}"));
    }

    None
}

fn migrate_legacy_remote_ids(conn: &Connection) -> Result<(), AppError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, source, remote_id, source_url, content_type
        FROM content_items
        WHERE remote_id NOT LIKE '%:%'
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut migrations = Vec::new();
    for row in rows {
        let (id, source, remote_id, source_url, content_type) = row?;
        if !remote_id.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        if let Some(canonical_id) = canonical_remote_id_from_url(&source_url, &content_type, &remote_id) {
            if canonical_id != remote_id {
                migrations.push((id, source, canonical_id));
            }
        }
    }
    drop(stmt);

    for (id, source, canonical_id) in migrations {
        let existing_id = conn
            .query_row(
                "SELECT id FROM content_items WHERE source = ?1 AND remote_id = ?2 AND id <> ?3 LIMIT 1",
                params![&source, &canonical_id, id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;

        if let Some(existing_id) = existing_id {
            conn.execute(
                r#"
                UPDATE content_items
                SET cover_path = COALESCE(cover_path, (SELECT cover_path FROM content_items WHERE id = ?2)),
                    article_path = COALESCE(article_path, (SELECT article_path FROM content_items WHERE id = ?2)),
                    video_path = COALESCE(video_path, (SELECT video_path FROM content_items WHERE id = ?2)),
                    local_dir = COALESCE(local_dir, (SELECT local_dir FROM content_items WHERE id = ?2)),
                    content_text = CASE
                      WHEN content_text = '' THEN COALESCE((SELECT content_text FROM content_items WHERE id = ?2), '')
                      ELSE content_text
                    END
                WHERE id = ?1
                "#,
                params![existing_id, id],
            )?;
            conn.execute("DELETE FROM content_items WHERE id = ?1", params![id])?;
        } else {
            conn.execute(
                "UPDATE content_items SET remote_id = ?1 WHERE id = ?2",
                params![canonical_id, id],
            )?;
        }
    }

    Ok(())
}

pub fn upsert_user_profile(conn: &Connection, profile: &UserProfile) -> Result<(), AppError> {
    conn.execute(
        r#"
        INSERT INTO user_profile (id, name, avatar_url, likes, followers, following, bio, updated_at)
        VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(id) DO UPDATE SET
          name = excluded.name,
          avatar_url = excluded.avatar_url,
          likes = excluded.likes,
          followers = excluded.followers,
          following = excluded.following,
          bio = excluded.bio,
          updated_at = excluded.updated_at
        "#,
        params![
            profile.name,
            profile.avatar_url,
            profile.likes,
            profile.followers,
            profile.following,
            profile.bio,
            profile.updated_at
        ],
    )?;
    Ok(())
}

pub fn get_user_profile(conn: &Connection) -> Result<Option<UserProfile>, AppError> {
    conn.query_row(
        r#"
        SELECT name, avatar_url, likes, followers, following, bio, updated_at
        FROM user_profile
        WHERE id = 1
        "#,
        [],
        |row| {
            Ok(UserProfile {
                name: row.get(0)?,
                avatar_url: row.get(1)?,
                likes: row.get(2)?,
                followers: row.get(3)?,
                following: row.get(4)?,
                bio: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<(), AppError> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut found = false;
    for value in columns {
        if value? == column {
            found = true;
            break;
        }
    }
    if !found {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
        conn.execute(&sql, [])?;
    }
    Ok(())
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<SyncSession>, AppError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, source, status, mode, total_candidates, total_skipped, total_discovered, total_saved, total_downloaded, message, started_at, finished_at
        FROM sync_sessions
        ORDER BY started_at DESC
        "#,
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(SyncSession {
            id: row.get(0)?,
            source: row.get(1)?,
            status: row.get(2)?,
            mode: row.get(3)?,
            total_candidates: row.get(4)?,
            total_skipped: row.get(5)?,
            total_discovered: row.get(6)?,
            total_saved: row.get(7)?,
            total_downloaded: row.get(8)?,
            message: row.get(9)?,
            started_at: row.get(10)?,
            finished_at: row.get(11)?,
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    Ok(sessions)
}

pub fn reset_running_sessions(conn: &Connection, message: &str, finished_at: &str) -> Result<(), AppError> {
    conn.execute(
        r#"
        UPDATE sync_sessions
        SET status = 'stopped',
            message = ?1,
            finished_at = ?2
        WHERE status = 'running'
        "#,
        params![message, finished_at],
    )?;
    Ok(())
}

pub fn list_remote_ids_requiring_download(conn: &Connection, source: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT remote_id
        FROM content_items
        WHERE source = ?1
          AND article_path IS NULL
          AND video_path IS NULL
        ORDER BY synced_at DESC
        "#,
    )?;
    let rows = stmt.query_map([source], |row| row.get::<_, String>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

pub fn list_sync_events(conn: &Connection, session_id: Option<&str>) -> Result<Vec<SyncEvent>, AppError> {
    let sql = if session_id.is_some() {
        r#"
        SELECT id, session_id, level, message, created_at
        FROM sync_events
        WHERE session_id = ?1
        ORDER BY id DESC
        LIMIT 100
        "#
    } else {
        r#"
        SELECT id, session_id, level, message, created_at
        FROM sync_events
        ORDER BY id DESC
        LIMIT 100
        "#
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(value) = session_id {
        stmt.query_map([value], map_sync_event)?
    } else {
        stmt.query_map([], map_sync_event)?
    };

    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

fn map_sync_event(row: &rusqlite::Row) -> rusqlite::Result<SyncEvent> {
    Ok(SyncEvent {
        id: row.get(0)?,
        session_id: row.get(1)?,
        level: row.get(2)?,
        message: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub fn insert_session(conn: &Connection, session: &SyncSession) -> Result<(), AppError> {
    conn.execute(
        r#"
        INSERT INTO sync_sessions (
          id, source, mode, status, total_candidates, total_skipped, total_discovered, total_saved, total_downloaded, message, started_at, finished_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            session.id,
            session.source,
            session.mode,
            session.status,
            session.total_candidates,
            session.total_skipped,
            session.total_discovered,
            session.total_saved,
            session.total_downloaded,
            session.message,
            session.started_at,
            session.finished_at
        ],
    )?;
    insert_sync_event(conn, &session.id, "info", "同步会话已创建")?;
    Ok(())
}

pub fn insert_sync_event(conn: &Connection, session_id: &str, level: &str, message: &str) -> Result<(), AppError> {
    conn.execute(
        r#"
        INSERT INTO sync_events (session_id, level, message)
        VALUES (?1, ?2, ?3)
        "#,
        params![session_id, level, message],
    )?;
    Ok(())
}

pub fn update_session_progress(
    conn: &Connection,
    session_id: &str,
    candidates: i64,
    skipped: i64,
    discovered: i64,
    saved: i64,
    downloaded: i64,
    status: &str,
    message: &str,
    finished_at: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        r#"
        UPDATE sync_sessions
        SET total_candidates = ?2,
            total_skipped = ?3,
            total_discovered = ?4,
            total_saved = ?5,
            total_downloaded = ?6,
            status = ?7,
            message = ?8,
            finished_at = ?9
        WHERE id = ?1
        "#,
        params![session_id, candidates, skipped, discovered, saved, downloaded, status, message, finished_at],
    )?;
    Ok(())
}

pub fn upsert_item(conn: &Connection, source: &str, item: &ScriptItem, synced_at: &str) -> Result<(), AppError> {
    conn.execute(
        r#"
        INSERT INTO content_items
          (remote_id, source, title, summary, content_text, author, content_type, source_url, cover_url, cover_path, article_path, video_path, local_dir, list_order, list_run_id, raw_json, synced_at)
        VALUES
          (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        ON CONFLICT(source, remote_id) DO UPDATE SET
          title = excluded.title,
          summary = excluded.summary,
          content_text = CASE WHEN excluded.content_text = '' THEN content_items.content_text ELSE excluded.content_text END,
          author = CASE WHEN excluded.author = '' THEN content_items.author ELSE excluded.author END,
          content_type = excluded.content_type,
          source_url = excluded.source_url,
          cover_url = COALESCE(excluded.cover_url, content_items.cover_url),
          cover_path = COALESCE(excluded.cover_path, content_items.cover_path),
          article_path = COALESCE(excluded.article_path, content_items.article_path),
          video_path = COALESCE(excluded.video_path, content_items.video_path),
          local_dir = COALESCE(excluded.local_dir, content_items.local_dir),
          list_order = COALESCE(excluded.list_order, content_items.list_order),
          list_run_id = COALESCE(excluded.list_run_id, content_items.list_run_id),
          raw_json = excluded.raw_json,
          synced_at = excluded.synced_at
        "#,
        params![
            item.remote_id,
            source,
            item.title,
            item.summary,
            item.content_text,
            item.author,
            item.content_type,
            item.source_url,
            item.cover_url,
            item.cover_path,
            item.article_path,
            item.video_path,
            item.local_dir,
            item.list_order,
            item.list_run_id,
            item.raw.to_string(),
            synced_at
        ],
    )?;
    let item_id = conn.query_row(
        "SELECT id FROM content_items WHERE source = ?1 AND remote_id = ?2",
        params![source, &item.remote_id],
        |row| row.get::<_, i64>(0),
    )?;
    classify_item_by_id(conn, item_id)?;
    Ok(())
}

pub fn search_items(
    conn: &Connection,
    query: &str,
    source: Option<&str>,
    content_type: Option<&str>,
    tag_filters: &[String],
) -> Result<Vec<ContentItem>, AppError> {
    let keyword = format!("%{}%", query.trim());
    let mut sql = String::from(
        r#"
        SELECT id, remote_id, source, title, summary, content_text, author, content_type, source_url, cover_url, cover_path, article_path, video_path, local_dir, synced_at, raw_json,
               CASE WHEN article_path IS NOT NULL OR video_path IS NOT NULL THEN 1 ELSE 0 END AS downloaded,
               COALESCE((
                 SELECT GROUP_CONCAT(tag, '||')
                 FROM content_item_tags tag_items
                 WHERE tag_items.item_id = content_items.id
                 ORDER BY tag
               ), '') AS tags
        FROM content_items
        WHERE 1 = 1
        "#,
    );
    let mut binds: Vec<String> = Vec::new();

    if !query.trim().is_empty() {
        sql.push_str(" AND (title LIKE ? OR summary LIKE ? OR content_text LIKE ? OR author LIKE ? OR raw_json LIKE ?)");
        binds.push(keyword.clone());
        binds.push(keyword.clone());
        binds.push(keyword.clone());
        binds.push(keyword.clone());
        binds.push(keyword);
    }

    if let Some(source_value) = source.filter(|value| !value.trim().is_empty()) {
        sql.push_str(" AND source = ?");
        binds.push(source_value.to_string());
        sql.push_str(
            r#"
            AND (
              list_run_id = (
                SELECT latest.list_run_id
                FROM content_items latest
                WHERE latest.source = ?
                  AND latest.list_run_id IS NOT NULL
                ORDER BY latest.synced_at DESC, latest.list_order ASC
                LIMIT 1
              )
              OR NOT EXISTS (
                SELECT 1
                FROM content_items latest
                WHERE latest.source = ?
                  AND latest.list_run_id IS NOT NULL
              )
            )
            "#,
        );
        binds.push(source_value.to_string());
        binds.push(source_value.to_string());
    }

    if let Some(content_type_value) = content_type.filter(|value| !value.trim().is_empty()) {
        sql.push_str(" AND content_type = ?");
        binds.push(content_type_value.to_string());
    }

    let clean_tag_filters = tag_filters
        .iter()
        .map(|tag| normalize_tag(tag))
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    if !clean_tag_filters.is_empty() {
        sql.push_str(" AND EXISTS (SELECT 1 FROM content_item_tags filter_tags WHERE filter_tags.item_id = content_items.id AND filter_tags.tag IN (");
        for index in 0..clean_tag_filters.len() {
            if index > 0 {
                sql.push_str(", ");
            }
            sql.push('?');
        }
        sql.push_str("))");
        binds.extend(clean_tag_filters);
    }

    if source.is_some() {
        sql.push_str(" ORDER BY CASE WHEN list_order IS NULL THEN 1 ELSE 0 END, list_order ASC, synced_at DESC LIMIT 200");
    } else {
        sql.push_str(" ORDER BY synced_at DESC LIMIT 200");
    }

    let mut stmt = conn.prepare(&sql)?;
    let params = rusqlite::params_from_iter(binds.iter());
    let rows = stmt.query_map(params, |row| {
        Ok(ContentItem {
            id: row.get(0)?,
            remote_id: row.get(1)?,
            source: row.get(2)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            content_text: row.get(5)?,
            author: row.get(6)?,
            content_type: row.get(7)?,
            source_url: row.get(8)?,
            cover_url: row.get(9)?,
            cover_path: row.get(10)?,
            article_path: row.get(11)?,
            video_path: row.get(12)?,
            local_dir: row.get(13)?,
            synced_at: row.get(14)?,
            raw_json: row.get(15)?,
            downloaded: row.get::<_, i64>(16)? == 1,
            tags: row
                .get::<_, String>(17)?
                .split("||")
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(ToString::to_string)
                .collect(),
        })
    })?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

fn latest_source_filter_sql(source: Option<&str>) -> (String, Vec<String>) {
    let Some(source_value) = source.filter(|value| !value.trim().is_empty()) else {
        return (String::new(), Vec::new());
    };
    (
        r#"
        AND content_items.source = ?
        AND (
          content_items.list_run_id = (
            SELECT latest.list_run_id
            FROM content_items latest
            WHERE latest.source = ?
              AND latest.list_run_id IS NOT NULL
            ORDER BY latest.synced_at DESC, latest.list_order ASC
            LIMIT 1
          )
          OR NOT EXISTS (
            SELECT 1
            FROM content_items latest
            WHERE latest.source = ?
              AND latest.list_run_id IS NOT NULL
          )
        )
        "#.to_string(),
        vec![source_value.to_string(), source_value.to_string(), source_value.to_string()],
    )
}

pub fn list_tag_options(conn: &Connection, source: Option<&str>) -> Result<Vec<TagOption>, AppError> {
    let (source_sql, source_binds) = latest_source_filter_sql(source);
    let total_sql = format!("SELECT COUNT(*) FROM content_items WHERE 1 = 1 {source_sql}");
    let total = conn.query_row(&total_sql, rusqlite::params_from_iter(source_binds.iter()), |row| row.get::<_, i64>(0))?;

    let mut options = vec![TagOption { name: "所有".to_string(), count: total }];
    for (name, content_type) in [("视频", "video"), ("文章", "article")] {
        let (source_sql, mut binds) = latest_source_filter_sql(source);
        let sql = format!("SELECT COUNT(*) FROM content_items WHERE 1 = 1 {source_sql} AND content_type = ?");
        binds.push(content_type.to_string());
        let count = conn.query_row(&sql, rusqlite::params_from_iter(binds.iter()), |row| row.get::<_, i64>(0))?;
        options.push(TagOption { name: name.to_string(), count });
    }

    let (source_sql, binds) = latest_source_filter_sql(source);
    let sql = format!(
        r#"
        SELECT tag_items.tag, COUNT(*) AS count
        FROM content_item_tags tag_items
        JOIN content_items ON content_items.id = tag_items.item_id
        WHERE 1 = 1
        {source_sql}
        GROUP BY tag_items.tag
        ORDER BY count DESC, tag_items.tag ASC
        "#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |row| {
        Ok(TagOption {
            name: row.get(0)?,
            count: row.get(1)?,
        })
    })?;

    let mut existing_names = options.iter().map(|option| option.name.clone()).collect::<Vec<_>>();
    for tag in DEFAULT_TAGS {
        if !existing_names.iter().any(|name| name == tag) {
            options.push(TagOption { name: tag.to_string(), count: 0 });
            existing_names.push(tag.to_string());
        }
    }
    for row in rows {
        let option = row?;
        if !existing_names.iter().any(|name| name == &option.name) {
            existing_names.push(option.name.clone());
            options.push(option);
        } else if let Some(existing) = options.iter_mut().find(|item| item.name == option.name) {
            existing.count = option.count;
        }
    }
    Ok(options)
}

pub fn add_item_tag(conn: &Connection, item_id: i64, tag: &str) -> Result<Vec<String>, AppError> {
    let tag = normalize_tag(tag);
    if tag.is_empty() {
        return Err(AppError::Message("标签不能为空".into()));
    }
    conn.execute(
        "DELETE FROM content_item_tag_blocks WHERE item_id = ?1 AND tag = ?2",
        params![item_id, &tag],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO content_item_tags (item_id, tag, source) VALUES (?1, ?2, 'manual')",
        params![item_id, &tag],
    )?;
    list_item_tags(conn, item_id)
}

pub fn remove_item_tag(conn: &Connection, item_id: i64, tag: &str) -> Result<Vec<String>, AppError> {
    let tag = normalize_tag(tag);
    if tag.is_empty() {
        return list_item_tags(conn, item_id);
    }
    conn.execute(
        "DELETE FROM content_item_tags WHERE item_id = ?1 AND tag = ?2",
        params![item_id, &tag],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO content_item_tag_blocks (item_id, tag) VALUES (?1, ?2)",
        params![item_id, &tag],
    )?;
    list_item_tags(conn, item_id)
}

pub fn list_item_tags(conn: &Connection, item_id: i64) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT tag
        FROM content_item_tags
        WHERE item_id = ?1
        ORDER BY tag ASC
        "#,
    )?;
    let rows = stmt.query_map([item_id], |row| row.get::<_, String>(0))?;
    let mut tags = Vec::new();
    for row in rows {
        tags.push(row?);
    }
    Ok(tags)
}

pub fn get_item_dir(conn: &Connection, item_id: i64) -> Result<Option<String>, AppError> {
    let value = conn
        .query_row(
            "SELECT local_dir FROM content_items WHERE id = ?1",
            [item_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(value.flatten())
}

pub fn get_item_file(conn: &Connection, item_id: i64, kind: &str) -> Result<Option<String>, AppError> {
    let column = match kind {
        "article" => "article_path",
        "video" => "video_path",
        "cover" => "cover_path",
        "raw" => "local_dir",
        _ => {
            return Err(AppError::Message(format!("不支持的文件类型: {kind}")));
        }
    };

    let value = conn
        .query_row(
            &format!("SELECT {column} FROM content_items WHERE id = ?1"),
            [item_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;

    if kind == "raw" {
        return Ok(value
            .flatten()
            .map(|directory| Path::new(&directory).join("article.json").display().to_string()));
    }

    Ok(value.flatten())
}

pub fn list_known_remote_ids(conn: &Connection, source: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT remote_id
        FROM content_items
        WHERE source = ?1
        "#,
    )?;
    let rows = stmt.query_map([source], |row| row.get::<_, String>(0))?;
    let mut remote_ids = Vec::new();
    for row in rows {
        remote_ids.push(row?);
    }
    Ok(remote_ids)
}
