use anyhow::anyhow;
use seshat::{
    Config, CrawlerCheckpoint, Database, DatabaseStats, Error as SeshatError, Event, LoadConfig,
    Profile, RecoveryDatabase, SearchBatch, SearchConfig,
};
use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::init::singletons::{SESHAT_DATABASE, TEMP_DIR, get_seshat_db_lock};

use super::utils::perform_manual_reindex;

pub async fn supports_event_indexing() -> bool {
    println!("[Seshat command] Supports event indexing");
    true
}

pub async fn init_event_index(passphrase: String) -> anyhow::Result<()> {
    println!("[Seshat command] init_event_index");

    let state_lock = get_seshat_db_lock();

    // Check if the database is already initialized
    if state_lock.is_some() {
        println!("[Seshat command] Database is already initialized.");
        return Ok(()); // No need to reinitialize
    }

    println!("[Seshat command] init_event_index - passphrase {passphrase:?}");
    let config = Config::new().set_passphrase(passphrase);
    let config = config.set_language(&seshat::Language::Unknown);

    let db_path = TEMP_DIR.get().unwrap().clone().join("seshat_db");

    println!("[Seshat command] init_event_index - db_path {:?}", &db_path);

    let _ = fs::create_dir_all(&db_path);

    let db_result = Database::new_with_config(&db_path, &config);

    let database = match db_result {
        Ok(db) => {
            println!("[Seshat command] Database opened successfully on first attempt.");
            db // Use the successfully opened database
        }
        Err(SeshatError::ReindexError) => {
            println!("[Seshat command] Database requires reindexing. Attempting recovery...");

            // --- Recovery Logic ---
            let recovery_config = config.clone(); // Clone config for recovery DB
            let recovery_db = RecoveryDatabase::new_with_config(&db_path, &recovery_config)
                .map_err(|e| anyhow!(format!("Failed to open recovery database: {e}")))?;

            let user_version = {
                // Scope the connection
                let connection = recovery_db
                    .get_connection()
                    .map_err(|e| anyhow!(format!("Failed to get recovery DB connection: {e}")))?;
                connection.get_user_version().map_err(|e| {
                    anyhow!(format!("Failed to get user version from recovery DB: {e}"))
                })?
            };

            println!("[Seshat command] Recovery DB user version: {user_version}");

            if user_version == 0 {
                println!(
                    "[Seshat command] User version is 0. Deleting database contents instead of reindexing."
                );
                // Drop recovery_db explicitly *before* deleting files to release file handles
                drop(recovery_db);
                fs::remove_dir_all(&db_path).map_err(|e| {
                    anyhow!(format!("Failed to delete database for re-creation: {e}"))
                })?;
                // Re-create the directory after deletion
                fs::create_dir_all(&db_path)
                    .map_err(|e| anyhow!(format!("Failed to re-create DB directory: {e}")))?;
            } else {
                println!("[Seshat command] Reindexing database...");
                // reindex() consumes the recovery_db
                perform_manual_reindex(recovery_db)
                    .map_err(|e| anyhow!(format!("Manual reindexing failed: {e}")))?;
                println!("[Seshat command] Reindexing complete.");
            }

            // --- Retry opening the main database after recovery/deletion ---
            println!("[Seshat command] Retrying to open main database after recovery/deletion...");
            Database::new_with_config(&db_path, &config).map_err(|e| {
                anyhow!(format!(
                    "Failed to open database even after recovery attempt: {e}"
                ))
            })?
        }
        Err(e) => {
            // Handle other database opening errors
            return Err(anyhow!(format!("Error opening the database: {e:?}")));
        }
    };

    // --- Store the successfully opened database (either first try or after recovery) ---
    let database_arc = Arc::new(Mutex::new(database));
    let _ = SESHAT_DATABASE.set(Arc::clone(&database_arc));
    println!("[Seshat command] init_event_index completed successfully.");

    Ok(())
}

// Closing the database
pub async fn close_event_index() -> anyhow::Result<()> {
    println!("[Seshat command] close_event_index");
    let mut state = get_seshat_db_lock();
    if let Some(db) = state.take() {
        match Arc::try_unwrap(db) {
            Ok(mutex) => {
                let db_inner = mutex.into_inner().unwrap(); // Extract the database
                // The shutdown meethod needs to take ownership
                db_inner.shutdown();
                Ok(())
            }
            Err(_arc) => {
                println!("Failed to take ownership: Database is still shared.");
                Ok(())
            }
        }
    } else {
        println!("[Seshat command] close_event_index no db found, already closed");
        Ok(())
    }
}

// Deleting the database contents and files
pub async fn delete_event_index() -> anyhow::Result<()> {
    println!("[Seshat command] delete_event_index");
    // The app_handle is a method introduce by tauri
    let db_path = TEMP_DIR.get().unwrap().clone().join("seshat_db");

    // Handle the case where the directory doesn't exist
    match fs::remove_dir_all(&db_path) {
        Ok(_) => println!("Successfully deleted index at: {db_path:?}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Index directory not found at: {db_path:?}, continuing anyway");
        }
        Err(e) => return Err(e.into()), // For other InvokeErrors, convert and return
    }

    Ok(())
}

pub async fn add_event_to_index(
    event: seshat::Event,
    profile: seshat::Profile,
) -> anyhow::Result<()> {
    println!("[Seshat command] add_event_to_index");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        println!("[Seshat command] add_event_to_index event {event:?}");
        println!("[Seshat command] add_event_to_index profile {profile:?}");
        db_lock.add_event(event, profile);
    }
    Ok(())
}

pub async fn delete_event(event_id: String) -> anyhow::Result<()> {
    println!("[Seshat command] delete_event {event_id:?}");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        db_lock.delete_event(&event_id);
    }
    Ok(())
}

pub async fn commit_live_events() -> anyhow::Result<()> {
    println!("[Seshat command] commit_live_events");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let mut db_lock = db.lock().unwrap();
        let _ = db_lock.commit();
    }
    Ok(())
}

pub async fn search_event_index(
    search_term: String,
    search_config: SearchConfig,
) -> anyhow::Result<SearchBatch> {
    println!("[Seshat command] search_event_index");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        println!("---- search_event_index config {search_config:?}");
        println!("---- search_event_index term {search_term:?}");
        let db_lock: std::sync::MutexGuard<'_, Database> = db.lock().unwrap();
        let result = db_lock.search(&search_term, &search_config).unwrap();

        Ok(result)
    } else {
        println!("[Seshat command] search_event_index result no database found");
        Err(anyhow::Error::msg("No seshat database found"))
    }
}

pub async fn is_room_indexed(room_id: String) -> anyhow::Result<bool> {
    println!("[Seshat command] is_room_indexed");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        connection
            .is_room_indexed(&room_id)
            .map_err(anyhow::Error::from)
    } else {
        Ok(false)
    }
}

pub async fn is_event_index_empty() -> anyhow::Result<bool> {
    println!("[Seshat command] is_event_index_empty");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        let result = connection.is_empty()?;

        println!("[Seshat command] is_event_index_empty {result:?}");
        Ok(result)
    } else {
        println!("[Seshat command] is_event_index_empty true");
        Ok(true)
    }
}

pub async fn add_historic_events(
    events: Vec<(Event, Profile)>,
    new_checkpoint: Option<CrawlerCheckpoint>,
    old_checkpoint: Option<CrawlerCheckpoint>,
) -> anyhow::Result<bool> {
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock: std::sync::MutexGuard<'_, Database> = db.lock().unwrap();

        let receiver = db_lock.add_historic_events(events, new_checkpoint, old_checkpoint);

        match receiver.recv() {
            Ok(result) => {
                let final_result = result.map_err(anyhow::Error::from)?;
                // Get stats after adding events
                let connection = db_lock.get_connection().map_err(anyhow::Error::from)?;
                let stats_after = connection.get_stats().map_err(anyhow::Error::from)?;
                println!(
                    "[Seshat command] Stats after: event_count={}, room_count={}",
                    stats_after.event_count, stats_after.room_count
                );

                Ok(final_result)
            }
            Err(recv_err) => {
                println!("[Error] Failed to receive result: {recv_err}");
                Err(anyhow::Error::from(recv_err))
            }
        }
    } else {
        // Create a dummy channel to return the expected type
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Ok(false));

        rx.recv().map_err(anyhow::Error::from).unwrap()
    }
}

pub async fn get_stats() -> anyhow::Result<DatabaseStats> {
    println!("[Seshat command] remove_crawler_checkpoint");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        connection.get_stats().map_err(anyhow::Error::from)
    } else {
        Err(anyhow!("No stats found".to_string()))
    }
}

// There is no remove_crawler_checkpoint in the api, but we are only useing add_historic_events with the correct parameters
pub async fn remove_crawler_checkpoint(
    checkpoint: Option<CrawlerCheckpoint>,
) -> anyhow::Result<bool> {
    println!("[Seshat command] remove_crawler_checkpoint");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let receiver = db_lock.add_historic_events(Vec::new(), None, checkpoint);

        receiver
            .recv()
            .map(|r| r.map_err(anyhow::Error::from))
            .map_err(anyhow::Error::from)
            .unwrap()
    } else {
        // Create a dummy channel to return the expected type
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Ok(false));

        rx.recv().map_err(anyhow::Error::from).unwrap()
    }
}

// There is no add_crawler_checkpoint in the api, but we are only useing add_historic_events with the correct parameters
pub async fn add_crawler_checkpoint(checkpoint: Option<CrawlerCheckpoint>) -> anyhow::Result<bool> {
    println!("[Seshat command] add_crawler_checkpoint ${checkpoint:?}");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();

        println!("[Debug] Processed checkpoint for adding: {checkpoint:?}");
        let receiver = db_lock.add_historic_events(Vec::new(), checkpoint, None);

        match receiver.recv() {
            Ok(result) => {
                let final_result = result.map_err(anyhow::Error::from)?;
                println!("[Debug] Result of adding checkpoint: {final_result:?}");
                Ok(final_result)
            }
            Err(recv_err) => {
                println!("[Error] Failed to receive result: {recv_err}");
                Err(anyhow::Error::from(recv_err))
            }
        }
    } else {
        // Create a dummy channel to return the expected type
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Ok(false));

        rx.recv().map_err(anyhow::Error::from).unwrap()
    }
}

pub async fn load_file_events(load_config: LoadConfig) -> anyhow::Result<Vec<(String, Profile)>> {
    println!("[Seshat command] load_file_events");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        let result = connection.load_file_events(&load_config)?;
        Ok(result)
    } else {
        Err(anyhow!("No database found".to_string()))
    }
}

pub async fn load_checkpoints() -> anyhow::Result<Vec<CrawlerCheckpoint>> {
    println!("[Seshat command] load_checkpoints");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        let checkpoints = connection.load_checkpoints().unwrap();

        println!("---- load_checkpoints raw results count: {checkpoints:?}");

        Ok(checkpoints)
    } else {
        Err(anyhow!("No database found".to_string()))
    }
}

pub async fn set_user_version(version: i64) -> anyhow::Result<()> {
    println!("[Seshat command] set_user_version");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        connection
            .set_user_version(version)
            .map_err(anyhow::Error::from)
    } else {
        Ok(())
    }
}

pub async fn get_user_version() -> anyhow::Result<i64> {
    println!("[Seshat command] get_user_version");
    let state_guard = get_seshat_db_lock();

    if let Some(ref db) = state_guard {
        let db_lock = db.lock().unwrap();
        let connection = db_lock.get_connection().unwrap();
        connection.get_user_version().map_err(anyhow::Error::from)
    } else {
        Ok(0)
    }
}
