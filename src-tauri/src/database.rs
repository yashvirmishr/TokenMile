//! SQLite persistence for normalized usage history.
//!
//! One table, many providers. There is deliberately no `claude_usage` or
//! `codex_usage` table: provider-specific detail lives in normalized columns
//! (`billing_unit`, `payment_mode`, `source_type`, `source_id`) so adding a
//! provider never means a schema fork.
//!
//! # Schema version 6
//!
//! * `event_time_ms` is the *event* timestamp in epoch milliseconds. Time-range
//!   filtering and activity ordering use it; `time_created` is only ever the
//!   ingestion time and must never drive a "today" query.
//! * token metrics are nullable — `NULL` means "the source does not report
//!   this", which is different from `0`
//! * `actual_cost_usd` is nullable — subscription usage has no per-token spend
//! * `provider_cost_usd` keeps what the provider's own tooling computed
//! * billing units (`ai_credits`, ...) are stored beside, not inside, USD
//! * `event_id` / `source_type` / `source_id` record provenance for
//!   provider-specific deduplication and debugging

use chrono::Utc;
use log::{info, warn};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::PathBuf;

// Re-exported so providers and commands share one import path for the whole
// normalized data model.
pub use crate::usage::{
    ActivityEntry, DashboardData, ModelUsage, ProviderStatus, ProviderUsage, SeriesPoint,
    UsageRecord, UsageSeries, UsageSummary,
};

/// Bucket width for the dashboard's usage chart.
///
/// The range decides it, so the chart never draws a single bar for "today" and
/// never draws 300 unreadable ones for "all time".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Hour,
    Day,
    Week,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::Hour => "hour",
            Granularity::Day => "day",
            Granularity::Week => "week",
        }
    }

    /// SQLite expression that produces this bucket's key.
    ///
    /// `localtime` on purpose: a bar belongs to the day the user experienced. The
    /// UI builds its axis with the same formats, from local `Date` objects.
    fn bucket_key_expr(self) -> &'static str {
        match self {
            Granularity::Hour => {
                "strftime('%Y-%m-%dT%H', event_time_ms / 1000, 'unixepoch', 'localtime')"
            }
            Granularity::Day => {
                "strftime('%Y-%m-%d', event_time_ms / 1000, 'unixepoch', 'localtime')"
            }
            Granularity::Week => {
                // Monday of the week the record falls in: advance to the next
                // Sunday, then step back six days.
                "strftime('%Y-%m-%d', event_time_ms / 1000, 'unixepoch', 'localtime', 'weekday 0', '-6 days')"
            }
        }
    }
}

/// Pick the chart granularity for a range in days (`None` = all history).
pub fn granularity_for(days: Option<i64>) -> Granularity {
    match days {
        Some(1) => Granularity::Hour,
        Some(d) if d <= 60 => Granularity::Day,
        _ => Granularity::Week,
    }
}

/// The `usage_records` table for schema v5 and later.
///
/// Indexes are created separately, *after* any migration, because an index over
/// a new column cannot exist while an older table shape is still in place.
const USAGE_TABLE_DDL: &str = "
CREATE TABLE IF NOT EXISTS usage_records (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cached_tokens INTEGER,
    cache_write_tokens INTEGER,
    reasoning_tokens INTEGER,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    provider_cost_usd REAL,
    actual_cost_usd REAL,
    reference_value REAL,
    free_value REAL,
    billing_unit TEXT NOT NULL DEFAULT 'unknown',
    billing_units REAL,
    billing_metric_name TEXT,
    payment_mode TEXT NOT NULL DEFAULT 'unknown',
    pricing_status TEXT NOT NULL DEFAULT 'unknown',
    pricing_source TEXT,
    pricing_version TEXT,
    reference_model TEXT,
    reference_input_rate REAL,
    reference_output_rate REAL,
    reference_cached_rate REAL,
    session_id TEXT,
    event_id TEXT,
    source_type TEXT NOT NULL DEFAULT 'unknown',
    source_id TEXT,
    source_version TEXT,
    time_created INTEGER NOT NULL,
    event_time_ms INTEGER NOT NULL DEFAULT 0
);
";

const SYNC_STATE_DDL: &str = "
CREATE TABLE IF NOT EXISTS sync_state (
    provider TEXT PRIMARY KEY,
    last_sync_time INTEGER NOT NULL DEFAULT 0,
    last_session_id TEXT,
    cursor_json TEXT,
    state TEXT NOT NULL DEFAULT 'unknown',
    detail TEXT
);
";

const USAGE_INDEXES_SQL: &str = "
CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON usage_records(timestamp);
CREATE INDEX IF NOT EXISTS idx_usage_provider ON usage_records(provider);
CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_records(model);
CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_records(session_id);
CREATE INDEX IF NOT EXISTS idx_usage_time_created ON usage_records(time_created);
CREATE INDEX IF NOT EXISTS idx_usage_source ON usage_records(source_type);
CREATE INDEX IF NOT EXISTS idx_usage_event_time ON usage_records(event_time_ms);
";

/// Current schema version, recorded in `PRAGMA user_version`.
///
/// Column-presence checks remain the source of truth for whether a migration is
/// needed (they are idempotent and survive a hand-edited database); the version
/// number is bookkeeping so the state of a database can be read at a glance.
pub const SCHEMA_VERSION: i64 = 6;

/// Parse an RFC 3339 timestamp into epoch milliseconds.
///
/// Falls back to `0` when the source wrote something unparseable; the caller
/// then substitutes the ingestion time so a record is never invisible to a
/// time-range query.
pub fn parse_event_ms(timestamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

/// Stored per-provider totals, keyed by canonical provider id, for the health
/// view. `source_version` is the newest tool version any of the provider's
/// records reported — `None` when none of them carried one.
#[derive(Debug, Clone, Default)]
pub struct ProviderStat {
    pub records: i64,
    pub tokens: i64,
    pub source_version: Option<String>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: &PathBuf) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;

        // WAL keeps readers (the dashboard) from blocking the writer (a sync
        // pass); `busy_timeout` turns a momentary lock into a short wait instead
        // of an immediate failure, which matters when a second instance or a
        // background pass is writing.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;

        // A fresh database gets the current schema straight away.
        conn.execute_batch(USAGE_TABLE_DDL)?;
        conn.execute_batch(SYNC_STATE_DDL)?;

        // Migrate an older database in place. `source_type` only exists from v5
        // on, so its absence identifies anything older.
        if !Self::column_exists(&conn, "usage_records", "source_type") {
            info!("Migrating usage schema to v5 (normalized multi-provider)");
            Self::migrate_to_v5(&conn)?;
        }

        // Columns added after v5 shipped are appended in place, so a database
        // that already holds real usage rows is never rebuilt for them. This
        // runs on every startup and is a no-op once the columns are there.
        Self::ensure_usage_columns(&conn)?;
        Self::ensure_sync_state_columns(&conn)?;

        // Records written before `event_time_ms` existed get it derived from
        // their own event timestamp, so historical rows are filterable by the
        // period they actually happened in.
        Self::backfill_event_time(&conn)?;

        // Indexes come last: an index over a new column cannot exist while an
        // older table shape is still in place.
        conn.execute_batch(USAGE_INDEXES_SQL)?;

        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;

        Ok(Self { conn })
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
        let sql = format!("PRAGMA table_info({})", table);
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return false;
        };
        let columns: Vec<String> = match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(_) => return false,
        };
        columns.iter().any(|name| name == column)
    }

    /// Append columns that were introduced after schema v5 was first created.
    ///
    /// `ALTER TABLE ... ADD COLUMN` only appends to the existing table, so unlike
    /// the v5 rebuild it never renames or recreates anything and existing rows
    /// are untouched. Each entry is guarded by a presence check, so this is safe
    /// to run against a fresh database, a just-rebuilt one, or a live one.
    fn ensure_usage_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
        const ADDED_COLUMNS: &[(&str, &str)] = &[
            (
                "billing_metric_name",
                "ALTER TABLE usage_records ADD COLUMN billing_metric_name TEXT",
            ),
            (
                "source_version",
                "ALTER TABLE usage_records ADD COLUMN source_version TEXT",
            ),
            (
                "event_time_ms",
                "ALTER TABLE usage_records ADD COLUMN event_time_ms INTEGER NOT NULL DEFAULT 0",
            ),
        ];

        for (column, ddl) in ADDED_COLUMNS {
            if !Self::column_exists(conn, "usage_records", column) {
                info!("Adding usage_records.{} (in-place, additive)", column);
                conn.execute_batch(ddl)?;
            }
        }
        Ok(())
    }

    /// Derive `event_time_ms` for rows written before the column existed.
    ///
    /// `julianday()` understands RFC 3339 including a fractional part and a
    /// numeric offset, so this works for every timestamp the providers emit. A
    /// row whose timestamp is unparseable falls back to its ingestion time so it
    /// stays visible rather than silently dropping out of every range query.
    fn backfill_event_time(conn: &Connection) -> Result<(), rusqlite::Error> {
        let derived = conn.execute(
            "UPDATE usage_records
                SET event_time_ms = CAST((julianday(timestamp) - 2440587.5) * 86400000.0 AS INTEGER)
              WHERE event_time_ms = 0
                AND timestamp IS NOT NULL
                AND timestamp <> ''",
            [],
        )?;

        let fallback = conn.execute(
            "UPDATE usage_records SET event_time_ms = time_created WHERE event_time_ms = 0",
            [],
        )?;

        if derived + fallback > 0 {
            info!(
                "Derived event_time_ms for {} record(s) ({} from timestamp, {} from ingestion time)",
                derived + fallback,
                derived,
                fallback
            );
        }
        Ok(())
    }

    fn ensure_sync_state_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
        for (column, ddl) in [
            (
                "cursor_json",
                "ALTER TABLE sync_state ADD COLUMN cursor_json TEXT",
            ),
            (
                "state",
                "ALTER TABLE sync_state ADD COLUMN state TEXT NOT NULL DEFAULT 'unknown'",
            ),
            ("detail", "ALTER TABLE sync_state ADD COLUMN detail TEXT"),
        ] {
            if !Self::column_exists(conn, "sync_state", column) {
                conn.execute_batch(ddl)?;
            }
        }
        Ok(())
    }

    /// Rebuild `usage_records` into the v5 shape, copying everything that was
    /// already there. Old per-token columns were `NOT NULL DEFAULT 0`, so the
    /// table has to be recreated rather than altered.
    fn migrate_to_v5(conn: &Connection) -> Result<(), rusqlite::Error> {
        let has = |column: &str| Self::column_exists(conn, "usage_records", column);
        let col = |column: &str, fallback: &str| {
            if has(column) {
                column.to_string()
            } else {
                fallback.to_string()
            }
        };

        // `estimated_cost` used to hold "what was charged"; v5 splits that into
        // an actual cost (nullable) and the provider-reported figure.
        let actual_cost = if has("estimated_cost") && has("pricing_status") {
            "CASE WHEN pricing_status = 'unknown' THEN NULL ELSE estimated_cost END"
        } else if has("estimated_cost") {
            "NULLIF(estimated_cost, 0.0)"
        } else {
            "NULL"
        };

        // Build the copy statement *before* touching the schema: every column
        // reference is resolved against the pre-migration table, and a column
        // that an older schema never had becomes a literal instead.
        let copy_sql = format!(
            "INSERT INTO usage_records (
                id, provider, model, timestamp,
                input_tokens, output_tokens, cached_tokens, cache_write_tokens,
                reasoning_tokens, total_tokens,
                provider_cost_usd, actual_cost_usd, reference_value, free_value,
                billing_unit, billing_units, payment_mode,
                pricing_status, pricing_source, pricing_version, reference_model,
                reference_input_rate, reference_output_rate, reference_cached_rate,
                session_id, event_id, source_type, source_id, time_created)
             SELECT
                id, provider, model, timestamp,
                {input}, {output}, {cached}, NULL,
                {reasoning}, COALESCE(total_tokens, 0),
                NULL, {actual}, {reference}, {free},
                'unknown', NULL, 'unknown',
                {status}, {pricing_source}, {pricing_version}, {reference_model},
                {ref_in}, {ref_out}, {ref_cached},
                {session}, NULL, 'unknown', NULL, COALESCE(time_created, 0)
             FROM usage_records_pre_v5",
            input = col("input_tokens", "NULL"),
            output = col("output_tokens", "NULL"),
            cached = col("cached_tokens", "NULL"),
            reasoning = col("reasoning_tokens", "NULL"),
            actual = actual_cost,
            reference = col("reference_value", "NULL"),
            free = col("free_value", "NULL"),
            status = col("pricing_status", "'unknown'"),
            pricing_source = col("pricing_source", "NULL"),
            pricing_version = col("pricing_version", "NULL"),
            reference_model = col("reference_model", "NULL"),
            ref_in = col("reference_input_rate", "NULL"),
            ref_out = col("reference_output_rate", "NULL"),
            ref_cached = col("reference_cached_rate", "NULL"),
            session = col("session_id", "NULL"),
        );

        conn.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| -> Result<(), rusqlite::Error> {
            conn.execute_batch("ALTER TABLE usage_records RENAME TO usage_records_pre_v5;")?;
            conn.execute_batch(USAGE_TABLE_DDL)?;
            // Cursors are re-derived on the next pass; the table shape is what
            // matters, so the old rows are simply dropped.
            conn.execute_batch("DROP TABLE IF EXISTS sync_state;")?;
            conn.execute_batch(SYNC_STATE_DDL)?;
            conn.execute_batch(&copy_sql)?;
            conn.execute_batch("DROP TABLE usage_records_pre_v5;")?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT;")?;
                info!("Usage schema migrated to v5");
                Ok(())
            }
            Err(e) => {
                warn!("v5 migration failed ({}); rolling back", e);
                let _ = conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Insert or update a batch of records in one transaction.
    ///
    /// Upsert is keyed on the record id, which is the single uniqueness rule for
    /// every provider: a session that grows after its first sync overwrites its
    /// own earlier row instead of double counting. `time_created` is deliberately
    /// **not** updated on conflict — it records when the row was first imported,
    /// so re-syncing old usage never moves it into "today".
    pub fn upsert_usage_records(&self, records: &[UsageRecord]) -> Result<usize, rusqlite::Error> {
        if records.is_empty() {
            return Ok(0);
        }

        let now = Utc::now().timestamp_millis();
        let tx = self.conn.unchecked_transaction()?;

        let mut written = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO usage_records
                 (id, provider, model, timestamp, event_time_ms,
                  input_tokens, output_tokens, cached_tokens, cache_write_tokens,
                  reasoning_tokens, total_tokens,
                  provider_cost_usd, actual_cost_usd, reference_value, free_value,
                  billing_unit, billing_units, billing_metric_name, payment_mode,
                  pricing_status, pricing_source, pricing_version, reference_model,
                  reference_input_rate, reference_output_rate, reference_cached_rate,
                  session_id, event_id, source_type, source_id, source_version, time_created)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                         ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26,
                         ?27, ?28, ?29, ?30, ?31, ?32)
                 ON CONFLICT(id) DO UPDATE SET
                     provider = excluded.provider,
                     model = excluded.model,
                     timestamp = excluded.timestamp,
                     event_time_ms = excluded.event_time_ms,
                     input_tokens = excluded.input_tokens,
                     output_tokens = excluded.output_tokens,
                     cached_tokens = excluded.cached_tokens,
                     cache_write_tokens = excluded.cache_write_tokens,
                     reasoning_tokens = excluded.reasoning_tokens,
                     total_tokens = excluded.total_tokens,
                     provider_cost_usd = excluded.provider_cost_usd,
                     actual_cost_usd = excluded.actual_cost_usd,
                     reference_value = excluded.reference_value,
                     free_value = excluded.free_value,
                     billing_unit = excluded.billing_unit,
                     billing_units = excluded.billing_units,
                     billing_metric_name = excluded.billing_metric_name,
                     payment_mode = excluded.payment_mode,
                     pricing_status = excluded.pricing_status,
                     pricing_source = excluded.pricing_source,
                     pricing_version = excluded.pricing_version,
                     reference_model = excluded.reference_model,
                     reference_input_rate = excluded.reference_input_rate,
                     reference_output_rate = excluded.reference_output_rate,
                     reference_cached_rate = excluded.reference_cached_rate,
                     session_id = excluded.session_id,
                     event_id = excluded.event_id,
                     source_type = excluded.source_type,
                     source_id = excluded.source_id,
                     source_version = excluded.source_version",
            )?;

            for record in records {
                // An unparseable timestamp must not make a record invisible to
                // every time-range query, so it falls back to the ingest time.
                let event_time_ms = match parse_event_ms(&record.timestamp) {
                    0 => now,
                    ms => ms,
                };

                stmt.execute(params![
                    record.id,
                    record.provider,
                    record.model,
                    record.timestamp,
                    event_time_ms,
                    record.input_tokens,
                    record.output_tokens,
                    record.cached_tokens,
                    record.cache_write_tokens,
                    record.reasoning_tokens,
                    record.total_tokens,
                    record.actual_cost_usd,
                    record.actual_cost_usd,
                    record.reference_value,
                    record.free_value,
                    record.billing_unit,
                    record.billing_units,
                    record.billing_metric_name,
                    record.payment_mode,
                    record.pricing_status,
                    record.pricing_source,
                    record.pricing_version,
                    record.reference_model,
                    record.reference_input_rate,
                    record.reference_output_rate,
                    record.reference_cached_rate,
                    record.session_id,
                    record.event_id,
                    record.source_type,
                    record.source_id,
                    record.source_version,
                    now,
                ])?;
                written += 1;
            }
        }

        tx.commit()?;
        Ok(written)
    }

    /// The persisted incremental cursor for a provider, as JSON.
    pub fn get_sync_cursor(&self, provider: &str) -> Result<Option<String>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT cursor_json FROM sync_state WHERE provider = ?1")?;
        let mut rows = stmt.query_map(params![provider], |row| row.get::<_, Option<String>>(0))?;
        match rows.next() {
            Some(row) => Ok(row?.filter(|s| !s.is_empty())),
            None => Ok(None),
        }
    }

    /// Milliseconds of the last successful sync for a provider.
    pub fn get_last_sync_time(&self, provider: &str) -> Result<Option<i64>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT last_sync_time FROM sync_state WHERE provider = ?1")?;
        let mut rows = stmt.query_map(params![provider], |row| row.get::<_, i64>(0))?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Record the outcome of one provider's sync pass.
    pub fn update_sync_state(
        &self,
        provider: &str,
        last_sync_time: i64,
        cursor_json: Option<&str>,
        state: &str,
        detail: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO sync_state (provider, last_sync_time, last_session_id, cursor_json, state, detail)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5)
             ON CONFLICT(provider) DO UPDATE SET
                 last_sync_time = CASE
                     WHEN excluded.last_sync_time > sync_state.last_sync_time
                     THEN excluded.last_sync_time
                     ELSE sync_state.last_sync_time
                 END,
                 cursor_json = COALESCE(excluded.cursor_json, sync_state.cursor_json),
                 state = excluded.state,
                 detail = excluded.detail",
            params![provider, last_sync_time, cursor_json, state, detail],
        )?;
        Ok(())
    }

    /// Stored state for a provider, for the health view.
    pub fn get_sync_state(
        &self,
        provider: &str,
    ) -> Result<Option<(String, Option<String>)>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT state, detail FROM sync_state WHERE provider = ?1")?;
        let mut rows = stmt.query_map(params![provider], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Per-provider totals plus the provenance fields the health view reports.
    /// Keyed by canonical provider id.
    pub fn provider_stats(&self) -> Result<HashMap<String, ProviderStat>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT provider,
                    COUNT(*),
                    COALESCE(SUM(total_tokens), 0),
                    MAX(source_version)
             FROM usage_records GROUP BY provider",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ProviderStat {
                    records: row.get(1)?,
                    tokens: row.get(2)?,
                    source_version: row.get(3)?,
                },
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Totals for one provider, used for sync deltas.
    pub fn get_provider_token_total(&self, provider: &str) -> Result<i64, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(SUM(total_tokens), 0) FROM usage_records WHERE provider = ?1",
        )?;
        let mut rows = stmt.query_map(params![provider], |row| row.get::<_, i64>(0))?;
        match rows.next() {
            Some(value) => value,
            None => Ok(0),
        }
    }

    /// Models in the local history with no pricing entry.
    /// Returns `(provider, model, tokens, records)`.
    pub fn get_unknown_models(&self) -> Result<Vec<(String, String, i64, i64)>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT provider, model, COALESCE(SUM(total_tokens), 0), COUNT(*)
             FROM usage_records
             WHERE pricing_status = 'unknown'
             GROUP BY provider, model
             ORDER BY 3 DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn count_records(&self) -> Result<usize, rusqlite::Error> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM usage_records")?;
        let mut rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        rows.next().unwrap_or(Ok(0)).map(|n| n as usize)
    }

    /// Build `WHERE ...` plus bound parameters for the time/provider filters.
    fn filters(
        days: Option<i64>,
        provider: Option<&str>,
    ) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
        let mut conditions = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(days) = days {
            let cutoff = Utc::now().timestamp_millis() - (days * 24 * 60 * 60 * 1000);
            // Event time, not ingest time: "today" means "usage that happened
            // today", not "rows imported today".
            conditions.push(format!("event_time_ms >= ?{}", values.len() + 1));
            values.push(Box::new(cutoff));
        }

        if let Some(provider) = provider {
            conditions.push(format!("provider = ?{}", values.len() + 1));
            values.push(Box::new(provider.to_string()));
        }

        let clause = if conditions.is_empty() {
            "WHERE 1=1".to_string()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        (clause, values)
    }

    pub fn get_usage_summary(
        &self,
        days: Option<i64>,
        provider: Option<&str>,
    ) -> Result<UsageSummary, rusqlite::Error> {
        let (where_clause, values) = Self::filters(days, provider);
        let query = format!(
            "SELECT
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(cached_tokens), 0),
                COUNT(*),
                SUM(actual_cost_usd),
                SUM(CASE WHEN actual_cost_usd IS NULL THEN 1 ELSE 0 END),
                SUM(reference_value),
                SUM(free_value),
                SUM(CASE WHEN billing_unit <> 'usd' THEN billing_units ELSE 0 END),
                SUM(CASE WHEN pricing_status = 'unknown' THEN 1 ELSE 0 END),
                SUM(CASE WHEN pricing_status = 'unknown' THEN total_tokens ELSE 0 END)
             FROM usage_records {}",
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let mut rows = stmt.query_map(refs.as_slice(), |row| {
            Ok(UsageSummary {
                total_tokens: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                reasoning_tokens: row.get(3)?,
                cached_tokens: row.get(4)?,
                request_count: row.get(5)?,
                actual_cost: row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                unbilled_records: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                reference_value: row.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                free_value: row.get::<_, Option<f64>>(9)?.unwrap_or(0.0),
                billing_units: row.get::<_, Option<f64>>(10)?.unwrap_or(0.0),
                unpriced_records: row.get::<_, Option<i64>>(11)?.unwrap_or(0),
                unpriced_tokens: row.get::<_, Option<i64>>(12)?.unwrap_or(0),
            })
        })?;

        match rows.next() {
            Some(row) => row,
            None => Ok(UsageSummary {
                total_tokens: 0,
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
                cached_tokens: 0,
                request_count: 0,
                actual_cost: 0.0,
                unbilled_records: 0,
                reference_value: 0.0,
                free_value: 0.0,
                billing_units: 0.0,
                unpriced_records: 0,
                unpriced_tokens: 0,
            }),
        }
    }

    /// Per-bucket, per-provider token totals for the dashboard chart.
    ///
    /// Buckets are local-time (see [`Granularity::bucket_key_expr`]) and emitted
    /// only where usage exists; the UI zero-fills the axis so a quiet period is a
    /// gap in the bars rather than a lie about missing data.
    pub fn get_series(
        &self,
        days: Option<i64>,
        provider: Option<&str>,
    ) -> Result<UsageSeries, rusqlite::Error> {
        self.get_series_with(granularity_for(days), days, provider)
    }

    /// One bucket per **day**, regardless of range, for the calendar view.
    ///
    /// The chart picks its bucket width from the range (hours, days, weeks); a
    /// year-shaped calendar cannot, because every cell is a single day. Asking
    /// for the same query with a fixed granularity keeps both views reading the
    /// same rows and the same local-time bucketing.
    pub fn get_calendar(
        &self,
        days: Option<i64>,
        provider: Option<&str>,
    ) -> Result<UsageSeries, rusqlite::Error> {
        self.get_series_with(Granularity::Day, days, provider)
    }

    fn get_series_with(
        &self,
        granularity: Granularity,
        days: Option<i64>,
        provider: Option<&str>,
    ) -> Result<UsageSeries, rusqlite::Error> {
        let (where_clause, values) = Self::filters(days, provider);
        let query = format!(
            "SELECT {} AS bucket, provider,
                    COALESCE(SUM(total_tokens), 0) AS tokens,
                    COUNT(*) AS requests
             FROM usage_records {} GROUP BY bucket, provider ORDER BY bucket",
            granularity.bucket_key_expr(),
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok(SeriesPoint {
                key: row.get(0)?,
                provider: row.get(1)?,
                tokens: row.get(2)?,
                requests: row.get(3)?,
            })
        })?;

        Ok(UsageSeries {
            granularity: granularity.as_str().to_string(),
            points: rows.filter_map(|r| r.ok()).collect(),
        })
    }

    /// Per-provider totals for the provider table.
    ///
    /// `provider` scopes the aggregate exactly like the summary and model
    /// totals, so selecting a provider in the UI filters this table too.
    pub fn get_provider_usage(
        &self,
        days: Option<i64>,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderUsage>, rusqlite::Error> {
        let (where_clause, values) = Self::filters(days, provider);
        let query = format!(
            "SELECT provider,
                    COALESCE(SUM(total_tokens), 0) AS tokens,
                    SUM(actual_cost_usd),
                    SUM(reference_value),
                    SUM(CASE WHEN billing_unit <> 'usd' THEN billing_units ELSE 0 END),
                    COALESCE(MIN(billing_unit), 'usd'),
                    MIN(billing_metric_name),
                    COUNT(*)
             FROM usage_records {} GROUP BY provider ORDER BY tokens DESC",
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;

        let raw: Vec<_> = rows.filter_map(|r| r.ok()).collect();
        let total_tokens: i64 = raw.iter().map(|(_, tokens, ..)| *tokens).sum();

        Ok(raw
            .into_iter()
            .map(
                |(
                    provider,
                    tokens,
                    actual_cost,
                    reference_value,
                    billing_units,
                    billing_unit,
                    billing_metric_name,
                    records,
                )| {
                    ProviderUsage {
                        provider,
                        tokens,
                        percentage: if total_tokens > 0 {
                            (tokens as f64 / total_tokens as f64) * 100.0
                        } else {
                            0.0
                        },
                        actual_cost,
                        reference_value,
                        billing_units: billing_units.filter(|units| *units > 0.0),
                        billing_unit,
                        billing_metric_name,
                        records,
                    }
                },
            )
            .collect())
    }

    /// Return the set of (provider, model) pairs that have at least one record.
    pub fn get_used_model_keys(&self) -> Result<Vec<(String, String)>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT provider, model FROM usage_records WHERE total_tokens > 0")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_model_usage(
        &self,
        days: Option<i64>,
        provider: Option<&str>,
    ) -> Result<Vec<ModelUsage>, rusqlite::Error> {
        let (where_clause, values) = Self::filters(days, provider);
        // MIN(pricing_status) picks the most resolved status seen for the model
        // ('actual_cost_known' sorts before 'unknown'), so a model that is priced
        // for at least one provider is not reported as unpriced.
        let query = format!(
            "SELECT provider, model,
                    COALESCE(SUM(total_tokens), 0) AS tokens,
                    SUM(actual_cost_usd),
                    SUM(reference_value),
                    SUM(CASE WHEN billing_unit <> 'usd' THEN billing_units ELSE 0 END),
                    COALESCE(MIN(billing_unit), 'usd'),
                    MIN(billing_metric_name),
                    MIN(pricing_status),
                    MIN(pricing_source)
             FROM usage_records {} GROUP BY provider, model HAVING tokens > 0 ORDER BY tokens DESC",
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;

        let raw: Vec<_> = rows.filter_map(|r| r.ok()).collect();
        let total_tokens: i64 = raw.iter().map(|(_, _, tokens, ..)| *tokens).sum();

        Ok(raw
            .into_iter()
            .map(
                |(
                    provider,
                    model,
                    tokens,
                    actual_cost,
                    reference_value,
                    billing_units,
                    billing_unit,
                    billing_metric_name,
                    pricing_status,
                    pricing_source,
                )| {
                    ModelUsage {
                        provider,
                        model,
                        tokens,
                        percentage: if total_tokens > 0 {
                            (tokens as f64 / total_tokens as f64) * 100.0
                        } else {
                            0.0
                        },
                        actual_cost,
                        reference_value,
                        billing_units: billing_units.filter(|units| *units > 0.0),
                        billing_unit,
                        billing_metric_name,
                        pricing_status,
                        pricing_source,
                    }
                },
            )
            .collect())
    }

    pub fn get_activity(
        &self,
        days: Option<i64>,
        limit: usize,
        provider: Option<&str>,
    ) -> Result<Vec<ActivityEntry>, rusqlite::Error> {
        let (where_clause, mut values) = Self::filters(days, provider);
        let query = format!(
            "SELECT timestamp, provider, model, total_tokens, billing_units, billing_unit
             FROM usage_records {} ORDER BY event_time_ms DESC LIMIT ?{}",
            where_clause,
            values.len() + 1
        );
        let limit_i64 = limit as i64;
        values.push(Box::new(limit_i64));

        let mut stmt = self.conn.prepare(&query)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok(ActivityEntry {
                timestamp: row.get(0)?,
                provider: row.get(1)?,
                model: row.get(2)?,
                tokens: row.get(3)?,
                billing_units: row.get(4)?,
                billing_unit: row.get(5)?,
            })
        })?;

        Ok(rows
            .filter_map(|r| {
                r.map_err(|e| warn!("Skipping malformed activity row: {}", e))
                    .ok()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability};

    fn temp_db(label: &str) -> Database {
        let dir = std::env::temp_dir().join(format!(
            "ftm-db-test-{}-{}-{}",
            label,
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Database::new(&dir.join("freetokenmeter.db")).expect("database")
    }

    fn record(id: &str, provider: &str, model: &str, total: i64) -> UsageRecord {
        UsageRecord {
            id: id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            input_tokens: Some(total),
            output_tokens: Some(0),
            cached_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
            total_tokens: total,
            actual_cost_usd: None,
            reference_value: None,
            free_value: None,
            billing_unit: BillingUnit::Usd.as_str().to_string(),
            billing_units: None,
            billing_metric_name: None,
            payment_mode: PaymentMode::Unknown.as_str().to_string(),
            pricing_status: "unknown".to_string(),
            pricing_source: None,
            pricing_version: None,
            reference_model: None,
            reference_input_rate: None,
            reference_output_rate: None,
            reference_cached_rate: None,
            session_id: None,
            event_id: None,
            source_type: "test_source".to_string(),
            source_id: None,
            source_version: None,
        }
    }

    #[test]
    fn upsert_is_idempotent_per_record_id() {
        let db = temp_db("upsert");
        let first = record("a", "codex", "gpt", 100);
        db.upsert_usage_records(&[first]).unwrap();
        db.upsert_usage_records(&[record("a", "codex", "gpt", 250)])
            .unwrap();

        assert_eq!(db.count_records().unwrap(), 1);
        assert_eq!(db.get_provider_token_total("codex").unwrap(), 250);
    }

    #[test]
    fn missing_metrics_stay_null_rather_than_zero() {
        let db = temp_db("nulls");
        db.upsert_usage_records(&[record("a", "claude-code", "sonnet", 500)])
            .unwrap();
        let summary = db.get_usage_summary(None, Some("claude-code")).unwrap();

        // input is reported; the cache columns were never reported at all.
        assert_eq!(summary.input_tokens, 500);
        assert_eq!(summary.cached_tokens, 0);
        let mut stmt = db
            .conn
            .prepare("SELECT cached_tokens, reasoning_tokens FROM usage_records WHERE id = 'a'")
            .unwrap();
        let (cached, reasoning): (Option<i64>, Option<i64>) = stmt
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert!(cached.is_none());
        assert!(reasoning.is_none());
    }

    #[test]
    fn subscription_records_have_no_actual_cost_but_keep_reference_value() {
        let db = temp_db("subscription");
        let mut rec = record("b", "codex", "gpt-5.3-codex", 1_000_000);
        rec.payment_mode = PaymentMode::Subscription.as_str().to_string();
        rec.reference_value = Some(5.91);
        rec.pricing_status = "reference_applied".to_string();
        db.upsert_usage_records(&[rec]).unwrap();

        let provider = db.get_provider_usage(None, None).unwrap();
        assert_eq!(provider.len(), 1);
        assert!(provider[0].actual_cost.is_none());
        assert_eq!(provider[0].reference_value, Some(5.91));

        let summary = db.get_usage_summary(None, None).unwrap();
        assert_eq!(summary.unbilled_records, 1);
        assert_eq!(summary.actual_cost, 0.0);
        assert_eq!(summary.reference_value, 5.91);
    }

    #[test]
    fn billing_units_are_aggregated_separately_from_usd() {
        let db = temp_db("credits");
        let mut rec = record("c", "github-copilot", "model-x", 910_000);
        rec.billing_unit = BillingUnit::AiCredits.as_str().to_string();
        rec.billing_units = Some(14.0);
        rec.payment_mode = PaymentMode::Subscription.as_str().to_string();
        rec.reference_value = Some(1.82);
        rec.pricing_status = "reference_applied".to_string();
        db.upsert_usage_records(&[rec]).unwrap();

        let provider = db.get_provider_usage(None, None).unwrap();
        assert_eq!(provider[0].billing_units, Some(14.0));
        assert_eq!(provider[0].billing_unit, "ai_credits");
        assert!(provider[0].actual_cost.is_none());

        let summary = db.get_usage_summary(None, None).unwrap();
        assert_eq!(summary.billing_units, 14.0);
    }

    #[test]
    fn unpriced_models_are_reported_separately_from_cost_totals() {
        let db = temp_db("unpriced");
        db.upsert_usage_records(&[record("d", "gemini-cli", "future-model", 2_000_000)])
            .unwrap();

        let summary = db.get_usage_summary(None, None).unwrap();
        assert_eq!(summary.unpriced_records, 1);
        assert_eq!(summary.unpriced_tokens, 2_000_000);
        assert_eq!(summary.actual_cost, 0.0);
        assert!(summary.unbilled_records == 1);

        let unknown = db.get_unknown_models().unwrap();
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].0, "gemini-cli");
        assert_eq!(unknown[0].2, 2_000_000);
    }

    #[test]
    fn provider_filter_scopes_every_aggregate() {
        let db = temp_db("filter");
        db.upsert_usage_records(&[
            record("e1", "opencode", "mimo-v2.5-free", 1_000),
            record("e2", "codex", "gpt", 2_000),
        ])
        .unwrap();

        let summary = db.get_usage_summary(None, Some("codex")).unwrap();
        assert_eq!(summary.total_tokens, 2_000);
        assert_eq!(summary.request_count, 1);

        // The provider table is scoped by the same filter: selecting [CODEX]
        // must not keep reporting every provider's totals.
        let providers = db.get_provider_usage(None, Some("codex")).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider, "codex");
        assert_eq!(providers[0].tokens, 2_000);
        assert_eq!(providers[0].percentage, 100.0);

        let all_providers = db.get_provider_usage(None, None).unwrap();
        assert_eq!(all_providers.len(), 2);

        let models = db.get_model_usage(None, Some("codex")).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model, "gpt");

        let activity = db.get_activity(None, 10, Some("opencode")).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].provider, "opencode");
    }

    #[test]
    fn sync_cursor_round_trips_and_never_regresses() {
        let db = temp_db("cursor");
        assert!(db.get_sync_cursor("codex").unwrap().is_none());

        db.update_sync_state("codex", 1_000, Some("{\"a\":1}"), "connected", None)
            .unwrap();
        assert_eq!(
            db.get_sync_cursor("codex").unwrap().as_deref(),
            Some("{\"a\":1}")
        );

        // An older timestamp must not move the watermark backwards.
        db.update_sync_state("codex", 500, None, "connected", None)
            .unwrap();
        assert_eq!(db.get_last_sync_time("codex").unwrap(), Some(1_000));
        assert_eq!(
            db.get_sync_cursor("codex").unwrap().as_deref(),
            Some("{\"a\":1}")
        );

        let (state, detail) = db.get_sync_state("codex").unwrap().unwrap();
        assert_eq!(state, "connected");
        assert!(detail.is_none());
    }

    #[test]
    fn legacy_schema_migrates_without_losing_history() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-db-legacy-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("freetokenmeter.db");

        // A v4-era database: token columns were NOT NULL DEFAULT 0.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_records (
                    id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    input_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                    cached_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    estimated_cost REAL NOT NULL DEFAULT 0.0,
                    reference_value REAL,
                    free_value REAL,
                    pricing_status TEXT NOT NULL DEFAULT 'unknown',
                    session_id TEXT,
                    time_created INTEGER NOT NULL,
                    pricing_version TEXT,
                    pricing_source TEXT,
                    reference_model TEXT,
                    reference_input_rate REAL,
                    reference_output_rate REAL,
                    reference_cached_rate REAL
                );
                INSERT INTO usage_records VALUES
                    ('old', 'opencode', 'mimo-v2.5-free', '2025-01-01T00:00:00Z',
                     100, 50, 0, 25, 175, 0.0, 2.5, 2.5,
                     'reference_applied', 'ses_old', 1000, 'v2', 'ftm_override',
                     'mimo-v2.5', 0.14, 0.28, 0.0028);",
            )
            .unwrap();
        }

        let db = Database::new(&path).unwrap();
        assert_eq!(db.count_records().unwrap(), 1);
        let summary = db.get_usage_summary(None, None).unwrap();
        assert_eq!(summary.total_tokens, 175);
        assert_eq!(summary.cached_tokens, 25);
        assert_eq!(summary.reference_value, 2.5);
        // The zero `estimated_cost` of a free model survives as a known $0.
        assert_eq!(summary.actual_cost, 0.0);
        assert_eq!(summary.free_value, 2.5);
        assert_eq!(summary.unbilled_records, 0);

        // And the migrated table accepts NULL metrics for new providers.
        db.upsert_usage_records(&[record("new", "codex", "gpt", 900)])
            .unwrap();
        assert_eq!(db.count_records().unwrap(), 2);
    }

    /// A v3-era database predates the provenance columns entirely. Every
    /// optional column must fall back to a literal instead of being selected,
    /// which is only safe if the copy statement is built before the schema is
    /// renamed.
    #[test]
    fn older_schema_without_provenance_columns_still_migrates() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-db-v3-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("freetokenmeter.db");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_records (
                    id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    input_tokens INTEGER NOT NULL DEFAULT 0,
                    output_tokens INTEGER NOT NULL DEFAULT 0,
                    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                    cached_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    estimated_cost REAL NOT NULL DEFAULT 0.0,
                    reference_value REAL,
                    free_value REAL,
                    pricing_status TEXT NOT NULL DEFAULT 'unknown',
                    session_id TEXT,
                    time_created INTEGER NOT NULL,
                    pricing_version TEXT
                );
                INSERT INTO usage_records VALUES
                    ('v3', 'freebuff', 'mimo/mimo-v2.5', '2025-06-01T00:00:00Z',
                     1000, 500, 0, 200, 1500, 0.0, 0.12, 0.12,
                     'reference_applied', 'thr_1', 500, '2025-07.1');",
            )
            .unwrap();
        }

        let db = Database::new(&path).unwrap();
        assert_eq!(db.count_records().unwrap(), 1);
        let summary = db.get_usage_summary(None, None).unwrap();
        assert_eq!(summary.total_tokens, 1_500);
        assert_eq!(summary.reference_value, 0.12);
        assert_eq!(summary.actual_cost, 0.0);

        // The migrated table must accept the new nullable metric columns.
        db.upsert_usage_records(&[record("v5-new", "codex", "gpt", 10)])
            .unwrap();
        assert_eq!(db.count_records().unwrap(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A database created before `billing_metric_name` and `source_version`
    /// existed already has every other v5 column, including `source_type`.
    /// Those two are added in place, so real usage rows must survive untouched
    /// — this is the migration bug that renamed the table and then referenced
    /// columns that no longer existed.
    #[test]
    fn post_v5_columns_are_added_in_place_without_touching_existing_rows() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-db-additive-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("freetokenmeter.db");

        // The v5 shape as it shipped, minus the two later columns.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_records (
                    id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    cached_tokens INTEGER,
                    cache_write_tokens INTEGER,
                    reasoning_tokens INTEGER,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    provider_cost_usd REAL,
                    actual_cost_usd REAL,
                    reference_value REAL,
                    free_value REAL,
                    billing_unit TEXT NOT NULL DEFAULT 'unknown',
                    billing_units REAL,
                    payment_mode TEXT NOT NULL DEFAULT 'unknown',
                    pricing_status TEXT NOT NULL DEFAULT 'unknown',
                    pricing_source TEXT,
                    pricing_version TEXT,
                    reference_model TEXT,
                    reference_input_rate REAL,
                    reference_output_rate REAL,
                    reference_cached_rate REAL,
                    session_id TEXT,
                    event_id TEXT,
                    source_type TEXT NOT NULL DEFAULT 'unknown',
                    source_id TEXT,
                    time_created INTEGER NOT NULL
                );
                INSERT INTO usage_records
                    (id, provider, model, timestamp, total_tokens, actual_cost_usd,
                     billing_unit, payment_mode, pricing_status, source_type, time_created)
                VALUES
                    ('pre1', 'codex', 'gpt-5.3-codex', '2026-09-01T00:00:00Z', 1234, NULL,
                     'usd', 'subscription', 'reference_applied', 'codex_session_file', 1000);",
            )
            .unwrap();
        }

        let db = Database::new(&path).unwrap();

        // Existing usage rows are intact, not rebuilt or dropped.
        assert_eq!(db.count_records().unwrap(), 1);
        let summary = db.get_usage_summary(None, None).unwrap();
        assert_eq!(summary.total_tokens, 1_234);
        assert_eq!(summary.unbilled_records, 1);

        // The two columns now exist and accept values.
        assert!(Database::column_exists(
            &db.conn,
            "usage_records",
            "billing_metric_name"
        ));
        assert!(Database::column_exists(
            &db.conn,
            "usage_records",
            "source_version"
        ));

        let mut rec = record("pre2", "codex", "gpt-5.3-codex", 1_000);
        rec.billing_metric_name = Some("plus".to_string());
        rec.source_version = Some("0.107.0-alpha.5".to_string());
        db.upsert_usage_records(&[rec]).unwrap();
        assert_eq!(db.count_records().unwrap(), 2);

        let stats = db.provider_stats().unwrap();
        assert_eq!(stats.get("codex").unwrap().records, 2);
        assert_eq!(
            stats.get("codex").unwrap().source_version.as_deref(),
            Some("0.107.0-alpha.5")
        );

        let models = db.get_model_usage(None, None).unwrap();
        assert_eq!(models[0].billing_metric_name.as_deref(), Some("plus"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provider_availability_labels_are_exposed_to_the_ui() {
        assert_eq!(ProviderAvailability::Connected.as_str(), "connected");
        assert_eq!(
            ProviderAvailability::UsageUnavailable.label(),
            "USAGE UNAVAILABLE"
        );
        assert!(!ProviderAvailability::Disabled.is_connected());
    }

    // ──────────────────────────────────────────────────────────────────
    // Time semantics: event time drives ranges, ingest time never does
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn ranges_filter_on_event_time_not_ingest_time() {
        let db = temp_db("eventtime");
        let mut old = record("old", "opencode", "mimo-v2.5-free", 1_000);
        old.timestamp = "2020-01-01T00:00:00Z".to_string();
        let fresh = record("fresh", "opencode", "mimo-v2.5-free", 2_000);
        db.upsert_usage_records(&[old, fresh]).unwrap();

        // Both rows were ingested just now, but only one happened today.
        let today = db.get_usage_summary(Some(1), None).unwrap();
        assert_eq!(today.request_count, 1, "only the fresh record is today's");
        assert_eq!(today.total_tokens, 2_000);

        let all = db.get_usage_summary(None, None).unwrap();
        assert_eq!(all.request_count, 2);
        assert_eq!(all.total_tokens, 3_000);
    }

    #[test]
    fn activity_respects_the_selected_range_and_orders_by_event_time() {
        let db = temp_db("activity-range");
        let mut old = record("old", "codex", "gpt", 10);
        old.timestamp = "2020-01-01T00:00:00Z".to_string();
        let fresh = record("fresh", "codex", "gpt", 20);
        db.upsert_usage_records(&[old, fresh]).unwrap();

        let today = db.get_activity(Some(1), 10, None).unwrap();
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].tokens, 20);

        // Newest event first, regardless of ingest order.
        let all = db.get_activity(None, 10, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].tokens, 20);
    }

    /// An RFC 3339 timestamp for `days_ago` at `hour` in the machine's local zone.
    ///
    /// The series buckets by *local* day, so a fixture written in UTC would land
    /// in a different bucket on a machine east of Greenwich — which is exactly the
    /// bug this test must not have.
    fn local_stamp(days_ago: i64, hour: u32) -> String {
        use chrono::TimeZone;
        let date = (chrono::Local::now() - chrono::Duration::days(days_ago)).date_naive();
        let naive = date.and_hms_opt(hour, 0, 0).expect("valid time");
        chrono::Local
            .from_local_datetime(&naive)
            .single()
            .expect("unambiguous local time")
            .to_rfc3339()
    }

    fn local_day_key(days_ago: i64) -> String {
        (chrono::Local::now() - chrono::Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn series_buckets_by_local_day_and_sums_per_provider() {
        let db = temp_db("series-day");
        let mut morning = record("a", "opencode", "m", 1_000);
        morning.timestamp = local_stamp(0, 9);
        let mut evening = record("b", "opencode", "m", 2_000);
        evening.timestamp = local_stamp(0, 18);
        let mut yesterday = record("c", "codex", "gpt", 500);
        yesterday.timestamp = local_stamp(1, 9);
        db.upsert_usage_records(&[morning, evening, yesterday])
            .unwrap();

        let series = db.get_series(Some(30), None).unwrap();
        assert_eq!(series.granularity, "day");

        // One bucket per day per provider: opencode's two records collapse into
        // today's bucket, codex has its own bucket yesterday.
        assert_eq!(
            series.points.len(),
            2,
            "opencode today, codex yesterday: {:?}",
            series.points
        );
        let opencode = series
            .points
            .iter()
            .find(|p| p.key == local_day_key(0) && p.provider == "opencode")
            .expect("opencode bucket for today");
        assert_eq!(opencode.tokens, 3_000);
        assert_eq!(opencode.requests, 2);

        let codex = series
            .points
            .iter()
            .find(|p| p.provider == "codex")
            .expect("codex bucket");
        assert_eq!(codex.key, local_day_key(1));
    }

    #[test]
    fn series_granularity_follows_the_selected_range() {
        assert_eq!(granularity_for(Some(1)), Granularity::Hour);
        assert_eq!(granularity_for(Some(7)), Granularity::Day);
        assert_eq!(granularity_for(Some(60)), Granularity::Day);
        assert_eq!(granularity_for(Some(90)), Granularity::Week);
        assert_eq!(granularity_for(None), Granularity::Week);
    }

    #[test]
    fn series_respects_the_range_and_provider_filter() {
        let db = temp_db("series-filter");
        let mut old = record("old", "codex", "gpt", 10);
        old.timestamp = "2020-01-01T00:00:00Z".to_string();
        let fresh = record("fresh", "opencode", "m", 20);
        db.upsert_usage_records(&[old, fresh]).unwrap();

        let today = db.get_series(Some(1), None).unwrap();
        assert_eq!(today.granularity, "hour");
        assert_eq!(
            today.points.len(),
            1,
            "the 2020 record is outside the range"
        );
        assert_eq!(today.points[0].tokens, 20);

        let only_codex = db.get_series(None, Some("codex")).unwrap();
        assert_eq!(only_codex.granularity, "week");
        assert!(only_codex.points.iter().all(|p| p.provider == "codex"));
    }

    #[test]
    fn calendar_stays_daily_even_over_a_year_and_respects_the_filter() {
        let db = temp_db("calendar-day");
        let mut today_oc = record("c1", "opencode", "m", 500);
        today_oc.timestamp = local_stamp(0, 10);
        let mut yesterday_oc = record("c2", "opencode", "m", 700);
        yesterday_oc.timestamp = local_stamp(1, 10);
        let mut yesterday_codex = record("c3", "codex", "g", 900);
        yesterday_codex.timestamp = local_stamp(1, 11);
        db.upsert_usage_records(&[today_oc, yesterday_oc, yesterday_codex])
            .unwrap();

        // A year is well past the chart's day/week cutoff, so this is the case
        // that would silently collapse the calendar into week columns.
        let year = db.get_calendar(Some(371), None).unwrap();
        assert_eq!(year.granularity, "day");
        assert!(
            year.points.iter().all(|p| p.key.len() == 10),
            "calendar keys must be dates, not weeks: {:?}",
            year.points.first().map(|p| p.key.clone())
        );
        assert!(year
            .points
            .iter()
            .any(|p| p.key == local_day_key(0) && p.tokens == 500));
        let yesterday: i64 = year
            .points
            .iter()
            .filter(|p| p.key == local_day_key(1))
            .map(|p| p.tokens)
            .sum();
        assert_eq!(yesterday, 1_600, "providers on one day are summed per day");

        let codex_only = db.get_calendar(Some(371), Some("codex")).unwrap();
        assert_eq!(codex_only.granularity, "day");
        assert!(codex_only.points.iter().all(|p| p.provider == "codex"));
        assert_eq!(codex_only.points.len(), 1);
    }

    #[test]
    fn reupsert_preserves_the_first_ingest_time() {
        let db = temp_db("ingest-time");
        db.upsert_usage_records(&[record("a", "codex", "gpt", 100)])
            .unwrap();

        let first: i64 = db
            .conn
            .query_row(
                "SELECT time_created FROM usage_records WHERE id = 'a'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        // A later sync of the same growing session must not re-date the row.
        db.upsert_usage_records(&[record("a", "codex", "gpt", 250)])
            .unwrap();
        let second: i64 = db
            .conn
            .query_row(
                "SELECT time_created FROM usage_records WHERE id = 'a'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(db.get_provider_token_total("codex").unwrap(), 250);
    }

    #[test]
    fn pre_existing_rows_get_event_time_backfilled_from_their_timestamp() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-db-eventtime-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("freetokenmeter.db");

        // A v5 database: every column except event_time_ms.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_records (
                    id TEXT PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL,
                    timestamp TEXT NOT NULL, input_tokens INTEGER, output_tokens INTEGER,
                    cached_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
                    total_tokens INTEGER NOT NULL DEFAULT 0, provider_cost_usd REAL,
                    actual_cost_usd REAL, reference_value REAL, free_value REAL,
                    billing_unit TEXT NOT NULL DEFAULT 'unknown', billing_units REAL,
                    billing_metric_name TEXT, payment_mode TEXT NOT NULL DEFAULT 'unknown',
                    pricing_status TEXT NOT NULL DEFAULT 'unknown', pricing_source TEXT,
                    pricing_version TEXT, reference_model TEXT, reference_input_rate REAL,
                    reference_output_rate REAL, reference_cached_rate REAL, session_id TEXT,
                    event_id TEXT, source_type TEXT NOT NULL DEFAULT 'unknown', source_id TEXT,
                    source_version TEXT, time_created INTEGER NOT NULL);
                 INSERT INTO usage_records
                    (id, provider, model, timestamp, total_tokens, source_type, time_created)
                 VALUES ('legacy', 'opencode', 'mimo-v2.5-free',
                         '2025-01-01T00:00:00Z', 500, 'opencode_database', 1000);",
            )
            .unwrap();
        }

        let db = Database::new(&path).unwrap();
        let ms: i64 = db
            .conn
            .query_row(
                "SELECT event_time_ms FROM usage_records WHERE id = 'legacy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ms, 1_735_689_600_000, "2025-01-01T00:00:00Z in epoch ms");

        // And it is now filterable by its real period, not by ingest time.
        let recent = db.get_usage_summary(Some(30), None).unwrap();
        assert_eq!(recent.request_count, 0);
        assert_eq!(db.get_usage_summary(None, None).unwrap().request_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_version_is_recorded() {
        let db = temp_db("user-version");
        let version: i64 = db
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }
}
