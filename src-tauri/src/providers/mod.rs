//! The provider catalogue.
//!
//! **This module is the single registration point.** Adding a provider is an edit
//! here plus one self-contained file — nothing in `sync.rs`, `database.rs`,
//! `pricing.rs`, the aggregation queries, the commands or the React UI changes,
//! because all of them iterate the registry built from [`catalogue`].
//!
//! # Two ways to add a provider
//!
//! ## 1. A SQLite recipe (the common case, ~20 lines)
//!
//! If the tool keeps usage in a local SQLite database — OpenCode, Freebuff,
//! GitHub Copilot, DeepAI Code and most desktop harnesses do — write a
//! [`SqliteRecipe`](recipe::SqliteRecipe) instead of an implementation. The shared
//! engine handles read-only access, schema probing, the sync window, the watermark
//! and the pricing hand-off; all the recipe supplies is a `SELECT` and a handful of
//! facts.
//!
//! ```ignore
//! // providers/my_tool.rs
//! pub static RECIPE: SqliteRecipe = SqliteRecipe {
//!     id: "my-tool",
//!     name: "My Tool",
//!     short_label: "MYTOOL",
//!     source_type: "my_tool_database",
//!     payment_mode: PaymentMode::Subscription,
//!     billing_unit: BillingUnit::Usd,
//!     backfill_days: 30,
//!     cached_is_subset_of_input: false,
//!     databases: || vec![home(".my-tool/usage.db")],
//!     requires: &[ColumnCheck::new("turns", "created_at")],
//!     rows_sql: "SELECT id, model, created_at, input, output, cached, cache_write,
//!                reasoning, NULL FROM turns WHERE created_at > ?1 ORDER BY created_at LIMIT ?2",
//!     watermark_sql: "SELECT MAX(created_at) FROM turns",
//! };
//! ```
//!
//! Then one line below. `providers::recipe` documents the row contract, the two
//! bound parameters, and the two cache semantics.
//!
//! ## 2. A bespoke reader
//!
//! For anything that is not one table in one database — append-only JSONL logs
//! (Codex, Claude Code), rewritten session documents (Gemini CLI), several project
//! databases (Freebuff), a schema that moved between releases (OpenCode's
//! `session` → `session_v2`) — implement [`UsageProvider`] directly. That is what
//! the existing providers do, and they stay responsible for their own format.
//!
//! # Rules that do not change with the approach
//!
//! * Machine-readable sources only. Never scrape a terminal, a web dashboard or
//!   pixels.
//! * Never write to the source: open databases read-only, never migrate them.
//! * `NULL` means "not reported"; it must never become `0`.
//! * Report `UsageUnavailable`/`Unsupported` with a reason instead of importing
//!   nothing quietly.

pub mod deepai;
pub mod recipe;

use crate::provider::UsageProvider;

/// Every provider the application ships, in presentation order.
///
/// The order here is the order of the provider tabs in the UI.
pub fn catalogue() -> Vec<Box<dyn UsageProvider>> {
    vec![
        // ── Bespoke readers ────────────────────────────────────────────────
        // File formats, multiple databases, or a schema that changed shape.
        Box::new(crate::opencode_provider::OpenCodeProvider::new()),
        Box::new(crate::freebuff_provider::FreebuffProvider::new()),
        Box::new(crate::claude_code_provider::ClaudeCodeProvider::new()),
        Box::new(crate::codex_provider::CodexProvider::new()),
        Box::new(crate::gemini_cli_provider::GeminiCliProvider::new()),
        Box::new(crate::copilot_provider::GitHubCopilotProvider::new()),
        // ── Recipe-backed readers ──────────────────────────────────────────
        // One SQL statement and a few facts each; see `recipe`.
        recipe::provider(&deepai::RECIPE),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_provider_id_is_unique() {
        let ids: Vec<&str> = catalogue().iter().map(|p| p.id()).collect();
        let unique: HashSet<&&str> = ids.iter().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "duplicate provider id in {:?}",
            ids
        );
    }

    #[test]
    fn every_provider_declares_a_complete_contract() {
        for provider in catalogue() {
            let id = provider.id();
            assert!(!id.is_empty(), "a provider has an empty id");
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{} must be a lowercase canonical id (it is a database key)",
                id
            );
            assert!(!provider.display_name().is_empty(), "{} has no name", id);
            assert!(!provider.short_label().is_empty(), "{} has no label", id);
            assert!(
                provider.short_label().len() <= 8,
                "{} has a label too wide for the provider column: {}",
                id,
                provider.short_label()
            );
            assert!(
                !provider.source_type().is_empty(),
                "{} has no source type",
                id
            );
            assert!(
                provider.default_backfill_days() > 0,
                "{} has a non-positive backfill window",
                id
            );
            // Detection must answer for every provider without panicking, and a
            // provider that is not connected must say why.
            let state = provider.availability();
            if state != crate::usage::ProviderAvailability::Connected {
                assert!(
                    provider.detail().is_some(),
                    "{} reports {:?} without a detail",
                    id,
                    state
                );
            }
        }
    }

    #[test]
    fn the_shipped_providers_are_all_present() {
        let registry = crate::provider::default_registry();
        for id in [
            "opencode",
            "freebuff",
            "claude-code",
            "codex",
            "gemini-cli",
            "github-copilot",
            "deepai",
        ] {
            assert!(registry.get(id).is_some(), "missing provider {}", id);
        }
    }

    #[test]
    fn the_registry_is_exactly_the_catalogue() {
        assert_eq!(
            crate::provider::default_registry().len(),
            catalogue().len(),
            "default_registry must not add or drop providers"
        );
    }
}
