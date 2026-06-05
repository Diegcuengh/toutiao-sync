use std::{env, path::Path};

use toutiao_sync_desktop_lib::{db, error::AppError};

fn main() -> Result<(), AppError> {
    let conn = db::connect(Path::new(r"D:\toutiao-sync\app.db"))?;
    let target_session_id = env::args().nth(1);

    println!("sessions:");
    let sessions = db::list_sessions(&conn)?;
    for session in sessions.iter().take(8) {
        println!(
            "{} | {} | {} | 候选 {} 跳过 {} 新增 {} 入库 {} 下载 {} | {}",
            session.started_at,
            session.source,
            session.status,
            session.total_candidates,
            session.total_skipped,
            session.total_discovered,
            session.total_saved,
            session.total_downloaded,
            session.message
        );
    }

    if let Some(session_id) = target_session_id {
        println!("\nselected session:");
        if let Some(session) = sessions.iter().find(|item| item.id == session_id) {
            println!(
                "{} | {} | {} | 候选 {} 跳过 {} 新增 {} 入库 {} 下载 {} | {}",
                session.started_at,
                session.source,
                session.status,
                session.total_candidates,
                session.total_skipped,
                session.total_discovered,
                session.total_saved,
                session.total_downloaded,
                session.message
            );
            println!("\nselected session events:");
            for event in db::list_sync_events(&conn, Some(&session_id))?.into_iter().rev() {
                println!("{} | {} | {}", event.created_at, event.level, event.message);
            }
        } else {
            println!("not found: {}", session_id);
        }
    }

    let favorites = db::search_items(&conn, "", Some("favorites"), None)?;
    let likes = db::search_items(&conn, "", Some("likes"), None)?;
    let article_count = favorites
        .iter()
        .chain(likes.iter())
        .filter(|item| item.content_type == "article")
        .count();
    let video_count = favorites
        .iter()
        .chain(likes.iter())
        .filter(|item| item.content_type == "video")
        .count();

    println!("\nitems:");
    println!(
        "favorites={} likes={} articles={} videos={}",
        favorites.len(),
        likes.len(),
        article_count,
        video_count
    );

    println!("\nlatest favorites:");
    for item in favorites.into_iter().take(5) {
        println!(
            "#{} | {} | {} | article={} | video={}",
            item.id,
            item.content_type,
            item.title,
            item.article_path.unwrap_or_default(),
            item.video_path.unwrap_or_default()
        );
    }

    println!("\nlatest events:");
    for event in db::list_sync_events(&conn, None)?.into_iter().take(10).rev() {
        println!("{} | {} | {}", event.created_at, event.level, event.message);
    }

    Ok(())
}
