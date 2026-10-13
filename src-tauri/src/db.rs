//! 数据库层：初始化、版本迁移、FTS5 全文索引、全部 CRUD、备份/恢复、Markdown 导出。
//! 所有函数返回 anyhow::Result，由 commands.rs 统一转换为前端可读的错误信息，
//! 本层不允许 panic。

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate};
use regex::Regex;
use rusqlite::functions::FunctionFlags;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;

/// 中文全文搜索的关键：unicode61 默认把连续 CJK 字符视为单个 token，
/// 「超时」这类词中缀无法被前缀/短语查询命中。
/// 方案：写入 FTS 时经 cjk_expand UDF 把连续 CJK 字符展开为单字（字符间插空格），
/// 查询词做同样展开后以短语匹配，中文任意子串均可命中。
/// 高亮与摘要由前端基于原文与查询词渲染，后端只负责召回。

/// 字段长度上限（按字符数，防异常输入撑爆数据库）
pub const MAX_TITLE_LEN: usize = 500;
pub const MAX_CONTENT_LEN: usize = 100_000;
pub const MAX_TAG_LEN: usize = 50;

/// 数据库迁移脚本。规则：只允许追加新版本，禁止修改已发布版本；
/// 当前版本号记录在 PRAGMA user_version 中，升级程序按版本递增执行，历史数据不丢失。
const MIGRATIONS: &[&str] = &[
    // v1: 初始表结构 + 索引 + FTS5 全文索引 + 增删改同步触发器。
    // 触发器写入时经 cjk_expand UDF 把连续 CJK 字符展开为单字，
    // 使中文任意子串（如「超时」在「修复登录接口超时」中）可被短语查询命中。
    // UDF 在 Db::open 时注册（migrate 之前），触发器执行时必然可用。
    r#"
CREATE TABLE IF NOT EXISTS records (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    record_date TEXT    NOT NULL,
    title       TEXT    NOT NULL DEFAULT '',
    content     TEXT    NOT NULL DEFAULT '',
    status      TEXT    NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed')),
    tags        TEXT    NOT NULL DEFAULT '',
    is_deleted  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_records_date ON records (record_date);
CREATE INDEX IF NOT EXISTS idx_records_deleted ON records (is_deleted);
CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5 (
    title, content, tags,
    content = 'records',
    content_rowid = 'id',
    tokenize = 'unicode61'
);
CREATE TRIGGER IF NOT EXISTS trg_records_ai AFTER INSERT ON records BEGIN
    INSERT INTO records_fts (rowid, title, content, tags)
    VALUES (new.id, cjk_expand(new.title), cjk_expand(new.content), cjk_expand(new.tags));
END;
CREATE TRIGGER IF NOT EXISTS trg_records_ad AFTER DELETE ON records BEGIN
    INSERT INTO records_fts (records_fts, rowid, title, content, tags)
    VALUES ('delete', old.id, cjk_expand(old.title), cjk_expand(old.content), cjk_expand(old.tags));
END;
-- 注意：FTS5 外部内容表要求索引与内容表行一一对应，触发器必须无条件同步。
-- 若按 is_deleted 条件跳过插入，会出现「内容表有行、索引无行」的中间态，
-- 后续对该行 UPDATE 时 FTS5 的 'delete' 校验会判为索引损坏（错误 267）。
-- 软删除行的过滤由 search 查询中的 r.is_deleted = 0 承担。
CREATE TRIGGER IF NOT EXISTS trg_records_au AFTER UPDATE ON records BEGIN
    INSERT INTO records_fts (records_fts, rowid, title, content, tags)
    VALUES ('delete', old.id, cjk_expand(old.title), cjk_expand(old.content), cjk_expand(old.tags));
    INSERT INTO records_fts (rowid, title, content, tags)
    VALUES (new.id, cjk_expand(new.title), cjk_expand(new.content), cjk_expand(new.tags));
END;
"#,
    // v2: 记录永久置顶。is_pinned 不参与 FTS 索引，无需触发器变更。
    "ALTER TABLE records ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0;",
    // v3: 媒体附件（图片/视频/音频）。文件存应用数据目录 attachments/，
    // 数据库只存元数据与存储文件名。记录物理删除时外键级联删除行
    // （foreign_keys=ON 已开启），磁盘文件由 purge/empty_trash 显式清理。
    r#"
CREATE TABLE IF NOT EXISTS attachments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    record_id   INTEGER NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    file_name   TEXT    NOT NULL,
    stored_name TEXT    NOT NULL UNIQUE,
    media_type  TEXT    NOT NULL CHECK (media_type IN ('image', 'video', 'audio')),
    size        INTEGER NOT NULL,
    created_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachments_record ON attachments (record_id);
"#,
    // v4: 附件显示宽度（编辑器中可拖拽调整媒体块大小；NULL = 默认尺寸）
    "ALTER TABLE attachments ADD COLUMN width INTEGER;",
    // v5: 富文本编辑器。content 存 HTML（<p>/<strong>/<span style>/<figure>），
    // content_text 存纯文本副本（FTS 索引 / 搜索摘要 / Markdown 导出用），
    // 由后端在写入时经 html_to_text 提取，触发器改索引 content_text。
    r#"
ALTER TABLE records ADD COLUMN content_text TEXT NOT NULL DEFAULT '';
DROP TRIGGER IF EXISTS trg_records_ai;
DROP TRIGGER IF EXISTS trg_records_ad;
DROP TRIGGER IF EXISTS trg_records_au;
CREATE TRIGGER trg_records_ai AFTER INSERT ON records BEGIN
    INSERT INTO records_fts (rowid, title, content, tags)
    VALUES (new.id, cjk_expand(new.title), cjk_expand(new.content_text), cjk_expand(new.tags));
END;
CREATE TRIGGER trg_records_ad AFTER DELETE ON records BEGIN
    INSERT INTO records_fts (records_fts, rowid, title, content, tags)
    VALUES ('delete', old.id, cjk_expand(old.title), cjk_expand(old.content_text), cjk_expand(old.tags));
END;
CREATE TRIGGER trg_records_au AFTER UPDATE ON records BEGIN
    INSERT INTO records_fts (records_fts, rowid, title, content, tags)
    VALUES ('delete', old.id, cjk_expand(old.title), cjk_expand(old.content_text), cjk_expand(old.tags));
    INSERT INTO records_fts (rowid, title, content, tags)
    VALUES (new.id, cjk_expand(new.title), cjk_expand(new.content_text), cjk_expand(new.tags));
END;
"#,
];

/// 单附件大小上限（500MB，覆盖常见视频文件）
pub const MAX_ATTACH_SIZE_BYTES: u64 = 500 * 1024 * 1024;

/// 媒体块显示宽度范围（px，拖拽调整大小用；与前端拖拽钳制一致）
pub const MIN_ATTACH_WIDTH: i64 = 120;
pub const MAX_ATTACH_WIDTH: i64 = 2000;

/// 支持的媒体扩展名白名单。svg 刻意排除（XML 类格式攻击面较大）。
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mov", "m4v", "mkv"];
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "aac", "ogg", "flac"];

/// 按扩展名分类媒体类型，白名单外返回 None
fn ext_to_media_type(ext: &str) -> Option<&'static str> {
    if IMAGE_EXTS.contains(&ext) {
        return Some("image");
    }
    if VIDEO_EXTS.contains(&ext) {
        return Some("video");
    }
    if AUDIO_EXTS.contains(&ext) {
        return Some("audio");
    }
    None
}

/// HTML → 纯文本（content_text 提取用，供 FTS 索引 / 搜索摘要 / 导出）：
/// - `<figure data-attach-id="N">…</figure>` → `[附件:N]`（与旧占位符格式一致，
///   前端摘要/导出继续用 mask 显示 📎）
/// - `</p>`/`<br>` → 换行；其余标签剥离；常见实体反转义
/// - 纯文本输入（旧数据、无标签）原样保留
fn html_to_text(html: &str) -> String {
    // 非 HTML（旧数据纯文本、无任何标签特征）原样返回，避免折叠用户手动空行
    let html_tag_re =
        Regex::new(r#"</?(p|div|br|strong|em|span|figure|ul|ol|li|h[1-6])\b"#).unwrap();
    if !html_tag_re.is_match(html) {
        return html.to_string();
    }
    let figure_re = Regex::new(r#"<figure[^>]*data-attach-id="(\d+)"[^>]*>.*?</figure>"#).unwrap();
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let mut s = html.to_string();
    s = s.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    // figure 替换自带前后换行，保证占位符独占一行（与旧文本格式一致）；
    // 相邻的 </p>/<p> 会产生连续换行，由下方折叠统一收敛
    s = figure_re.replace_all(&s, "\n[附件:$1]\n").into_owned();
    s = s.replace("</p>", "\n").replace("</div>", "\n");
    s = tag_re.replace_all(&s, "").into_owned();
    s = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    // 折叠连续换行并去掉首尾换行：content_text 仅供 FTS 索引/摘要/导出，空行无意义
    Regex::new(r"\n{2,}")
        .unwrap()
        .replace_all(&s, "\n")
        .trim_matches('\n')
        .to_string()
}

/// 附件扩展名 → MIME 类型（复制到剪贴板时 data URL 前缀用）
fn ext_to_mime(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

/// 生成存储文件名（纳秒时间戳 + 原扩展名；不含任何用户输入字符，防路径注入）
fn new_stored_name(ext: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("a{nanos}.{ext}")
}

/// 一条工作记录，与前端 TS 接口 WorkRecord 字段一一对应。
/// content 为富文本 HTML（编辑器存储格式）；content_text 为纯文本副本
/// （后端写入时自动提取，FTS 索引 / 搜索摘要 / 导出用）
#[derive(Debug, Clone, Serialize)]
pub struct WorkRecord {
    pub id: i64,
    pub record_date: String,
    pub title: String,
    pub content: String,
    pub content_text: String,
    pub status: String,
    pub tags: String,
    pub is_deleted: bool,
    pub is_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct TagStat {
    pub tag: String,
    pub count: i64,
}

/// 一条媒体附件元数据，与前端 TS 接口 Attachment 一一对应。
/// path 为磁盘绝对路径（前端 convertFileSrc 加载预览用）
#[derive(Debug, Clone, Serialize)]
pub struct Attachment {
    pub id: i64,
    pub record_id: i64,
    pub file_name: String,
    pub stored_name: String,
    pub media_type: String,
    pub size: i64,
    pub created_at: String,
    /// 编辑器内显示宽度（px）；None = 默认尺寸
    pub width: Option<i64>,
    pub path: String,
}

/// 数据库封装。连接在 Tauri AppState 中以 Mutex<Db> 持有，跨命令共享。
pub struct Db {
    conn: Connection,
}

const RECORD_COLS: &str =
    "id, record_date, title, content, content_text, status, tags, is_deleted, is_pinned, created_at, updated_at";

/// search JOIN records_fts 时使用的 r. 前缀列清单，
/// 避免 title/content/tags 与 FTS 虚拟表同名列产生歧义。
/// 必须与 RECORD_COLS 保持同序，否则 row_to_record 按索引取值会错位。
const RECORD_COLS_R: &str =
    "r.id, r.record_date, r.title, r.content, r.content_text, r.status, r.tags, r.is_deleted, r.is_pinned, r.created_at, r.updated_at";

fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<WorkRecord> {
    Ok(WorkRecord {
        id: row.get(0)?,
        record_date: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        content_text: row.get(4)?,
        status: row.get(5)?,
        tags: row.get(6)?,
        is_deleted: row.get::<_, i64>(7)? != 0,
        is_pinned: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn row_to_attachment(row: &rusqlite::Row, attach_dir: &Path) -> rusqlite::Result<Attachment> {
    let stored: String = row.get(3)?;
    Ok(Attachment {
        id: row.get(0)?,
        record_id: row.get(1)?,
        file_name: row.get(2)?,
        stored_name: stored.clone(),
        media_type: row.get(4)?,
        size: row.get(5)?,
        created_at: row.get(6)?,
        width: row.get(7)?,
        path: attach_dir.join(&stored).display().to_string(),
    })
}

fn now_str() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

impl Db {
    /// 打开（不存在则创建）数据库文件：应用 PRAGMA、注册 UDF 并执行版本迁移
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("无法打开数据库文件：{}", path.display()))?;
        let mut db = Self { conn };
        db.apply_pragmas()?;
        register_functions(&db.conn)?; // 迁移触发器依赖 cjk_expand，必须先注册
        db.migrate()?;
        Ok(db)
    }

    /// 内存库，仅供单元测试使用
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let mut db = Self {
            conn: Connection::open_in_memory()?,
        };
        db.apply_pragmas()?;
        register_functions(&db.conn)?;
        db.migrate()?;
        Ok(db)
    }

    fn apply_pragmas(&self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(())
    }

    /// 按 user_version 递增执行未应用的迁移脚本，每个版本一个事务
    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (idx, sql) in MIGRATIONS.iter().enumerate() {
            let version = (idx + 1) as i64;
            if version <= current {
                continue;
            }
            let tx = self.conn.transaction().context("开启迁移事务失败")?;
            tx.execute_batch(sql)
                .with_context(|| format!("执行数据库迁移 v{version} 失败"))?;
            // v5 升级时回填 content_text（旧库 content 为纯文本，html_to_text 原样保留）。
            // 回填 UPDATE 会触发新触发器把 FTS 索引同步刷新为纯文本。
            if version == 5 {
                let rows: Vec<(i64, String)> = {
                    let mut stmt = tx.prepare("SELECT id, content FROM records")?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };
                for (id, content) in rows {
                    tx.execute(
                        "UPDATE records SET content_text = ?2 WHERE id = ?1",
                        params![id, html_to_text(&content)],
                    )?;
                }
            }
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// 重建 FTS 索引（恢复备份后调用，保证索引与数据一致）。
    /// 注意：不能使用 FTS5 的 'rebuild' 命令——它直接从内容表读原始列值，
    /// 会绕过 cjk_expand 单字展开；这里手动 DELETE + INSERT SELECT 重建。
    pub fn rebuild_fts(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM records_fts;
             INSERT INTO records_fts (rowid, title, content, tags)
             SELECT id, cjk_expand(title), cjk_expand(content_text), cjk_expand(tags) FROM records;",
        )?;
        Ok(())
    }

    // ---------- 查询 ----------

    /// 单日记录：置顶恒在最前（全局置顶语义），其余按创建时间升序
    pub fn list_by_date(&self, date: &str) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records
             WHERE record_date = ?1 AND is_deleted = 0
             ORDER BY is_pinned DESC, created_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map(params![date], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 日期范围记录（范围视图）：按日期升序，同日期内置顶在前、按创建时间升序
    pub fn list_by_range(&self, from: &str, to: &str) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records
             WHERE is_deleted = 0 AND record_date BETWEEN ?1 AND ?2
             ORDER BY record_date ASC, is_pinned DESC, created_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map(params![from, to], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<WorkRecord>> {
        let row = self
            .conn
            .query_row(
                &format!("SELECT {RECORD_COLS} FROM records WHERE id = ?1"),
                params![id],
                row_to_record,
            )
            .optional()?;
        Ok(row)
    }

    pub fn trash(&self) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records WHERE is_deleted = 1 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map([], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 时间线分页查询：置顶恒在最前（分页时置顶记录集中在第一页），
    /// 其余按日期倒序、同日按 id 倒序
    pub fn timeline(&self, offset: i64, limit: i64) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records WHERE is_deleted = 0
             ORDER BY is_pinned DESC, record_date DESC, id DESC LIMIT ?1 OFFSET ?2"
        ))?;
        let rows = stmt.query_map(params![limit, offset], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 全部置顶且未删除的记录（侧边栏置顶区固定展示，最近更新的在前；
    /// 数量由用户置顶行为决定，预期很小，故不加 LIMIT）
    pub fn list_pinned(&self) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records
             WHERE is_deleted = 0 AND is_pinned = 1
             ORDER BY updated_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 某日期最近 N 条记录（按修改时间倒序、同日 id 倒序兜底；日历悬浮预览用）
    pub fn list_recent_by_date(&self, date: &str, limit: i64) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records
             WHERE record_date = ?1 AND is_deleted = 0
             ORDER BY updated_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![date, limit], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 按标签精确匹配的全部记录（跨日期，标签弹窗视图用）：
    /// 走 tag_contains UDF（复用 split_tags 分割语义），比 LIKE 严格——
    /// 恰含标签才命中，子串如「修复」不误配「修复测试」。
    /// 排序：日期倒序、同日 id 倒序（最近的在前）
    pub fn list_by_tag(&self, tag: &str, limit: i64) -> Result<Vec<WorkRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records
             WHERE is_deleted = 0 AND tag_contains(tags, ?1)
             ORDER BY record_date DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![tag, limit], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 全部有记录日期（日历高亮用）
    pub fn record_dates(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT record_date FROM records WHERE is_deleted = 0")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 全部已使用标签及计数（按使用次数倒序）
    pub fn all_tags(&self) -> Result<Vec<TagStat>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tags FROM records WHERE is_deleted = 0")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut map: HashMap<String, i64> = HashMap::new();
        for tags in rows {
            for t in split_tags(&tags?) {
                *map.entry(t).or_default() += 1;
            }
        }
        let mut v: Vec<TagStat> = map
            .into_iter()
            .map(|(tag, count)| TagStat { tag, count })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
        Ok(v)
    }

    /// FTS5 全文搜索：命中标题/内容/标签，支持日期范围。
    /// 只负责召回记录本体；关键词高亮与内容摘要由前端基于原文渲染。
    pub fn search(
        &self,
        query: &str,
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> Result<Vec<WorkRecord>> {
        let match_expr = build_fts_query(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let f = date_from.unwrap_or("0000-00-00");
        let t = date_to.unwrap_or("9999-99-99");
        let sql = format!(
            "SELECT {RECORD_COLS_R} FROM records_fts
             JOIN records r ON r.id = records_fts.rowid
             WHERE records_fts MATCH ?1 AND r.is_deleted = 0
               AND r.record_date BETWEEN ?2 AND ?3
             ORDER BY r.record_date DESC, r.id DESC
             LIMIT 200"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![match_expr, f, t], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- 写入 ----------

    pub fn create(
        &self,
        record_date: &str,
        title: &str,
        content: &str,
        status: &str,
        tags: &str,
    ) -> Result<WorkRecord> {
        let now = now_str();
        let content_text = html_to_text(content);
        self.conn.execute(
            "INSERT INTO records (record_date, title, content, content_text, status, tags, is_deleted, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
            params![record_date, title, content, content_text, status, tags, now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get(id)?.ok_or_else(|| anyhow!("记录创建后读取失败"))
    }

    pub fn update(
        &self,
        id: i64,
        title: &str,
        content: &str,
        status: &str,
        tags: &str,
    ) -> Result<WorkRecord> {
        let content_text = html_to_text(content);
        let n = self.conn.execute(
            "UPDATE records SET title = ?2, content = ?3, content_text = ?4, status = ?5, tags = ?6, updated_at = ?7
             WHERE id = ?1 AND is_deleted = 0",
            params![id, title, content, content_text, status, tags, now_str()],
        )?;
        if n == 0 {
            bail!("记录不存在或已在回收站中");
        }
        self.get(id)?.ok_or_else(|| anyhow!("记录更新后读取失败"))
    }

    /// 置顶/取消置顶。is_pinned 不参与 FTS 索引，触发器无需联动。
    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<WorkRecord> {
        let n = self.conn.execute(
            "UPDATE records SET is_pinned = ?2, updated_at = ?3 WHERE id = ?1 AND is_deleted = 0",
            params![id, pinned as i64, now_str()],
        )?;
        if n == 0 {
            bail!("记录不存在或已在回收站中");
        }
        self.get(id)?.ok_or_else(|| anyhow!("记录置顶更新后读取失败"))
    }

    /// 软删除：移入回收站（触发器同步移除 FTS 索引）
    pub fn soft_delete(&self, id: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE records SET is_deleted = 1, updated_at = ?2 WHERE id = ?1 AND is_deleted = 0",
            params![id, now_str()],
        )?;
        if n == 0 {
            bail!("记录不存在或已在回收站中");
        }
        Ok(())
    }

    pub fn restore(&self, id: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE records SET is_deleted = 0, updated_at = ?2 WHERE id = ?1 AND is_deleted = 1",
            params![id, now_str()],
        )?;
        if n == 0 {
            bail!("记录不存在或不在回收站中");
        }
        Ok(())
    }

    /// 物理删除（仅回收站中可执行），触发器同步清理 FTS 索引；
    /// 附件行由外键 ON DELETE CASCADE 自动删除，磁盘文件在此处尽力清理
    pub fn purge(&self, id: i64, attach_dir: &Path) -> Result<()> {
        // 先收集附件（含 path）供删行后清理磁盘文件
        let atts = self.attachments(id, attach_dir)?;
        let n = self.conn.execute("DELETE FROM records WHERE id = ?1 AND is_deleted = 1", params![id])?;
        if n == 0 {
            bail!("记录不存在或不在回收站中");
        }
        remove_attach_files_best_effort(&atts);
        Ok(())
    }

    /// 清空回收站，返回删除条数（附件磁盘文件同样尽力清理）
    pub fn empty_trash(&self, attach_dir: &Path) -> Result<i64> {
        let atts = self.attachments_of_deleted(attach_dir)?;
        let n = self.conn.execute("DELETE FROM records WHERE is_deleted = 1", [])?;
        remove_attach_files_best_effort(&atts);
        Ok(n as i64)
    }

    // ---------- 媒体附件 ----------

    /// 某条记录的全部附件（按创建时间升序）
    pub fn attachments(&self, record_id: i64, attach_dir: &Path) -> Result<Vec<Attachment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, record_id, file_name, stored_name, media_type, size, created_at, width
             FROM attachments WHERE record_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![record_id], |row| row_to_attachment(row, attach_dir))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 全部已删除记录的附件（清空回收站时清理磁盘文件用）
    fn attachments_of_deleted(&self, attach_dir: &Path) -> Result<Vec<Attachment>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.record_id, a.file_name, a.stored_name, a.media_type, a.size, a.created_at, a.width
             FROM attachments a JOIN records r ON r.id = a.record_id
             WHERE r.is_deleted = 1 ORDER BY a.id ASC",
        )?;
        let rows = stmt.query_map([], |row| row_to_attachment(row, attach_dir))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 全部附件元数据（备份打包媒体文件用）
    fn all_attachments(&self, attach_dir: &Path) -> Result<Vec<Attachment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, record_id, file_name, stored_name, media_type, size, created_at, width
             FROM attachments ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| row_to_attachment(row, attach_dir))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 添加附件：校验类型与大小 → 复制文件到附件目录 → 写入元数据。
    /// 元数据插行失败时清理已复制的文件，不留下孤儿。
    pub fn add_attachment(&self, record_id: i64, src: &Path, attach_dir: &Path) -> Result<Attachment> {
        let rec = self
            .get(record_id)?
            .ok_or_else(|| anyhow!("记录不存在"))?;
        if rec.is_deleted {
            bail!("记录已在回收站中，不能添加附件");
        }
        let file_name = src
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("无法读取文件名：{}", src.display()))?
            .to_string();
        let ext = src
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .ok_or_else(|| anyhow!("无法识别文件类型（缺少扩展名）：{file_name}"))?;
        let media_type = ext_to_media_type(&ext).ok_or_else(|| {
            anyhow!(
                "不支持的文件格式：.{ext}（图片支持 jpg/png/gif/webp/bmp，视频支持 mp4/webm/mov/m4v/mkv，音频支持 mp3/wav/m4a/aac/ogg/flac）"
            )
        })?;
        let meta = std::fs::metadata(src)
            .with_context(|| format!("无法读取文件：{}", src.display()))?;
        if !meta.is_file() {
            bail!("不是普通文件：{}", src.display());
        }
        if meta.len() > MAX_ATTACH_SIZE_BYTES {
            bail!("文件过大：{file_name}（单文件上限 500MB）");
        }
        std::fs::create_dir_all(attach_dir)
            .with_context(|| format!("无法创建附件目录：{}", attach_dir.display()))?;
        let stored_name = new_stored_name(&ext);
        let dest = attach_dir.join(&stored_name);
        std::fs::copy(src, &dest)
            .with_context(|| format!("复制文件失败：{} -> {}", src.display(), dest.display()))?;
        let created_at = now_str();
        let inserted = self.conn.execute(
            "INSERT INTO attachments (record_id, file_name, stored_name, media_type, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![record_id, file_name.clone(), stored_name.clone(), media_type, meta.len() as i64, created_at],
        );
        let id = match inserted {
            Ok(_) => self.conn.last_insert_rowid(),
            Err(e) => {
                // 元数据写库失败：清理刚复制的文件，避免孤儿文件
                let _ = std::fs::remove_file(&dest);
                return Err(e.into());
            }
        };
        Ok(Attachment {
            id,
            record_id,
            file_name,
            stored_name,
            media_type: media_type.to_string(),
            size: meta.len() as i64,
            created_at,
            width: None,
            path: dest.display().to_string(),
        })
    }

    /// 添加附件（字节版）：剪贴板粘贴等内容直接落盘存储，校验与路径版一致。
    pub fn add_attachment_bytes(
        &self,
        record_id: i64,
        file_name: &str,
        data: &[u8],
        attach_dir: &Path,
    ) -> Result<Attachment> {
        let rec = self
            .get(record_id)?
            .ok_or_else(|| anyhow!("记录不存在"))?;
        if rec.is_deleted {
            bail!("记录已在回收站中，不能添加附件");
        }
        if file_name.is_empty() || file_name.len() > 255 {
            bail!("文件名无效（长度需在 1~255 字节之间）");
        }
        let ext = Path::new(file_name)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .ok_or_else(|| anyhow!("无法识别文件类型（缺少扩展名）：{file_name}"))?;
        let media_type = ext_to_media_type(&ext).ok_or_else(|| {
            anyhow!(
                "不支持的文件格式：.{ext}（图片支持 jpg/png/gif/webp/bmp，视频支持 mp4/webm/mov/m4v/mkv，音频支持 mp3/wav/m4a/aac/ogg/flac）"
            )
        })?;
        if data.is_empty() {
            bail!("文件内容为空：{file_name}");
        }
        if data.len() as u64 > MAX_ATTACH_SIZE_BYTES {
            bail!("文件过大：{file_name}（单文件上限 500MB）");
        }
        std::fs::create_dir_all(attach_dir)
            .with_context(|| format!("无法创建附件目录：{}", attach_dir.display()))?;
        let stored_name = new_stored_name(&ext);
        let dest = attach_dir.join(&stored_name);
        std::fs::write(&dest, data)
            .with_context(|| format!("写入附件文件失败：{}", dest.display()))?;
        let created_at = now_str();
        let inserted = self.conn.execute(
            "INSERT INTO attachments (record_id, file_name, stored_name, media_type, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![record_id, file_name, stored_name.clone(), media_type, data.len() as i64, created_at],
        );
        let id = match inserted {
            Ok(_) => self.conn.last_insert_rowid(),
            Err(e) => {
                // 元数据写库失败：清理刚写入的文件，避免孤儿文件
                let _ = std::fs::remove_file(&dest);
                return Err(e.into());
            }
        };
        Ok(Attachment {
            id,
            record_id,
            file_name: file_name.to_string(),
            stored_name,
            media_type: media_type.to_string(),
            size: data.len() as i64,
            created_at,
            width: None,
            path: dest.display().to_string(),
        })
    }

    /// 删除附件：删库行成功后删磁盘文件
    pub fn delete_attachment(&self, id: i64, attach_dir: &Path) -> Result<()> {
        let stored: Option<String> = self
            .conn
            .query_row("SELECT stored_name FROM attachments WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        let stored = stored.ok_or_else(|| anyhow!("附件不存在"))?;
        let n = self.conn.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
        if n == 0 {
            bail!("附件不存在");
        }
        let p = attach_dir.join(&stored);
        if p.exists() {
            std::fs::remove_file(&p)
                .with_context(|| format!("删除附件文件失败：{}", p.display()))?;
        }
        Ok(())
    }

    /// 设置附件显示宽度（编辑器拖拽调整媒体块大小后持久化）
    pub fn set_attachment_width(&self, id: i64, width: i64) -> Result<()> {
        if !(MIN_ATTACH_WIDTH..=MAX_ATTACH_WIDTH).contains(&width) {
            bail!("附件宽度需在 {MIN_ATTACH_WIDTH}~{MAX_ATTACH_WIDTH} 像素之间");
        }
        let n = self.conn.execute(
            "UPDATE attachments SET width = ?2 WHERE id = ?1",
            params![id, width],
        )?;
        if n == 0 {
            bail!("附件不存在或已被删除");
        }
        Ok(())
    }

    /// 附件内容 → data URL（媒体块 Ctrl+C 复制到剪贴板用）。
    /// 超过 100MB 拒绝（剪贴板承载大文件会导致系统卡顿）
    pub fn attachment_data_url(&self, id: i64, attach_dir: &Path) -> Result<String> {
        const MAX_COPY_BYTES: u64 = 100 * 1024 * 1024;
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT stored_name, file_name FROM attachments WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (stored, file_name) = row.ok_or_else(|| anyhow!("附件不存在"))?;
        let p = attach_dir.join(&stored);
        let bytes =
            std::fs::read(&p).with_context(|| format!("读取附件文件失败：{}", p.display()))?;
        if bytes.len() as u64 > MAX_COPY_BYTES {
            bail!("附件过大（超过 100MB），无法复制到剪贴板");
        }
        let mime = Path::new(&file_name)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| ext_to_mime(&s.to_ascii_lowercase()))
            .unwrap_or("application/octet-stream");
        use base64::Engine;
        Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }

    // ---------- 导出 / 备份 / 恢复 ----------

    /// 生成 Markdown 报告内容（daily/weekly/monthly/all），日期按升序
    pub fn export_markdown(&self, kind: &str, date: &str) -> Result<String> {
        let anchor = NaiveDate::parse_from_str(date, "%Y-%m-%d").context("导出日期无效")?;
        let (from, to) = match kind {
            "daily" => (Some(anchor), Some(anchor)),
            "weekly" => {
                // 周报口径：所选日期所在周的周一 ~ 周日（业界通用口径）
                let start = anchor - Duration::days(anchor.weekday().num_days_from_monday() as i64);
                (Some(start), Some(start + Duration::days(6)))
            }
            "monthly" => {
                let start = anchor
                    .with_day(1)
                    .ok_or_else(|| anyhow!("无法计算月初日期"))?;
                let next = if anchor.month() == 12 {
                    NaiveDate::from_ymd_opt(anchor.year() + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(anchor.year(), anchor.month() + 1, 1)
                }
                .ok_or_else(|| anyhow!("无法计算下月初日期"))?;
                (Some(start), Some(next - Duration::days(1)))
            }
            _ => (None, None),
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECORD_COLS} FROM records
             WHERE is_deleted = 0
               AND (?1 IS NULL OR record_date >= ?1)
               AND (?2 IS NULL OR record_date <= ?2)
             ORDER BY record_date ASC, id ASC"
        ))?;
        // NaiveDate 未实现 ToSql，转成 yyyy-MM-dd 字符串（字典序即日期序）再绑定
        let from_s = from.map(|d| d.to_string());
        let to_s = to.map(|d| d.to_string());
        let rows: Vec<WorkRecord> = stmt
            .query_map(params![from_s, to_s], row_to_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        build_markdown(kind, from, to, &rows)
    }

    /// 备份：数据库一致性快照 + 全部媒体附件打包为单个 zip（流式写入，防大文件 OOM）。
    /// 只打包数据库引用的附件，磁盘上的孤儿文件自然排除。
    pub fn backup_to(&self, dest: &Path, attach_dir: &Path) -> Result<()> {
        // 1. 数据库快照到临时文件（SQLite 在线备份 API，自动处理 WAL）
        let tmp_db = std::env::temp_dir().join(format!("wj-backup-{}.db", new_stored_name("db")));
        let result = (|| -> Result<()> {
            {
                let mut dest_conn = Connection::open(&tmp_db)
                    .with_context(|| format!("无法创建备份临时文件：{}", tmp_db.display()))?;
                let backup = rusqlite::backup::Backup::new(&self.conn, &mut dest_conn)?;
                backup.run_to_completion(64, std::time::Duration::from_millis(10), None)?;
            }
            // 2. zip 打包：数据库快照 + attachments/ 下的媒体文件
            let out = File::create(dest)
                .with_context(|| format!("无法创建备份文件：{}", dest.display()))?;
            let mut zip = zip::ZipWriter::new(out);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("work-journal.db", opts)?;
            let mut db_file = File::open(&tmp_db)?;
            std::io::copy(&mut db_file, &mut zip)?;
            for a in self.all_attachments(attach_dir)? {
                let p = attach_dir.join(&a.stored_name);
                let mut f = File::open(&p)
                    .with_context(|| format!("附件文件缺失，备份中止：{}", p.display()))?;
                zip.start_file(format!("attachments/{}", a.stored_name), opts)?;
                std::io::copy(&mut f, &mut zip)?;
            }
            zip.finish()?;
            Ok(())
        })();
        let _ = std::fs::remove_file(&tmp_db);
        result
    }

    /// 恢复：支持两种备份格式——.zip（新格式：数据库快照 + 媒体附件）与
    /// .db（旧格式：纯数据库，媒体目录不动）。先校验再覆盖，最后重建 FTS。
    /// 返回恢复后的有效记录数。
    pub fn restore_from(&mut self, src: &Path, attach_dir: &Path) -> Result<i64> {
        let is_zip = src
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        if is_zip {
            self.restore_from_zip(src, attach_dir)
        } else {
            self.restore_from_db(src)
        }
    }

    /// 新格式 .zip：先解出数据库快照恢复，再解出 attachments/ 覆盖附件目录
    fn restore_from_zip(&mut self, src: &Path, attach_dir: &Path) -> Result<i64> {
        let file = File::open(src)
            .with_context(|| format!("无法打开备份文件：{}", src.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| "无法解析备份文件（文件损坏或不是工作日志备份）")?;
        let tmp_db = std::env::temp_dir().join(format!("wj-restore-{}.db", new_stored_name("db")));
        let result = (|| -> Result<i64> {
            {
                let mut entry = archive
                    .by_name("work-journal.db")
                    .with_context(|| "备份中缺少数据库文件，不是有效的工作日志备份")?;
                let mut out = File::create(&tmp_db)?;
                std::io::copy(&mut entry, &mut out)?;
            }
            let count = self.restore_from_db(&tmp_db)?;
            // 恢复媒体附件：条目名 strip_prefix 后拒绝含 ..、/、\ 的路径穿越条目；
            // 同 stored_name 覆盖旧文件（备份即当时的完整状态）
            std::fs::create_dir_all(attach_dir)
                .with_context(|| format!("无法创建附件目录：{}", attach_dir.display()))?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name()?.to_string();
                let Some(rest) = name.strip_prefix("attachments/") else {
                    continue;
                };
                if rest.is_empty() || rest.contains("..") || rest.contains('/') || rest.contains('\\') {
                    continue; // 拒绝路径穿越条目
                }
                let mut out = File::create(attach_dir.join(rest))?;
                std::io::copy(&mut entry, &mut out)?;
            }
            Ok(count)
        })();
        let _ = std::fs::remove_file(&tmp_db);
        result
    }

    /// 旧格式（纯 .db）：先校验备份文件有效性，再用备份 API 覆盖当前库，最后重建 FTS
    fn restore_from_db(&mut self, src: &Path) -> Result<i64> {
        let src_conn = Connection::open(src)
            .with_context(|| format!("无法打开备份文件：{}", src.display()))?;
        src_conn
            .query_row("SELECT 1 FROM records LIMIT 1", [], |_| Ok(()))
            .with_context(|| "所选文件不是有效的工作日志备份（缺少 records 表）")?;
        let count: i64 = src_conn.query_row(
            "SELECT COUNT(*) FROM records WHERE is_deleted = 0",
            [],
            |r| r.get(0),
        )?;
        // backup 持有 &mut self.conn，需在借用结束（drop）后再重建 FTS
        {
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut self.conn)?;
            backup.run_to_completion(64, std::time::Duration::from_millis(10), None)?;
        }
        // 备份 API 覆盖整个库（连 schema 与 user_version），恢复旧版本（如 v1 无 is_pinned 列）
        // 的备份后必须补跑迁移，否则后续查询引用新列会报错
        self.migrate()?;
        self.rebuild_fts()?;
        Ok(count)
    }
}

/// 尽力删除附件磁盘文件（行已删或删除失败时调用；失败仅提示，不中断主流程）。
/// 遗留的孤儿文件不会被打进备份（备份只打包数据库引用的附件）。
fn remove_attach_files_best_effort(atts: &[Attachment]) {
    for a in atts {
        let p = Path::new(&a.path);
        if p.exists() {
            if let Err(e) = std::fs::remove_file(p) {
                eprintln!("清理附件文件失败（遗留孤儿文件）：{}：{e:#}", p.display());
            }
        }
    }
}

// ---------- 纯函数：标签 / 校验 / FTS 查询构造 / Markdown 组装 ----------

/// 拆分标签（兼容中英文逗号、分号）
pub fn split_tags(tags: &str) -> Vec<String> {
    tags.split([',', '，', ';', '；'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 规范化标签：去空白、去重、限制单个标签长度
pub fn normalize_tags(tags: &str) -> Result<String> {
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for t in split_tags(tags) {
        if t.chars().count() > MAX_TAG_LEN {
            bail!("单个标签不能超过 {MAX_TAG_LEN} 个字符：{t}");
        }
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    Ok(out.join(","))
}

pub fn validate_date(date: &str) -> Result<()> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| anyhow!("日期格式无效，应为 yyyy-MM-dd：{date}"))
}

/// 内容中的附件占位符 `[附件:123]` 转为可读文本（导出报告、纯文本展示场景用）
fn mask_attachment_markers(s: &str) -> String {
    const MARK: &str = "[附件:";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(MARK) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + MARK.len()..];
        match after.find(']') {
            Some(end) => {
                out.push_str("📎 附件");
                rest = &after[end + 1..];
            }
            None => {
                // 未闭合的占位符原样保留
                out.push_str(MARK);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 解析前端粘贴传来的 data URL（data:image/png;base64,xxx）：校验前缀、
/// base64 解码并返回字节。解码前按 base64 长度预检大小，防止超大输入撑爆内存。
pub fn decode_data_url(data_url: &str) -> Result<Vec<u8>> {
    let rest = data_url
        .strip_prefix("data:")
        .ok_or_else(|| anyhow!("无效的粘贴数据（缺少 data: 前缀）"))?;
    let (_meta, b64) = rest
        .split_once(',')
        .ok_or_else(|| anyhow!("无效的粘贴数据（无法解析）"))?;
    if b64.is_empty() {
        bail!("粘贴数据为空");
    }
    // base64 每 4 字符编码 3 字节：预检解码后大小，超限直接拒绝
    if (b64.len() as u64) * 3 / 4 > MAX_ATTACH_SIZE_BYTES {
        bail!("文件过大（单文件上限 500MB）");
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("粘贴数据 base64 解码失败")
}

pub fn validate_status(status: &str) -> Result<()> {
    if matches!(status, "pending" | "completed") {
        Ok(())
    } else {
        bail!("状态值无效：{status}")
    }
}

/// 判定是否 CJK 字符（表意文字主区、扩展 A、兼容表意、扩展 B~F）。
/// 这些字符会被 unicode61 视为连续串的一部分，需要展开为单字。
fn is_cjk_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

/// 连续 CJK 字符之间插入空格，使 FTS5 unicode61 按单字建索引。
/// 非 CJK 字符（英文/数字/标点）原样保留，由 tokenizer 自行分词。
pub fn cjk_expand_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.chars().count() / 2);
    let mut prev_cjk = false;
    for c in s.chars() {
        let is_cjk = is_cjk_char(c);
        if is_cjk && prev_cjk {
            out.push(' ');
        }
        out.push(c);
        prev_cjk = is_cjk;
    }
    out
}

/// 注册 FTS 索引所需的 UDF（每个连接注册一次，必须在迁移/触发器执行前完成）
fn register_functions(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "cjk_expand",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(cjk_expand_str(&ctx.get::<String>(0)?)),
    )?;
    // 标签精确匹配：tags 字段是否为「恰含某标签」（复用 split_tags 的分割语义，
    // 与前端 filteredRecords 一致）。不用 LIKE 的原因：% 通配符可跨越分隔逗号，
    // 「%,a,%」会误配 tags="ab,b" 这类记录，无法做到严格精确。
    conn.create_scalar_function(
        "tag_contains",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let tags: String = ctx.get(0)?;
            let tag: String = ctx.get(1)?;
            Ok(split_tags(&tags).iter().any(|t| t == &tag) as i64)
        },
    )?;
    Ok(())
}

/// 空白分隔段内的一个子串 → FTS5 表达式片段：
/// CJK 子串展开为单字短语（"超 时"），非 CJK 子串为转义后的前缀词（"API"*）。
fn push_query_part(out: &mut String, part: &mut String, is_cjk: bool) {
    if part.is_empty() {
        return;
    }
    // 多个片段间以空格分隔（FTS5 中相邻 token 为隐式 AND）
    if !out.is_empty() {
        out.push(' ');
    }
    if is_cjk {
        out.push('"');
        out.push_str(&cjk_expand_str(part));
        out.push('"');
    } else {
        // 双引号转义为 ""，并用引号包裹整词，规避 FTS5 特殊语法（AND/OR/NOT/括号等）注入
        out.push('"');
        out.push_str(&part.replace('"', "\"\""));
        out.push_str("\"*");
    }
    part.clear();
}

/// 构造 FTS5 MATCH 表达式（与 cjk_expand 单字索引配套）：
/// 1. 按空白分词；
/// 2. 每段内的连续 CJK 字符展开为单字短语，保证中文任意子串（词中缀）可命中；
/// 3. 非 CJK 串作为带引号的前缀词，支持英文/数字前缀匹配；
/// 4. 多个片段间以空格连接（FTS5 默认 AND 语义）。
pub fn build_fts_query(input: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for seg in input.split_whitespace() {
        let mut expr = String::new();
        let mut part = String::new();
        let mut part_is_cjk: Option<bool> = None;
        for c in seg.chars() {
            let is_cjk = is_cjk_char(c);
            if let Some(prev) = part_is_cjk {
                if prev != is_cjk {
                    push_query_part(&mut expr, &mut part, prev);
                }
            }
            part_is_cjk = Some(is_cjk);
            part.push(c);
        }
        if let Some(prev) = part_is_cjk {
            push_query_part(&mut expr, &mut part, prev);
        }
        if !expr.is_empty() {
            parts.push(expr);
        }
    }
    parts.join(" ")
}

fn weekday_cn(d: NaiveDate) -> &'static str {
    const W: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    W[d.weekday().num_days_from_monday() as usize]
}

fn fmt_date_cn(d: NaiveDate) -> String {
    format!("{}（{}）", d.format("%Y-%m-%d"), weekday_cn(d))
}

/// 标题中的换行与 Markdown 符号会破坏文档结构，做安全处理
fn sanitize_title(title: &str) -> String {
    let t = title.replace(['\n', '\r'], " ").trim().to_string();
    if t.is_empty() {
        "（无标题）".to_string()
    } else {
        t
    }
}

fn build_markdown(
    kind: &str,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    records: &[WorkRecord],
) -> Result<String> {
    let mut out = String::new();
    let heading = match kind {
        "daily" => format!("# 工作日报 · {}", fmt_date_cn(from.unwrap_or_default())),
        "weekly" => format!(
            "# 工作周报 · {} ~ {}",
            fmt_date_cn(from.unwrap_or_default()),
            fmt_date_cn(to.unwrap_or_default())
        ),
        "monthly" => {
            let d = from.unwrap_or_default();
            format!("# 工作月报 · {}年{}月", d.year(), d.month())
        }
        _ => "# 工作记录总览".to_string(),
    };
    let completed = records.iter().filter(|r| r.status == "completed").count();
    out.push_str(&heading);
    out.push('\n');
    out.push_str(&format!(
        "> 生成时间：{} · 共 {} 条 · 已完成 {}\n",
        now_str(),
        records.len(),
        completed
    ));
    if records.is_empty() {
        out.push_str("\n（该范围内暂无记录）\n");
        return Ok(out);
    }
    out.push_str("\n---\n");

    let mut last_date = String::new();
    let mut idx = 0usize;
    for r in records {
        if kind != "daily" && r.record_date != last_date {
            last_date = r.record_date.clone();
            let d = NaiveDate::parse_from_str(&last_date, "%Y-%m-%d").unwrap_or_default();
            out.push_str(&format!("\n## {}\n", fmt_date_cn(d)));
        }
        idx += 1;
        out.push_str(&format!("\n### {}. {}\n", idx, sanitize_title(&r.title)));
        out.push_str(if r.status == "completed" {
            "状态：✅ 已完成\n"
        } else {
            "状态：⏳ 待完成\n"
        });
        let tags = split_tags(&r.tags);
        if !tags.is_empty() {
            let joined = tags.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(" ");
            out.push_str(&format!("标签：{joined}\n"));
        }
        // 导出用纯文本副本（content_text），附件占位符显示为 📎
        let content = mask_attachment_markers(r.content_text.trim_end());
        if !content.is_empty() {
            out.push_str(&format!("\n{content}\n"));
        }
        out.push_str("\n---\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "work-journal-test-{}-{tag}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fts5_available_and_chinese_search_works() {
        let db = Db::open_in_memory().unwrap();
        let r = db
            .create(
                "2026-10-09",
                "修复登录接口超时",
                "排查发现网关超时配置过小，已调大并压测通过",
                "completed",
                "后端,修复",
            )
            .unwrap();
        // 中文按单字建索引：词中缀（「超时」在标题/内容中段）也能命中
        assert_eq!(db.search("超时", None, None).unwrap().len(), 1);
        // 标题开头词命中
        assert_eq!(db.search("修复登录", None, None).unwrap().len(), 1);
        // 标签也可命中
        assert_eq!(db.search("修复", None, None).unwrap().len(), 1);
        // 无结果时为空
        assert!(db.search("不存在的词", None, None).unwrap().is_empty());
        // 软删除后从索引移除，搜索不可见
        db.soft_delete(r.id).unwrap();
        assert!(db.search("超时", None, None).unwrap().is_empty());
    }

    #[test]
    fn cjk_expand_splits_single_chars() {
        assert_eq!(cjk_expand_str("修复登录"), "修 复 登 录");
        // 非 CJK 原样保留（英文/数字/标点），由 tokenizer 自行分词
        assert_eq!(cjk_expand_str("abc修复"), "abc修 复");
        assert_eq!(cjk_expand_str("修复 API 2.0"), "修 复 API 2.0");
        assert_eq!(cjk_expand_str(""), "");
    }

    #[test]
    fn fts_query_builder_expands_cjk_and_escapes_special_chars() {
        // 中文词 → 单字短语
        assert_eq!(build_fts_query("超时"), "\"超 时\"");
        // 多词以空格连接（隐式 AND）；英文词带引号前缀
        assert_eq!(build_fts_query("修复 API"), "\"修 复\" \"API\"*");
        // 中英混合段分别处理
        assert_eq!(build_fts_query("API超时"), "\"API\"* \"超 时\"");
        // FTS5 特殊字符转义，规避语法注入
        assert_eq!(build_fts_query("a\"b OR c"), "\"a\"\"b\"* \"OR\"* \"c\"*");
        assert_eq!(build_fts_query(""), "");
    }

    #[test]
    fn crud_soft_delete_restore_purge_roundtrip() {
        let dir = tmp_dir("crud");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let r = db
            .create("2026-10-09", "标题", "内容", "pending", "a,b")
            .unwrap();
        assert_eq!(db.list_by_date("2026-10-09").unwrap().len(), 1);
        let updated = db
            .update(r.id, "新标题", "新内容", "completed", "a,c")
            .unwrap();
        assert_eq!(updated.title, "新标题");
        assert_eq!(updated.status, "completed");
        // 软删除 → 列表/时间线不可见，回收站可见
        db.soft_delete(r.id).unwrap();
        assert!(db.list_by_date("2026-10-09").unwrap().is_empty());
        assert_eq!(db.trash().unwrap().len(), 1);
        // 恢复 → 回到列表
        db.restore(r.id).unwrap();
        assert_eq!(db.list_by_date("2026-10-09").unwrap().len(), 1);
        // 再次删除并物理清除
        db.soft_delete(r.id).unwrap();
        db.purge(r.id, &attach_dir).unwrap();
        assert!(db.trash().unwrap().is_empty());
        // 非法 id 操作报错而非 panic
        assert!(db.soft_delete(9999).is_err());
        assert!(db.purge(9999, &attach_dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migration_is_idempotent_across_reopen() {
        let dir = tmp_dir("migration");
        let path = dir.join("m.db");
        {
            let db = Db::open(&path).unwrap();
            db.create("2026-10-09", "a", "", "pending", "").unwrap();
        }
        {
            // 第二次打开：迁移应跳过已执行版本，数据完好
            let db = Db::open(&path).unwrap();
            let ver: i64 = db
                .conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(ver, MIGRATIONS.len() as i64);
            assert_eq!(db.list_by_date("2026-10-09").unwrap().len(), 1);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_respects_date_range() {
        let db = Db::open_in_memory().unwrap();
        db.create("2026-10-01", "周会纪要", "讨论进度", "pending", "")
            .unwrap();
        db.create("2026-11-01", "周会纪要", "讨论预算", "pending", "")
            .unwrap();
        let all = db.search("周会", None, None).unwrap();
        assert_eq!(all.len(), 2);
        let oct = db.search("周会", Some("2026-10-01"), Some("2026-10-31")).unwrap();
        assert_eq!(oct.len(), 1);
        assert_eq!(oct[0].record_date, "2026-10-01");
    }

    #[test]
    fn pinned_sorts_first_in_day_list_and_timeline() {
        let db = Db::open_in_memory().unwrap();
        // 同日两条：后置顶的仍排最前
        let r1 = db.create("2026-10-09", "普通记录", "", "pending", "").unwrap();
        let r2 = db.create("2026-10-09", "置顶记录", "", "pending", "").unwrap();
        db.set_pinned(r1.id, true).unwrap();
        let day = db.list_by_date("2026-10-09").unwrap();
        assert_eq!(day[0].id, r1.id);
        assert!(day[0].is_pinned);
        assert_eq!(day[1].id, r2.id);
        // 跨日期：两条置顶记录排在最前（置顶之间按日期倒序，10-09 在 10-01 前）
        let r3 = db.create("2026-10-01", "跨日期置顶", "", "pending", "").unwrap();
        db.set_pinned(r3.id, true).unwrap();
        let tl = db.timeline(0, 100).unwrap();
        assert_eq!(tl[0].id, r1.id);
        assert_eq!(tl[1].id, r3.id);
        assert_eq!(tl[2].id, r2.id);
        // 取消置顶后恢复普通排序（日期倒序）
        db.set_pinned(r3.id, false).unwrap();
        let tl2 = db.timeline(0, 100).unwrap();
        assert_eq!(tl2[0].id, r1.id);
        assert_eq!(tl2[1].id, r2.id);
        assert_eq!(tl2[2].id, r3.id);
    }

    #[test]
    fn list_by_range_inclusive_bounds_and_order() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create("2026-10-01", "起点", "", "pending", "").unwrap();
        let b = db.create("2026-10-05", "中间", "", "pending", "").unwrap();
        let c = db.create("2026-10-10", "终点", "", "pending", "").unwrap();
        let d = db.create("2026-10-11", "范围外", "", "pending", "").unwrap();
        db.soft_delete(b.id).unwrap(); // 软删除不出现在范围结果中
        let rows = db.list_by_range("2026-10-01", "2026-10-10").unwrap();
        // 含两端边界、排除范围外与软删除，按日期升序
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, a.id);
        assert_eq!(rows[1].id, c.id);
        // 同日置顶在前
        let e = db.create("2026-10-10", "同日置顶", "", "pending", "").unwrap();
        db.set_pinned(e.id, true).unwrap();
        let rows2 = db.list_by_range("2026-10-10", "2026-10-10").unwrap();
        assert_eq!(rows2[0].id, e.id);
        assert_eq!(rows2[1].id, c.id);
        let _ = d;
    }

    #[test]
    fn set_pinned_toggles_and_rejects_invalid() {
        let db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "标题", "", "pending", "").unwrap();
        assert!(!r.is_pinned);
        let pinned = db.set_pinned(r.id, true).unwrap();
        assert!(pinned.is_pinned);
        // 普通 update（自动保存路径）不重置置顶状态
        let updated = db.update(r.id, "新标题", "", "pending", "").unwrap();
        assert!(updated.is_pinned);
        // 翻转回未置顶
        let unpinned = db.set_pinned(r.id, false).unwrap();
        assert!(!unpinned.is_pinned);
        // 不存在与已软删除的记录报错
        assert!(db.set_pinned(9999, true).is_err());
        db.soft_delete(r.id).unwrap();
        assert!(db.set_pinned(r.id, true).is_err());
    }

    #[test]
    fn list_pinned_returns_only_pinned_and_not_deleted() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create("2026-10-01", "置顶甲", "", "pending", "").unwrap();
        let b = db.create("2026-10-09", "置顶乙", "", "pending", "").unwrap();
        let c = db.create("2026-10-09", "普通", "", "pending", "").unwrap();
        db.set_pinned(a.id, true).unwrap();
        db.set_pinned(b.id, true).unwrap();
        let d = db.create("2026-10-05", "已删置顶", "", "pending", "").unwrap();
        db.set_pinned(d.id, true).unwrap();
        db.soft_delete(d.id).unwrap();
        let rows = db.list_pinned().unwrap();
        // 只含置顶且未删除的两条，最近更新的在前（b 后置顶，updated_at 更大）
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, b.id);
        assert_eq!(rows[1].id, a.id);
        // 取消置顶后从列表消失
        db.set_pinned(b.id, false).unwrap();
        let rows2 = db.list_pinned().unwrap();
        assert_eq!(rows2.len(), 1);
        assert_eq!(rows2[0].id, a.id);
        let _ = c;
    }

    #[test]
    fn restore_v1_backup_runs_migration() {
        // 手工构造 v1 版备份（无 is_pinned 列，user_version = 1）
        let dir = tmp_dir("restore-v1");
        let src = dir.join("v1.db");
        {
            let conn = rusqlite::Connection::open(&src).unwrap();
            // v1 迁移 SQL 中的触发器引用 cjk_expand，执行前必须注册（与 Db::open 同序）
            register_functions(&conn).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO records (record_date, title, content, status, tags, is_deleted, created_at, updated_at)
                 VALUES ('2026-10-09', '旧数据', '', 'pending', '', 0, '2026-10-09 10:00:00', '2026-10-09 10:00:00')",
                [],
            )
            .unwrap();
        }
        let mut db = Db::open_in_memory().unwrap();
        let count = db.restore_from(&src, &dir.join("attachments")).unwrap();
        assert_eq!(count, 1);
        // 恢复后迁移补齐 v2：user_version 到位、is_pinned 列可查询且默认未置顶
        let ver: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ver, MIGRATIONS.len() as i64);
        let rows = db.list_by_date("2026-10-09").unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_pinned);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn export_daily_markdown_contains_all_fields() {
        let db = Db::open_in_memory().unwrap();
        db.create(
            "2026-10-09",
            "修 bug",
            "第一行\n第二行",
            "completed",
            "后端, 修复",
        )
        .unwrap();
        let md = db.export_markdown("daily", "2026-10-09").unwrap();
        assert!(md.contains("工作日报"));
        assert!(md.contains("✅ 已完成"));
        assert!(md.contains("修 bug"));
        assert!(md.contains("第一行"));
        assert!(md.contains("`后端`"));
        // 周报口径：2026-10-09 是周五，周报应覆盖 10-05(周一) ~ 10-11(周日)
        let wk = db.export_markdown("weekly", "2026-10-09").unwrap();
        assert!(wk.contains("2026-10-05"));
        assert!(wk.contains("2026-10-11"));
        // 无记录日期导出不报错
        let empty = db.export_markdown("daily", "2020-01-01").unwrap();
        assert!(empty.contains("暂无记录"));
    }

    #[test]
    fn backup_and_restore_roundtrip() {
        let dir = tmp_dir("backup");
        let attach_dir = dir.join("attachments");
        let file = dir.join("bak.zip");
        let mut db = Db::open_in_memory().unwrap();
        db.create("2026-10-09", "原始数据", "", "completed", "x")
            .unwrap();
        db.backup_to(&file, &attach_dir).unwrap();
        // 备份后再新增，恢复后应回到备份时点
        db.create("2026-10-10", "备份后新增", "", "pending", "")
            .unwrap();
        let count = db.restore_from(&file, &attach_dir).unwrap();
        assert_eq!(count, 1);
        assert!(db.list_by_date("2026-10-10").unwrap().is_empty());
        assert_eq!(db.list_by_date("2026-10-09").unwrap().len(), 1);
        // 恢复后 FTS 可用
        assert_eq!(db.search("原始", None, None).unwrap().len(), 1);
        // 无效文件被拒绝
        let bad = dir.join("bad.zip");
        std::fs::write(&bad, b"not a zip file").unwrap();
        assert!(db.restore_from(&bad, &attach_dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_add_validates_and_copies() {
        let dir = tmp_dir("att-add");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "带附件", "", "pending", "").unwrap();
        // 正常图片：复制成功 + 元数据正确
        let png = dir.join("photo.png");
        std::fs::write(&png, b"fake-png-bytes").unwrap();
        let a = db.add_attachment(r.id, &png, &attach_dir).unwrap();
        assert_eq!(a.record_id, r.id);
        assert_eq!(a.file_name, "photo.png");
        assert_eq!(a.media_type, "image");
        assert_eq!(a.size, 14);
        assert!(a.stored_name.ends_with(".png"));
        assert!(Path::new(&a.path).exists());
        assert_eq!(db.attachments(r.id, &attach_dir).unwrap().len(), 1);
        // 白名单外扩展名拒绝
        let exe = dir.join("evil.exe");
        std::fs::write(&exe, b"x").unwrap();
        assert!(db.add_attachment(r.id, &exe, &attach_dir).is_err());
        // 无扩展名拒绝
        let noext = dir.join("noext");
        std::fs::write(&noext, b"x").unwrap();
        assert!(db.add_attachment(r.id, &noext, &attach_dir).is_err());
        // 记录不存在拒绝
        assert!(db.add_attachment(9999, &png, &attach_dir).is_err());
        // 已删除记录拒绝
        let r2 = db.create("2026-10-10", "将删", "", "pending", "").unwrap();
        db.soft_delete(r2.id).unwrap();
        assert!(db.add_attachment(r2.id, &png, &attach_dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_bytes_add_validates_and_writes() {
        let dir = tmp_dir("att-bytes");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "粘贴", "", "pending", "").unwrap();
        // 正常写入
        let a = db
            .add_attachment_bytes(r.id, "粘贴截图.png", b"png-data", &attach_dir)
            .unwrap();
        assert_eq!(a.media_type, "image");
        assert_eq!(a.size, 8);
        assert!(Path::new(&a.path).exists());
        assert_eq!(db.attachments(r.id, &attach_dir).unwrap().len(), 1);
        // 白名单外扩展名拒绝
        assert!(db
            .add_attachment_bytes(r.id, "x.exe", b"x", &attach_dir)
            .is_err());
        // 空内容拒绝
        assert!(db
            .add_attachment_bytes(r.id, "x.png", b"", &attach_dir)
            .is_err());
        // 无扩展名拒绝
        assert!(db
            .add_attachment_bytes(r.id, "noext", b"x", &attach_dir)
            .is_err());
        // 记录不存在拒绝
        assert!(db
            .add_attachment_bytes(9999, "x.png", b"x", &attach_dir)
            .is_err());
        // 已删除记录拒绝
        let r2 = db.create("2026-10-10", "将删", "", "pending", "").unwrap();
        db.soft_delete(r2.id).unwrap();
        assert!(db
            .add_attachment_bytes(r2.id, "x.png", b"x", &attach_dir)
            .is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_width_set_validates_and_persists() {
        let dir = tmp_dir("att-width");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "宽", "", "pending", "").unwrap();
        let a = db
            .add_attachment_bytes(r.id, "图.png", b"png", &attach_dir)
            .unwrap();
        // 新附件默认宽度为空
        assert_eq!(a.width, None);
        assert_eq!(db.attachments(r.id, &attach_dir).unwrap()[0].width, None);
        // 合法宽度持久化
        db.set_attachment_width(a.id, 480).unwrap();
        assert_eq!(db.attachments(r.id, &attach_dir).unwrap()[0].width, Some(480));
        // 边界值（最小/最大）
        db.set_attachment_width(a.id, MIN_ATTACH_WIDTH).unwrap();
        db.set_attachment_width(a.id, MAX_ATTACH_WIDTH).unwrap();
        // 范围外拒绝
        assert!(db.set_attachment_width(a.id, MIN_ATTACH_WIDTH - 1).is_err());
        assert!(db.set_attachment_width(a.id, MAX_ATTACH_WIDTH + 1).is_err());
        // 不存在/已删除附件拒绝
        assert!(db.set_attachment_width(9999, 480).is_err());
        db.delete_attachment(a.id, &attach_dir).unwrap();
        assert!(db.set_attachment_width(a.id, 480).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn html_to_text_extracts_text_and_placeholder_lines() {
        // 富文本：段落 + 加粗 + 样式 span + 附件节点
        let html = r#"<p>第一行 <strong>加粗</strong> 文字</p><figure data-attach-id="12"></figure><p><span style="color: #ff0000; font-size: 18px">红字</span>结尾</p>"#;
        assert_eq!(html_to_text(html), "第一行 加粗 文字\n[附件:12]\n红字结尾");
        // 旧纯文本数据原样保留（占位符行不变）
        assert_eq!(html_to_text("旧内容\n[附件:3]\n下一行"), "旧内容\n[附件:3]\n下一行");
        // br 换行与实体反转义
        assert_eq!(html_to_text("a<br>b &amp; c &lt;x&gt; &quot;q&quot;"), "a\nb & c <x> \"q\"");
        // 空内容
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn attachment_data_url_returns_mime_prefixed_base64() {
        use base64::Engine;
        let dir = tmp_dir("att-copy");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "复制", "", "pending", "").unwrap();
        let a = db
            .add_attachment_bytes(r.id, "图.png", b"png-bytes", &attach_dir)
            .unwrap();
        let expect = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b"png-bytes")
        );
        assert_eq!(db.attachment_data_url(a.id, &attach_dir).unwrap(), expect);
        // 附件不存在报错
        assert!(db.attachment_data_url(9999, &attach_dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mask_attachment_markers_replaces_placeholders() {
        assert_eq!(mask_attachment_markers("前面[附件:3]后面"), "前面📎 附件后面");
        assert_eq!(
            mask_attachment_markers("多行\n[附件:12]\n文本[附件:34]"),
            "多行\n📎 附件\n文本📎 附件"
        );
        assert_eq!(mask_attachment_markers("无占位符"), "无占位符");
        // 未闭合的占位符原样保留
        assert_eq!(mask_attachment_markers("[附件:未闭合"), "[附件:未闭合");
    }

    #[test]
    fn decode_data_url_validates_and_decodes() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello-attachment");
        let ok = decode_data_url(&format!("data:image/png;base64,{b64}")).unwrap();
        assert_eq!(ok, b"hello-attachment");
        // 前缀缺失 / 无法解析 / 空内容 / 非法 base64 均报错
        assert!(decode_data_url("not-a-data-url").is_err());
        assert!(decode_data_url("data:image/png;base64").is_err());
        assert!(decode_data_url("data:,x").is_err());
        assert!(decode_data_url("data:image/png;base64,@@@").is_err());
    }

    #[test]
    fn attachment_delete_removes_row_and_file() {
        let dir = tmp_dir("att-del");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "x", "", "pending", "").unwrap();
        let png = dir.join("p.png");
        std::fs::write(&png, b"png").unwrap();
        let a = db.add_attachment(r.id, &png, &attach_dir).unwrap();
        let path = a.path.clone();
        db.delete_attachment(a.id, &attach_dir).unwrap();
        assert!(!Path::new(&path).exists());
        assert!(db.attachments(r.id, &attach_dir).unwrap().is_empty());
        // 重复删除报错
        assert!(db.delete_attachment(a.id, &attach_dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purge_and_empty_trash_remove_attachment_files() {
        let dir = tmp_dir("att-purge");
        let attach_dir = dir.join("attachments");
        let db = Db::open_in_memory().unwrap();
        let png = dir.join("p.png");
        std::fs::write(&png, b"png").unwrap();
        // 记录 A：purge 删除附件文件
        let a = db.create("2026-10-09", "A", "", "pending", "").unwrap();
        let at_a = db.add_attachment(a.id, &png, &attach_dir).unwrap();
        db.soft_delete(a.id).unwrap();
        db.purge(a.id, &attach_dir).unwrap();
        assert!(!Path::new(&at_a.path).exists());
        // 记录 B：empty_trash 删除附件文件
        let b = db.create("2026-10-09", "B", "", "pending", "").unwrap();
        let at_b = db.add_attachment(b.id, &png, &attach_dir).unwrap();
        db.soft_delete(b.id).unwrap();
        let n = db.empty_trash(&attach_dir).unwrap();
        assert_eq!(n, 1);
        assert!(!Path::new(&at_b.path).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zip_backup_roundtrip_includes_attachments() {
        let dir = tmp_dir("att-bak");
        let attach_dir = dir.join("attachments");
        let zip_path = dir.join("bak.zip");
        let mut db = Db::open_in_memory().unwrap();
        let r = db.create("2026-10-09", "带附件", "", "pending", "").unwrap();
        let png = dir.join("p.png");
        std::fs::write(&png, b"png-bytes").unwrap();
        let a = db.add_attachment(r.id, &png, &attach_dir).unwrap();
        db.backup_to(&zip_path, &attach_dir).unwrap();
        // 恢复前删除附件（模拟数据丢失），恢复后应完整还原
        std::fs::remove_file(&a.path).unwrap();
        let mut db2 = Db::open_in_memory().unwrap();
        let count = db2.restore_from(&zip_path, &attach_dir).unwrap();
        assert_eq!(count, 1);
        let atts = db2.attachments(r.id, &attach_dir).unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0].file_name, "p.png");
        assert!(Path::new(&atts[0].path).exists());
        // 磁盘上的孤儿文件（无数据库引用）不进备份：备份后恢复到一个全新目录，
        // 只应出现 DB 引用的附件
        let orphan = dir.join("orphan.png");
        std::fs::write(&orphan, b"orphan").unwrap();
        std::fs::copy(&orphan, attach_dir.join("o999.png")).unwrap();
        let zip2 = dir.join("bak2.zip");
        db.backup_to(&zip2, &attach_dir).unwrap();
        let mut db3 = Db::open_in_memory().unwrap();
        let rest_dir = dir.join("restored-att");
        db3.restore_from(&zip2, &rest_dir).unwrap();
        assert!(rest_dir.join(&a.stored_name).exists());
        assert!(!rest_dir.join("o999.png").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_zip_rejects_path_traversal() {
        let dir = tmp_dir("att-trav");
        let attach_dir = dir.join("attachments");
        let evil_zip = dir.join("evil.zip");
        // 构造恶意 zip：合法 db 快照 + 路径穿越条目
        let db_path = dir.join("v1.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            register_functions(&conn).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute(
                "INSERT INTO records (record_date, title, content, status, tags, is_deleted, created_at, updated_at)
                 VALUES ('2026-10-09', 'x', '', 'pending', '', 0, '2026-10-09 10:00:00', '2026-10-09 10:00:00')",
                [],
            )
            .unwrap();
        }
        {
            let out = File::create(&evil_zip).unwrap();
            let mut zip = zip::ZipWriter::new(out);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("work-journal.db", opts).unwrap();
            let mut f = File::open(&db_path).unwrap();
            std::io::copy(&mut f, &mut zip).unwrap();
            zip.start_file("attachments/../evil.txt", opts).unwrap();
            zip.write_all(b"evil").unwrap();
            zip.start_file("attachments/sub/evil.txt", opts).unwrap();
            zip.write_all(b"evil").unwrap();
            zip.finish().unwrap();
        }
        let mut db = Db::open_in_memory().unwrap();
        db.restore_from(&evil_zip, &attach_dir).unwrap();
        // 穿越条目均被拒绝：attach_dir 外无 evil.txt，子目录也不存在
        assert!(!dir.join("evil.txt").exists());
        assert!(!attach_dir.join("sub").exists());
        assert!(!attach_dir.join("evil.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_tags_dedupes_and_validates() {
        assert_eq!(
            normalize_tags(" 后端, 修复，后端; ").unwrap(),
            "后端,修复"
        );
        assert!(normalize_tags("ok").is_ok());
        let too_long = "长".repeat(MAX_TAG_LEN + 1);
        assert!(normalize_tags(&too_long).is_err());
    }

    #[test]
    fn validators_reject_bad_input() {
        assert!(validate_date("2026-10-09").is_ok());
        assert!(validate_date("2026/10/09").is_err());
        assert!(validate_date("not-a-date").is_err());
        assert!(validate_status("completed").is_ok());
        assert!(validate_status("done").is_err());
    }

    #[test]
    fn recent_by_date_orders_by_updated_desc_and_limits() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create("2026-10-09", "甲", "", "pending", "").unwrap();
        let b = db.create("2026-10-09", "乙", "", "pending", "").unwrap();
        let c = db.create("2026-10-09", "丙", "", "pending", "").unwrap();
        // 同秒创建：updated_at 相同，按 id 倒序兜底
        let ids: Vec<i64> = db
            .list_recent_by_date("2026-10-09", 10)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![c.id, b.id, a.id]);
        // 甲的修改时间改为最新（直接改库取远未来值，避免与创建时间同秒/更早不可控）→ 排最前
        db.conn
            .execute(
                "UPDATE records SET updated_at = ?1 WHERE id = ?2",
                params!["2099-01-01 00:00:00", a.id],
            )
            .unwrap();
        let ids: Vec<i64> = db
            .list_recent_by_date("2026-10-09", 10)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![a.id, c.id, b.id]);
        // limit 截断
        assert_eq!(db.list_recent_by_date("2026-10-09", 2).unwrap().len(), 2);
        // 软删排除
        db.soft_delete(b.id).unwrap();
        let ids: Vec<i64> = db
            .list_recent_by_date("2026-10-09", 10)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![a.id, c.id]);
        // 无记录日期为空
        assert!(db.list_recent_by_date("2026-10-10", 10).unwrap().is_empty());
    }

    #[test]
    fn list_by_tag_exact_match_and_limits() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create("2026-10-08", "甲", "", "pending", "修复").unwrap();
        let b = db
            .create("2026-10-09", "乙", "", "pending", "修复,修复测试")
            .unwrap();
        let c = db
            .create("2026-10-10", "丙", "", "pending", "前端,修复测试")
            .unwrap();
        db.create("2026-10-11", "丁", "", "pending", "50%完成,a_b")
            .unwrap();
        // 精确命中「修复」：a 与 b（子串「修复测试」不误配），按日期倒序
        let ids: Vec<i64> = db
            .list_by_tag("修复", 100)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![b.id, a.id]);
        // 命中「修复测试」
        let ids: Vec<i64> = db
            .list_by_tag("修复测试", 100)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![c.id, b.id]);
        // 无命中为空
        assert!(db.list_by_tag("不存在", 100).unwrap().is_empty());
        // % 与 _ 是普通字符：精确命中各自标签，且不互相误配
        assert_eq!(db.list_by_tag("50%完成", 100).unwrap().len(), 1);
        assert_eq!(db.list_by_tag("a_b", 100).unwrap().len(), 1);
        assert!(db.list_by_tag("50", 100).unwrap().is_empty());
        assert!(db.list_by_tag("ab", 100).unwrap().is_empty());
        // 软删排除
        db.soft_delete(a.id).unwrap();
        assert_eq!(db.list_by_tag("修复", 100).unwrap().len(), 1);
        // limit 截断
        assert_eq!(db.list_by_tag("修复", 1).unwrap().len(), 1);
        assert_eq!(db.list_by_tag("修复", 1).unwrap()[0].id, b.id);
    }
}
