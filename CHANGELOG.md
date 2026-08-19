# Changelog

All notable changes to Talaria are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project has not
cut a release yet, so everything to date sits under Unreleased.

## [Unreleased]

### Fixed

- **The vault can no longer hang startup on a machine with no session D-Bus.**
  `Vault::load()` runs on the main thread and asked the OS keychain for its key
  through `keyring`, which on Linux talks to the Secret Service over D-Bus. With
  no `DBUS_SESSION_BUS_ADDRESS`, libdbus does not fail — it tries to *start* a
  session bus and blocks forever when it cannot, which froze the event loop while
  the control socket kept answering `hello`. The shell looked alive and served
  nothing. Fixed in two layers in `crates/talaria-shell/src/vault.rs`: the
  keychain is skipped outright when no bus can exist, and the lookup is
  time-bounded (1500ms, `TALARIA_KEYCHAIN_TIMEOUT_MS`) when one is merely
  unreachable. Affected every headless server, container, SSH session and CI job.

### Added

- **`Event::TabOpened { tab_id, opener_tab_id }`** (AGENT-04). A popup adopted
  into an agent's session — a page it drives calling `window.open` or following a
  `target=_blank` link — is now announced to the owning session instead of being
  discoverable only by polling `tabs_list`. Reaches MCP clients as a
  `notifications/message`, like the existing crash and close events.
- `tests/e2e/vault_nobus_test.py`, pinning the no-bus startup path.

### Changed

- The e2e CI job no longer wraps the suite in `dbus-run-session`. That wrapper
  existed to hide the startup hang above; with the hang fixed at the source,
  running bare is what proves the fallback works.
