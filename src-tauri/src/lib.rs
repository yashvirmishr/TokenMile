//! FreeTokenMeter — a local-first monitor for AI coding tool usage.
//!
//! # Architecture
//!
//! ```text
//!                         FreeTokenMeter
//!                               │
//!                        Provider Registry
//!                               │
//!     ┌──────────┬─────────────┼─────────────┬──────────────┬─────────────┐
//!     ▼          ▼             ▼             ▼              ▼             ▼
//! OpenCode   Freebuff    Claude Code      Codex       Gemini CLI   GitHub Copilot
//!     │          │             │             │              │             │
//!     └──────────┴─────────────┴─────────────┴──────────────┴─────────────┘
//!                               │
//!                     Normalized UsageRecord
//!                               │
//!                          Deduplication
//!                               │
//!                            SQLite
//!                               │
//!                    Dynamic Pricing Registry
//!                               │
//!                       Value Calculation
//!                               │
//!                    Aggregation / Analytics
//!                               │
//!                         Terminal UI
//! ```
//!
//! Provider-specific knowledge is confined to the provider modules: everything
//! from [`database`] onwards is provider-agnostic.

mod claude_code_provider;
mod codex_provider;
mod copilot_provider;
mod database;
mod freebuff_provider;
mod gemini_cli_provider;
mod logging;
mod model_normalization;
mod opencode_provider;
mod providers;

/// Helpers shared by the test suite.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    /// A process-unique id for scratch file and directory names.
    ///
    /// Deliberately a counter rather than a timestamp. Two threads can read the
    /// same `Utc::now()` tick on Windows (100 ns clock granularity), which let two
    /// parallel tests build the same temp path and read each other's fixtures —
    /// a failure that looked like a provider bug and appeared only sometimes.
    pub fn unique_id() -> u64 {
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    }
}
mod preferences;
mod pricing;
mod provider;
mod registry;
mod sync;
mod themes;
mod tray;
mod usage;
mod window;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use log::{info, warn};
use serde::Serialize;
use tauri::{Manager, State};

use database::{DashboardData, Database, ProviderStat, ProviderStatus, UsageSeries};
use preferences::{AppPreferences, PreferencesStore};
use pricing::PricingRates;
use provider::{ProviderDescriptor, ProviderRegistry};
use registry::RegistryState;
use sync::SyncReport;
use usage::ProviderAvailability;

/// Application state shared by commands, the tray and background tasks.
pub(crate) struct AppState {
    pub(crate) db: Mutex<Database>,
    /// Every known provider, iterated rather than referenced by name.
    pub(crate) providers: ProviderRegistry,
    /// Persisted desktop preferences (JSON file, not SQLite).
    pub(crate) prefs: Mutex<PreferencesStore>,
    pub(crate) last_sync_report: Mutex<Option<SyncReport>>,
    /// Dynamic pricing registry (LiteLLM), shared with the provider threads.
    pub(crate) pricing_registry: Arc<RegistryState>,
    /// Guards against overlapping sync passes.
    pub(crate) syncing: AtomicBool,
    /// Set when the user quits; background loops use it to stop.
    pub(crate) shutdown: AtomicBool,
    /// Incremented on every move/resize so geometry can be persisted debounced.
    pub(crate) geometry_events: AtomicU64,
}

fn get_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
        .join("freetokenmeter")
}

/// Format a timestamp (millis since epoch) into a human-readable "X ago" string.
fn format_time_ago(millis: i64) -> String {
    let diff = Utc::now().timestamp_millis() - millis;
    if diff < 0 {
        return "just now".to_string();
    }
    if diff < 60_000 {
        format!("{}s ago", diff / 1000)
    } else if diff < 3_600_000 {
        format!("{}m ago", diff / 60_000)
    } else if diff < 86_400_000 {
        format!("{}h ago", diff / 3_600_000)
    } else {
        format!("{}d ago", diff / 86_400_000)
    }
}

/// Detection + health snapshot for every registered provider.
///
/// The state comes from the provider's own detection, overridden for providers
/// the user switched off, and augmented with the outcome of the last sync.
fn build_provider_status(state: &AppState) -> Vec<ProviderStatus> {
    let prefs = state
        .prefs
        .lock()
        .map(|p| p.data().clone())
        .unwrap_or_default();

    let stats: HashMap<String, ProviderStat> = state
        .db
        .lock()
        .ok()
        .and_then(|db| db.provider_stats().ok())
        .unwrap_or_default();

    let now = Utc::now().timestamp_millis();

    state
        .providers
        .iter()
        .map(|provider| {
            let id = provider.id();
            let enabled = prefs.is_provider_enabled(id);

            let mut availability = provider.availability();
            if !enabled && availability.is_connected() {
                availability = ProviderAvailability::Disabled;
            }

            let (stored_state, stored_detail) = state
                .db
                .lock()
                .ok()
                .and_then(|db| db.get_sync_state(id).ok().flatten())
                .map(|(stored, detail)| (Some(stored), detail))
                .unwrap_or((None, None));

            let last_sync_ms = state
                .db
                .lock()
                .ok()
                .and_then(|db| db.get_last_sync_time(id).ok().flatten());

            let mut state_label = availability.as_str().to_string();
            let mut status_label = availability.label().to_string();
            let mut detail = provider.detail();

            // A connected provider whose last pass failed is surfaced as an
            // error rather than silently reported as healthy.
            if availability.is_connected() && stored_state.as_deref() == Some("error") {
                state_label = "error".to_string();
                status_label = "SYNC ERROR".to_string();
                detail = stored_detail.clone().or(detail);
            }

            let stat = stats.get(id);

            ProviderStatus {
                id: id.to_string(),
                name: provider.display_name().to_string(),
                short_label: provider.short_label().to_string(),
                state: state_label,
                connected: availability.is_connected(),
                status: status_label,
                detail,
                enabled,
                source_type: provider.source_type().to_string(),
                payment_mode: provider.payment_mode().as_str().to_string(),
                billing_unit: provider.billing_unit().as_str().to_string(),
                billing_metric_name: provider.billing_unit().metric_name().map(str::to_string),
                source_version: stat.and_then(|s| s.source_version.clone()),
                last_sync_age_secs: last_sync_ms.map(|ms| (now - ms).max(0) / 1000),
                last_sync: last_sync_ms.map(format_time_ago),
                records: stat.map(|s| s.records).unwrap_or(0),
                tokens: stat.map(|s| s.tokens).unwrap_or(0),
            }
        })
        .collect()
}

// ──────────────────────────────────────────────────────────────────────
// Sync commands — all of them go through `sync::sync_all`
// ──────────────────────────────────────────────────────────────────────

/// Sync every enabled provider. Never fails: per-provider errors are in the report.
///
/// Runs on a blocking thread: a first backfill parses provider files and writes
/// up to [`sync::SYNC_RECORD_LIMIT`] rows, which must not occupy the IPC thread.
#[tauri::command]
async fn sync_all(app: tauri::AppHandle) -> SyncReport {
    let handle = app.clone();
    match tauri::async_runtime::spawn_blocking(move || sync::sync_once_guarded(&handle)).await {
        Ok(report) => report,
        Err(e) => {
            warn!("Sync task failed to run: {}", e);
            SyncReport::default()
        }
    }
}

#[tauri::command]
fn get_last_sync_report(state: State<'_, AppState>) -> Option<SyncReport> {
    sync::last_report(&state)
}

// ──────────────────────────────────────────────────────────────────────
// Dashboard
// ──────────────────────────────────────────────────────────────────────

#[tauri::command(async)]
fn get_dashboard(
    state: State<'_, AppState>,
    days: Option<i64>,
    provider: Option<String>,
) -> Result<DashboardData, String> {
    let (summary, provider_usage, model_usage, activity, series) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let summary = db
            .get_usage_summary(days, provider.as_deref())
            .map_err(|e| e.to_string())?;
        let provider_usage = db
            .get_provider_usage(days, provider.as_deref())
            .map_err(|e| e.to_string())?;
        let model_usage = db
            .get_model_usage(days, provider.as_deref())
            .map_err(|e| e.to_string())?;
        let activity = db
            .get_activity(days, 20, provider.as_deref())
            .map_err(|e| e.to_string())?;
        let series = db
            .get_series(days, provider.as_deref())
            .map_err(|e| e.to_string())?;
        (summary, provider_usage, model_usage, activity, series)
    };

    let providers = build_provider_status(&state);

    Ok(DashboardData {
        summary,
        provider_usage,
        model_usage,
        activity,
        series,
        providers,
    })
}

/// Daily token totals for the calendar view.
///
/// Separate from `get_dashboard` on purpose: the calendar always spans the last
/// year at day resolution, while the dashboard series follows the selected
/// range. Bundling it into the dashboard would make every range switch refetch a
/// year of daily buckets it cannot use.
#[tauri::command(async)]
fn get_calendar(
    state: State<'_, AppState>,
    days: Option<i64>,
    provider: Option<String>,
) -> Result<UsageSeries, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_calendar(days, provider.as_deref())
        .map_err(|e| e.to_string())
}

/// Provider health: detection state, last sync, stored records and tokens.
#[tauri::command(async)]
fn get_providers(state: State<'_, AppState>) -> Vec<ProviderStatus> {
    build_provider_status(&state)
}

/// Static metadata for every registered provider, including the providers that
/// exist but have no supported source yet.
#[tauri::command]
fn get_provider_descriptors(state: State<'_, AppState>) -> Vec<ProviderDescriptor> {
    state.providers.descriptors()
}

#[derive(Debug, Clone, Serialize)]
pub struct FutureProviderView {
    pub id: String,
    pub name: String,
    pub reason: String,
}

/// Providers that are documented as future work rather than integrations.
#[tauri::command]
fn get_future_providers() -> Vec<FutureProviderView> {
    provider::FUTURE_PROVIDERS
        .iter()
        .map(|(id, name, reason)| FutureProviderView {
            id: id.to_string(),
            name: name.to_string(),
            reason: reason.to_string(),
        })
        .collect()
}

/// Turn one provider's synchronization on or off. Persisted across restarts.
#[tauri::command]
fn set_provider_enabled(
    state: State<'_, AppState>,
    provider: String,
    enabled: bool,
) -> Result<AppPreferences, String> {
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    prefs.set_provider_enabled(&provider, enabled)
}

/// Change the first-import window used when a provider has no cursor yet.
#[tauri::command]
fn set_backfill_days(state: State<'_, AppState>, days: i64) -> Result<AppPreferences, String> {
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    prefs.set_backfill_days(days)
}

// ──────────────────────────────────────────────────────────────────────
// Pricing registry
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PricingProfileView {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub actual: PricingRates,
    pub reference: Option<PricingRates>,
    pub reference_model: Option<String>,
    pub source: String,
    pub version: String,
    pub verified_at: String,
    pub is_free: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnknownModelView {
    pub provider: String,
    pub model: String,
    pub tokens: i64,
    pub records: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PricingRegistryView {
    pub version: String,
    pub registry_version: Option<String>,
    pub registry_models: usize,
    pub known: Vec<PricingProfileView>,
    pub unknown: Vec<UnknownModelView>,
}

/// The pricing maintenance view: every configured model plus any model seen in
/// the local history that has no pricing entry.
#[tauri::command(async)]
fn get_pricing_registry(state: State<'_, AppState>) -> Result<PricingRegistryView, String> {
    // Collect the set of (provider, model) pairs the user has actually used.
    let used_keys: std::collections::HashSet<(String, String)> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_used_model_keys()
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect()
    };

    // FTM overrides: only show if actually used.
    let mut known: Vec<PricingProfileView> = pricing::ftm_overrides()
        .iter()
        .filter(|p| used_keys.contains(&(p.provider.to_string(), p.model_id.to_string())))
        .map(|profile| PricingProfileView {
            provider: profile.provider.to_string(),
            model_id: profile.model_id.to_string(),
            display_name: profile.display_name.to_string(),
            actual: profile.actual,
            reference: profile.reference,
            reference_model: profile.reference_model.map(|m| m.to_string()),
            source: profile.source.to_string(),
            version: profile.version.to_string(),
            verified_at: profile.verified_at.to_string(),
            is_free: profile.actual.is_free(),
        })
        .collect();

    // Dynamic registry: only show if actually used.
    let registry_status = state.pricing_registry.status();
    for entry in state.pricing_registry.all_entries() {
        if used_keys.iter().any(|(_, m)| {
            m == &entry.model_id
                || entry.model_id.to_lowercase().contains(&m.to_lowercase())
                || m.to_lowercase().contains(&entry.model_id.to_lowercase())
        }) {
            known.push(PricingProfileView {
                provider: entry.litellm_provider.to_string(),
                model_id: entry.model_id.to_string(),
                display_name: entry.display_name.to_string(),
                actual: PricingRates {
                    input_per_million: entry.rates.input_per_million,
                    output_per_million: entry.rates.output_per_million,
                    cached_per_million: entry.rates.cached_per_million,
                },
                reference: None,
                reference_model: None,
                source: "litellm".to_string(),
                version: registry_status.last_updated.clone().unwrap_or_default(),
                verified_at: String::new(),
                is_free: false,
            });
        }
    }

    known.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    let unknown = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_unknown_models()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(provider, model, tokens, records)| UnknownModelView {
                provider,
                model,
                tokens,
                records,
            })
            .collect()
    };

    Ok(PricingRegistryView {
        version: pricing::PRICING_VERSION.to_string(),
        registry_version: registry_status.last_updated.clone(),
        registry_models: registry_status.model_count,
        known,
        unknown,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryStatusView {
    pub model_count: usize,
    pub last_updated: Option<String>,
    pub cache_age: Option<String>,
    pub needs_refresh: bool,
}

#[tauri::command]
fn get_registry_status(state: State<'_, AppState>) -> RegistryStatusView {
    let status = state.pricing_registry.status();
    RegistryStatusView {
        model_count: status.model_count,
        last_updated: status.last_updated,
        cache_age: status.age.clone(),
        needs_refresh: state.pricing_registry.needs_refresh(),
    }
}

/// Re-download the pricing registry.
///
/// The HTTP call (up to 30 s) runs on a blocking thread, so a slow network cannot
/// stall the UI or the rest of the IPC surface.
#[tauri::command]
async fn refresh_pricing(state: State<'_, AppState>) -> Result<RegistryStatusView, String> {
    let registry = state.pricing_registry.clone();
    tauri::async_runtime::spawn_blocking(move || registry.refresh())
        .await
        .map_err(|e| format!("Pricing refresh task failed: {}", e))??;

    let status = state.pricing_registry.status();
    Ok(RegistryStatusView {
        model_count: status.model_count,
        last_updated: status.last_updated,
        cache_age: status.age,
        needs_refresh: state.pricing_registry.needs_refresh(),
    })
}

#[tauri::command(async)]
fn search_pricing_registry(state: State<'_, AppState>, query: String) -> Vec<PricingProfileView> {
    let lower = query.to_lowercase();
    let mut results: Vec<PricingProfileView> = pricing::ftm_overrides()
        .iter()
        .filter(|p| {
            p.model_id.to_lowercase().contains(&lower)
                || p.display_name.to_lowercase().contains(&lower)
        })
        .map(|profile| PricingProfileView {
            provider: profile.provider.to_string(),
            model_id: profile.model_id.to_string(),
            display_name: profile.display_name.to_string(),
            actual: profile.actual,
            reference: profile.reference,
            reference_model: profile.reference_model.map(|m| m.to_string()),
            source: profile.source.to_string(),
            version: profile.version.to_string(),
            verified_at: profile.verified_at.to_string(),
            is_free: profile.actual.is_free(),
        })
        .collect();

    let registry_status = state.pricing_registry.status();
    for entry in state.pricing_registry.all_entries() {
        if entry.model_id.to_lowercase().contains(&lower)
            || entry.display_name.to_lowercase().contains(&lower)
        {
            results.push(PricingProfileView {
                provider: entry.litellm_provider.to_string(),
                model_id: entry.model_id.to_string(),
                display_name: entry.display_name.to_string(),
                actual: PricingRates {
                    input_per_million: entry.rates.input_per_million,
                    output_per_million: entry.rates.output_per_million,
                    cached_per_million: entry.rates.cached_per_million,
                },
                reference: None,
                reference_model: None,
                source: "litellm".to_string(),
                version: registry_status.last_updated.clone().unwrap_or_default(),
                verified_at: String::new(),
                is_free: false,
            });
        }
    }

    results.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.model_id.cmp(&b.model_id))
    });
    results
}

// ──────────────────────────────────────────────────────────────────────
// Desktop shell commands (window / preferences / tray equivalents)
// ──────────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_preferences(state: State<'_, AppState>) -> Result<AppPreferences, String> {
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    Ok(prefs.data().clone())
}

/// Persist the selected UI theme key. The theme document itself is validated in
/// the frontend and loaded from the built-in bundle or the themes folder.
#[tauri::command]
fn set_ui_theme(state: State<'_, AppState>, theme: String) -> Result<AppPreferences, String> {
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    prefs.set_ui_theme(&theme)
}

/// Every community theme file, plus a per-file reason for anything skipped.
/// A malformed file never prevents the valid themes from loading.
#[tauri::command(async)]
fn list_user_themes() -> Result<themes::ThemeLibrary, String> {
    let dir = themes::themes_dir(&get_data_dir());
    themes::ensure_dir(&dir)?;
    Ok(themes::discover(&dir))
}

/// Save a copy of a theme into the user themes folder, choosing a unique name.
#[tauri::command]
fn save_user_theme(name: String, contents: String) -> Result<themes::UserThemeFile, String> {
    let dir = themes::themes_dir(&get_data_dir());
    themes::save(&dir, &name, &contents)
}

/// Open the drop-in themes folder in the platform file manager.
#[tauri::command]
fn open_themes_folder() -> Result<String, String> {
    let dir = themes::themes_dir(&get_data_dir());
    themes::open_folder(&dir)?;
    Ok(dir.display().to_string())
}

#[tauri::command]
fn get_window_state(app: tauri::AppHandle) -> window::WindowState {
    window::window_state(&app)
}

/// Turn always-on-top on/off. Returns the real native state afterwards.
#[tauri::command]
fn set_always_on_top(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    window::apply_always_on_top(&app, enabled)
}

#[tauri::command]
fn toggle_always_on_top(app: tauri::AppHandle) -> Result<bool, String> {
    window::toggle_always_on_top(&app)
}

#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    window::show_main_window(&app);
}

/// Hide the window to the tray; background synchronization continues.
#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    window::hide_main_window(&app);
}

/// Terminate the application (same path as the tray's Quit item).
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    window::quit_application(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = get_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!(
            "Warning: Failed to create data directory {}: {}",
            data_dir.display(),
            e
        );
    }

    // Logging and crash capture come before anything that can fail, so a startup
    // failure is diagnosable instead of silent (a Windows release build has no
    // console to print to).
    logging::init(&data_dir);
    logging::install_panic_hook(&data_dir);

    // 1. Usage history. A broken database must not leave the user with a window
    //    that silently does nothing: log the reason and exit with a non-zero
    //    code, pointing at the file that holds the detail.
    let db_path: PathBuf = data_dir.join("freetokenmeter.db");
    let db = match Database::new(&db_path) {
        Ok(db) => db,
        Err(e) => {
            log::error!("Failed to initialize database {}: {}", db_path.display(), e);
            eprintln!(
                "FreeTokenMeter could not open its history database:\n  {}\n  {}\nSee {} for details.",
                db_path.display(),
                e,
                logging::log_path(&data_dir).display()
            );
            std::process::exit(1);
        }
    };

    // 2. Preferences (JSON, separate from usage history).
    let prefs_path = data_dir.join("preferences.json");
    let prefs = PreferencesStore::load(prefs_path);
    let restore_always_on_top = prefs.data().always_on_top;

    // 3. Pricing registry: show cached prices immediately, refresh in background.
    let pricing_registry = Arc::new(RegistryState::new(&data_dir));
    pricing_registry.load_cache();

    let state = AppState {
        db: Mutex::new(db),
        providers: provider::default_registry(),
        prefs: Mutex::new(prefs),
        last_sync_report: Mutex::new(None),
        pricing_registry: pricing_registry.clone(),
        syncing: AtomicBool::new(false),
        shutdown: AtomicBool::new(false),
        geometry_events: AtomicU64::new(0),
    };

    let provider_count = state.providers.len();

    tauri::Builder::default()
        // Two meters writing to one SQLite file would race on the same cursors.
        // A second launch surfaces the window that is already running instead.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            info!("Another instance tried to start; focusing the running window");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();

            // 1. Restore always-on-top before the window is shown.
            if restore_always_on_top {
                if let Err(e) = window::apply_always_on_top(&handle, true) {
                    warn!("Could not restore always-on-top: {}", e);
                }
            }

            // 2. Validate and restore the saved window geometry.
            window::restore_geometry(&handle);

            // 3. System tray (optional component — never fatal).
            tray::setup(&handle);

            // 4. Provider synchronization + debounced geometry writes.
            sync::spawn_background_scheduler(handle.clone());
            window::spawn_geometry_persister(handle.clone());

            // 5. Refresh the pricing registry off the UI thread.
            std::thread::spawn(move || {
                if pricing_registry.needs_refresh() {
                    match pricing_registry.refresh() {
                        Ok(true) => info!("Pricing registry refreshed"),
                        Ok(false) => info!("Pricing registry cache is current"),
                        Err(e) => warn!("Pricing registry refresh failed: {}", e),
                    }
                }
            });

            // 6. Show the window last so it appears in its restored geometry.
            window::show_main_window(&handle);

            info!(
                "FreeTokenMeter ready ({} providers, pricing {}, log {})",
                provider_count,
                pricing::PRICING_VERSION,
                logging::log_path(&data_dir).display()
            );
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let app = window.app_handle();
                if window::is_shutting_down(app) {
                    // Quit was requested: let the close go through.
                    window::persist_geometry(app);
                } else {
                    // Hide to tray; synchronization keeps running.
                    api.prevent_close();
                    window::persist_geometry(app);
                    window::hide_main_window(app);
                }
            }
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                window::mark_geometry_dirty(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            sync_all,
            get_last_sync_report,
            get_dashboard,
            get_calendar,
            get_providers,
            get_provider_descriptors,
            get_future_providers,
            set_provider_enabled,
            set_backfill_days,
            get_pricing_registry,
            get_registry_status,
            refresh_pricing,
            search_pricing_registry,
            get_preferences,
            set_ui_theme,
            list_user_themes,
            save_user_theme,
            open_themes_folder,
            get_window_state,
            set_always_on_top,
            toggle_always_on_top,
            show_window,
            hide_window,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
