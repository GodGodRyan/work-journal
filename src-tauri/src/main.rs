// 工作日志 —— Tauri 2 入口。
// 职责单一：注册插件、初始化应用数据目录、打开数据库并执行迁移、注册全部命令。
// 业务 SQL 见 db.rs，对外命令见 commands.rs。

// Windows 发布版不弹出附加控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;

use std::sync::Mutex;
use tauri::Manager;

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("无法获取应用数据目录：{e}"))?;
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| format!("无法创建数据目录 {}：{e}", data_dir.display()))?;
            let db_path = data_dir.join("work-journal.db");
            let attach_dir = data_dir.join("attachments");
            std::fs::create_dir_all(&attach_dir)
                .map_err(|e| format!("无法创建附件目录 {}：{e}", attach_dir.display()))?;
            let db = db::Db::open(&db_path).map_err(|e| format!("数据库初始化失败：{e:#}"))?;
            app.manage(commands::AppState {
                db: Mutex::new(db),
                db_path,
                attach_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_records_by_date,
            commands::get_records_by_range,
            commands::create_record,
            commands::update_record,
            commands::soft_delete_record,
            commands::set_record_pinned,
            commands::get_pinned_records,
            commands::restore_record,
            commands::purge_record,
            commands::empty_trash,
            commands::get_trash_records,
            commands::get_timeline,
            commands::search_records,
            commands::get_record_dates,
            commands::get_all_tags,
            commands::get_recent_records_by_date,
            commands::get_records_by_tag,
            commands::export_report,
            commands::backup_database,
            commands::restore_database,
            commands::add_attachments,
            commands::list_attachments,
            commands::add_attachment_data,
            commands::delete_attachment,
            commands::set_attachment_width,
            commands::get_attachment_data,
            commands::get_db_path,
            commands::get_app_version,
        ])
        .build(tauri::generate_context!())
        .expect("工作日志构建失败");

    // Tauri 2 的 run 返回 ()，运行期错误由框架内部记录日志；
    // setup/构建阶段的错误已在上面通过 expect/Result 兜底，不会静默吞掉。
    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            // 预留：退出前可在此刷新未保存状态（当前无后台任务）
        }
    });
}
