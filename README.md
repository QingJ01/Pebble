<p align="center">
  <img src="site/screenshots/main.png" alt="Pebble" width="800">
</p>

<h1 align="center">Pebble</h1>

<p align="center">
  A privacy-first desktop email client built with Rust and React.
  <br>
  Your data stays on your device — no telemetry, no compromise. Optional WebDAV backup for settings.
</p>

<p align="center">
  <a href="https://github.com/QingJ01/Pebble/releases"><img src="https://img.shields.io/github/v/release/QingJ01/Pebble?style=flat-square&color=d4714e" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square" alt="License"></a>
  <a href="https://github.com/QingJ01/Pebble/actions"><img src="https://img.shields.io/github/actions/workflow/status/QingJ01/Pebble/ci.yml?style=flat-square&label=build" alt="Build"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="Platform">
</p>

---

## Features

**Privacy & Security**
- 100% local storage — SQLite database, search index, attachments all on your device
- AES-256 encrypted OAuth tokens and credentials with per-device key
- No telemetry, no third-party data access; optional user-initiated settings backup via WebDAV
- Open source under AGPL-3.0

**Email Management**
- Gmail, Outlook, and IMAP support — all accounts in one place
- Kanban board — drag emails across Todo, Waiting, and Done columns
- Smart search powered by Tantivy full-text search engine
- Snooze & Star — resurface emails later, mark what matters
- Rules engine — auto-label, move, or flag with custom conditions

**User Experience**
- Dark & Light themes with system-aware auto-switching
- Keyboard-first navigation — compose, navigate, and manage from the keyboard
- Built-in translation with DeepL / LLM bilingual view
- i18n support — English and Chinese built-in

## Screenshots

<table>
  <tr>
    <td><img src="site/screenshots/inbox.png" alt="Inbox"><br><b>Inbox</b> — Three-panel layout</td>
    <td><img src="site/screenshots/kanban.png" alt="Kanban"><br><b>Kanban</b> — Drag-and-drop email board</td>
  </tr>
  <tr>
    <td><img src="site/screenshots/dark.png" alt="Dark Mode"><br><b>Dark Mode</b> — Beautiful dark theme</td>
    <td><img src="site/screenshots/settings.png" alt="Settings"><br><b>Settings</b> — Privacy & appearance</td>
  </tr>
</table>

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust |
| Desktop Framework | Tauri v2 |
| Frontend | React 19 + TypeScript |
| State Management | Zustand + TanStack Query |
| Database | SQLite (rusqlite) |
| Search | Tantivy |
| Styling | Tailwind CSS |
| i18n | i18next |

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable)
- [Node.js](https://nodejs.org/) (v18+)
- [pnpm](https://pnpm.io/) (v8+)

### Setup

```bash
# Clone the repository
git clone https://github.com/QingJ01/Pebble.git
cd Pebble

# Install frontend dependencies
pnpm install

# Set up environment variables
cp .env.example .env
# Edit .env with your OAuth credentials (see below)

# Run in development mode
pnpm dev
```

### OAuth Configuration

To use Gmail or Outlook, you need OAuth credentials:

| Variable | Description |
|----------|-------------|
| `GOOGLE_CLIENT_ID` | Google OAuth 2.0 Client ID (Desktop app type, no secret needed — uses PKCE) |
| `MICROSOFT_CLIENT_ID` | Microsoft Azure App Client ID |
| `MICROSOFT_CLIENT_SECRET` | Microsoft Azure App Client Secret |

See `.env.example` for the full template.

### Build

```bash
# Build the desktop app
pnpm build
```

Build artifacts will be in `src-tauri/target/release/bundle/`.

## Project Structure

```
Pebble/
├── src/                    # React frontend
│   ├── components/         # Shared UI components
│   ├── features/           # Feature modules (inbox, kanban, settings...)
│   ├── hooks/              # React hooks & mutations
│   ├── stores/             # Zustand state stores
│   └── lib/                # Utilities, API, i18n
├── src-tauri/              # Tauri backend
│   └── src/commands/       # Tauri IPC commands
├── crates/                 # Rust workspace crates
│   ├── pebble-core/        # Shared types & errors
│   ├── pebble-store/       # SQLite persistence
│   ├── pebble-mail/        # Email sync (Gmail, IMAP)
│   ├── pebble-search/      # Tantivy search index
│   ├── pebble-crypto/      # AES-256 encryption
│   ├── pebble-oauth/       # OAuth 2.0 + PKCE flow
│   ├── pebble-rules/       # Rules engine
│   ├── pebble-translate/   # Translation providers
│   └── pebble-privacy/     # Privacy utilities
└── site/                   # Landing page (static HTML)
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `J` / `K` | Navigate up / down |
| `Enter` | Open email |
| `E` | Archive |
| `S` | Toggle star |
| `R` | Reply |
| `A` | Reply all |
| `F` | Forward |
| `C` | Compose new email |
| `/` | Focus search |
| `Esc` | Close / go back |

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the [GNU Affero General Public License v3.0](LICENSE).

---

<p align="center">
  Built with care by <a href="https://github.com/QingJ01">QingJ</a>
</p>
