# Pebble Phase 4: Translate + Settings Enhancement

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the translation module (selection translate + bilingual view) and enhance Settings with tabbed UI covering accounts, theme switching, rules management, translate engine config, and keyboard shortcut customization.

**Architecture:** New Rust crate `pebble-translate` with pluggable provider backends (DeepLX, DeepL, generic API, LLM/OpenAI-compatible). New `translate_config` table in SQLite. Frontend Settings redesigned as tabbed view. Dark theme via CSS variable swap. Translation popover on text selection.

**Tech Stack:** reqwest 0.12 (HTTP client for translate APIs), existing CSS variables (theme), existing Zustand stores

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md` § 4.6, § 6.6

**Depends on:** Phase 3 complete (command palette, kanban, snooze, rules, compose)

**Note on Gmail/Outlook providers:** OAuth2 provider integration (Gmail API, Microsoft Graph) is deferred to Phase 5. It requires significant work (OAuth2 flow, token refresh, platform-specific credential storage, API-specific sync logic) that deserves its own dedicated plan. Phase 4 focuses on translate + settings which are independent and higher user-value.

---

## File Structure (Phase 4 additions)

```
pebble/
├── crates/
│   └── pebble-translate/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs              # TranslateService: translate text via configured provider
│           ├── types.rs            # TranslateProvider enum, TranslateResult, BilingualSegment
│           ├── deeplx.rs           # DeepLX provider implementation
│           ├── deepl.rs            # DeepL official API implementation
│           ├── generic.rs          # Generic translate API implementation
│           └── llm.rs              # LLM/OpenAI-compatible implementation
│
├── crates/pebble-store/src/
│   └── translate_config.rs         # translate_config CRUD (store/load/delete)
│
├── src-tauri/src/
│   ├── commands/
│   │   └── translate.rs            # translate_text, get_translate_config, save_translate_config, test_translate_connection
│   └── lib.rs                      # Register new commands
│
├── src/
│   ├── stores/
│   │   └── settings.store.ts       # SettingsStore: active tab, translate config, rules list, keybindings
│   ├── features/
│   │   ├── translate/
│   │   │   ├── TranslatePopover.tsx # Selection-triggered translate popover
│   │   │   └── BilingualView.tsx    # Side-by-side bilingual display
│   │   └── settings/
│   │       ├── SettingsView.tsx     # Tabbed settings (rewrite)
│   │       ├── AccountsTab.tsx      # Existing account management (extracted)
│   │       ├── AppearanceTab.tsx    # Theme switching (light/dark/system)
│   │       ├── RulesTab.tsx         # Rules list + create/edit/delete UI
│   │       ├── TranslateTab.tsx     # Translate engine configuration
│   │       └── ShortcutsTab.tsx     # Keyboard shortcut display + customization
│   └── styles/
│       └── index.css               # Add dark theme CSS variables
```

---

### Task 1: Database + Store — Translate config persistence

**Files:**
- Create: `crates/pebble-store/src/translate_config.rs`
- Modify: `crates/pebble-store/src/lib.rs` (add module)
- Modify: `crates/pebble-store/src/migrations.rs` (add translate_config table)

**Context:** We need a table to persist the user's translate engine configuration. The config is a single JSON blob per provider type, with only one active at a time.

- [ ] **Step 1: Add translate_config table to migrations**

In `crates/pebble-store/src/migrations.rs`, append to `SCHEMA_V1`:

```sql
CREATE TABLE IF NOT EXISTS translate_config (
    id TEXT PRIMARY KEY DEFAULT 'active',
    provider_type TEXT NOT NULL CHECK(provider_type IN ('deeplx', 'deepl', 'generic_api', 'llm')),
    config TEXT NOT NULL DEFAULT '{}',
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

- [ ] **Step 2: Add TranslateConfig type to pebble-core/types.rs**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateConfig {
    pub id: String,
    pub provider_type: String,
    pub config: String,   // JSON blob with provider-specific settings
    pub is_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
```

- [ ] **Step 3: Create translate_config.rs**

```rust
use pebble_core::{PebbleError, Result, TranslateConfig};
use rusqlite::params;
use crate::Store;

impl Store {
    pub fn save_translate_config(&self, config: &TranslateConfig) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO translate_config (id, provider_type, config, is_enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     provider_type = excluded.provider_type,
                     config = excluded.config,
                     is_enabled = excluded.is_enabled,
                     updated_at = excluded.updated_at",
                params![config.id, config.provider_type, config.config, config.is_enabled as i32, config.created_at, config.updated_at],
            ).map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    pub fn get_translate_config(&self) -> Result<Option<TranslateConfig>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, provider_type, config, is_enabled, created_at, updated_at FROM translate_config WHERE id = 'active'"
            ).map_err(|e| PebbleError::Storage(e.to_string()))?;
            let mut rows = stmt.query_map([], |row| {
                Ok(TranslateConfig {
                    id: row.get(0)?,
                    provider_type: row.get(1)?,
                    config: row.get(2)?,
                    is_enabled: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            }).map_err(|e| PebbleError::Storage(e.to_string()))?;
            match rows.next() {
                Some(Ok(config)) => Ok(Some(config)),
                Some(Err(e)) => Err(PebbleError::Storage(e.to_string())),
                None => Ok(None),
            }
        })
    }

    pub fn delete_translate_config(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM translate_config WHERE id = 'active'", [])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }
}
```

Tests: `test_translate_config_save_and_load`, `test_translate_config_upsert`, `test_translate_config_delete`

- [ ] **Step 4: Add module to lib.rs**

Add `pub mod translate_config;` to `crates/pebble-store/src/lib.rs`.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p pebble-store -p pebble-core`
Commit: `feat(store): add translate_config table and CRUD operations`

---

### Task 2: pebble-translate crate — Translation engine

**Files:**
- Create: `crates/pebble-translate/Cargo.toml`
- Create: `crates/pebble-translate/src/lib.rs`
- Create: `crates/pebble-translate/src/types.rs`
- Create: `crates/pebble-translate/src/deeplx.rs`
- Create: `crates/pebble-translate/src/deepl.rs`
- Create: `crates/pebble-translate/src/generic.rs`
- Create: `crates/pebble-translate/src/llm.rs`
- Modify: `Cargo.toml` (workspace members + reqwest dep)

**Context:** The translate module supports 4 provider backends. Each provider implements an async `translate` method. The service dispatches to the configured provider. Translation results include bilingual segments for side-by-side display. LLM mode uses OpenAI-compatible chat completions API.

- [ ] **Step 1: Create crate**

`crates/pebble-translate/Cargo.toml`:
```toml
[package]
name = "pebble-translate"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }
reqwest = { workspace = true }
```

Add to workspace members in root `Cargo.toml`.
Add workspace dep: `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }`

- [ ] **Step 2: Create types.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TranslateProviderConfig {
    #[serde(rename = "deeplx")]
    DeepLX { endpoint: String },
    #[serde(rename = "deepl")]
    DeepL { api_key: String, use_free_api: bool },
    #[serde(rename = "generic_api")]
    GenericApi {
        endpoint: String,
        api_key: Option<String>,
        method: Option<String>,          // POST by default
        source_lang_param: String,       // e.g. "source_lang"
        target_lang_param: String,       // e.g. "target_lang"
        text_param: String,              // e.g. "text"
        result_path: String,             // JSON path to result, e.g. "data.translations.0.translatedText"
    },
    #[serde(rename = "llm")]
    LLM {
        endpoint: String,                // OpenAI-compatible API endpoint
        api_key: String,
        model: String,
        mode: LLMMode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LLMMode {
    Completions,   // /v1/chat/completions
    Responses,     // /v1/responses
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateResult {
    pub translated: String,
    pub segments: Vec<BilingualSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BilingualSegment {
    pub source: String,
    pub target: String,
}
```

- [ ] **Step 3: Create deeplx.rs**

```rust
use pebble_core::{PebbleError, Result};
use crate::types::{TranslateResult, BilingualSegment};

pub async fn translate(endpoint: &str, text: &str, from: &str, to: &str) -> Result<TranslateResult> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "text": text,
        "source_lang": from.to_uppercase(),
        "target_lang": to.to_uppercase(),
    });

    let resp = client.post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| PebbleError::Translate(format!("DeepLX request failed: {e}")))?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await
        .map_err(|e| PebbleError::Translate(format!("DeepLX response parse failed: {e}")))?;

    if !status.is_success() {
        return Err(PebbleError::Translate(format!("DeepLX error {status}: {json}")));
    }

    let translated = json.get("data")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();

    Ok(TranslateResult {
        segments: build_segments(text, &translated),
        translated,
    })
}

fn build_segments(source: &str, target: &str) -> Vec<BilingualSegment> {
    source.split('\n').zip(target.split('\n'))
        .filter(|(s, _)| !s.trim().is_empty())
        .map(|(s, t)| BilingualSegment { source: s.to_string(), target: t.to_string() })
        .collect()
}
```

Tests: `test_build_segments`

- [ ] **Step 4: Create deepl.rs**

```rust
pub async fn translate(api_key: &str, use_free_api: bool, text: &str, from: &str, to: &str) -> Result<TranslateResult> {
    let base = if use_free_api {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };

    let client = reqwest::Client::new();
    let resp = client.post(base)
        .header("Authorization", format!("DeepL-Auth-Key {api_key}"))
        .form(&[
            ("text", text),
            ("source_lang", &from.to_uppercase()),
            ("target_lang", &to.to_uppercase()),
        ])
        .send().await
        .map_err(|e| PebbleError::Translate(format!("DeepL request failed: {e}")))?;

    // Parse response { "translations": [{ "text": "..." }] }
    let json: serde_json::Value = resp.json().await
        .map_err(|e| PebbleError::Translate(format!("DeepL parse failed: {e}")))?;

    let translated = json.get("translations")
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    Ok(TranslateResult {
        segments: build_segments(text, &translated),
        translated,
    })
}
```

- [ ] **Step 5: Create generic.rs**

Generic API provider that uses configurable parameter names and result JSON path.

```rust
pub async fn translate(
    endpoint: &str, api_key: Option<&str>,
    source_lang_param: &str, target_lang_param: &str, text_param: &str,
    result_path: &str, text: &str, from: &str, to: &str,
) -> Result<TranslateResult> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        text_param: text,
        source_lang_param: from,
        target_lang_param: to,
    });

    let mut req = client.post(endpoint).json(&body);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let resp = req.send().await
        .map_err(|e| PebbleError::Translate(format!("Translate API request failed: {e}")))?;

    let json: serde_json::Value = resp.json().await
        .map_err(|e| PebbleError::Translate(format!("Translate API parse failed: {e}")))?;

    // Navigate JSON path like "data.translations.0.translatedText"
    let translated = resolve_json_path(&json, result_path)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(TranslateResult {
        segments: build_segments(text, &translated),
        translated,
    })
}

fn resolve_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for part in path.split('.') {
        if let Ok(index) = part.parse::<usize>() {
            current = current.get(index)?;
        } else {
            current = current.get(part)?;
        }
    }
    Some(current)
}
```

Tests: `test_resolve_json_path`

- [ ] **Step 6: Create llm.rs**

LLM/OpenAI-compatible provider with a built-in translation prompt.

```rust
pub async fn translate(
    endpoint: &str, api_key: &str, model: &str,
    mode: &LLMMode, text: &str, from: &str, to: &str,
) -> Result<TranslateResult> {
    let client = reqwest::Client::new();

    let system_prompt = format!(
        "You are a professional translator. Translate the following text from {from} to {to}. \
         Output ONLY the translation, nothing else. Preserve formatting and line breaks."
    );

    let url = match mode {
        LLMMode::Completions => format!("{}/v1/chat/completions", endpoint.trim_end_matches('/')),
        LLMMode::Responses => format!("{}/v1/responses", endpoint.trim_end_matches('/')),
    };

    let body = match mode {
        LLMMode::Completions => serde_json::json!({
            "model": model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": text }
            ],
            "temperature": 0.3,
        }),
        LLMMode::Responses => serde_json::json!({
            "model": model,
            "input": format!("{system_prompt}\n\n{text}"),
        }),
    };

    let resp = client.post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send().await
        .map_err(|e| PebbleError::Translate(format!("LLM request failed: {e}")))?;

    let json: serde_json::Value = resp.json().await
        .map_err(|e| PebbleError::Translate(format!("LLM parse failed: {e}")))?;

    let translated = match mode {
        LLMMode::Completions => json
            .get("choices").and_then(|c| c.get(0))
            .and_then(|c| c.get("message")).and_then(|m| m.get("content"))
            .and_then(|c| c.as_str()).unwrap_or("").to_string(),
        LLMMode::Responses => json
            .get("output").and_then(|o| o.get(0))
            .and_then(|o| o.get("content")).and_then(|c| c.get(0))
            .and_then(|c| c.get("text")).and_then(|t| t.as_str())
            .unwrap_or("").to_string(),
    };

    Ok(TranslateResult {
        segments: build_segments(text, &translated),
        translated,
    })
}
```

- [ ] **Step 7: Create lib.rs — TranslateService dispatcher**

```rust
pub mod types;
pub mod deeplx;
pub mod deepl;
pub mod generic;
pub mod llm;

use pebble_core::Result;
use types::{TranslateProviderConfig, TranslateResult};

pub struct TranslateService;

impl TranslateService {
    pub async fn translate(
        config: &TranslateProviderConfig,
        text: &str,
        from: &str,
        to: &str,
    ) -> Result<TranslateResult> {
        match config {
            TranslateProviderConfig::DeepLX { endpoint } => {
                deeplx::translate(endpoint, text, from, to).await
            }
            TranslateProviderConfig::DeepL { api_key, use_free_api } => {
                deepl::translate(api_key, *use_free_api, text, from, to).await
            }
            TranslateProviderConfig::GenericApi {
                endpoint, api_key, source_lang_param, target_lang_param,
                text_param, result_path, ..
            } => {
                generic::translate(
                    endpoint, api_key.as_deref(),
                    source_lang_param, target_lang_param, text_param,
                    result_path, text, from, to,
                ).await
            }
            TranslateProviderConfig::LLM { endpoint, api_key, model, mode } => {
                llm::translate(endpoint, api_key, model, mode, text, from, to).await
            }
        }
    }
}
```

- [ ] **Step 8: Run tests and commit**

Run: `cargo test -p pebble-translate`
Commit: `feat(translate): add pebble-translate crate with DeepLX, DeepL, generic API, and LLM providers`

---

### Task 3: Translate IPC commands + wiring

**Files:**
- Create: `src-tauri/src/commands/translate.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs` (register commands, add pebble-translate dep)
- Modify: `src-tauri/Cargo.toml` (add pebble-translate dep)

**Context:** Wire translate service to frontend via Tauri IPC. Commands: translate text, get/save config, test connection.

- [ ] **Step 1: Add pebble-translate dep to src-tauri/Cargo.toml**

```toml
pebble-translate = { path = "../crates/pebble-translate" }
```

- [ ] **Step 2: Create translate.rs commands**

```rust
use crate::state::AppState;
use pebble_core::{PebbleError, TranslateConfig, now_timestamp};
use pebble_translate::types::{TranslateProviderConfig, TranslateResult};
use pebble_translate::TranslateService;
use tauri::State;

#[tauri::command]
pub async fn translate_text(
    state: State<'_, AppState>,
    text: String,
    from_lang: String,
    to_lang: String,
) -> std::result::Result<TranslateResult, PebbleError> {
    let config = state.store.get_translate_config()?
        .ok_or_else(|| PebbleError::Translate("No translate engine configured".to_string()))?;

    if !config.is_enabled {
        return Err(PebbleError::Translate("Translation is disabled".to_string()));
    }

    let provider_config: TranslateProviderConfig = serde_json::from_str(&config.config)
        .map_err(|e| PebbleError::Translate(format!("Invalid config: {e}")))?;

    TranslateService::translate(&provider_config, &text, &from_lang, &to_lang).await
}

#[tauri::command]
pub async fn get_translate_config(
    state: State<'_, AppState>,
) -> std::result::Result<Option<TranslateConfig>, PebbleError> {
    state.store.get_translate_config()
}

#[tauri::command]
pub async fn save_translate_config(
    state: State<'_, AppState>,
    provider_type: String,
    config: String,
    is_enabled: bool,
) -> std::result::Result<(), PebbleError> {
    let now = now_timestamp();
    let tc = TranslateConfig {
        id: "active".to_string(),
        provider_type,
        config,
        is_enabled,
        created_at: now,
        updated_at: now,
    };
    state.store.save_translate_config(&tc)
}

#[tauri::command]
pub async fn test_translate_connection(
    config: String,
) -> std::result::Result<String, PebbleError> {
    let provider_config: TranslateProviderConfig = serde_json::from_str(&config)
        .map_err(|e| PebbleError::Translate(format!("Invalid config: {e}")))?;

    let result = TranslateService::translate(&provider_config, "Hello", "en", "zh").await?;
    Ok(result.translated)
}
```

- [ ] **Step 3: Register in mod.rs and lib.rs**

Add `pub mod translate;` to `src-tauri/src/commands/mod.rs`.

Add to `generate_handler!` in lib.rs:
```rust
commands::translate::translate_text,
commands::translate::get_translate_config,
commands::translate::save_translate_config,
commands::translate::test_translate_connection,
```

- [ ] **Step 4: Run clippy and commit**

Run: `cargo clippy --workspace -- -D warnings`
Commit: `feat(ipc): add translate commands for text translation and config management`

---

### Task 4: Dark theme CSS + theme switching logic

**Files:**
- Modify: `src/styles/index.css` (add dark theme variables)
- Modify: `src/stores/ui.store.ts` (persist theme, apply on change)
- Modify: `src/app/Layout.tsx` (apply theme effect)

**Context:** Add dark theme via CSS variables. The UIStore already has `theme: "light" | "dark" | "system"` and `setTheme`. We need to add the dark CSS variables and a `useEffect` to apply the data-theme attribute.

- [ ] **Step 1: Add dark theme CSS variables to index.css**

```css
[data-theme="dark"] {
  --color-sidebar-bg: #1e1e1e;
  --color-sidebar-hover: #2a2a2a;
  --color-sidebar-active: #333333;
  --color-titlebar-bg: #1e1e1e;
  --color-main-bg: #141414;
  --color-statusbar-bg: #1e1e1e;
  --color-border: #2e2e2e;
  --color-text-primary: #e0e0e0;
  --color-text-secondary: #888888;
  --color-accent: #d4714e;
  --color-bg: #1a1a1a;
  --color-bg-hover: rgba(255,255,255,0.06);
  --color-bg-secondary: rgba(255,255,255,0.08);
}
```

Also add fallback `--color-bg`, `--color-bg-hover`, `--color-bg-secondary` to the `:root` block (light theme defaults used by Phase 3 components):

```css
:root {
  /* ... existing vars ... */
  --color-bg: #ffffff;
  --color-bg-hover: rgba(0,0,0,0.05);
  --color-bg-secondary: rgba(0,0,0,0.06);
}
```

- [ ] **Step 2: Add theme application effect to Layout.tsx**

```tsx
import { useUIStore } from "../stores/ui.store";

// Inside Layout component:
const theme = useUIStore((s) => s.theme);

useEffect(() => {
  const root = document.documentElement;
  if (theme === "system") {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    root.setAttribute("data-theme", prefersDark ? "dark" : "light");
    const listener = (e: MediaQueryListEvent) => {
      root.setAttribute("data-theme", e.matches ? "dark" : "light");
    };
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    mql.addEventListener("change", listener);
    return () => mql.removeEventListener("change", listener);
  } else {
    root.setAttribute("data-theme", theme);
  }
}, [theme]);
```

- [ ] **Step 3: Run tests and commit**

Run: `npx tsc --noEmit && npx vitest run`
Commit: `feat(ui): add dark theme with CSS variables and system theme detection`

---

### Task 5: Settings tabbed UI — Accounts + Appearance

**Files:**
- Rewrite: `src/features/settings/SettingsView.tsx` (tabbed layout)
- Create: `src/features/settings/AccountsTab.tsx` (extracted from current SettingsView)
- Create: `src/features/settings/AppearanceTab.tsx` (theme picker)

**Context:** Settings currently only shows accounts. Redesign as a tabbed view with sidebar tabs: Accounts, Appearance, Rules, Translation, Shortcuts. Start with Accounts (extracted) and Appearance (theme picker).

- [ ] **Step 1: Create AccountsTab.tsx**

Extract the entire current `SettingsView` body into `AccountsTab.tsx`. Same props, same UI, just renamed.

- [ ] **Step 2: Create AppearanceTab.tsx**

Theme picker with three options (Light, Dark, System). Each rendered as a selectable card. Calls `useUIStore.getState().setTheme(theme)`.

```tsx
import { useUIStore } from "@/stores/ui.store";
import type { Theme } from "@/stores/ui.store";

const THEMES: { id: Theme; label: string; description: string }[] = [
  { id: "light", label: "Light", description: "Clean, bright interface" },
  { id: "dark", label: "Dark", description: "Easy on the eyes" },
  { id: "system", label: "System", description: "Follows your OS setting" },
];

export default function AppearanceTab() {
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);

  return (
    <div>
      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px" }}>Theme</h3>
      <div style={{ display: "flex", gap: "12px" }}>
        {THEMES.map((t) => (
          <button
            key={t.id}
            onClick={() => setTheme(t.id)}
            style={{
              flex: 1,
              padding: "16px",
              borderRadius: "8px",
              border: theme === t.id ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
              backgroundColor: theme === t.id ? "var(--color-bg-hover, rgba(0,0,0,0.03))" : "transparent",
              cursor: "pointer",
              textAlign: "left",
              color: "var(--color-text-primary)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "13px", marginBottom: "4px" }}>{t.label}</div>
            <div style={{ fontSize: "12px", color: "var(--color-text-secondary)" }}>{t.description}</div>
          </button>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Rewrite SettingsView.tsx as tabbed layout**

```tsx
import { useState } from "react";
import AccountsTab from "./AccountsTab";
import AppearanceTab from "./AppearanceTab";

const TABS = [
  { id: "accounts", label: "Accounts" },
  { id: "appearance", label: "Appearance" },
  { id: "rules", label: "Rules" },
  { id: "translation", label: "Translation" },
  { id: "shortcuts", label: "Shortcuts" },
];

export default function SettingsView() {
  const [activeTab, setActiveTab] = useState("accounts");

  return (
    <div style={{ display: "flex", height: "100%" }}>
      {/* Tab sidebar */}
      <div style={{
        width: "180px",
        borderRight: "1px solid var(--color-border)",
        padding: "16px 0",
        flexShrink: 0,
      }}>
        {TABS.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            style={{
              display: "block",
              width: "100%",
              textAlign: "left",
              padding: "8px 20px",
              border: "none",
              background: activeTab === tab.id ? "var(--color-bg-hover, rgba(0,0,0,0.05))" : "none",
              color: activeTab === tab.id ? "var(--color-text-primary)" : "var(--color-text-secondary)",
              fontWeight: activeTab === tab.id ? 600 : 400,
              fontSize: "13px",
              cursor: "pointer",
              borderRight: activeTab === tab.id ? "2px solid var(--color-accent)" : "2px solid transparent",
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>
      {/* Tab content */}
      <div style={{ flex: 1, padding: "32px", maxWidth: "640px", overflow: "auto" }}>
        {activeTab === "accounts" && <AccountsTab />}
        {activeTab === "appearance" && <AppearanceTab />}
        {activeTab === "rules" && <div style={{ color: "var(--color-text-secondary)" }}>Rules management coming soon</div>}
        {activeTab === "translation" && <div style={{ color: "var(--color-text-secondary)" }}>Translation settings coming soon</div>}
        {activeTab === "shortcuts" && <div style={{ color: "var(--color-text-secondary)" }}>Shortcut customization coming soon</div>}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run tests and commit**

Commit: `feat(settings): redesign settings as tabbed view with accounts and appearance tabs`

---

### Task 6: Settings — Rules management tab

**Files:**
- Create: `src/features/settings/RulesTab.tsx`
- Modify: `src/features/settings/SettingsView.tsx` (replace placeholder)
- Modify: `src/lib/api.ts` (ensure rule types/APIs exist — already added in Phase 3)

**Context:** Rules CRUD already exists in the backend and api.ts. This task builds the frontend UI for managing rules: list, create, edit, delete.

- [ ] **Step 1: Create RulesTab.tsx**

Shows a list of existing rules (name, priority, enabled toggle). An "Add Rule" button opens an inline form. Each rule has edit/delete buttons.

The rule editor form has:
- Name (text input)
- Priority (number input)
- Enabled toggle
- Conditions JSON (textarea — MVP, raw JSON editing. Visual builder deferred)
- Actions JSON (textarea — same)
- Save / Cancel buttons

```tsx
import { useState, useEffect } from "react";
import { listRules, createRule, updateRule, deleteRule } from "@/lib/api";
import type { Rule } from "@/lib/api";
import { Plus, Trash2, Pencil } from "lucide-react";
// ... full component with list + inline editor
```

- [ ] **Step 2: Wire into SettingsView**

Replace the rules placeholder with `<RulesTab />`.

- [ ] **Step 3: Run tests and commit**

Commit: `feat(settings): add rules management tab with list and editor`

---

### Task 7: Settings — Translation config tab

**Files:**
- Create: `src/features/settings/TranslateTab.tsx`
- Modify: `src/features/settings/SettingsView.tsx` (replace placeholder)
- Modify: `src/lib/api.ts` (add translate API wrappers)

**Context:** The translate config tab lets users configure their translation engine. They select a provider type and fill in provider-specific settings. A "Test Connection" button validates the config.

- [ ] **Step 1: Add translate API wrappers to api.ts**

```typescript
export interface TranslateConfig {
  id: string;
  provider_type: string;
  config: string;
  is_enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface TranslateResult {
  translated: string;
  segments: { source: string; target: string }[];
}

export async function translateText(text: string, fromLang: string, toLang: string): Promise<TranslateResult> {
  return invoke<TranslateResult>("translate_text", { text, from_lang: fromLang, to_lang: toLang });
}

export async function getTranslateConfig(): Promise<TranslateConfig | null> {
  return invoke<TranslateConfig | null>("get_translate_config");
}

export async function saveTranslateConfig(providerType: string, config: string, isEnabled: boolean): Promise<void> {
  return invoke<void>("save_translate_config", { provider_type: providerType, config, is_enabled: isEnabled });
}

export async function testTranslateConnection(config: string): Promise<string> {
  return invoke<string>("test_translate_connection", { config });
}
```

- [ ] **Step 2: Create TranslateTab.tsx**

Provider selector dropdown (DeepLX / DeepL / Generic API / LLM). Dynamic form fields per provider type:

- **DeepLX**: Endpoint URL
- **DeepL**: API Key, Free API toggle
- **Generic API**: Endpoint, API Key (optional), source/target/text param names, result JSON path
- **LLM**: Endpoint, API Key, Model name, Mode (Completions/Responses)

Plus:
- Enable/Disable toggle
- "Test Connection" button (calls `testTranslateConnection`, shows result or error)
- Save button

- [ ] **Step 3: Wire into SettingsView**

Replace the translation placeholder with `<TranslateTab />`.

- [ ] **Step 4: Run tests and commit**

Commit: `feat(settings): add translation engine configuration tab`

---

### Task 8: Settings — Shortcuts display tab

**Files:**
- Create: `src/features/settings/ShortcutsTab.tsx`
- Modify: `src/features/settings/SettingsView.tsx` (replace placeholder)

**Context:** Display current keyboard shortcuts in a readable table. MVP shows the defaults as read-only. Full customization deferred to later.

- [ ] **Step 1: Create ShortcutsTab.tsx**

Render a categorized table of all keyboard shortcuts:

```tsx
const SHORTCUT_GROUPS = [
  {
    category: "General",
    shortcuts: [
      { keys: "Ctrl+K", action: "Open command palette" },
      { keys: "Escape", action: "Close modal / popover" },
    ],
  },
  {
    category: "Navigation",
    shortcuts: [
      { keys: "Ctrl+Shift+I", action: "Go to Inbox" },
      { keys: "Ctrl+Shift+K", action: "Go to Kanban" },
      { keys: "J", action: "Next message" },
      { keys: "K", action: "Previous message" },
    ],
  },
  {
    category: "Mail Actions",
    shortcuts: [
      { keys: "S", action: "Toggle star" },
      { keys: "H", action: "Snooze message" },
    ],
  },
];
```

Each group renders as a section header + rows with key badges on left and description on right.

- [ ] **Step 2: Wire into SettingsView**

Replace shortcuts placeholder with `<ShortcutsTab />`.

- [ ] **Step 3: Commit**

Commit: `feat(settings): add keyboard shortcuts reference tab`

---

### Task 9: Translate frontend — Selection popover + bilingual view

**Files:**
- Create: `src/features/translate/TranslatePopover.tsx`
- Create: `src/features/translate/BilingualView.tsx`
- Modify: `src/components/MessageDetail.tsx` (add translate button + selection handler)

**Context:** Selection translate: user selects text in message detail, a small popover appears with translated text. Bilingual view: a toggle button in the message header switches to side-by-side original + translated view.

- [ ] **Step 1: Create TranslatePopover.tsx**

A floating popover that appears near the text selection with:
- Auto-detected source language (default "auto")
- Target language selector (default "zh" for Chinese user base)
- Translated text display
- Loading state
- Copy button for translated text

```tsx
interface Props {
  text: string;
  position: { x: number; y: number };
  onClose: () => void;
}
```

Calls `translateText(text, "auto", targetLang)` on mount.

- [ ] **Step 2: Create BilingualView.tsx**

Side-by-side display of bilingual segments.

```tsx
interface Props {
  segments: { source: string; target: string }[];
}
```

Renders two columns: left = source segments, right = target segments, aligned row-by-row.

- [ ] **Step 3: Add translate to MessageDetail**

Add a translate icon button (Languages from lucide-react) in the message header, next to the snooze button.

Add a `mouseup` event handler on the message body that detects text selection:
- If selection exists and > 5 chars, show TranslatePopover at cursor position
- Click elsewhere to dismiss

Add a bilingual mode toggle:
- When clicked, translate full `body_text`, show BilingualView instead of normal body

- [ ] **Step 4: Run tests and commit**

Commit: `feat(ui): add translate popover on text selection and bilingual view`

---

### Task 10: Integration tests + full verification

**Files:**
- Create: `tests/stores/settings.test.ts` (if settings store created)
- Modify: existing test files if needed

- [ ] **Step 1: Run full backend test suite**

```bash
cargo test -p pebble-core -p pebble-store -p pebble-search -p pebble-privacy -p pebble-mail -p pebble-rules -p pebble-translate
cargo clippy --workspace -- -D warnings
```

- [ ] **Step 2: Run full frontend test suite**

```bash
npx tsc --noEmit
npx vitest run
```

- [ ] **Step 3: Commit final test additions**

```bash
git commit -m "test: add Phase 4 integration tests"
```

---

## Summary

| Task | Description | Complexity |
|------|-------------|------------|
| 1 | Database + Store for translate config | Low |
| 2 | pebble-translate crate (4 providers) | High |
| 3 | Translate IPC commands | Low |
| 4 | Dark theme CSS + switching | Low |
| 5 | Settings tabbed UI + Accounts + Appearance | Medium |
| 6 | Settings — Rules management tab | Medium |
| 7 | Settings — Translation config tab | Medium |
| 8 | Settings — Shortcuts display tab | Low |
| 9 | Translate frontend (popover + bilingual) | High |
| 10 | Integration tests + verification | Low |

**Total:** 10 tasks, ~45 steps
