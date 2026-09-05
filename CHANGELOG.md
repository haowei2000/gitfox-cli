# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0 the CLI syntax, JSON schema, config format and exit codes may still
change; from 1.0 they are covered by the version promise. See
[docs/json-schema.md](docs/json-schema.md) and
[docs/exit-codes.md](docs/exit-codes.md).

## [Unreleased]

### Fixed

* `fx pipeline logs` now returns a running step's output. GitFox does not
  persist a step's log until it finishes, so the static log endpoint answers
  404 while it runs and the output is only reachable over an SSE stream that
  the instance's OpenAPI document does not mention. fx was asking the static
  endpoint for every step and swallowing the 404, so a step in progress looked
  like a step that had printed nothing.
* A log that could not be fetched is no longer indistinguishable from a step
  that produced no output. Every step now reports `log_available`, and `live`
  says whether its lines came from the stream; the human view says which of the
  three cases it is rather than printing an empty block.

## [0.6.0] — 2026-09-05

First release, published to crates.io as
[`gitfox-cli`](https://crates.io/crates/gitfox-cli) (the `fx` binary) and
[`gitfox-client`](https://crates.io/crates/gitfox-client) (the API client). Everything below was verified against a live GitFox instance
(API v1.3.0), not only against mocks.

### Commands

* `fx api` — the escape hatch: any method, any endpoint, `--field` /
  `--raw-field` / `--body` / `--input` bodies, extra headers, `--include`.
* `fx auth login | logout | status` — tokens validated before storage, kept in
  the OS keychain, never printed back.
* `fx repo list | view | clone` — listing narrows to a named space, then
  `--org`, then the current checkout's space, and otherwise spans the instance.
  Cloning names the directory after the repository and never puts the token in
  the URL.
* `fx pr list | view | create | merge | diff | checks | checkout` — inside a
  checkout none of it needs arguments: the repository comes from the git remote
  and the pull request from the current branch.
* `fx pipeline list | view | logs | run | retry` — including
  `fx pipeline logs --failed`, which reads the run, finds the steps that failed
  and fetches only those.
* `fx config get | set | list` — token keys are not addressable, so a
  credential cannot be written into the plain-text config even by accident.
* `fx completion bash | zsh | fish | …`.

### The machine interface

* `--agent` / `GITFOX_AGENT` — shorthand for `--output json
  --non-interactive --no-color`.
* One JSON envelope for every command: `{"ok":true,"data":…}` /
  `{"ok":false,"error":{"code","message","details"}}`, with `jsonl` for
  line-oriented tooling.
* Stable error codes and process exit codes.
* Transparent pagination: `--limit` is a total, not a page size, and every list
  reports `truncated` — observed by asking for one row more than requested
  rather than guessed, because GitFox publishes no pagination headers.
* Retries for transient failures with exponential backoff and jitter.
  `Retry-After` is honoured up to five seconds. `POST`, `PATCH` and `500` are
  never retried: repeating a `POST /pullreq` that already reached the server
  opens a second pull request.

### Configuration

* One documented precedence chain — CLI flag > environment > config file > git
  context > default — resolved by a pure function, so the whole chain is
  unit-tested without touching the process environment.
* Nine `GITFOX_*` variables plus `NO_COLOR`.
* The git tier only speaks for remotes pointing at the resolved GitFox host, so
  running fx inside a checkout of another host does not ask GitFox about that
  project.

### Security

* Tokens live in the OS keychain or the environment, never in the config file.
* `Secret` redacts itself in `Debug` and `Display`; the `Authorization` header
  is marked sensitive. A test asserts the token reaches neither stdout nor
  stderr, even under `-vvv`.
* `--insecure` works and warns on stderr every time it does.

### Architecture

* `gitfox-client` knows HTTP and GitFox and nothing about terminals or exit
  codes, so a second front-end can reuse it rather than shelling out to this
  binary — see [#1](https://github.com/haowei2000/gitfox-cli/issues/1).
* The CLI's JSON schema is its own: commands map API responses onto models the
  CLI owns, so a GitFox API change need not break an agent.
