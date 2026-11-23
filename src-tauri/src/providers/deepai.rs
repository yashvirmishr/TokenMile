//! DeepAI Code provider, defined as a [`SqliteRecipe`].
//!
//! # Source
//!
//! DeepAI Code (the desktop harness this repository's sibling app runs on) keeps
//! its state in `~/.deepai-code/userdata/statev2.sqlite`. Usage lives on provider
//! turns, not on messages:
//!
//! ```text
//! orchestration_v2_projection_provider_turns
//!   provider_turn_id  TEXT      stable per attempt
//!   thread_id         TEXT      → provider_sessions.thread_id
//!   started_at        TEXT      ISO-8601 UTC
//!   completed_at      TEXT      NULL while the turn is still running
//!   payload_json      TEXT      {"tokenUsage":…, "turnTokenUsage":…}
//! ```
//!
//! Two different token objects sit in that payload, and picking the wrong one is
//! the whole risk in this provider:
//!
//! * `tokenUsage` — a **context-window snapshot** (`usedTokens`/`maxTokens` plus
//!   the current input/cached/output counters). It resets and repeats as the
//!   context is managed, so summing it would be meaningless.
//! * `turnTokenUsage` — the **per-turn delta** for the main agent:
//!   `inputTokens`, `cachedInputTokens`, `cacheCreationTokens`, `outputTokens`,
//!   plus `usageScope` and `usageStatus`.
//!
//! Only `turnTokenUsage` is accounted here. The four fields are disjoint (the
//! harness reports cache reads separately from fresh input, like the Anthropic
//! API it wraps), so the total adds them.
//!
//! # Watermark
//!
//! `COALESCE(completed_at, started_at)` is used instead of `started_at` so a turn
//! that was still running when it was first read is re-read once it finishes and
//! its usage grows. A completed turn's row is immutable, so the upsert on
//! `provider_turn_id` replaces it rather than duplicating it.
//!
//! # Known limitation
//!
//! `turnTokenUsage.usageScope` is `main_agent`: subagent turns are not itemised in
//! this table, and a turn whose `usageStatus` is `partial` is stored with the
//! counts it had at read time (it is re-read when it completes).

use std::path::PathBuf;

use crate::providers::recipe::{ColumnCheck, SqliteRecipe};
use crate::usage::{provider_id, BillingUnit, PaymentMode};

/// `~/.deepai-code/userdata/statev2.sqlite` (plus a `DEEPAI_CODE_HOME` override,
/// which is what the tests use).
fn databases() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|home| vec![home.join(".deepai-code/userdata/statev2.sqlite")])
        .unwrap_or_default()
}

const REQUIRES: &[ColumnCheck] = &[
    ColumnCheck::new(
        "orchestration_v2_projection_provider_turns",
        "provider_turn_id",
    ),
    ColumnCheck::new("orchestration_v2_projection_provider_turns", "payload_json"),
    ColumnCheck::new("orchestration_v2_projection_provider_turns", "started_at"),
    ColumnCheck::new("orchestration_v2_projection_provider_sessions", "model"),
];

/// Rows: id, model, timestamp, input, output, cached, cache-write, reasoning, cost, session.
///
/// JSON extraction happens in SQL (SQLite's JSON1 is compiled into the bundled
/// rusqlite), so the recorder never has to know this payload's shape.
const ROWS_SQL: &str = "
    SELECT
        t.provider_turn_id,
        COALESCE(NULLIF(s.model, ''), 'unknown'),
        t.started_at,
        COALESCE(json_extract(t.payload_json, '$.turnTokenUsage.inputTokens'), 0),
        COALESCE(json_extract(t.payload_json, '$.turnTokenUsage.outputTokens'), 0),
        COALESCE(json_extract(t.payload_json, '$.turnTokenUsage.cachedInputTokens'), 0),
        COALESCE(json_extract(t.payload_json, '$.turnTokenUsage.cacheCreationTokens'), 0),
        NULL,
        NULL,
        t.thread_id
    FROM orchestration_v2_projection_provider_turns t
    LEFT JOIN orchestration_v2_projection_provider_sessions s
        ON s.thread_id = t.thread_id
    WHERE t.started_at IS NOT NULL
      AND json_extract(t.payload_json, '$.turnTokenUsage') IS NOT NULL
      AND COALESCE(t.completed_at, t.started_at) > ?1
    ORDER BY COALESCE(t.completed_at, t.started_at) ASC
    LIMIT ?2
";

const WATERMARK_SQL: &str = "
    SELECT MAX(COALESCE(completed_at, started_at))
    FROM orchestration_v2_projection_provider_turns
    WHERE started_at IS NOT NULL
      AND json_extract(payload_json, '$.turnTokenUsage') IS NOT NULL
";

/// The shipped recipe. Registering it is one line in [`crate::providers`].
pub static RECIPE: SqliteRecipe = SqliteRecipe {
    id: provider_id::DEEP_AI,
    name: "DeepAI Code",
    short_label: "DEEPAI",
    source_type: "deepai_database",
    // The harness is used through a plan/subscription; it reports no per-turn
    // dollar cost, so the actual cost stays N/A and the reference value carries
    // the comparison.
    payment_mode: PaymentMode::Subscription,
    billing_unit: BillingUnit::Usd,
    backfill_days: 30,
    cached_is_subset_of_input: false,
    databases,
    requires: REQUIRES,
    rows_sql: ROWS_SQL,
    watermark_sql: WATERMARK_SQL,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{FetchRequest, ProviderCursor, UsageProvider};
    use rusqlite::Connection;

    /// `(turn id, thread id, started_at, completed_at, input, output, cached, cache_write)`.
    type Turn = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        i64,
        i64,
        i64,
        i64,
    );

    /// Build a miniature `statev2.sqlite` with the two tables the recipe joins.
    fn scratch(turns: &[Turn]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ftm-deepai-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let userdata = root.join("userdata");
        std::fs::create_dir_all(&userdata).unwrap();
        let path = userdata.join("statev2.sqlite");

        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE orchestration_v2_projection_provider_turns (
                 provider_turn_id TEXT PRIMARY KEY,
                 thread_id TEXT,
                 status TEXT,
                 started_at TEXT,
                 completed_at TEXT,
                 payload_json TEXT
             );
             CREATE TABLE orchestration_v2_projection_provider_sessions (
                 provider_session_id TEXT PRIMARY KEY,
                 thread_id TEXT,
                 provider TEXT,
                 model TEXT
             );
             INSERT INTO orchestration_v2_projection_provider_sessions
                 VALUES ('ps1', 'th1', 'deepai', 'deepai/mimo-2.6-pro');",
        )
        .unwrap();

        for (turn_id, thread_id, started, completed, input, output, cached, cache_write) in turns {
            let payload = format!(
                r#"{{"tokenUsage":{{"usedTokens":999,"maxTokens":320000,"inputTokens":1,"cachedInputTokens":2,"outputTokens":3}},
                    "turnTokenUsage":{{"usageScope":"main_agent","usageStatus":"complete","inputTokens":{},"outputTokens":{},"cachedInputTokens":{},"cacheCreationTokens":{}}}}}"#,
                input, output, cached, cache_write
            );
            // `None` binds as SQL NULL, which is what an unfinished turn has.
            let completed: Option<&str> = if completed.is_empty() {
                None
            } else {
                Some(completed)
            };
            conn.execute(
                "INSERT INTO orchestration_v2_projection_provider_turns
                     (provider_turn_id, thread_id, status, started_at, completed_at, payload_json)
                 VALUES (?1, ?2, 'completed', ?3, ?4, ?5)",
                rusqlite::params![turn_id, thread_id, started, completed, payload],
            )
            .unwrap();
        }
        drop(conn);
        path
    }

    /// The recipe with an explicit path: no process-wide state, so parallel tests
    /// cannot redirect each other.
    fn provider_for(path: &std::path::Path) -> Box<dyn UsageProvider> {
        Box::new(crate::providers::recipe::SqliteProvider::with_path(
            &RECIPE,
            path.to_path_buf(),
        ))
    }

    fn fetch(
        p: &dyn UsageProvider,
        cursor: Option<&ProviderCursor>,
        limit: usize,
    ) -> crate::provider::FetchOutcome {
        p.fetch(&FetchRequest {
            cursor,
            since_ms: None,
            registry: None,
            limit,
        })
        .expect("fetch")
    }

    #[test]
    fn reads_per_turn_usage_and_ignores_the_context_snapshot() {
        let path = scratch(&[
            (
                "pt1",
                "th1",
                "2026-11-20T10:00:00.000Z",
                "2026-11-20T10:05:00.000Z",
                1_000,
                100,
                5_000,
                0,
            ),
            ("pt2", "th1", "2026-11-20T11:00:00.000Z", "", 200, 20, 0, 0),
        ]);
        let p = provider_for(&path);

        let outcome = fetch(p.as_ref(), None, 100);
        assert_eq!(outcome.records.len(), 2);

        let first = &outcome.records[0];
        assert_eq!(first.provider, "deepai");
        assert_eq!(first.model, "deepai/mimo-2.6-pro");
        assert_eq!(first.id, "deepai:pt1");
        // 1,000 input + 5,000 cached + 100 output; the 999 `usedTokens` snapshot
        // and the payload's 1/2/3 snapshot counters are deliberately not counted.
        assert_eq!(first.total_tokens, 6_100);
        assert_eq!(first.session_id.as_deref(), Some("th1"));
        assert_eq!(first.source_type, "deepai_database");
    }

    #[test]
    fn a_turn_that_is_still_running_is_re_read_after_it_completes() {
        let path = scratch(&[(
            "pt1",
            "th1",
            "2026-11-20T10:00:00.000Z",
            "",
            1_000,
            100,
            0,
            0,
        )]);
        let p = provider_for(&path);

        let first = fetch(p.as_ref(), None, 100);
        assert_eq!(first.records.len(), 1);
        let cursor = first.cursor.clone();
        assert_eq!(
            cursor.last_timestamp_ms,
            Some(crate::database::parse_event_ms("2026-11-20T10:00:00.000Z"))
        );

        // The turn finishes later: the same id now carries a later completion
        // time, so the next pass must pick it up again rather than skip it.
        let finished = r#"{"turnTokenUsage":{"usageScope":"main_agent","usageStatus":"complete","inputTokens":2000,"outputTokens":300,"cachedInputTokens":0,"cacheCreationTokens":0}}"#;
        let conn = Connection::open(&path).unwrap();
        let changed = conn
            .execute(
                "UPDATE orchestration_v2_projection_provider_turns
                    SET completed_at = ?1, payload_json = ?2
                  WHERE provider_turn_id = ?3",
                rusqlite::params!["2026-11-20T12:00:00.000Z", finished, "pt1"],
            )
            .unwrap();
        assert_eq!(changed, 1, "the fixture row must exist to be finished");

        let stored: (String, String) = conn
            .query_row(
                "SELECT completed_at, payload_json
                   FROM orchestration_v2_projection_provider_turns
                  WHERE provider_turn_id = 'pt1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored.0, "2026-11-20T12:00:00.000Z");
        assert_eq!(stored.1, finished);
        drop(conn);

        let second = fetch(p.as_ref(), Some(&cursor), 100);
        assert_eq!(second.records.len(), 1, "the completed turn is re-read");
        assert_eq!(
            second.records[0].id, "deepai:pt1",
            "same identity, no duplicate"
        );
        assert_eq!(second.records[0].total_tokens, 2_300);
    }

    #[test]
    fn turns_without_usage_payloads_are_skipped() {
        let path = scratch(&[]);
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO orchestration_v2_projection_provider_turns
                 (provider_turn_id, thread_id, status, started_at, completed_at, payload_json)
             VALUES ('no-usage', 'th1', 'completed', '2026-11-20T10:00:00.000Z',
                     '2026-11-20T10:01:00.000Z', '{\"status\":\"completed\"}')",
            [],
        )
        .unwrap();
        drop(conn);

        let p = provider_for(&path);
        assert!(fetch(p.as_ref(), None, 100).records.is_empty());
    }

    #[test]
    fn the_recipe_points_at_the_documented_location() {
        // The production resolver must name DeepAI Code's real path, not just the
        // test override.
        let candidates = databases();
        assert!(
            candidates
                .iter()
                .any(|p| p.ends_with("statev2.sqlite")
                    && p.to_string_lossy().contains(".deepai-code")),
            "candidates were {:?}",
            candidates
        );
    }
}
