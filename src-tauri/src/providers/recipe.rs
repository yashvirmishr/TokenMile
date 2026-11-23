//! Declarative SQLite provider recipes.
//!
//! Most local AI tools that keep usage history keep it in one SQLite database:
//! OpenCode, Freebuff, GitHub Copilot and DeepAI Code all do. Writing a bespoke
//! `UsageProvider` for each of them means re-implementing the same read-only open,
//! schema probe, watermark, truncation and mapping logic four times, and every
//! copy is a place for the last fix to be missing.
//!
//! A [`SqliteRecipe`] replaces that with a **SELECT statement plus a few facts**.
//! [`SqliteProvider`] then provides the whole [`UsageProvider`] contract:
//!
//! ```text
//! SqliteRecipe                      SqliteProvider (shared)
//!   databases()   ── candidates ──▶  pick the first that exists
//!   requires      ── schema ──────▶  probe tables/columns, report why not
//!   rows_sql      ── ?1 ?2 ──────▶  read-only query, bounded by the pass limit
//!   watermark_sql ── newest ──────▶  advance only what was actually consumed
//!   flags         ── semantics ──▶  normalization + pricing hand-off
//! ```
//!
//! # Row contract
//!
//! `rows_sql` must produce **nine** columns, in this exact order, followed by one
//! optional tenth:
//!
//! | # | column | type | notes |
//! |---|--------|------|-------|
//! | 0 | event id | TEXT | provider-native, stable, unique per record |
//! | 1 | model | TEXT | `NULL` or empty → `unknown` |
//! | 2 | timestamp | TEXT | ISO-8601 UTC, e.g. `2026-11-21T11:39:58.762Z` |
//! | 3 | input tokens | INTEGER | `NULL` when not reported |
//! | 4 | output tokens | INTEGER | |
//! | 5 | cached tokens | INTEGER | |
//! | 6 | cache-write tokens | INTEGER | |
//! | 7 | reasoning tokens | INTEGER | |
//! | 8 | provider cost | REAL | USD the tool itself reported; usually `NULL` |
//! | 9 | session id | TEXT | optional, for grouping |
//!
//! Two bound parameters are always available:
//!
//! * `?1` — the sync window floor as ISO-8601 text, or `''` for "everything".
//!   Comparing text works because ISO-8601 UTC sorts chronologically.
//! * `?2` — the maximum number of rows to return.
//!
//! # What the recipe does not do
//!
//! It never computes prices (that is [`crate::pricing::value_usage`]), never writes
//! to the source database, and never invents a metric: a `NULL` token column stays
//! `NULL` rather than becoming `0`.

use std::path::PathBuf;

use chrono::{SecondsFormat, TimeZone, Utc};
use log::{info, warn};
use rusqlite::{Connection, OpenFlags};

use crate::database::{parse_event_ms, UsageRecord};
use crate::pricing::{value_usage, PricingInput};
use crate::provider::{FetchOutcome, FetchRequest, UsageProvider};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability};

/// One table/column a recipe's source schema must contain.
#[derive(Debug, Clone, Copy)]
pub struct ColumnCheck {
    pub table: &'static str,
    pub column: &'static str,
}

impl ColumnCheck {
    pub const fn new(table: &'static str, column: &'static str) -> Self {
        Self { table, column }
    }
}

/// A provider defined by a database, a query and a handful of facts.
#[derive(Debug)]
pub struct SqliteRecipe {
    /// Canonical provider id (database key, never a display name).
    pub id: &'static str,
    pub name: &'static str,
    /// Short uppercase label for narrow UI columns.
    pub short_label: &'static str,
    /// Source kind recorded on every record, e.g. `deepai_database`.
    pub source_type: &'static str,
    pub payment_mode: PaymentMode,
    pub billing_unit: BillingUnit,
    /// Backfill window applied on the provider's first sync.
    pub backfill_days: i64,
    /// `true` when the source's input-token figure already counts cached tokens,
    /// so adding `cached` again would double count. Get this right per provider;
    /// it is the difference between a real total and one inflated by its cache.
    pub cached_is_subset_of_input: bool,
    /// Candidate database files, highest priority first. Called on every
    /// detection, so it may look at the environment.
    pub databases: fn() -> Vec<PathBuf>,
    /// Schema probe. Every entry must exist for the recipe to be usable.
    pub requires: &'static [ColumnCheck],
    /// Row query; see the module docs for the column contract.
    pub rows_sql: &'static str,
    /// Newest watermark in the source as ISO-8601 text (`NULL` when empty).
    /// Used only when a pass consumed the source completely.
    pub watermark_sql: &'static str,
}

/// A [`UsageProvider`] driven by a [`SqliteRecipe`].
pub struct SqliteProvider {
    recipe: &'static SqliteRecipe,
    /// Explicit database path, bypassing the recipe's resolver. Used by tests and
    /// by anyone whose installation lives somewhere unusual.
    path: Option<PathBuf>,
}

/// A row already mapped out of the source database.
#[derive(Debug, Clone)]
struct RawRow {
    event_id: String,
    model: String,
    timestamp: String,
    input: Option<i64>,
    output: Option<i64>,
    cached: Option<i64>,
    cache_write: Option<i64>,
    reasoning: Option<i64>,
    cost: Option<f64>,
    session_id: Option<String>,
}

impl SqliteProvider {
    pub fn new(recipe: &'static SqliteRecipe) -> Self {
        Self { recipe, path: None }
    }

    /// Use one specific database instead of the recipe's candidate list.
    ///
    /// The recipe's `databases()` resolver is the production path; this is the
    /// seam used by tests, and what an override for a non-standard installation
    /// would call.
    #[allow(dead_code)]
    pub fn with_path(recipe: &'static SqliteRecipe, path: PathBuf) -> Self {
        Self {
            recipe,
            path: Some(path),
        }
    }

    /// The database to read: an explicit override, else the first candidate that
    /// exists.
    fn database(&self) -> Option<PathBuf> {
        if let Some(path) = &self.path {
            return Some(path.clone());
        }
        (self.recipe.databases)().into_iter().find(|p| p.exists())
    }

    /// Open a source database read-only.
    ///
    /// The files belong to another application: this process must never create,
    /// lock for writing, or migrate them. A 5 s busy timeout rides out the short
    /// write locks a live tool takes while checkpointing its WAL.
    fn open(&self, path: &std::path::Path) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        Ok(conn)
    }

    /// Verify the schema this recipe depends on.
    fn probe(&self, conn: &Connection) -> Result<(), String> {
        for check in self.recipe.requires {
            let sql = format!("PRAGMA table_info({})", check.table);
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("cannot read schema of {}: {}", check.table, e))?;
            let columns: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| format!("cannot read schema of {}: {}", check.table, e))?
                .filter_map(Result::ok)
                .collect();

            if columns.is_empty() {
                return Err(format!("table {} is missing", check.table));
            }
            if !columns.iter().any(|c| c == check.column) {
                return Err(format!(
                    "column {}.{} is missing",
                    check.table, check.column
                ));
            }
        }
        Ok(())
    }

    fn query_rows(
        &self,
        conn: &Connection,
        since: &str,
        limit: usize,
    ) -> Result<Vec<RawRow>, String> {
        let mut stmt = conn
            .prepare(self.recipe.rows_sql)
            .map_err(|e| format!("query rejected by the schema: {}", e))?;

        let mapped = stmt
            .query_map(rusqlite::params![since, limit as i64], |row| {
                // The tenth column is optional; a recipe that omits it still
                // satisfies the contract.
                let session_id = if row.as_ref().column_count() > 9 {
                    row.get::<_, Option<String>>(9)?
                } else {
                    None
                };

                Ok(RawRow {
                    event_id: row.get::<_, String>(0)?,
                    model: row
                        .get::<_, Option<String>>(1)?
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "unknown".to_string()),
                    timestamp: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    input: row.get::<_, Option<i64>>(3)?,
                    output: row.get::<_, Option<i64>>(4)?,
                    cached: row.get::<_, Option<i64>>(5)?,
                    cache_write: row.get::<_, Option<i64>>(6)?,
                    reasoning: row.get::<_, Option<i64>>(7)?,
                    cost: row.get::<_, Option<f64>>(8)?,
                    session_id,
                })
            })
            .map_err(|e| format!("query failed: {}", e))?;

        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(|e| format!("row could not be read: {}", e))?);
        }
        Ok(rows)
    }

    fn watermark(&self, conn: &Connection) -> Option<i64> {
        let text: Option<String> = conn
            .query_row(self.recipe.watermark_sql, [], |row| row.get(0))
            .ok()
            .flatten();
        text.map(|t| parse_event_ms(&t)).filter(|ms| *ms > 0)
    }

    /// Total tokens under this recipe's cache semantics.
    fn total_tokens(&self, row: &RawRow) -> i64 {
        let input = row.input.unwrap_or(0);
        let output = row.output.unwrap_or(0);
        let reasoning = row.reasoning.unwrap_or(0);

        if self.recipe.cached_is_subset_of_input {
            input + output + reasoning
        } else {
            input + output + reasoning + row.cached.unwrap_or(0) + row.cache_write.unwrap_or(0)
        }
    }
}

impl UsageProvider for SqliteProvider {
    fn id(&self) -> &'static str {
        self.recipe.id
    }

    fn display_name(&self) -> &'static str {
        self.recipe.name
    }

    fn short_label(&self) -> &'static str {
        self.recipe.short_label
    }

    fn source_type(&self) -> &'static str {
        self.recipe.source_type
    }

    fn payment_mode(&self) -> PaymentMode {
        self.recipe.payment_mode
    }

    fn billing_unit(&self) -> BillingUnit {
        self.recipe.billing_unit
    }

    fn default_backfill_days(&self) -> i64 {
        self.recipe.backfill_days
    }

    fn availability(&self) -> ProviderAvailability {
        let Some(path) = self.database() else {
            return ProviderAvailability::NotInstalled;
        };

        match self.open(&path).and_then(|conn| self.probe(&conn)) {
            Ok(()) => ProviderAvailability::Connected,
            // The tool is installed but this version's schema is not the one the
            // recipe knows: report that instead of silently syncing nothing.
            Err(_) => ProviderAvailability::UsageUnavailable,
        }
    }

    fn detail(&self) -> Option<String> {
        let path = self.database()?;

        match self.open(&path).and_then(|conn| self.probe(&conn)) {
            Ok(()) => Some(format!("Reading {}", path.display())),
            Err(e) => Some(format!(
                "{} exists but is not readable as expected: {}",
                path.display(),
                e
            )),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        let path = self
            .database()
            .ok_or_else(|| format!("no {} database found", self.recipe.name))?;
        let conn = self.open(&path)?;
        self.probe(&conn)?;

        let cursor = request.cursor.cloned().unwrap_or_default();
        let mut next = cursor.clone();

        // Resume from whichever is further along: the persisted cursor or the
        // configured backfill window. On the first pass the cursor is empty.
        let since_ms = match (cursor.last_timestamp_ms, request.since_ms) {
            (Some(cursor_ms), Some(window_ms)) => Some(cursor_ms.max(window_ms)),
            (Some(cursor_ms), None) => Some(cursor_ms),
            (None, window_ms) => window_ms,
        };

        // ISO-8601 text rather than epoch arithmetic: it keeps the filter a plain
        // comparison on the column the source already indexes.
        let since_text = since_ms
            .map(|ms| {
                Utc.timestamp_millis_opt(ms)
                    .single()
                    .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let rows = self.query_rows(&conn, &since_text, request.limit)?;
        let truncated = rows.len() >= request.limit;

        let source_id = path.to_string_lossy().to_string();
        let mut records: Vec<UsageRecord> = Vec::with_capacity(rows.len());
        let mut newest: Option<i64> = None;
        let mut skipped = 0usize;

        for row in &rows {
            if row.timestamp.is_empty() {
                // Without an event time the record cannot be placed in any range,
                // so it is skipped rather than filed under "now".
                warn!(
                    "{}: skipping {} (no timestamp)",
                    self.recipe.name, row.event_id
                );
                skipped += 1;
                continue;
            }

            let event_ms = parse_event_ms(&row.timestamp);
            if event_ms > 0 {
                newest = Some(newest.unwrap_or(0).max(event_ms));
            }
            next.last_event_id = Some(row.event_id.clone());
            records.push(self.to_priced_record(row, &source_id, request));
        }

        // A truncated pass advances only as far as it actually read; a complete
        // pass can trust the source's own newest watermark (which also picks up
        // rows whose timestamp moved after they were first seen).
        next.last_timestamp_ms = if truncated {
            newest.or(cursor.last_timestamp_ms)
        } else {
            self.watermark(&conn)
                .or(newest)
                .or(cursor.last_timestamp_ms)
        };

        next.extra
            .insert("database".to_string(), path.to_string_lossy().to_string());

        info!(
            "{}: scanned {} row(s), imported {} record(s) from {}",
            self.recipe.name,
            rows.len(),
            records.len(),
            path.display()
        );

        Ok(FetchOutcome {
            records,
            cursor: next,
            scanned: rows.len(),
            skipped,
        })
    }
}

impl SqliteProvider {
    /// Map a row and price it against the shared registry.
    ///
    /// Kept separate from [`Self::to_record`] so the pricing call has the pass's
    /// registry available.
    fn to_priced_record(
        &self,
        row: &RawRow,
        source_id: &str,
        request: &FetchRequest<'_>,
    ) -> UsageRecord {
        let value = value_usage(
            &PricingInput {
                provider: self.recipe.id,
                model: &row.model,
                provider_cost_usd: row.cost,
                billing_units: None,
                billing_unit: self.recipe.billing_unit,
                payment_mode: self.recipe.payment_mode,
                input_tokens: row.input,
                output_tokens: row.output,
                cached_tokens: row.cached,
                cache_write_tokens: row.cache_write,
            },
            request.registry,
        );

        UsageRecord {
            id: format!("{}:{}", self.recipe.id, row.event_id),
            provider: self.recipe.id.to_string(),
            model: row.model.clone(),
            timestamp: row.timestamp.clone(),
            input_tokens: row.input,
            output_tokens: row.output,
            cached_tokens: row.cached,
            cache_write_tokens: row.cache_write,
            reasoning_tokens: row.reasoning,
            total_tokens: self.total_tokens(row),
            actual_cost_usd: value.actual_cost_usd,
            reference_value: value.reference_value,
            free_value: value.free_value,
            billing_unit: value.billing_unit.as_str().to_string(),
            billing_units: value.billing_units,
            billing_metric_name: None,
            payment_mode: value.payment_mode.as_str().to_string(),
            pricing_status: value.pricing_status.as_str().to_string(),
            pricing_source: Some(value.pricing_source.as_str().to_string()),
            pricing_version: Some(value.pricing_version),
            reference_model: value.reference_model,
            reference_input_rate: value.reference_input_rate,
            reference_output_rate: value.reference_output_rate,
            reference_cached_rate: value.reference_cached_rate,
            session_id: row.session_id.clone(),
            event_id: Some(row.event_id.clone()),
            source_type: self.recipe.source_type.to_string(),
            source_id: Some(source_id.to_string()),
            source_version: None,
        }
    }
}

/// Build a provider from a recipe. This is the one call a recipe needs.
pub fn provider(recipe: &'static SqliteRecipe) -> Box<dyn UsageProvider> {
    Box::new(SqliteProvider::new(recipe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderCursor;
    use std::path::Path;

    const SCHEMA: &str = "
        CREATE TABLE turns (
            id TEXT PRIMARY KEY,
            model TEXT,
            started_at TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cached_tokens INTEGER,
            cost REAL
        );";

    const ROWS_SQL: &str = "
        SELECT id, model, started_at, input_tokens, output_tokens, cached_tokens, NULL, NULL, cost
        FROM turns
        WHERE started_at > ?1
        ORDER BY started_at ASC
        LIMIT ?2";

    const WATERMARK_SQL: &str = "SELECT MAX(started_at) FROM turns";

    static REQUIRES: &[ColumnCheck] = &[
        ColumnCheck::new("turns", "started_at"),
        ColumnCheck::new("turns", "model"),
    ];

    /// No database anywhere: used to test detection, never to read.
    fn missing() -> Vec<PathBuf> {
        vec![PathBuf::from("/definitely/not/here.db")]
    }

    /// A leaked recipe so it can be handed out as `&'static`, exactly like the
    /// shipped recipes in `providers::deepai`.
    fn recipe(cached_is_subset_of_input: bool) -> &'static SqliteRecipe {
        Box::leak(Box::new(SqliteRecipe {
            id: "testprov",
            name: "Test Provider",
            short_label: "TEST",
            source_type: "test_database",
            payment_mode: PaymentMode::Subscription,
            billing_unit: BillingUnit::Usd,
            backfill_days: 30,
            cached_is_subset_of_input,
            databases: missing,
            requires: REQUIRES,
            rows_sql: ROWS_SQL,
            watermark_sql: WATERMARK_SQL,
        }))
    }

    fn provider_at(path: &Path, cached_is_subset_of_input: bool) -> SqliteProvider {
        SqliteProvider::with_path(recipe(cached_is_subset_of_input), path.to_path_buf())
    }

    /// `(id, model, started_at, input, output, cached)`.
    fn scratch(label: &str, rows: &[(&str, &str, &str, i64, i64, i64)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ftm-recipe-{}-{}-{}",
            label,
            std::process::id(),
            crate::test_support::unique_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provider.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        for (id, model, ts, input, output, cached) in rows {
            conn.execute(
                "INSERT INTO turns VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                rusqlite::params![id, model, ts, input, output, cached],
            )
            .unwrap();
        }
        drop(conn);
        path
    }

    fn fetch(
        provider: &dyn UsageProvider,
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
    fn a_missing_database_is_not_installed() {
        let p = SqliteProvider::new(recipe(true));
        assert_eq!(p.availability(), ProviderAvailability::NotInstalled);
        assert!(p.detail().is_none());
    }

    #[test]
    fn maps_rows_and_keeps_unknown_metrics_null() {
        let path = scratch(
            "maps",
            &[
                (
                    "t1",
                    "model-a",
                    "2026-11-20T10:00:00.000Z",
                    1_000,
                    100,
                    8_000,
                ),
                ("t2", "", "2026-11-20T11:00:00.000Z", 10, 0, 0),
            ],
        );
        let p = provider_at(&path, false);

        let outcome = fetch(&p, None, None, 100);
        assert_eq!(outcome.records.len(), 2);

        let first = &outcome.records[0];
        assert_eq!(first.id, "testprov:t1");
        assert_eq!(first.provider, "testprov");
        assert_eq!(first.source_type, "test_database");
        assert_eq!(
            first.total_tokens,
            1_000 + 100 + 8_000,
            "cached is additive here"
        );
        assert!(
            first.reasoning_tokens.is_none(),
            "an absent metric stays NULL"
        );

        let second = &outcome.records[1];
        assert_eq!(second.model, "unknown", "an empty model is never invented");
    }

    #[test]
    fn cache_inside_input_is_not_counted_twice() {
        let path = scratch(
            "subset",
            &[(
                "t1",
                "model-a",
                "2026-11-20T10:00:00.000Z",
                1_000,
                100,
                8_000,
            )],
        );
        let p = provider_at(&path, true);

        let outcome = fetch(&p, None, None, 100);
        assert_eq!(outcome.records[0].total_tokens, 1_100);
    }

    #[test]
    fn first_pass_respects_the_backfill_window() {
        let path = scratch(
            "window",
            &[
                ("old", "m", "2026-01-01T00:00:00.000Z", 1, 1, 0),
                ("new", "m", "2026-11-20T00:00:00.000Z", 1, 1, 0),
            ],
        );
        let p = provider_at(&path, false);

        let cutoff = parse_event_ms("2026-06-01T00:00:00.000Z");
        let outcome = fetch(&p, None, Some(cutoff), 100);

        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.records[0].event_id.as_deref(), Some("new"));
    }

    #[test]
    fn a_truncated_pass_advances_only_as_far_as_it_read() {
        let path = scratch(
            "truncate",
            &[
                ("t1", "m", "2026-11-01T00:00:00.000Z", 1, 0, 0),
                ("t2", "m", "2026-11-02T00:00:00.000Z", 1, 0, 0),
                ("t3", "m", "2026-11-03T00:00:00.000Z", 1, 0, 0),
                ("t4", "m", "2026-11-04T00:00:00.000Z", 1, 0, 0),
            ],
        );
        let p = provider_at(&path, false);

        let first = fetch(&p, None, None, 2);
        assert_eq!(first.records.len(), 2);
        assert_eq!(
            first.cursor.last_timestamp_ms,
            Some(parse_event_ms("2026-11-02T00:00:00.000Z")),
            "the watermark must stop at the last row that was actually read"
        );

        let second = fetch(&p, Some(&first.cursor), None, 2);
        let ids: Vec<&str> = second
            .records
            .iter()
            .map(|r| r.event_id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, vec!["t3", "t4"], "nothing is skipped or repeated");

        // A complete pass is trusted to report the source's own newest row.
        let third = fetch(&p, Some(&second.cursor), None, 10);
        assert!(third.records.is_empty());
        assert_eq!(
            third.cursor.last_timestamp_ms,
            Some(parse_event_ms("2026-11-04T00:00:00.000Z"))
        );
    }

    #[test]
    fn an_unexpected_schema_is_reported_not_silently_empty() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-recipe-schema-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("other.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE something_else (id TEXT);")
            .unwrap();
        drop(conn);

        let p = provider_at(&path, false);

        assert_eq!(p.availability(), ProviderAvailability::UsageUnavailable);
        let detail = p.detail().unwrap();
        assert!(
            detail.contains("turns is missing"),
            "detail was: {}",
            detail
        );
        assert!(p
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 10,
            })
            .is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_source_is_opened_read_only() {
        let path = scratch(
            "readonly",
            &[("t1", "m", "2026-11-20T00:00:00.000Z", 1, 0, 0)],
        );
        let before = std::fs::read(&path).unwrap();

        let p = provider_at(&path, false);
        assert_eq!(p.availability(), ProviderAvailability::Connected);
        assert_eq!(fetch(&p, None, None, 10).records.len(), 1);

        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("db-wal").exists());
    }
}
