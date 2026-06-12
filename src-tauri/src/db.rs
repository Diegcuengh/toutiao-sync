use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    error::AppError,
    models::{ContentItem, ScriptItem, SyncEvent, SyncSession, UserProfile},
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
        "#,
    )?;

    ensure_column(&conn, "sync_sessions", "total_candidates", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(&conn, "sync_sessions", "total_skipped", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(&conn, "content_items", "content_text", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(&conn, "content_items", "cover_path", "TEXT")?;
    ensure_column(&conn, "content_items", "list_order", "INTEGER")?;
    ensure_column(&conn, "content_items", "list_run_id", "TEXT")?;
    Ok(conn)
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
    Ok(())
}

pub fn search_items(
    conn: &Connection,
    query: &str,
    source: Option<&str>,
    content_type: Option<&str>,
) -> Result<Vec<ContentItem>, AppError> {
    let keyword = format!("%{}%", query.trim());
    let mut sql = String::from(
        r#"
        SELECT id, remote_id, source, title, summary, content_text, author, content_type, source_url, cover_url, cover_path, article_path, video_path, local_dir, synced_at, raw_json,
               CASE WHEN article_path IS NOT NULL OR video_path IS NOT NULL THEN 1 ELSE 0 END AS downloaded
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
        })
    })?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
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
