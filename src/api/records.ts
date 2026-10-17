/**
 * Tauri 命令封装：前端所有持久化操作唯一入口。
 * 入参使用 camelCase（Tauri 自动映射为 Rust 侧 snake_case 参数）。
 * 失败时 invoke 会 reject，由调用方捕获后 toast 提示。
 */
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { Attachment, ExportKind, RecordStatus, TagStat, WorkRecord } from "@/types/work-record";

/** 查询某日全部有效记录（置顶恒在最前） */
export function listByDate(date: string): Promise<WorkRecord[]> {
  return invoke("get_records_by_date", { date });
}

/** 查询日期范围 [from, to] 内全部有效记录（日期升序，同日置顶在前） */
export function listByRange(from: string, to: string): Promise<WorkRecord[]> {
  return invoke("get_records_by_range", { from, to });
}

/** 新建记录（创建即入库，后续编辑走自动保存） */
export function createRecord(
  date: string,
  title: string,
  content: string,
  status: RecordStatus,
  tags: string,
): Promise<WorkRecord> {
  return invoke("create_record", { date, title, content, status, tags });
}

/** 更新记录（自动保存 / 手动保存共用） */
export function updateRecord(
  id: number,
  title: string,
  content: string,
  status: RecordStatus,
  tags: string,
): Promise<WorkRecord> {
  return invoke("update_record", { id, title, content, status, tags });
}

/** 软删除（移入回收站） */
export function softDelete(id: number): Promise<void> {
  return invoke("soft_delete_record", { id });
}

/** 置顶/取消置顶，返回更新后的记录 */
export function setPinned(id: number, pinned: boolean): Promise<WorkRecord> {
  return invoke("set_record_pinned", { id, pinned });
}

/** 全部置顶且未删除的记录（侧边栏置顶区固定展示） */
export function listPinned(): Promise<WorkRecord[]> {
  return invoke("get_pinned_records");
}

/** 从回收站恢复 */
export function restoreRecord(id: number): Promise<void> {
  return invoke("restore_record", { id });
}

/** 物理删除（仅回收站内） */
export function purgeRecord(id: number): Promise<void> {
  return invoke("purge_record", { id });
}

/** 清空回收站，返回删除条数 */
export function emptyTrash(): Promise<number> {
  return invoke("empty_trash");
}

/** 回收站列表 */
export function trashRecords(): Promise<WorkRecord[]> {
  return invoke("get_trash_records");
}

/** 时间线分页（按日期倒序） */
export function timeline(offset: number, limit: number): Promise<WorkRecord[]> {
  return invoke("get_timeline", { offset, limit });
}

/** 全文搜索（FTS5），可限定日期范围；dateFrom/dateTo 为 yyyy-MM-dd 或 null。
 * 只返回命中记录本体，关键词高亮与摘要由前端基于原文渲染。 */
export function search(
  query: string,
  dateFrom: string | null,
  dateTo: string | null,
): Promise<WorkRecord[]> {
  return invoke("search_records", { query, dateFrom, dateTo });
}

/** 全部有记录的日期（日历高亮） */
export function recordDates(): Promise<string[]> {
  return invoke("get_record_dates");
}

/** 全部已使用标签及计数 */
export function allTags(): Promise<TagStat[]> {
  return invoke("get_all_tags");
}

/** 某日期最近 N 条记录（按修改时间倒序；日历悬浮预览用） */
export function recentByDate(date: string, limit = 10): Promise<WorkRecord[]> {
  return invoke("get_recent_records_by_date", { date, limit });
}

/** 按标签精确匹配的全部记录（跨日期；标签弹窗视图用） */
export function recordsByTag(tag: string, limit = 500): Promise<WorkRecord[]> {
  return invoke("get_records_by_tag", { tag, limit });
}

/** 导出 Markdown 报告；用户取消返回 null，成功返回保存路径 */
export function exportReport(kind: ExportKind, date: string): Promise<string | null> {
  return invoke("export_report", { kind, date });
}

/** 备份数据库到用户选择文件夹；用户取消返回 null，成功返回备份路径 */
export function backupDb(): Promise<string | null> {
  return invoke("backup_database");
}

/** 从备份文件恢复；用户取消返回 null，成功返回恢复后的有效记录数 */
export function restoreDb(): Promise<number | null> {
  return invoke("restore_database");
}

/** 当前数据库文件路径 */
export function dbPath(): Promise<string> {
  return invoke("get_db_path");
}

// ---------- 媒体附件 ----------

/** 为磁盘绝对路径生成可加载的媒体 URL（asset protocol）；
 * 非 Tauri 环境（浏览器调试）下 convertFileSrc 不可用，原样返回路径兜底 */
export function attachmentUrl(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

/** 添加媒体附件（弹文件选择框多选）；用户取消返回空数组 */
export function addAttachments(recordId: number): Promise<Attachment[]> {
  return invoke("add_attachments", { recordId });
}

/** 粘贴添加附件：剪贴板媒体转 data URL（base64）直接入库 */
export function addAttachmentData(
  recordId: number,
  fileName: string,
  dataUrl: string,
): Promise<Attachment> {
  return invoke("add_attachment_data", { recordId, fileName, dataUrl });
}

/** 查询某条记录的全部附件 */
export function listAttachments(recordId: number): Promise<Attachment[]> {
  return invoke("list_attachments", { recordId });
}

/** 删除附件（数据库行 + 磁盘文件） */
export function deleteAttachment(id: number): Promise<void> {
  return invoke("delete_attachment", { id });
}

/** 设置附件显示宽度（编辑器拖拽调整媒体块大小后持久化） */
export function setAttachmentWidth(id: number, width: number): Promise<void> {
  return invoke("set_attachment_width", { id, width });
}

/** 读取附件内容为 data URL（媒体块 Ctrl+C 复制到剪贴板用） */
export function getAttachmentData(id: number): Promise<string> {
  return invoke("get_attachment_data", { id });
}

/** 导出附件到本地文件（右键「导出媒体文件」）。返回保存路径，用户取消返回 null */
export function exportAttachment(id: number): Promise<string | null> {
  return invoke("export_attachment", { id });
}

/** 应用版本号 */
export function appVersion(): Promise<string> {
  return invoke("get_app_version");
}
