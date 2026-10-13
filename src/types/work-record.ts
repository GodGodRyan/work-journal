/** 记录完成状态 */
export type RecordStatus = "pending" | "completed";

/** 与 Rust db::WorkRecord 字段一一对应（snake_case 与后端序列化保持一致） */
export interface WorkRecord {
  id: number;
  /** 记录日期 yyyy-MM-dd */
  record_date: string;
  title: string;
  /** 富文本 HTML（Tiptap 编辑器格式：<p>/<strong>/<span style>/<figure data-attach-id>） */
  content: string;
  /** 纯文本副本（后端 html_to_text 自动提取，供 FTS 索引/摘要/导出） */
  content_text: string;
  status: RecordStatus;
  /** 逗号分隔的标签 */
  tags: string;
  is_deleted: boolean;
  /** 是否永久置顶（主页任意日期、时间线均显示在最前） */
  is_pinned: boolean;
  created_at: string;
  updated_at: string;
}

/** 标签及使用次数 */
export interface TagStat {
  tag: string;
  count: number;
}

/** 媒体附件元数据，与 Rust db::Attachment 字段一一对应 */
export interface Attachment {
  id: number;
  record_id: number;
  /** 原始文件名（展示用） */
  file_name: string;
  /** 磁盘存储文件名（应用目录内，不暴露给用户） */
  stored_name: string;
  /** 媒体类型：image / video / audio */
  media_type: "image" | "video" | "audio";
  /** 文件字节数 */
  size: number;
  created_at: string;
  /** 编辑器内显示宽度（px）；null = 默认尺寸 */
  width: number | null;
  /** 磁盘绝对路径（convertFileSrc 生成预览 URL 用） */
  path: string;
}

/** 导出类型 */
export type ExportKind = "daily" | "weekly" | "monthly" | "all";

/** 主题模式 */
export type ThemeMode = "light" | "dark" | "system";
