# Changelog

All notable changes to Pebble will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project uses semantic version tags.

## [Unreleased]

## [0.1.6] - 2026-11-05

### Added

- Added per-account sync ranges (all mail, 30/90/365 days, or a custom number of days) for IMAP, Gmail and Outlook, selectable before the first sync and editable later. Widening the range backfills earlier mail; narrowing it keeps existing local mail (#99, #100).
- Added a grouped sidebar style with a switch back to the classic layout in Appearance settings; the grouped layout keeps every folder reachable (#97).
- Added collapsible recipient chips for messages with many participants, plus file-type icons and card styling for attachments (#97).

### Changed

- Strengthened typography in the message list and message detail, and restyled the thread sender card (#97).

### Fixed

- Fixed unreadable default links in dark-mode email bodies on macOS WebKit and other webviews (#76, #98).
- Fixed email body styles (including the dark-mode light canvas and authored inline styles) being blocked in packaged builds, where Tauri's injected `style-src` nonce disabled `'unsafe-inline'`.
- Fixed archiving with IMAP when the server Archive folder was selected after the local fallback Archive had been created: archive now targets the server folder, the empty fallback is removed, and flag changes never target local-only folders.
- Fixed replies not joining their original conversation because parsed `In-Reply-To`/`References` IDs lacked the angle brackets used by stored Message-IDs. Previously mis-threaded messages are not regrouped.
- Fixed closing the window during startup quitting the app instead of hiding to the tray; an explicit "quit on close" preference is still respected (#97).
- Pinned the macOS traffic-light buttons to the top-left titlebar corner (#97).
- Hardened sync recovery for range changes: IMAP UIDVALIDITY resets rebuild the mailbox without discarding cached mail outside the range, Gmail replays pending history before switching ranges, and Outlook avoids redundant per-page cache copies (#100).

### Upgrade notes

- POP3 cannot filter by date and always syncs all mail. Sync range cutoffs start at midnight UTC.
- Automated tests, package builds and native macOS checks against a local IMAP server cover this release. Live Gmail/Outlook range syncing, POP3 downloads and native Windows/Linux UI have not been exercised.

## [0.1.5] - 2026-09-09

### Added

- Added independent local account labels, visible alongside mailbox addresses, with v3 backup support and imports from v1/v2 backups (#79).
- Added OAuth mailbox verification and an in-place repair preview that preserves existing messages, user names, labels and account IDs (#78, #79).
- Added a one-time sender-name notice with a preview and a shortcut to account settings (#77).
- Added a local address book with searchable contacts, multiple labeled email addresses, favorites, notes, and quick create/edit actions from message participants (#81).
- Added recipient suggestions that prioritize saved contacts while retaining recent correspondents, with controls to suppress unwanted recent addresses (#81).
- Added standards-compatible vCard import and export with duplicate merging, partial-error reporting, UTF-8/folded-line support, and safe file/card limits (#81).
- Added contacts to local file and WebDAV settings backups, with backward-compatible imports (#81).

### Fixed

- Fixed partial Outlook sync updates clearing message subjects, bodies, metadata, and attachments after read or flag changes (#90).
- Fixed SMTP and Gmail dropping configured sender names. Structured From headers now support Unicode and punctuation, and outgoing retries preserve the stored sender identity (#77).
- Outlook now clearly shows its provider-managed sender name; changing a local label does not change the external identity. New OAuth accounts bind to a verified provider identity, and OAuth addresses cannot be changed through ordinary account editing (#77, #79).
- Settings restore preserves existing OAuth mailbox addresses and connected credentials instead of silently replacing them with another mailbox from a backup.
- Fixed copied-text feedback leaving a timer running after its popover was closed.
- Fixed vCard export/import losing whitespace at folded boundaries and treating carriage returns inside notes as new contact properties.
- Fixed participant actions associating a newly saved contact with an email address removed in the editor; recent-suggestion removal is now reachable by keyboard.
- Blocked queued SMTP sends after the account mailbox address changes, while retaining frozen names when only a name changes.
- OAuth remote drafts now use the same verified identity and connection as sending. Failed identity checks preserve encrypted local drafts without uploading them.
- Corrected backup restore feedback to explain that existing OAuth connections and mailboxes with local data are preserved.

### Upgrade notes

- Review existing sender names before sending: the old “Display name” setting is now included in SMTP/Gmail outgoing mail. Use the separate “Account label” for internal descriptions. Previously delivered messages are unchanged.
- Queued messages retain their original identity. Messages with missing or incompatible identities stop automatic retries and remain visible for review; unknown send outcomes are never automatically resent.
- Outlook sender names remain controlled by the mailbox service. This change does not enable aliases or delegated sending, and recipient address books may affect how names appear.
- New backups use schema v3. Upgrade other clients before importing them. Existing OAuth connections are preserved during restore; a new account can still restore its saved credentials. Settings backups do not include message bodies or attachments.
- Automated tests and package builds cover client behavior. Live SMTP/Gmail/Outlook recipient-side delivery has not been exercised for this release; provider rewriting and recipient address-book display remain outside the client guarantees. Verify your sender name with a test message before formal use.

## [0.1.4] - 2026-08-08

### Added

- Added custom and portable profile directories, including environment-variable, pointer-file, and executable-adjacent portable modes with profile-scoped frontend storage.
- Added CC recipient display and selectable IMAP sync folders; re-enabling a folder now backfills its complete message history (#74, #82).
- Added automatic compose subjects derived from the first attachment when the subject is otherwise empty.

### Changed

- Updated the desktop release metadata to version 0.1.4.
- Stored data-encryption keys in a keychain-safe hexadecimal format, with automatic migration and zeroization for legacy binary secrets (#22).
- Improved Linux AppImage startup by preferring Wayland when available while retaining X11 fallback, respecting explicit user configuration, and disabling binary stripping for release bundles.

### Fixed

- Fixed Outlook account setup so mailbox identity is resolved from Microsoft Graph without trusting incomplete local profile data.
- Hardened Gmail, Outlook, IMAP, and POP3 synchronization against UIDVALIDITY changes, stale cursors, pagination loops, attachment failures, duplicate provider IDs, and interrupted indexing or database commits.
- Made outgoing mail crash-safe by persisting the message, attachments, and send state before dispatch; uncertain outcomes are never retried automatically, API-provider placeholders are removed after confirmation, and SMTP copies move atomically to Sent.
- Made attachment downloads, draft replacement, trash deletion, and pending remote operations transactional and idempotent, including recovery after restarts and already-absent remote messages.
- Bound encrypted records to their storage identity with authenticated encryption, migrated legacy ciphertext safely, and prevented proxy, tracker, signature, template, and cloud-setting state from leaking across records or racing with user updates.
- Fixed compose attachment staging, autosave retries, Strict Mode lifecycle handling, rapid realtime preference changes, and several confirmation, selection, accessibility, and starred-message UI races.
- Hardened tagged release automation against unsafe tag interpolation, incomplete artifact sets, concurrent publication, and accidental overwrites of existing GitHub Releases.

## [0.1.3] - 2026-07-01

### Added

- Added an option to launch Pebble automatically when you sign in to your computer, under General settings → Startup Behavior (#64).
- Added a per-account option to allow unencrypted (plaintext) IMAP/SMTP connections for legacy servers that only support plaintext, shown as an explicit opt-in with a security warning when a connection's security is set to "None" (#70).

### Changed

- Updated the desktop release metadata to version 0.1.3.

### Fixed

- Fixed some email links being shown with a literal `&amp;` (for example in password-reset URLs) by no longer double-escaping ampersands when auto-linking plain-text URLs (#68).
- Fixed the account connection test reporting a false "passed" for accounts with a blank username: it now performs a real authenticated login, so authentication failures surface during testing instead of only after the account is added (#60).
- Fixed the startup window background color and title-bar theme so the window matches the active theme immediately on launch.

## [0.1.2] - 2026-06-15

### Changed

- Updated the desktop release metadata to version 0.1.2.

### Fixed

- Fixed DeepL and DeepLX translation always failing with a `400 "source_lang not supported"` error by omitting the source language field when auto-detection is requested (#65).
- Fixed adding Outlook/Hotmail accounts: the username field is no longer required, and a blank username now defaults to the account email so app-password logins connect (#60).
- Prevented duplicate messages from appearing in the destination mailbox when a move operation is retried after a partial COPY/EXPUNGE failure.
- Prevented duplicate message rows from accumulating after a soft-delete followed by a re-sync, via a unique index on live `(account_id, remote_id)` rows.
- Prevented newly-arrived messages from being permanently missing from search results due to a race between the startup background reindex and concurrent sync.
- Preserved attachment filenames that were being altered on download and forwarding.
- Registered Pebble as the Windows mail client in the registry before opening the default-apps settings so "set as default" works reliably.
- Improved POP3 connectivity for legacy servers with a rustls→native-tls TLS backend fallback and optional TLS 1.0.
- Synced the native window background color with the active theme on macOS.

## [0.1.1] - 2026-06-03

### Added

- Added a tray preference to start Pebble hidden in the system tray at launch.
- Added native macOS traffic-light window controls in the title bar for a platform-idiomatic close/minimize/maximize experience.
- Added translation keyboard shortcuts, automatic settings backup, a default mail client preference, and a lightweight mode for the tray.
- Added local settings backup file export/import for cross-platform migration without WebDAV.
- Added optional encrypted WebDAV backup and restore for account passwords, OAuth tokens, and translation API keys.

### Changed

- Updated the desktop release metadata to version 0.1.1.

### Fixed

- Honored IMAP special-use folder attributes so the Trash, Sent, Drafts, Junk, and Archive flags reported by servers map to the expected local folders.
- Preserved email HTML container styles (such as `html`, `body`, and wrapper element rules) so sanitized messages keep their original layout.
- Stripped HTML and CSS from email preview snippets so list rows show clean plain-text previews instead of markup.
- Stopped retrying permanent authentication errors for pending mail operations, surfaced clearer status, and allowed dismissing failed operations from settings.
- Added missing English and Chinese translations for the new automatic backup, default mail client, and pending operations controls.
- Surfaced translation keyboard shortcuts in the settings page and fixed the wallpaper mode settings background rendering.

## [0.1.0] - 2026-05-28

### Added

- Added POP3 account support, including account setup, sync, provider dispatch, folder/message operations, and backup/restore metadata handling.
- Added an account-level option to allow invalid TLS certificates for IMAP and SMTP connections, including connection testing and settings UI support.

### Changed

- Updated the desktop release metadata to version 0.1.0.

### Fixed

- Recovered from invalid stored data-encryption keys by replacing malformed keyring entries instead of failing startup.
- Avoided repeated IMAP incremental fetches by checking UIDNEXT and searching for UIDs newer than the local cursor before fetching message bodies.
- Fixed TLS certificate policy localization in English and Chinese.
- Disabled WebKitGTK accelerated compositing for Linux AppImage launches when the user has not set `WEBKIT_DISABLE_COMPOSITING_MODE`, working around Wayland/Hyprland repaint stalls where the UI only updates after resizing.

## [0.0.9] - 2026-05-23

### Added

- Added Pebble Web self-hosted edition information to the public site and README documentation.
- Added Arch Linux AUR installation instructions.

### Changed

- Expanded release automation to build and upload Windows, macOS, and Linux packages from tagged releases.
- Improved Linux package build handling and form focus-visible styling.

### Fixed

- Added `native-tls` fallback for IMAP TLS and STARTTLS connections when servers only offer DHE cipher suites unsupported by `rustls`, including `imap.sina.cn`.
- Improved email CSS rendering by preserving embedded styles in relaxed mode and allowing remote stylesheets in unrestricted or trusted-sender modes.
- Removed an overly strict home-directory check that could block attachment downloads.
- Hid the compose floating action button while reading message details.
- Fixed sidebar view switching so repeated navigation cannot get stuck on stale UI state.

## [0.0.8] - 2026-05-16

### Added

- Added configurable custom background images with fit, contain, tile, and opacity controls.

### Fixed

- Displayed recipients in sent-mail message lists instead of showing the sender as the current account.
- Fixed Gmail history sync handling and modal backdrop behavior.
- Routed translation requests through the configured global proxy.

## [0.0.7] - 2026-05-09

### Added

- Added image paste support in the compose editor so clipboard images can be staged and sent as attachments.

### Fixed

- Updated Outlook local remote IDs after Graph API folder moves, including batch archive and delete paths, so follow-up actions no longer use stale message IDs.
- Enabled TLS 1.2 support for IMAP and SMTP connections to improve compatibility with older mail servers.
- Treated rustls unexpected EOF and missing TLS `close_notify` disconnects during IMAP polling as retryable connection interruptions.
- Improved IMAP compatibility for Tencent enterprise mail and other Coremail-based providers that require IMAP ID before login.
- Displayed the actual translate settings save and test error instead of a generic object string.
- Prevented modal dialogs from closing when text selection drags finish outside the window.

## [0.0.6] - 2026-05-05

### Added

- Added the receiving account to the message detail header so all-account views show which mailbox received the selected email.

### Changed

- Polished the compose leave confirmation flow.
- Simplified unread row indicators while keeping unread messages visually distinct.

### Fixed

- Fixed notification clicks so they route directly to the target message.
- Improved mail sync reliability, including IMAP realtime polling fallback handling for empty Inbox baselines, UIDVALIDITY resets, same-count mailbox changes, sync failures, and local UID baselines.
- Refreshed folder unread counts immediately after read-state changes, message moves, batch actions, secondary message actions, command read changes, and sync completion events.
- Made unread message and thread rows more visible in dark mode.
- Preserved sanitized HTML email layouts and stabilized Shadow DOM rendering so full-height wrappers, gray canvases, and delayed layout jumps do not obscure message content.
- Preserved hidden email preheader clipping styles so preview text remains hidden instead of rendering as one character per line.
- Honored fully trusted senders in privacy rendering so trusted senders can load images and tracker resources according to the selected trust level.
- Stabilized the sidebar bottom navigation so Snoozed, Kanban, and Settings remain clickable when wide message content is visible.

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

[Unreleased]: https://github.com/QingJ01/Pebble/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/QingJ01/Pebble/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/QingJ01/Pebble/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/QingJ01/Pebble/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/QingJ01/Pebble/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/QingJ01/Pebble/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/QingJ01/Pebble/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/QingJ01/Pebble/compare/v0.0.9...v0.1.0
[0.0.9]: https://github.com/QingJ01/Pebble/compare/v0.0.8...v0.0.9
[0.0.8]: https://github.com/QingJ01/Pebble/compare/v0.0.7...v0.0.8
[0.0.7]: https://github.com/QingJ01/Pebble/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/QingJ01/Pebble/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/QingJ01/Pebble/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/QingJ01/Pebble/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/QingJ01/Pebble/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/QingJ01/Pebble/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/QingJ01/Pebble/releases/tag/v0.0.1
