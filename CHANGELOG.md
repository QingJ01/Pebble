# Changelog

All notable changes to Pebble will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project uses semantic version tags.

## [Unreleased]

### Fixed

- Fixed IMAP realtime polling fallback so background sync does not miss new mail after empty Inbox baselines, UIDVALIDITY resets, or same-count mailbox changes.
- Refreshed folder unread counts immediately after read-state changes, message moves, batch actions, and sync completion events.
- Made unread message and thread rows more visible in dark mode.

## [0.0.5] - 2026-05-04

### Added

- Added `mailto:` deep-link support so email links can open Pebble compose with parsed recipients, subject, and body.
- Linkified plain-text URLs and email addresses in rendered message bodies.
- Added Linux AppImage packaging, Ubuntu CI package builds, tagged-release AppImage uploads, and Linux native credential storage support.

### Changed

- Improved desktop notification setup and status reporting, including Windows toast environment handling and development loading behavior.
- Refined the sidebar account selector width, alignment, and spacing.

### Fixed

- Fixed sending compose messages while contact recipient selection is still pending.
- Fixed opening links from rendered email bodies, including browser links and email-address links.
- Kept the new-notification red dot on the tray icon only.
- Clarified invalid WebDAV backup errors when a server returns an empty or HTML response instead of a Pebble backup file.
- Fixed CI issues that blocked Linux AppImage artifact generation.

## [0.0.4] - 2026-05-01

### Added

- Added global mail proxy settings for account connectivity.
- Added OAuth account proxy controls so Google and Microsoft account flows can use account-specific proxy settings.
- Added account color presets and automatic default colors for newly added accounts.
- Added account color markers in the all-accounts message list when multiple accounts are visible.
- Added first-launch language detection: Chinese system locales start in Chinese, and other locales start in English.

### Changed

- Reorganized proxy settings into clearer global and per-account sections.
- Refined the compose editor layout with a single compact toolbar, a full-height editor surface, and consistent rich text, Markdown, and HTML mode controls.
- Replies now open with a clean editable reply area while the original message is shown as a collapsed read-only quote; the quote is still appended when the reply is sent.
- Unified sidebar system folder ordering across all-accounts and single-account views so folders no longer jump when switching accounts.

### Fixed

- Persisted automatically assigned default account colors.
- Preserved existing account colors when restoring older WebDAV backups that do not contain color metadata.
- Fixed OAuth account editing so disabled/custom proxy mode is preserved correctly.
- Prevented account proxy settings from temporarily losing account metadata while settings are loading.
- Hid account color markers when a single account is selected or only one account exists.

### Documentation

- Documented the macOS quarantine workaround command `sudo xattr -cr` for users who need to run unsigned builds.

## [0.0.3] - 2026-04-30

### Added

- Added unsigned macOS app and DMG build scripts, current-platform desktop build routing, macOS CI packaging, and tagged release DMG artifact uploads.
- Added the macOS `.icns` bundle icon required by Tauri's macOS application bundle.

### Changed

- WebDAV restore now replaces local rules and Kanban cards/notes while merging account metadata from the backup, and restore previews disclose Kanban note counts.

### Fixed

- Enabled the native macOS Keychain backend for local credential encryption.
- Made search over subject, sender, and recipient short fields case-insensitive for Latin text, and trigger a search index rebuild for older case-sensitive indexes.
- Indexed locally saved sent and queued outgoing messages so they appear in search results.
- Moved compose drafts, templates, and signatures out of frontend `localStorage` and into encrypted backend secure storage.
- Protected in-progress compose content from being overwritten when account, signature, or language-dependent defaults change.
- Added retry scheduling, exponential backoff, and a maximum attempt limit for pending mail operations.
- Aligned offline batch mail operations with single-message optimistic local commit behavior.
- Staged compose attachments through the backend so valid selected files no longer depend on fragile frontend path handling.
- Moved Kanban context notes out of frontend `localStorage` and into encrypted backend secure storage, with one-time legacy note migration.
- Hardened HTML email CSS sanitization against escaped `url()` tokens that could trigger remote loads in strict privacy mode.
- Prevented duplicate same-account sync workers by keeping the startup placeholder lock alive until the real worker replaces it.
- Report realtime restart failures back to the UI instead of silently accepting preference changes after all or part of sync restart failed.

## [0.0.2] - 2026-04-29

### Added

- Added tray and background-running controls so Pebble can close to the system tray, restore from the tray menu, and keep the close-to-background preference in app state.
- Added localized tray menu labels and status bar copy for background sync behavior.
- Added public privacy policy and terms of service pages for Google OAuth app verification.
- Added English and Chinese language switching for the privacy policy and terms pages.
- Added Cloudflare Workers site deployment configuration for the public site.
- Added the LINUX DO friend link to the English and Chinese README files.

### Changed

- Themed native form controls and focus-visible styling so inputs, selects, textareas, and buttons fit the dark UI.

### Fixed

- Improved attachment download reliability by saving duplicate target filenames with a unique suffix instead of failing.
- Staged local draft, outbox, and sent-message attachments into Pebble's app data directory so downloads no longer depend on the original selected file path.
- Persisted IMAP attachments before notifying the frontend about newly synced messages.
- Refined Gmail attachment parsing so large body parts are not shown as attachments and inline content-ID images stay out of the download list.
- Added clearer attachment download failure messages and backend download logging.
- Fixed the Cloudflare Worker site target and migrated the site config to the JSONC Workers format.

## [0.0.1] - 2026-04-27

### Initial Release

Pebble 0.0.1 is the first public test release.

This release includes:

- Gmail, IMAP, and experimental Outlook account support.
- Aggregated mailbox views across connected accounts.
- Local mail storage, search indexing, attachments, rules, trusted senders, and application settings.
- Message reading, compose, drafts, sent mail persistence, local outbox fallback, and pending remote write retries.
- Realtime and near-realtime sync infrastructure for Gmail, IMAP, and Outlook.
- Inbox, search, starred, snoozed, kanban, settings, diagnostics, and pending remote writes views.
- Privacy controls for remote images, trusted senders, tracker blocking, sanitized HTML rendering, and safer attachment filenames.
- Desktop notifications with click navigation.
- Custom title bar with consistent app logo rendering on Windows.
- OAuth client secrets are included in release builds when configured.
- English and Chinese README documentation.
- GitHub Actions CI and tag-driven Windows NSIS installer packaging with SHA256 checksum files.

### Notes

- Windows installers are not code-signed yet, so Windows SmartScreen may show a warning.
- Outlook support is still experimental and depends on Microsoft Graph permissions configured by the user.

[Unreleased]: https://github.com/QingJ01/Pebble/compare/v0.0.5...HEAD
[0.0.5]: https://github.com/QingJ01/Pebble/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/QingJ01/Pebble/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/QingJ01/Pebble/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/QingJ01/Pebble/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/QingJ01/Pebble/releases/tag/v0.0.1
