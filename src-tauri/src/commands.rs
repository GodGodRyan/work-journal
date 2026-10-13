//! Tauri 对外命令层：入参校验 → 调用 db.rs → 统一把 anyhow 错误转为前端可读字符串。
//! 所有命令均为同步函数（Tauri 将其调度到主线程执行），
//! 阻塞式保存/选择对话框（macOS 要求主线程）与数据库操作均安全。

use crate::db::{self, Db};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// 全局共享状态：数据库连接（互斥锁保护）、数据库文件路径与媒体附件目录
pub struct AppState {
    pub db: Mutex<Db>,
    pub db_path: PathBuf,
    pub attach_dir: PathBuf,
}

/// 提取 anyhow 错误链的第一行（含上下文），作为前端可读信息
fn err_str(e: anyhow::Error) -> String {
    let s = format!("{e:#}");
    s.lines().next().unwrap_or("操作失败").to_string()
}

fn db_of<'a>(state: &'a State<AppState>) -> Result<MutexGuard<'a, Db>, String> {
    state
        .db
        .lock()
        .map_err(|_| "数据库连接异常，请重启应用".to_string())
}

/// 文本入参统一校验：长度上限
fn check_text(value: &str, field: &str, max: usize) -> Result<(), String> {
    if value.chars().count() > max {
        return Err(format!("{field}过长（最多 {max} 个字符）"));
    }
    Ok(())
}

// ---------- 记录 CRUD ----------

#[tauri::command]
pub fn get_records_by_date(state: State<AppState>, date: String) -> Result<Vec<db::WorkRecord>, String> {
    db::validate_date(&date).map_err(err_str)?;
    db_of(&state)?.list_by_date(&date).map_err(err_str)
}

#[tauri::command]
pub fn create_record(
    state: State<AppState>,
    date: String,
    title: String,
    content: String,
    status: String,
    tags: String,
) -> Result<db::WorkRecord, String> {
    db::validate_date(&date).map_err(err_str)?;
    db::validate_status(&status).map_err(err_str)?;
    check_text(&title, "标题", db::MAX_TITLE_LEN)?;
    check_text(&content, "内容", db::MAX_CONTENT_LEN)?;
    let title = title.trim().to_string();
    let tags = db::normalize_tags(&tags).map_err(err_str)?;
    db_of(&state)?
        .create(&date, &title, &content, &status, &tags)
        .map_err(err_str)
}

#[tauri::command]
pub fn update_record(
    state: State<AppState>,
    id: i64,
    title: String,
    content: String,
    status: String,
    tags: String,
) -> Result<db::WorkRecord, String> {
    if id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    db::validate_status(&status).map_err(err_str)?;
    check_text(&title, "标题", db::MAX_TITLE_LEN)?;
    check_text(&content, "内容", db::MAX_CONTENT_LEN)?;
    let title = title.trim().to_string();
    let tags = db::normalize_tags(&tags).map_err(err_str)?;
    db_of(&state)?
        .update(id, &title, &content, &status, &tags)
        .map_err(err_str)
}

#[tauri::command]
pub fn soft_delete_record(state: State<AppState>, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    db_of(&state)?.soft_delete(id).map_err(err_str)
}

#[tauri::command]
pub fn set_record_pinned(
    state: State<AppState>,
    id: i64,
    pinned: bool,
) -> Result<db::WorkRecord, String> {
    if id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    db_of(&state)?.set_pinned(id, pinned).map_err(err_str)
}

#[tauri::command]
pub fn get_pinned_records(state: State<AppState>) -> Result<Vec<db::WorkRecord>, String> {
    db_of(&state)?.list_pinned().map_err(err_str)
}

#[tauri::command]
pub fn restore_record(state: State<AppState>, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    db_of(&state)?.restore(id).map_err(err_str)
}

#[tauri::command]
pub fn purge_record(state: State<AppState>, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    db_of(&state)?.purge(id, &state.attach_dir).map_err(err_str)
}

#[tauri::command]
pub fn empty_trash(state: State<AppState>) -> Result<i64, String> {
    db_of(&state)?.empty_trash(&state.attach_dir).map_err(err_str)
}

#[tauri::command]
pub fn get_trash_records(state: State<AppState>) -> Result<Vec<db::WorkRecord>, String> {
    db_of(&state)?.trash().map_err(err_str)
}

// ---------- 媒体附件 ----------

#[tauri::command]
// async：blocking 文件选择器不能在 sync 命令（主线程）里调用，否则面板弹出后主线程被阻塞、整个应用冻结
pub async fn add_attachments(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: i64,
) -> Result<Vec<db::Attachment>, String> {
    if record_id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    let picked = app
        .dialog()
        .file()
        .set_title("选择媒体文件（可多选）")
        .add_filter("图片", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])
        .add_filter("视频", &["mp4", "webm", "mov", "m4v", "mkv"])
        .add_filter("音频", &["mp3", "wav", "m4a", "aac", "ogg", "flac"])
        .add_filter("全部文件", &["*"])
        .blocking_pick_files();
    let Some(files) = picked else {
        return Ok(vec![]); // 用户取消
    };
    let db = db_of(&state)?;
    // 逐文件容错：单个文件失败不中断其余文件，已成功的已入库（前端收错后仍会刷新列表）
    let mut added = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for f in files {
        let p = match f.into_path() {
            Ok(p) => p,
            Err(e) => {
                errors.push(e.to_string());
                continue;
            }
        };
        match db.add_attachment(record_id, &p, &state.attach_dir) {
            Ok(a) => added.push(a),
            Err(e) => errors.push(format!("{}：{}", p.display(), err_str(e))),
        }
    }
    if !errors.is_empty() {
        return Err(format!("部分文件添加失败：{}", errors.join("；")));
    }
    Ok(added)
}

#[tauri::command]
pub fn list_attachments(state: State<AppState>, record_id: i64) -> Result<Vec<db::Attachment>, String> {
    if record_id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    db_of(&state)?
        .attachments(record_id, &state.attach_dir)
        .map_err(err_str)
}

/// 粘贴添加附件：前端把剪贴板媒体转成 data URL（base64）传入，
/// 后端解码后落盘存储（扩展名白名单 / 大小上限与路径版一致）
#[tauri::command]
pub fn add_attachment_data(
    state: State<AppState>,
    record_id: i64,
    file_name: String,
    data_url: String,
) -> Result<db::Attachment, String> {
    if record_id <= 0 {
        return Err("记录 id 无效".to_string());
    }
    check_text(&file_name, "文件名", 255)?;
    let bytes = db::decode_data_url(&data_url).map_err(err_str)?;
    db_of(&state)?
        .add_attachment_bytes(record_id, &file_name, &bytes, &state.attach_dir)
        .map_err(err_str)
}

#[tauri::command]
pub fn delete_attachment(state: State<AppState>, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("附件 id 无效".to_string());
    }
    db_of(&state)?
        .delete_attachment(id, &state.attach_dir)
        .map_err(err_str)
}

/// 设置附件显示宽度（编辑器拖拽调整媒体块大小后持久化）
#[tauri::command]
pub fn set_attachment_width(state: State<AppState>, id: i64, width: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("附件 id 无效".to_string());
    }
    db_of(&state)?
        .set_attachment_width(id, width)
        .map_err(err_str)
}

/// 读取附件内容为 data URL（媒体块 Ctrl+C 复制到剪贴板用）
#[tauri::command]
pub fn get_attachment_data(state: State<AppState>, id: i64) -> Result<String, String> {
    if id <= 0 {
        return Err("附件 id 无效".to_string());
    }
    db_of(&state)?
        .attachment_data_url(id, &state.attach_dir)
        .map_err(err_str)
}

// ---------- 时间线 / 搜索 / 标签 / 统计 ----------

#[tauri::command]
pub fn get_timeline(
    state: State<AppState>,
    offset: i64,
    limit: i64,
) -> Result<Vec<db::WorkRecord>, String> {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    db_of(&state)?.timeline(offset, limit).map_err(err_str)
}

#[tauri::command]
pub fn search_records(
    state: State<AppState>,
    query: String,
    date_from: Option<String>,
    date_to: Option<String>,
) -> Result<Vec<db::WorkRecord>, String> {
    if let Some(f) = &date_from {
        db::validate_date(f).map_err(err_str)?;
    }
    if let Some(t) = &date_to {
        db::validate_date(t).map_err(err_str)?;
    }
    if let (Some(f), Some(t)) = (&date_from, &date_to) {
        if f > t {
            return Err("开始日期不能晚于结束日期".to_string());
        }
    }
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    db_of(&state)?
        .search(query, date_from.as_deref(), date_to.as_deref())
        .map_err(err_str)
}

#[tauri::command]
pub fn get_record_dates(state: State<AppState>) -> Result<Vec<String>, String> {
    db_of(&state)?.record_dates().map_err(err_str)
}

#[tauri::command]
pub fn get_all_tags(state: State<AppState>) -> Result<Vec<db::TagStat>, String> {
    db_of(&state)?.all_tags().map_err(err_str)
}

#[tauri::command]
pub fn get_recent_records_by_date(
    state: State<AppState>,
    date: String,
    limit: i64,
) -> Result<Vec<db::WorkRecord>, String> {
    db::validate_date(&date).map_err(err_str)?;
    let limit = limit.clamp(1, 50);
    db_of(&state)?
        .list_recent_by_date(&date, limit)
        .map_err(err_str)
}

#[tauri::command]
pub fn get_records_by_tag(
    state: State<AppState>,
    tag: String,
    limit: i64,
) -> Result<Vec<db::WorkRecord>, String> {
    let tag = tag.trim().to_string();
    if tag.is_empty() {
        return Err("标签不能为空".to_string());
    }
    if tag.chars().count() > db::MAX_TAG_LEN {
        return Err(format!("标签过长（最多 {} 个字符）", db::MAX_TAG_LEN));
    }
    let limit = limit.clamp(1, 500);
    db_of(&state)?.list_by_tag(&tag, limit).map_err(err_str)
}

#[tauri::command]
pub fn get_records_by_range(
    state: State<AppState>,
    from: String,
    to: String,
) -> Result<Vec<db::WorkRecord>, String> {
    db::validate_date(&from).map_err(err_str)?;
    db::validate_date(&to).map_err(err_str)?;
    if from > to {
        return Err("开始日期不能晚于结束日期".to_string());
    }
    // 跨度上限防御：日历划选不会超过一年，超出视为异常请求。
    // 格式已由 validate_date 保证，此处解析失败属不可能路径，兜底返回可读错误
    let f = chrono::NaiveDate::parse_from_str(&from, "%Y-%m-%d")
        .map_err(|e| format!("日期解析失败：{e}"))?;
    let t = chrono::NaiveDate::parse_from_str(&to, "%Y-%m-%d")
        .map_err(|e| format!("日期解析失败：{e}"))?;
    if (t - f).num_days() > 366 {
        return Err("日期范围过长（最多 366 天）".to_string());
    }
    db_of(&state)?.list_by_range(&from, &to).map_err(err_str)
}

// ---------- 导出 / 备份 / 恢复 ----------

#[tauri::command]
// async：blocking 保存对话框不能在 sync 命令（主线程）里调用，否则面板弹出后主线程被阻塞、整个应用冻结
pub async fn export_report(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: String,
    date: String,
) -> Result<Option<String>, String> {
    if !matches!(kind.as_str(), "daily" | "weekly" | "monthly" | "all") {
        return Err("导出类型无效".to_string());
    }
    db::validate_date(&date).map_err(err_str)?;
    let markdown = db_of(&state)?
        .export_markdown(&kind, &date)
        .map_err(err_str)?;
    let default_name = match kind.as_str() {
        "daily" => format!("工作日报-{date}.md"),
        "weekly" => "工作周报.md".to_string(),
        "monthly" => "工作月报.md".to_string(),
        _ => "工作记录总览.md".to_string(),
    };
    let picked = app
        .dialog()
        .file()
        .set_title("导出为 Markdown")
        .set_file_name(&default_name)
        .add_filter("Markdown", &["md"])
        .blocking_save_file();
    let Some(path) = picked else {
        return Ok(None); // 用户取消
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, markdown.as_bytes())
        .map_err(|e| format!("写入文件失败：{e}"))?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
// async：blocking 文件夹选择器不能在 sync 命令（主线程）里调用，否则面板弹出后主线程被阻塞、整个应用冻结
pub async fn backup_database(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("选择备份保存文件夹")
        .blocking_pick_folder();
    let Some(dir) = picked else {
        return Ok(None); // 用户取消
    };
    let dir = dir.into_path().map_err(|e| e.to_string())?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = dir.join(format!("工作日志备份-{ts}.zip"));
    db_of(&state)?.backup_to(&dest, &state.attach_dir).map_err(err_str)?;
    Ok(Some(dest.display().to_string()))
}

#[tauri::command]
// async：blocking 文件选择器不能在 sync 命令（主线程）里调用，否则面板弹出后主线程被阻塞、整个应用冻结
pub async fn restore_database(app: AppHandle, state: State<'_, AppState>) -> Result<Option<i64>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("选择备份文件（.zip 或旧版 .db）")
        .add_filter("工作日志备份", &["zip", "db"])
        .blocking_pick_file();
    let Some(file) = picked else {
        return Ok(None); // 用户取消
    };
    let src = file.into_path().map_err(|e| e.to_string())?;
    let mut db = db_of(&state)?; // restore_from 需要 &mut self
    // 恢复前先把当前数据（含附件）自动备份到应用数据目录，防止误操作造成不可逆丢失
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let safety = state
        .db_path
        .with_file_name(format!("恢复前自动备份-{ts}.zip"));
    db.backup_to(&safety, &state.attach_dir)
        .map_err(|e| format!("恢复前安全备份失败，已中止恢复：{e:#}"))?;
    let count = db.restore_from(&src, &state.attach_dir).map_err(err_str)?;
    Ok(Some(count))
}

// ---------- 元信息 ----------

#[tauri::command]
pub fn get_db_path(state: State<AppState>) -> Result<String, String> {
    Ok(state.db_path.display().to_string())
}

#[tauri::command]
pub fn get_app_version() -> Result<String, String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}
