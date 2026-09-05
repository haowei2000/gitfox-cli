# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

* `gitfox-client` — the GitFox API client: base URL normalisation, bearer auth,
  typed errors, `GET /api/v1/user`, and one generic request path used by `fx api`.
* `fx api` — send any method to any endpoint, with `--field` / `--raw-field` /
  `--body` / `--input` bodies, extra headers and `--include`.
* `fx auth login | logout | status` — tokens validated before storage and kept
  in the OS keychain.
* `fx config get | set | list` — read and write the config file; token keys are
  not addressable.
* Configuration precedence: CLI flag > environment > config file > git context >
  default, resolved by a pure, unit-tested function.
* Output system: `table`, `json` and `jsonl`, with the `{"ok":…}` envelope.
* Stable error codes and process exit codes, documented in `docs/exit-codes.md`.
* `--agent` / `GITFOX_AGENT`: JSON output, no prompts, no colour.
* Command tree for `fx repo`, `fx pr` and `fx pipeline`; each returns a
  structured `NOT_IMPLEMENTED` error naming the release it lands in.
