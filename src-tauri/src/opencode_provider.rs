//! OpenCode provider.
//!
//! # Source
//!
//! OpenCode stores its own SQLite database at
//! `~/.local/share/opencode/opencode.db` (or `$XDG_DATA_HOME/opencode`). The
//! session table already carries per-session token accounting and, for paid
//! models, a real cost:
//!
//! ```text
//! session(id, model, tokens_input, tokens_output, tokens_reasoning,
//!         tokens_cache_read, tokens_cache_write, cost, time_created, time_updated)
//! ```
//!
//! Recent OpenCode versions moved this state to `session_v2` and stopped writing
//! the old `session` table, so that table is read only as a fallback. The column
//! set is the same; `session_v2` simply keeps history that `session` no longer
//! receives.
//!
//! `model` is a JSON object (`{"id":"mimo-v2.5-free","providerID":"opencode"}`).
//!
//! # Cost semantics
//!
//! OpenCode reports a non-zero `cost` only for paid usage, so the payment mode
//! is derived per session: a positive cost means an API-style charge, zero cost
//! means a promotional/free model whose value is expressed through reference
//! pricing instead.
//!
//! # Incremental sync
//!
//! `time_updated` is the change marker: sessions touched since the last pass are
//! re-read and upserted, so a session that grows after its first sync replaces
//! its own earlier row instead of double counting.
//!
//! # Read-only
//!
//! The database belongs to another application. It is opened read-only, with a
//! busy timeout, so FreeTokenMeter can never create, repair or journal OpenCode's
//! own files.

use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use log::{error, info, warn};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::database::UsageRecord;
use crate::pricing::{value_usage, PricingInput};
use crate::provider::{FetchOutcome, FetchRequest, UsageProvider};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

/// A row from OpenCode's session table.
#[derive(Debug, Clone)]
pub(crate) struct OpenCodeSession {
    pub(crate) id: String,
    pub(crate) model_json: Option<String>,
    pub(crate) tokens_input: Option<i64>,
    pub(crate) tokens_output: Option<i64>,
    pub(crate) tokens_reasoning: Option<i64>,
    pub(crate) tokens_cache_read: Option<i64>,
    pub(crate) tokens_cache_write: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) time_created: i64,
    pub(crate) time_updated: i64,
}

/// Parsed model info from OpenCode's JSON model field.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeModel {
    id: String,
    #[serde(rename = "providerID")]
    provider_id: String,
    variant: Option<String>,
}

pub struct OpenCodeProvider {
    db_path: PathBuf,
}

impl OpenCodeProvider {
    pub const ID: &'static str = crate::usage::provider_id::OPENCODE;

    pub fn new() -> Self {
        Self::with_path(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local")
                .join("share")
                .join("opencode")
                .join("opencode.db"),
        )
    }

    /// Point the provider at an explicit database (tests, portable installs).
    pub fn with_path(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Parse the model JSON into `(model_id, provider_id, variant)`.
    pub fn parse_model(model_json: &Option<String>) -> (String, String, Option<String>) {
        match model_json {
            Some(json) => match serde_json::from_str::<OpenCodeModel>(json) {
                Ok(m) => (m.id, m.provider_id, m.variant),
                Err(e) => {
                    warn!("Failed to parse OpenCode model JSON '{}': {}", json, e);
                    ("unknown".to_string(), "opencode".to_string(), None)
                }
            },
            None => ("unknown".to_string(), "opencode".to_string(), None),
        }
    }

    /// Normalize one session row into the shared usage model.
    pub fn normalize_session(
        session: &OpenCodeSession,
        registry: Option<&crate::registry::RegistryState>,
    ) -> UsageRecord {
        let (model_name, _provider_id, _variant) = Self::parse_model(&session.model_json);
        let cost = session.cost.filter(|c| *c > 0.0);

        // OpenCode's own total counts every token class it tracks.
        let total_tokens = session.tokens_input.unwrap_or(0)
            + session.tokens_output.unwrap_or(0)
            + session.tokens_reasoning.unwrap_or(0)
            + session.tokens_cache_read.unwrap_or(0)
            + session.tokens_cache_write.unwrap_or(0);

        let payment_mode = if cost.is_some() {
            PaymentMode::Api
        } else {
            PaymentMode::Promotional
        };

        let value = value_usage(
            &PricingInput {
                provider: Self::ID,
                model: &model_name,
                provider_cost_usd: cost,
                billing_units: None,
                billing_unit: BillingUnit::Usd,
                payment_mode,
                input_tokens: session.tokens_input,
                output_tokens: session.tokens_output,
                cached_tokens: session.tokens_cache_read,
                cache_write_tokens: session.tokens_cache_write,
            },
            registry,
        );

        UsageRecord {
            id: format!("oc_{}", session.id),
            provider: Self::ID.to_string(),
            model: model_name,
            timestamp: Utc
                .timestamp_millis_opt(session.time_created)
                .single()
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
            input_tokens: session.tokens_input,
            output_tokens: session.tokens_output,
            cached_tokens: session.tokens_cache_read,
            cache_write_tokens: session.tokens_cache_write,
            reasoning_tokens: session.tokens_reasoning,
            total_tokens,
            actual_cost_usd: value.actual_cost_usd,
            reference_value: value.reference_value,
            free_value: value.free_value,
            billing_unit: value.billing_unit.as_str().to_string(),
            billing_units: value.billing_units,
            // OpenCode bills in plain USD and records no tool version.
            billing_metric_name: None,
            payment_mode: value.payment_mode.as_str().to_string(),
            pricing_status: value.pricing_status.as_str().to_string(),
            pricing_source: Some(value.pricing_source.as_str().to_string()),
            pricing_version: Some(value.pricing_version),
            reference_model: value.reference_model,
            reference_input_rate: value.reference_input_rate,
            reference_output_rate: value.reference_output_rate,
            reference_cached_rate: value.reference_cached_rate,
            session_id: Some(session.id.clone()),
            event_id: Some(session.id.clone()),
            source_type: SourceType::OpenCodeDatabase.as_str().to_string(),
            source_id: Some(session.id.clone()),
            source_version: None,
        }
    }

    /// Sessions touched since `since_ms` (all sessions when `None`).
    fn fetch_sessions(
        conn: &Connection,
        table: &str,
        since_ms: Option<i64>,
    ) -> Result<Vec<OpenCodeSession>, String> {
        let base = format!(
            "SELECT id, model, tokens_input, tokens_output, tokens_reasoning,
                    tokens_cache_read, tokens_cache_write, cost, time_created, time_updated
             FROM {}",
            table
        );
        let sql = if since_ms.is_some() {
            format!("{} WHERE time_updated > ?1 ORDER BY time_created ASC", base)
        } else {
            format!("{} ORDER BY time_created ASC", base)
        };

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            error!("Failed to prepare OpenCode session query: {}", e);
            format!("Failed to prepare session query: {}", e)
        })?;

        let row_mapper = |row: &rusqlite::Row| {
            Ok(OpenCodeSession {
                id: row.get(0)?,
                model_json: row.get(1)?,
                tokens_input: row.get(2)?,
                tokens_output: row.get(3)?,
                tokens_reasoning: row.get(4)?,
                tokens_cache_read: row.get(5)?,
                tokens_cache_write: row.get(6)?,
                cost: row.get(7)?,
                time_created: row.get(8)?,
                time_updated: row.get(9)?,
            })
        };

        let sessions: Vec<OpenCodeSession> = if let Some(sync_time) = since_ms {
            stmt.query_map(rusqlite::params![sync_time], row_mapper)
        } else {
            stmt.query_map([], row_mapper)
        }
        .map_err(|e| {
            error!("Failed to query OpenCode sessions: {}", e);
            format!("Failed to query sessions: {}", e)
        })?
        .filter_map(|row| match row {
            Ok(session) => Some(session),
            Err(e) => {
                warn!("Skipping malformed OpenCode session row: {}", e);
                None
            }
        })
        .collect();

        info!(
            "Fetched {} session(s) from OpenCode ({})",
            sessions.len(),
            table
        );
        Ok(sessions)
    }

    /// Highest `time_updated` in the database.
    ///
    /// This is the correct incremental watermark because it comes from the
    /// database's own clock rather than the application's: a wall-clock marker
    /// would skip sessions whose rows are stamped earlier than "now".
    fn max_time_updated(conn: &Connection, table: &str) -> Result<i64, String> {
        conn.query_row(
            &format!("SELECT COALESCE(MAX(time_updated), 0) FROM {}", table),
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| format!("Failed to read the OpenCode session watermark: {}", e))
    }

    /// Open OpenCode's database read-only.
    ///
    /// Never `READ_WRITE`: a missing file must not be created here, and the host
    /// application's journal must not be touched. A busy timeout absorbs a
    /// momentary write from OpenCode itself instead of failing the whole pass.
    fn open_read_only(&self) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| {
            error!("Failed to open OpenCode database: {}", e);
            format!("Failed to open OpenCode database: {}", e)
        })?;
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        Ok(conn)
    }

    /// Which session table this OpenCode build actually writes.
    ///
    /// `session_v2` is preferred when it exists and still has the columns this
    /// adapter reads; older installations only have `session`.
    fn session_table(conn: &Connection) -> &'static str {
        const REQUIRED: &[&str] = &[
            "id",
            "model",
            "tokens_input",
            "tokens_output",
            "tokens_reasoning",
            "tokens_cache_read",
            "tokens_cache_write",
            "cost",
            "time_created",
            "time_updated",
        ];

        for candidate in ["session_v2", "session"] {
            let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({})", candidate)) else {
                continue;
            };
            let columns: Vec<String> = match stmt.query_map([], |row| row.get::<_, String>(1)) {
                Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
                Err(_) => continue,
            };
            if REQUIRED
                .iter()
                .all(|name| columns.iter().any(|c| c == name))
            {
                return candidate;
            }
        }

        "session"
    }
}

impl Default for OpenCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageProvider for OpenCodeProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn short_label(&self) -> &'static str {
        "OPENCODE"
    }

    fn source_type(&self) -> &'static str {
        SourceType::OpenCodeDatabase.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        PaymentMode::Promotional
    }

    fn availability(&self) -> ProviderAvailability {
        if self.db_path.exists() {
            ProviderAvailability::Connected
        } else {
            ProviderAvailability::NotInstalled
        }
    }

    fn detail(&self) -> Option<String> {
        match self.availability() {
            ProviderAvailability::Connected => Some(format!(
                "Database: {} (session_v2 when present)",
                self.db_path.display()
            )),
            _ => Some("OpenCode installation was not detected.".to_string()),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        if !self.db_path.exists() {
            return Err("OpenCode database not found".to_string());
        }

        let conn = self.open_read_only()?;
        let table = Self::session_table(&conn);

        let cursor = request.cursor.cloned().unwrap_or_default();
        // The backfill window constrains the *first* pass; afterwards the cursor
        // is authoritative so only sessions touched since the last pass are read.
        let since_ms = match (cursor.last_timestamp_ms, request.since_ms) {
            (Some(cursor_ms), Some(window_ms)) => Some(cursor_ms.max(window_ms)),
            (Some(cursor_ms), None) => Some(cursor_ms),
            (None, window) => window,
        };

        let sessions = Self::fetch_sessions(&conn, table, since_ms)?;
        let watermark = Self::max_time_updated(&conn, table)?;

        let truncated = sessions.len() > request.limit;
        let taken = &sessions[..sessions.len().min(request.limit)];

        let records: Vec<UsageRecord> = taken
            .iter()
            .map(|session| Self::normalize_session(session, request.registry))
            .collect();

        let mut next = cursor.clone();
        // Advance past what was actually imported. When the pass hit the record
        // ceiling the watermark becomes the newest imported session, so the
        // remainder is picked up next time; freezing it (the previous behaviour)
        // re-read the same prefix on every pass and never made progress.
        next.last_timestamp_ms = Some(if truncated {
            taken
                .iter()
                .map(|session| session.time_updated)
                .max()
                .unwrap_or(0)
        } else {
            watermark.max(cursor.last_timestamp_ms.unwrap_or(0))
        });
        if let Some(last) = taken.last() {
            next.last_event_id = Some(last.id.clone());
        }

        let scanned = records.len();
        Ok(FetchOutcome {
            records,
            cursor: next,
            scanned,
            skipped: 0,
        })
    }

    fn default_backfill_days(&self) -> i64 {
        30
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderCursor;
    use chrono::Utc;

    fn session(cost: f64, input: i64) -> OpenCodeSession {
        OpenCodeSession {
            id: "ses_1".to_string(),
            model_json: Some(
                r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":null}"#.to_string(),
            ),
            tokens_input: Some(input),
            tokens_output: Some(500),
            tokens_reasoning: Some(0),
            tokens_cache_read: Some(100),
            tokens_cache_write: Some(0),
            cost: Some(cost),
            time_created: 1_700_000_000_000,
            time_updated: 1_700_000_000_000,
        }
    }

    /// A temporary OpenCode-shaped database with the given session table(s).
    ///
    /// `rows` is `(id, time_created, time_updated, tokens_input)`.
    fn scratch_db(
        label: &str,
        tables: &[&str],
        rows: &[(&str, i64, i64, i64)],
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ftm-opencode-{}-{}-{}",
            label,
            std::process::id(),
            crate::test_support::unique_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        for table in tables {
            conn.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {} (
                    id TEXT PRIMARY KEY, model TEXT,
                    tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,
                    tokens_cache_read INTEGER, tokens_cache_write INTEGER, cost REAL,
                    time_created INTEGER, time_updated INTEGER);",
                table
            ))
            .unwrap();
            for (id, created, updated, input) in rows {
                conn.execute(
                    &format!(
                        "INSERT OR REPLACE INTO {} VALUES (?1, ?2, ?3, 0, 0, 0, 0, 0.0, ?4, ?5)",
                        table
                    ),
                    rusqlite::params![
                        id,
                        r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":null}"#,
                        input,
                        created,
                        updated
                    ],
                )
                .unwrap();
            }
        }
        drop(conn);
        (dir, db_path)
    }

    fn fetch(
        provider: &OpenCodeProvider,
        cursor: Option<&ProviderCursor>,
        since: Option<i64>,
        limit: usize,
    ) -> FetchOutcome {
        provider
            .fetch(&FetchRequest {
                cursor,
                since_ms: since,
                registry: None,
                limit,
            })
            .expect("fetch")
    }

    #[test]
    fn prefers_session_v2_when_the_current_opencode_schema_is_present() {
        // The new table holds strictly more history; reading only `session`
        // silently dropped everything OpenCode wrote after the migration.
        let dir = std::env::temp_dir().join(format!(
            "ftm-opencode-v2-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("opencode.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            for table in ["session", "session_v2"] {
                conn.execute_batch(&format!(
                    "CREATE TABLE {} (id TEXT PRIMARY KEY, model TEXT,
                        tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,
                        tokens_cache_read INTEGER, tokens_cache_write INTEGER, cost REAL,
                        time_created INTEGER, time_updated INTEGER);",
                    table
                ))
                .unwrap();
            }
            let model = r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":null}"#;
            conn.execute(
                "INSERT INTO session VALUES ('legacy', ?1, 10, 0, 0, 0, 0, 0.0, 1000, 1000)",
                rusqlite::params![model],
            )
            .unwrap();
            for (id, ts) in [("a", 2000i64), ("b", 3000i64)] {
                conn.execute(
                    "INSERT INTO session_v2 VALUES (?1, ?2, 10, 0, 0, 0, 0, 0.0, ?3, ?3)",
                    rusqlite::params![id, model, ts],
                )
                .unwrap();
            }
        }

        let provider = OpenCodeProvider::with_path(db_path);
        let outcome = fetch(&provider, None, None, 1_000);

        let mut ids: Vec<&str> = outcome
            .records
            .iter()
            .map(|r| r.session_id.as_deref().unwrap())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"], "session_v2 is the live table");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_pass_honours_the_backfill_window() {
        let now = Utc::now().timestamp_millis();
        let (dir, db_path) = scratch_db(
            "backfill",
            &["session"],
            &[
                ("old", now - 90 * 86_400_000, now - 90 * 86_400_000, 5),
                ("new", now, now, 7),
            ],
        );

        let provider = OpenCodeProvider::with_path(db_path);
        let outcome = fetch(&provider, None, Some(now - 30 * 86_400_000), 1_000);

        assert_eq!(
            outcome.records.len(),
            1,
            "the 30-day window excludes the stale session"
        );
        assert_eq!(outcome.records[0].session_id.as_deref(), Some("new"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncation_advances_instead_of_stalling_forever() {
        let (dir, db_path) = scratch_db(
            "truncate",
            &["session"],
            &[
                ("s1", 1_000, 1_000, 1),
                ("s2", 2_000, 2_000, 2),
                ("s3", 3_000, 3_000, 3),
            ],
        );
        let provider = OpenCodeProvider::with_path(db_path);

        let first = fetch(&provider, None, None, 2);
        assert_eq!(first.records.len(), 2);
        assert_eq!(first.cursor.last_timestamp_ms, Some(2_000));

        // The previous behaviour left the watermark unset and re-imported s1+s2
        // on every pass, so s3 was unreachable.
        let second = fetch(&provider, Some(&first.cursor), None, 2);
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].session_id.as_deref(), Some("s3"));

        let third = fetch(&provider, Some(&second.cursor), None, 2);
        assert!(
            third.records.is_empty(),
            "a complete pass imports nothing new"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_source_database_is_opened_read_only() {
        let (dir, db_path) = scratch_db("readonly", &["session"], &[("s1", 1_000, 1_000, 1)]);
        let before = std::fs::read(&db_path).unwrap();

        let provider = OpenCodeProvider::with_path(db_path.clone());
        let outcome = fetch(&provider, None, None, 100);
        assert_eq!(outcome.records.len(), 1);

        assert_eq!(
            std::fs::read(&db_path).unwrap(),
            before,
            "the database must not be written"
        );
        assert!(
            !db_path.with_extension("db-wal").exists(),
            "no WAL sidecar in OpenCode's directory"
        );
        assert!(
            !db_path.with_extension("db-journal").exists(),
            "no journal in OpenCode's directory"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_the_model_object() {
        let (model, provider, variant) = OpenCodeProvider::parse_model(&Some(
            r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":"x"}"#.to_string(),
        ));
        assert_eq!(model, "mimo-v2.5-free");
        assert_eq!(provider, "opencode");
        assert_eq!(variant.as_deref(), Some("x"));
    }

    #[test]
    fn unparseable_model_json_does_not_invent_a_model() {
        let (model, _, _) = OpenCodeProvider::parse_model(&Some("not json".to_string()));
        assert_eq!(model, "unknown");
    }

    #[test]
    fn free_session_is_promotional_with_a_known_zero_cost() {
        let record = OpenCodeProvider::normalize_session(&session(0.0, 1_000_000), None);
        assert_eq!(record.payment_mode, "promotional");
        assert_eq!(record.actual_cost_usd, Some(0.0));
        assert_eq!(record.total_tokens, 1_000_500 + 100);
        assert_eq!(record.source_type, "opencode_database");
    }

    #[test]
    fn paid_session_uses_the_provider_reported_cost() {
        let record = OpenCodeProvider::normalize_session(&session(1.25, 1_000_000), None);
        assert_eq!(record.payment_mode, "api");
        assert_eq!(record.actual_cost_usd, Some(1.25));
        assert_eq!(record.pricing_status, "actual_cost_known");
    }

    #[test]
    fn missing_token_classes_stay_null() {
        let mut raw = session(0.0, 10);
        raw.tokens_cache_write = None;
        raw.tokens_reasoning = None;
        let record = OpenCodeProvider::normalize_session(&raw, None);
        assert!(record.cache_write_tokens.is_none());
        assert!(record.reasoning_tokens.is_none());
    }

    #[test]
    fn reports_not_installed_without_a_database() {
        let provider =
            OpenCodeProvider::with_path(PathBuf::from("/definitely/missing/opencode.db"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
    }

    /// The session identity is stable, so repeated syncs upsert the same row.
    #[test]
    fn record_id_is_derived_from_the_session_id() {
        let record = OpenCodeProvider::normalize_session(&session(0.0, 1), None);
        assert_eq!(record.id, "oc_ses_1");
        assert_eq!(record.event_id.as_deref(), Some("ses_1"));
    }

    /// A cursor advanced by a pass must not re-import unchanged sessions.
    #[test]
    fn cursor_uses_the_database_watermark() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-opencode-inc-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("opencode.db");

        let create = |updated: i64| {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS session (
                    id TEXT PRIMARY KEY, model TEXT,
                    tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,
                    tokens_cache_read INTEGER, tokens_cache_write INTEGER, cost REAL,
                    time_created INTEGER, time_updated INTEGER);",
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    "ses_1",
                    r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":null}"#,
                    1_000i64,
                    500i64,
                    0i64,
                    100i64,
                    0i64,
                    0.0f64,
                    1_700_000_000_000i64,
                    updated
                ],
            )
            .unwrap();
        };

        create(1_700_000_000_000);
        let provider = OpenCodeProvider::with_path(db_path.clone());
        let first = provider
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(first.records.len(), 1);

        let second = provider
            .fetch(&FetchRequest {
                cursor: Some(&first.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert!(
            second.records.is_empty(),
            "unchanged session must not be re-read"
        );

        create(1_700_000_100_000);
        let third = provider
            .fetch(&FetchRequest {
                cursor: Some(&second.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(third.records.len(), 1, "a touched session is re-read");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
