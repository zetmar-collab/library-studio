mod commands;
mod domain;
mod infrastructure;

use std::fs::OpenOptions;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use fs4::fs_std::FileExt;
use tauri::Manager;

use infrastructure::db::Database;

pub struct AppState {
    pub db: Database,
    pub data_dir: std::path::PathBuf,
    pub cancel: AtomicBool,
    /// Czy trwa długa operacja (kopia, synchronizacja, import). Naraz może iść tylko jedna.
    pub job_running: AtomicBool,
    pub instance_lock: Mutex<std::fs::File>,
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let lock_path = dir.join("app.lock");
            let lock_file = OpenOptions::new().create(true).read(true).write(true).open(&lock_path)?;
            if lock_file.try_lock_exclusive().is_err() {
                let english = std::fs::read_to_string(dir.join("locale.txt")).map(|text| text.trim() == "en").unwrap_or(false);
                let description = if english {
                    "Library Studio is already running."
                } else {
                    "Library Studio jest już uruchomione."
                };
                rfd::MessageDialog::new()
                    .set_title("Library Studio")
                    .set_description(description)
                    .show();
                return Err(description.into());
            }
            let db_path = dir.join("biblioteka.sqlite");
            let db = Database::open(&db_path)?;
            if let Ok(settings) = db.with(infrastructure::store::get_settings) {
                let _ = std::fs::write(dir.join("locale.txt"), settings.locale);
            }
            infrastructure::log::write(&dir, "info", "otwarto bazę biblioteki");
            app.manage(AppState {
                db,
                data_dir: dir.clone(),
                cancel: AtomicBool::new(false),
                job_running: AtomicBool::new(false),
                instance_lock: Mutex::new(lock_file),
            });
            commands::allow_assets(app.handle(), app.state::<AppState>().inner());
            let handle = app.handle().clone();
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(60 * 30));
                if let Some(state) = handle.try_state::<AppState>() {
                    let result = state.db.with(|conn| {
                        let settings = infrastructure::store::get_settings(conn)?;
                        if infrastructure::backup::due(&settings, conn)? {
                            infrastructure::log::write(&state.data_dir, "info", "automatyczna kopia bazy");
                            infrastructure::backup::backup_database(conn, &settings, "database")?;
                        }
                        Ok(())
                    });
                    if let Err(error) = result {
                        infrastructure::log::write(&state.data_dir, "error", &format!("automatyczna kopia: {error}"));
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::get_settings,
            commands::save_settings,
            commands::set_secret,
            commands::pick_folder,
            commands::pick_file,
            commands::pick_csv_file,
            commands::pick_backup_file,
            commands::list_books,
            commands::get_book,
            commands::save_book,
            commands::enrich_book,
            commands::enrich_library,
            commands::delete_book,
            commands::search_metadata,
            commands::search_covers,
            commands::preview_cover,
            commands::attach_cover,
            commands::save_physical,
            commands::delete_physical,
            commands::save_progress,
            commands::list_shelves,
            commands::save_shelf,
            commands::delete_shelf,
            commands::reorder_shelf,
            commands::add_to_shelf,
            commands::remove_from_shelf,
            commands::add_file,
            commands::list_readers,
            commands::open_reader_folder,
            commands::list_reader_books,
            commands::send_to_reader,
            commands::import_from_reader,
            commands::list_files,
            commands::verify_files,
            commands::open_file,
            commands::cover_data,
            commands::list_backups,
            commands::backup_now,
            commands::restore_backup,
            commands::export_library,
            commands::import_library,
            commands::sync_now,
            commands::list_conflicts,
            commands::resolve_conflict,
            commands::cancel_job,
            commands::list_trash,
            commands::restore_book,
            commands::purge_book,
            commands::empty_trash,
            commands::find_duplicates,
            commands::ignore_duplicates,
            commands::merge_books,
            commands::reading_stats,
            commands::list_facets,
            commands::export_csv,
            commands::print_rows,
            commands::preview_import,
            commands::commit_import
        ])
        .run(tauri::generate_context!())
        .expect("nie udało się uruchomić okna Biblioteki");
}
