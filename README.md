# FreeTokenMeter

A local-first desktop monitor for AI coding tool usage. FreeTokenMeter reads the
structured usage data that coding agents already write to disk, normalizes it into
one model, prices it through a shared registry and reports it in a terminal-style
widget.

It is a **monitoring layer over AI coding tools**, not a collection of unrelated
one-off integrations: every provider plugs into the same pipeline and nothing
downstream knows where the data came from.

```text
provider source
    → provider adapter        (the only provider-specific code)
    → normalized UsageRecord
    → deduplication
    → SQLite
    → dynamic pricing registry
    → value calculation
    → aggregation
    → terminal UI
```

## Providers

| Provider | Usage | Cost | Status |
|----------|-------|------|--------|
| OpenCode | ✓ | ✓ | Supported — verified end to end on this machine |
| Freebuff | ✓ | ✓ | Supported — verified end to end on this machine |
| Codex | ✓ | ✓ (reference) | Supported — verified end to end on this machine |
| DeepAI Code | ✓ | ✓ (reference) | Supported — verified end to end on this machine (recipe-based) |
| Claude Code | ✓ | ✓ (reference) | Implemented — **not verified**: no local transcripts on this machine |
| Gemini CLI | ✓ | ✓ (reference) | Implemented — **not verified**: no local session files on this machine |
| GitHub Copilot | — | — | **Unsupported**: no structured local usage source in the installed version |
| Cline · Kiro · Cursor · Windsurf | — | — | Future / unsupported — see below |

"Verified end to end" means a real session's usage was imported, priced,
deduplicated on a second pass and kept after a restart. A provider is never listed
as supported on the strength of fixtures alone.

Detection is reported honestly. A provider is in exactly one of these states:

```text
● CONNECTED            installed, with a readable structured source
− DISABLED             switched off in settings
○ NOT INSTALLED        nothing detected
! USAGE UNAVAILABLE    installed, but no supported machine-readable source
! SYNC ERROR           connected, but the last sync failed (reason shown)
? UNSUPPORTED          no reliable structured source exists for the product
```

A provider in any state other than `CONNECTED` contributes **no** records. It is
never shown as `0 tokens`.

## Provider details

### OpenCode — supported

- **Source:** `~/.local/share/opencode/opencode.db` (or `$XDG_DATA_HOME/opencode`),
  the `session_v2` table, falling back to the legacy `session` table when the current
  schema is absent. Both carry `tokens_input`, `tokens_output`, `tokens_reasoning`,
  `tokens_cache_read`, `tokens_cache_write`, `cost`, `time_updated`. `model` is JSON:
  `{"id":"mimo-v2.5-free","providerID":"opencode"}`.
  Reading only `session` silently dropped everything OpenCode wrote after its
  `session_v2` migration — on the machine this was verified on, that was 74 of 257
  sessions. The table is chosen per pass, so a downgrade still works.
- **Collected:** model, per-session token classes, provider-reported cost, timestamp.
- **Cost semantics:** a positive `cost` is a real API-style charge
  (`PAYMENT MODE: API`). A zero cost means a promotional model, reported as a known
  `$0.00` next to a reference value.
- **Incremental:** `time_updated` is the watermark, so only sessions touched since
  the last pass are re-read. A session that grows after its first sync replaces its
  own row instead of double counting.
- **Dedup key:** session id (`oc_<session-id>`).
- **Limitations:** OpenCode's own total counts every token class it tracks, which is
  what is reported here; it is not comparable token-for-token with Codex totals.
- **Authentication:** none required — FreeTokenMeter opens the database read-only
  (plus a 5 s `busy_timeout`), so it can never create, lock or modify OpenCode's state.

### DeepAI Code — supported (defined as a recipe)

- **Source:** `~/.deepai-code/userdata/statev2.sqlite`,
  `orchestration_v2_projection_provider_turns`. One row per provider turn, joined to
  `orchestration_v2_projection_provider_sessions` for the model.
- **Collected:** model, `turnTokenUsage` (`inputTokens`, `outputTokens`,
  `cachedInputTokens`, `cacheCreationTokens`), turn start/completion time, thread id.
- **Not collected:** `tokenUsage` in the same payload is a *context-window snapshot*
  (`usedTokens`/`maxTokens`) that repeats and resets; summing it would be
  meaningless, so it is ignored deliberately.
- **Cost semantics:** the harness reports no per-turn dollar amount, so the payment
  mode is `SUBSCRIPTION`, actual cost is `N/A`, and the reference value comes from the
  pricing registry.
- **Incremental:** the watermark is `COALESCE(completed_at, started_at)`, so a turn
  that was still running when first read is re-read once it finishes. The record id is
  the provider turn id, so the re-read replaces the row instead of adding one.
- **Implementation:** this provider is not a bespoke integration. It is a
  [`SqliteRecipe`](src-tauri/src/providers/recipe.rs) — one `SELECT` and eight facts —
  driven by the shared engine. See [Adding a new provider](#adding-a-new-provider).

### Freebuff — supported

- **Source:** `~/.config/freebuff-desktop/projects/<project>/desktop-v2.db`, the
  `messages.metrics_json` column for `role = 'assistant'`.
- **Collected:** model (from `threads`), input/cached/output/reasoning tokens,
  `costUsd`, timestamp, thread id.
- **Cost semantics:** Freebuff models are promotional (`costUsd` = 0), so the actual
  cost is a known `$0.00` and the value is expressed through reference pricing.
  `cachedInputTokens` is a subset of `inputTokens`, so cache traffic is never counted
  twice. A missing `costUsd` key stays absent rather than becoming zero.
- **Incremental:** a per-project timestamp watermark; only newer messages are read.
  One unreadable project database never stops the others.
- **Dedup key:** `<project>:<seq>` (`fb_<project>_<seq>`).
- **Authentication:** none required.

### Codex — supported

- **Source:** `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`. Usage comes from
  `"type":"token_usage_record"` events; the active model comes from the preceding
  `turn_context`.
- **Collected:** model, `input_tokens`, `cached_input_tokens`,
  `cache_write_input_tokens`, `output_tokens`, `reasoning_output_tokens`,
  `total_tokens`, session/thread id, response id, timestamp.
- **Cost semantics:** a rollout contains **no monetary cost**. Codex usage is
  entitlement-based, so FreeTokenMeter reports:

  ```text
  ACTUAL COST          N/A          (no per-token invoice exists locally)
  REFERENCE API VALUE  $X.XX        (from the pricing registry)
  VALUE PROVIDED       —            (not claimed: the alternative spend is unknown)
  ```

  The plan type recorded in the rollout (`free`, `premium`, …) is what marks the
  records as `PAYMENT MODE: SUBSCRIPTION`.
- **Incremental:** a byte offset per rollout file, so only newly appended lines are
  parsed. The last model seen per file is carried in the cursor, so a resumed read
  still attributes usage correctly.
- **Dedup key:** the API `response_id` (`codex_<response_id>`).
- **Limitations:** `cached_input_tokens` is already part of `input_tokens`, so the
  billable total is `input + output`. Cumulative `token_count` events are not
  imported (only per-response records are), which is why scanned-line counts are much
  higher than imported-record counts.
- **Authentication:** none required — FreeTokenMeter reads the local rollout files
  written by the signed-in CLI.

### Claude Code — implemented, not yet verified

- **Source:** `~/.claude/projects/<project-slug>/<session-uuid>.jsonl` assistant
  turns, `message.usage`.
- **Collected:** model, `input_tokens`, `output_tokens`,
  `cache_read_input_tokens`, `cache_creation_input_tokens` (including the nested
  `cache_creation.ephemeral_*` breakdown), transcript `costUSD`, session id,
  message id, timestamp.
- **Cost semantics:** Claude Code's displayed cost is a **list-price calculation**,
  not the user's contracted rate. FreeTokenMeter therefore keeps it as
  `provider_reported_cost_usd` and reports `ACTUAL COST: N/A` for subscription usage.
  When the pricing registry knows the model, the registry rate is used for the
  reference value; when it does not, the transcript's own figure is used and the
  pricing source is recorded as `provider_reported`.
  The interactive `/usage` screen is deliberately **not** scraped.
- **Incremental:** byte offset per transcript file; the cursor persists.
- **Dedup key:** `message.id`, falling back to `uuid` (`claude_<id>`).
- **Limitations:** field names vary between Claude Code versions, so every metric is
  read defensively and stays `null` when not reported. On the machine this was
  developed on, `~/.claude/projects` contains no transcripts, so the provider reports
  `USAGE UNAVAILABLE` and the parser is covered by fixtures only.

### Gemini CLI — implemented, not yet verified

- **Source:** `~/.gemini/tmp/<project-hash>/chats/session-*.json`, `gemini` turns
  with a `tokens` block.
- **Collected:** model, `input`, `output`, `cached`, `thoughts`, `total`, session id,
  message id, timestamp.
- **Cost semantics:** the access method decides the money story and is read from
  configuration rather than guessed. A Google sign-in (`~/.gemini/oauth_creds.json`)
  means the free promotional tier, reported as `ACTUAL COST: $0.00` with a reference
  value. Otherwise the payment mode is `UNKNOWN` and the actual cost stays `N/A`.
  Gemini is never assumed to be free because of its name.
- **Null preservation:** cached-token detail is only available to API-key/Vertex
  users; OAuth sessions omit it, and it stays `null` rather than being flattened to
  `0`.
- **Incremental:** session documents are rewritten rather than appended, so the file
  modification time is the change marker.
- **Dedup key:** message id (`gemini_<id>`).
- **Limitations:** the interactive `/stats` screen is not scraped. On the machine
  this was developed on, `~/.gemini` holds no session files, so the provider reports
  `USAGE UNAVAILABLE` and the parser is covered by fixtures only.

### GitHub Copilot — unsupported

- **Investigated:** `~/.copilot` contains `config.json` (a first-launch timestamp),
  `ide/` locks and one unstructured `logs/process-*.log` per CLI launch. No file with
  historical per-session token counts exists in the installed version.
- **Decision:** the CLI's `/usage` terminal display is **not** scraped, so Copilot
  reports `USAGE UNAVAILABLE` and contributes no records. It is not listed as
  supported until a supported structured source exists and has been verified.
- **Modelled for later:** GitHub meters Copilot usage in AI credits (1 credit = $0.01
  in the documented billing model) and subscriptions include entitlements. When a
  source does land, `provider reported value`, `reference API value` and
  `actual user cost` stay three separate figures — credits are never translated
  blindly into money spent.

### Cline, Kiro, Cursor, Windsurf — future work, not implemented

These are listed in the UI as `UNSUPPORTED` with the reason, and are **not**
registered as providers:

| Product | Why it is not implemented |
|---------|---------------------------|
| Cline | Token/cost fields are part of its extension API; no stable local usage store was found. |
| Kiro | `/usage` reports context-window occupancy, which is not historical token consumption. |
| Cursor | Usage is published in the web dashboard; no supported local machine-readable source. |
| Windsurf | Plan usage is in the web dashboard only; no supported local source. |

Scraping a web dashboard because the data is visible there is explicitly out of scope.

## How value is calculated

FreeTokenMeter keeps three figures apart:

| Figure | Meaning |
|--------|---------|
| **Actual cost** | What the user is actually billed. `N/A` when no per-token spend exists. |
| **Reference API value** | What the same usage would cost at list rates. |
| **Free value** | `reference − actual`. `N/A` unless **both** sides are known. |

`ACTUAL COST: N/A` is not the same as `$0.00`:

```text
$0.00   the provider charges nothing (promotional / explicitly free)
N/A     no per-token spend exists (subscription entitlement) or nothing is known
```

Because `free_value` requires both sides, FreeTokenMeter never claims the user
"saved $X" under a flat-fee plan — the alternative expenditure is not known.

### Payment modes and billing units

Each record carries how it is paid for and what it is metered in. Neither is
inferred from the provider's name.

```text
PAYMENT MODE      API · SUBSCRIPTION · PROMOTIONAL · UNKNOWN
BILLING UNIT      usd · ai_credits · subscription_units · unknown
```

```text
OpenCode          2.41M        $0.00       $2.14
Freebuff          1.18M        $0.00       $0.83
Claude Code       4.82M         N/A        $8.42
Codex             3.14M         N/A        $5.91
Gemini CLI        2.77M        $0.00       $3.10
GitHub Copilot    USAGE UNAVAILABLE
```

## Pricing

Pricing is maintained through a **centralized versioned pricing registry**. Unknown
models are reported as unpriced rather than assumed to be free.

Resolution order:

```text
provider + model
    → explicit FreeTokenMeter override       (promotional / free models)
    → exact match in the dynamic registry    (LiteLLM model cost map)
    → normalized match / provider-prefix stripping / common aliases
    → unknown
```

There is exactly one pricing engine. Providers never carry their own rates — they
pass what they observed (tokens, any provider-reported cost, billing units, payment
mode) into `pricing::value_usage` and get back a fully described value.

### Pricing status

| Status | Meaning |
|--------|---------|
| `actual_cost_known` | The provider reported a real, positive cost |
| `reference_applied` | A reference value exists (list pricing or a free-model mapping) |
| `reference_unavailable` | No reference pricing configured for this model |
| `unknown` | Model missing from every source — **unpriced** |
| `no_tokens` | Nothing to price |

Unpriced records are excluded from the cost totals — the UI shows `N/A` — and are
listed in the `$ pricing` view so a new model is visible instead of quietly
contributing a misleading `$0`.

### Maintaining pricing

Adding a **free or promotional** model is a data-only change: append one entry to
`OVERRIDES` in `src-tauri/src/pricing.rs`. Adding a **paid** model usually needs no
change at all, because the dynamic registry already knows it.

```rust
ModelPricingProfile {
    provider: "opencode",              // "opencode", "freebuff", or "api" (provider-agnostic)
    model_id: "my-new-free-model",
    aliases: &[],                      // optional extra identifiers
    display_name: "My New Free Model",
    actual: PricingRates::FREE,        // only when the provider confirms it is free
    reference: Some(PricingRates {     // omit (None) when no paid equivalent exists
        input_per_million: 0.10,
        output_per_million: 0.40,
        cached_per_million: 0.025,
    }),
    reference_model: Some("my-new-model"),
    source: "Provider API pricing documentation",
    version: "2025-07.1",
    verified_at: "2025-07",
},
```

Rules:

- Never assume that removing `-free` from a model id gives the paid equivalent —
  use explicit mappings.
- Never list a model as free unless the provider confirms it. Anything unlisted is
  **unpriced**, not `$0`.
- Bump `PRICING_VERSION` when rates change so historical records stay reproducible;
  every record stores the pricing version it was valued with.
- Bump the entry's `version` / `verified_at` when re-verifying a rate, so stale data
  is visible in the `$ pricing` view.

Press `p` (or click `[$]`) to open the maintenance view, which lists every configured
model plus every model seen locally that has no pricing entry.

## Desktop features

- **System tray** — Show, Hide, Sync Now, ✓ Always on Top and Quit
- **Always-on-top toggle** — a real native window toggle in the settings panel, no restart required
- **Persistent window size and position** — validated against the connected monitors before restore
- **Persistent preferences** — `preferences.json` (SQLite is used for usage history only)
- **Background synchronization** — every 30 seconds, including while hidden to the tray
- **Manual Sync Now** — tray menu or `r` in the UI, both through the same sync path
- **Provider health** — per-provider detection state, source kind, last sync, records and tokens
- **Provider toggles** — enable/disable individual providers; the choice persists
- **Controlled backfill** — 7 days / 30 days / all available on a provider's first sync
- **Multi-provider dashboard** — provider filter, provider-aware model table, activity feed with providers
- **Widget mode** — usable down to 360 × 300 with reflowing tables

### Window and tray behaviour

```text
window close  →  hide to tray  →  background synchronization continues
```

Quitting (`Quit` in the tray, or `[x] QUIT` in settings) shuts down the tray, the
background timers, the sync tasks and the database connection and exits the process.

### Preferences and window geometry

Preferences live in `<data-dir>/freetokenmeter/preferences.json`:

```json
{
  "always_on_top": false,
  "window_width": 900,
  "window_height": 600,
  "window_x": 120,
  "window_y": 80,
  "disabled_providers": [],
  "backfill_days": 30
}
```

- Geometry is written atomically and **debounced** (~750 ms after the last
  resize/move), so dragging never causes a write per pixel.
- On startup saved geometry is validated against the monitors that currently exist:
  off-screen positions, removed secondary displays, changed resolutions and sizes
  below the widget minimum are clamped back into a visible area.
- The minimum usable footprint is **360 × 300**.

## Synchronization

```text
registered providers
    → filter: enabled by the user + a readable source
    → bounded parallel fetch (at most 4 at a time)
    → deduplicated, serialized write (one SQLite connection)
    → per-provider cursor persisted
    → SyncReport → UI + tray
```

- **Isolation:** each provider's fetch runs inside `catch_unwind`; an error or a
  panic becomes a per-provider result. One broken provider never blocks another and
  never takes the app down.
- **Incremental:** providers keep their own cursor (a database watermark, a byte
  offset per append-only file, or a per-database timestamp). History is not
  re-imported on every cycle.
- **Backfill:** the first sync of a provider uses the configured window (default
  30 days) so a large history cannot stall the dashboard.
- **Bounded passes are lossless:** a pass stops *between* sources at the 20,000-record
  cap. A source that has been started (a rollout file, a transcript, a project
  database, the OpenCode table) is always consumed to its end before its own offset or
  watermark advances, so the cap costs a later pass rather than the tail of the data.
  OpenCode, which has a single watermark instead of per-file offsets, advances to the
  newest `time_updated` it actually took, so a truncated pass still makes progress.
- **Dedup:** one record per provider-native event id, with `INSERT OR REPLACE` as the
  database-level uniqueness rule.

## Architecture

```text
                        FreeTokenMeter
                              │
                       Provider Registry
                              │
        ┌─────────┬───────────┼───────────┬───────────┬──────────────┐
        ▼         ▼           ▼           ▼           ▼              ▼
    OpenCode  Freebuff   Claude Code    Codex     Gemini CLI   GitHub Copilot
        │         │           │           │           │              │
        └─────────┴───────────┴───────────┴───────────┴──────────────┘
                              │
                    Normalized UsageRecord
                              │
                         Deduplication
                              │
                           SQLite
                              │
                    Dynamic Pricing Registry
                              │
                        Value Calculation
                              │
                     Aggregation / Analytics
                              │
                        Terminal UI
```

```
FreeTokenMeter/
├── src/                          # React frontend
│   ├── components/               # UI components
│   ├── hooks/                    # React hooks
│   ├── theme/                    # theme-pack schema, loader and CSS application
│   ├── themes/                   # built-in community theme JSON
│   └── types/                    # TypeScript types (mirror usage.rs)
├── src-tauri/
│   ├── src/
│   │   ├── main.rs               # Entry point
│   │   ├── lib.rs                # Tauri commands, state, startup order
│   │   ├── provider.rs           # UsageProvider trait + ProviderRegistry
│   │   ├── usage.rs              # Normalized model, payment modes, billing units
│   │   ├── database.rs           # SQLite layer (usage history)
│   │   ├── opencode_provider.rs  # OpenCode integration
│   │   ├── freebuff_provider.rs  # Freebuff integration
│   │   ├── claude_code_provider.rs
│   │   ├── codex_provider.rs
│   │   ├── gemini_cli_provider.rs
│   │   ├── copilot_provider.rs   # Detection only — no supported source
│   │   ├── pricing.rs            # Versioned pricing registry + valuation
│   │   ├── registry.rs           # Dynamic LiteLLM registry (cached, refreshed daily)
│   │   ├── model_normalization.rs# Provider-prefix stripping and aliases
│   │   ├── sync.rs               # Shared sync manager (UI, tray, scheduler)
│   │   ├── themes.rs             # Community theme discovery and safe file IO
│   │   ├── tray.rs               # System tray menu and events
│   │   ├── window.rs             # Show/hide, always-on-top, geometry
│   │   └── preferences.rs        # preferences.json + geometry validation
│   └── tauri.conf.json
└── public/
```

### Database schema (v6)

One normalized table, no provider-specific tables. Provider detail lives in
normalized columns so adding a provider never forks the schema.

```sql
CREATE TABLE usage_records (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,          -- canonical id: 'opencode', 'codex', ...
    model TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    input_tokens INTEGER,            -- NULL = not reported (not the same as 0)
    output_tokens INTEGER,
    cached_tokens INTEGER,
    cache_write_tokens INTEGER,
    reasoning_tokens INTEGER,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    provider_cost_usd REAL,          -- what the provider's own tooling computed
    actual_cost_usd REAL,            -- what the user is billed; NULL = N/A
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
    event_id TEXT,                   -- provider-native event identity
    source_type TEXT NOT NULL DEFAULT 'unknown',
    source_id TEXT,
    time_created INTEGER NOT NULL,   -- when FTM ingested the row (never updated)
    event_time_ms INTEGER NOT NULL DEFAULT 0  -- when the usage actually happened
);

CREATE INDEX idx_usage_event_time ON usage_records(event_time_ms);

CREATE TABLE sync_state (
    provider TEXT PRIMARY KEY,
    last_sync_time INTEGER NOT NULL DEFAULT 0,
    last_session_id TEXT,
    cursor_json TEXT,                -- per-provider incremental cursor
    state TEXT NOT NULL DEFAULT 'unknown',
    detail TEXT
);
```

Existing databases are migrated in place; the previous schema's `NOT NULL
DEFAULT 0` token columns are rebuilt so unknown metrics can be stored as `NULL`.
A migration test asserts that existing history survives with its reference values and
known-zero costs intact.

`event_time_ms` exists because `time_created` is an *ingestion* timestamp. Filtering
ranges on ingestion time made a 90-day backfill report all of its history as `TODAY`.
Every range query and the activity feed now use `event_time_ms` (indexed), which is
derived from the provider's own timestamp at write time. Rows written before v6 are
backfilled once from `timestamp`, falling back to `time_created` when the timestamp is
unparseable; `time_created` itself is preserved on re-sync so a record's ingest time
never moves.

## Privacy

FreeTokenMeter is local-first.

- No usage data is uploaded anywhere.
- Provider credentials are never read, logged, uploaded, stored in SQLite or exposed
  to the frontend. FreeTokenMeter only reads local files the providers already write.
- No API keys are requested: every provider that is supported authenticates through
  its own tool.
- The webview loads nothing from the network: the Google Fonts stylesheet was removed
  and the window runs under a restrictive CSP (`default-src 'self'`, no remote
  origins in `connect-src`). Only the pricing-registry download in the Rust backend
  touches the network, and only list prices are exchanged.

### Logs

`%APPDATA%/freetokenmeter/logs/freetokenmeter.log` (Windows;
`~/.local/share/freetokenmeter/logs/` elsewhere) records startup, each sync pass, and
per-provider failures, and panics are appended there too. The level defaults to
`info` and can be raised with `FTM_LOG=debug`. The log holds counts, provider names
and error messages — never message content, prompts or credentials.

## Tech stack

- **Tauri 2** — desktop shell (Rust backend)
- **React 19** + **TypeScript** — UI
- **Vite 8** — build tooling
- **Tailwind CSS 4** — styling
- **SQLite** (`rusqlite`) — local usage history

## Setup

Prerequisites: Node.js 18+, a stable Rust toolchain (MSVC on Windows), and
`@tauri-apps/cli` (already a dev dependency).

```bash
npm install
npm run tauri dev
```

Production build:

```bash
npm run tauri build
```

On Windows this writes an NSIS installer to
`src-tauri/target/release/bundle/nsis/FreeTokenMeter_<version>_x64-setup.exe`
(current-user install, no admin prompt). The bundle configuration lives under
`bundle` in `src-tauri/tauri.conf.json`.

A second launch focuses the window that is already running instead of starting a
second meter that would sync against the same database
(`tauri-plugin-single-instance`).

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `r` | Sync all enabled providers |
| `s` | Open/close the settings panel |
| `p` | Open/close the pricing registry |
| `1` | Time range: Today (in settings: toggle Always on Top) |
| `2` | Time range: Week |
| `3` | Time range: Month |
| `4` | Time range: All Time |
| `q` | Provider filter: All |
| `h` | Hide to tray (settings panel only) |
| `x` | Quit FreeTokenMeter (settings panel only) |
| `Esc` | Close the open panel |

The provider filter tabs are clickable and are generated from the providers the
backend reports, so adding a provider needs no frontend change.

## Customizing the UI

The dashboard is designed to be forked. There are three levels, in increasing
order of effort:

1. **Pick a built-in pack.** Instrument, Midnight, Paper and Focus ship with the
   app. Switch them in `Settings → Appearance`.
2. **Write a JSON theme pack.** A pack is a small token document in
   `%APPDATA%\freetokenmeter\themes` (Windows; `~/Library/Application Support`
   on macOS, `~/.local/share` on Linux). It can recolour the app, change fonts,
   choose compact/roomy density, disable motion, and reorder or hide dashboard
   sections. Drop the file in, press **reload**, and the window updates without
   a rebuild. **start from this pack** writes an editable copy for you.
3. **Fork the frontend.** `useDashboard()` owns the backend data contract,
   `DashboardLayout` owns section order and `useTheme()` owns packs. Replace the
   body of `App.tsx` with your own layout and keep those seams; no Rust changes
   are needed.

Theme files are validated, size-limited and applied only as CSS custom
properties; a pack cannot execute code. The full schema, layout section ids,
community checklist and fork guide live in
[docs/UI_CUSTOMIZATION.md](docs/UI_CUSTOMIZATION.md).

## Adding a new provider

Every provider is registered in exactly one place —
`src-tauri/src/providers/mod.rs::catalogue()` — and nothing else in the application
knows the list. Sync, storage, pricing, aggregation, commands and the UI all iterate
the registry built from it, so a new provider needs no edits there.

### Path A — a SQLite recipe (the common case, ~20 lines)

If the tool keeps usage in a local SQLite database, do not write a provider. Add a
recipe:

```rust
// src-tauri/src/providers/my_tool.rs
use crate::providers::recipe::{ColumnCheck, SqliteRecipe};
use crate::usage::{BillingUnit, PaymentMode};
use std::path::PathBuf;

fn databases() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|home| vec![home.join(".my-tool/usage.db")])
        .unwrap_or_default()
}

pub static RECIPE: SqliteRecipe = SqliteRecipe {
    id: "my-tool",                 // canonical id, lowercase, a database key
    name: "My Tool",
    short_label: "MYTOOL",         // ≤ 8 chars, fits the provider column
    source_type: "my_tool_database",
    payment_mode: PaymentMode::Subscription,
    billing_unit: BillingUnit::Usd,
    backfill_days: 30,
    cached_is_subset_of_input: false, // true when `input` already counts cache reads
    databases,
    requires: &[ColumnCheck::new("turns", "created_at")],
    rows_sql: "
        SELECT id, model, created_at, input_tokens, output_tokens, cached_tokens,
               NULL, NULL, NULL
        FROM turns
        WHERE created_at > ?1
        ORDER BY created_at ASC
        LIMIT ?2",
    watermark_sql: "SELECT MAX(created_at) FROM turns",
};
```

Then register it with one line:

```rust
// providers/mod.rs
recipe::provider(&my_tool::RECIPE),
```

The shared engine handles read-only access, the schema probe, the sync window, the
watermark, truncation and the pricing hand-off. `src-tauri/src/providers/recipe.rs`
documents the row contract (nine columns, one optional tenth), the two bound
parameters (`?1` = ISO-8601 window floor, `?2` = row limit), and the two cache
semantics. JSON payloads need no Rust: extract them in SQL with `json_extract`, as the
DeepAI Code recipe does.

### Path B — a bespoke reader

For append-only JSONL logs, rewritten session documents, several project databases, or
a schema that has changed between releases, implement `UsageProvider`:

1. Create `src-tauri/src/my_provider.rs` and implement `UsageProvider`:

```rust
impl UsageProvider for MyProvider {
    fn id(&self) -> &'static str { "my-provider" }          // canonical id, not a display name
    fn display_name(&self) -> &'static str { "My Provider" }
    fn short_label(&self) -> &'static str { "MYPROV" }
    fn source_type(&self) -> &'static str { "my_provider_source" }
    fn payment_mode(&self) -> PaymentMode { PaymentMode::Unknown }
    fn availability(&self) -> ProviderAvailability { /* Connected / NotInstalled / UsageUnavailable */ }
    fn detail(&self) -> Option<String> { /* why it is in that state */ }
    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        // request.cursor   — resume from the previous pass
        // request.since_ms — backfill window on the first pass
        // request.registry — the shared pricing registry
        // Return normalized records; the caller deduplicates and stores them.
    }
}
```

2. Normalize each event with the shared engine — never compute prices yourself:

```rust
let value = pricing::value_usage(&PricingInput {
    provider: "my-provider",
    model: &model,
    provider_cost_usd: observed_cost,     // None when the source reports no money
    billing_units: observed_units,        // e.g. AI credits
    billing_unit: BillingUnit::Usd,
    payment_mode: PaymentMode::Subscription,
    input_tokens: input,                  // None when the source omits it
    output_tokens: output,
    cached_tokens: cached,
    cache_write_tokens: cache_write,
}, request.registry);
```

3. Register it in `providers::catalogue()`:

```rust
Box::new(crate::my_provider::MyProvider::new()),
```

That is the whole integration. Synchronization, deduplication, the database,
pricing, aggregation, the UI, provider health and provider toggles all iterate the
registry, so none of them change. `providers::mod` has a test that fails if an id is a
placeholder, a label is too wide, or a provider reports a state without a reason.

**Provider rules**

- Read machine-readable sources only: an official local database or API, structured
  CLI output, an official export format, or a stable session/history file.
- Never scrape terminal output, web dashboards or OCR, and never simulate input.
- If no reliable source exists, report `Unsupported` or `UsageUnavailable` with the
  reason. Do not invent a parser.
- Preserve `null` for missing metrics. Never turn "not reported" into `0`.
- Implement provider-specific dedup on the provider's own event identity, and keep
  the database's `INSERT OR REPLACE` as the final guard.

## Tests

```bash
cd src-tauri && cargo test          # provider parsers, availability, dedup,
                                    # incremental sync, cursors, pricing, migration,
                                    # failure isolation, theme-pack discovery
npm run build                       # TypeScript + production frontend build
```

Fixtures cover each provider's parser, including missing-field (`null`, not `0`),
duplicate-event, backfill-window and failure-isolation cases.

There is also a real-data smoke test, excluded from the normal run because it
depends on what is installed on the machine:

```bash
cd src-tauri && cargo test --lib -- --ignored --nocapture real_local_data
```

It prints the detection state of every provider, runs two sync passes and asserts
that a connected provider imports records, an unavailable provider reports failure,
and the second pass adds nothing (deduplication).

## Manual verification

1. `npx tauri dev`
2. Tray: Show, Hide, Sync Now, Always on Top, Quit; closing the window hides it and
   background sync continues.
3. Always on Top: toggle ON, focus another app, confirm FreeTokenMeter stays above
   it, toggle OFF, restart and confirm the state persisted.
4. Window geometry: resize and move, hide, restart, confirm the geometry is restored;
   if the saved position is off-screen it is clamped back into view.
5. Providers: open `$ settings providers`, disable one, confirm it stops syncing and
   shows `DISABLED`, re-enable it and confirm it resumes.
6. Hide to tray, generate new usage in a provider, wait one cycle, reopen and confirm
   the new usage appears.
7. Appearance: switch Instrument → Paper → Focus; confirm the palette changes, Focus
   hides the calendar, activity, models and status, and the choice survives a restart.
   Then use
   **start from → save copy**, edit the JSON, press **reload**, and confirm the
   edited pack is applied without a rebuild.

## Not yet implemented

- No OS-level notifications (tray feedback appears in the tray tooltip and the in-app
  `SYNC COMPLETE` status line)
- No launch-at-login/autostart support
- Claude Code and Gemini CLI integrations are written, tested with fixtures and
  awaiting a machine that actually has their local session data
- GitHub Copilot, Cline, Kiro, Cursor and Windsurf await a supported structured
  source; the UI lists them as `UNSUPPORTED` with the reason. A Copilot `data.db`
  exists on some installations and is the most promising next source.
- No code signing, no auto-updater and no licence file yet, so the installer is
  unsigned (SmartScreen warns on first run). Signing needs a certificate and the
  updater needs a key pair, both of which are deployment decisions.
