# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

* `fx pr diff`, `fx pr checks` and `fx pr checkout`, completing the command
  surface — nothing answers `NOT_IMPLEMENTED` any more.
  * `pr diff` uses the endpoint's content negotiation: a raw unified diff for
    people, per-file entries with patches for machines. `--name-only` drops the
    patches.
  * `pr checks` separates *failing* from *blocking*: a required check that is
    still running blocks a merge without having failed.
  * `pr checkout` fetches the source branch and switches to it, fast-forward
    only for an existing local branch. A pull request from a fork is refused
    with an explanation rather than guessed at.
* `fx completion bash|zsh|fish|…`.

### Fixed

* `fx <anything> | head` no longer fails or panics. Rust ignores `SIGPIPE`, so a
  closed reader surfaced as a write error — and `clap_complete` panicked on it,
  making `fx completion zsh | head` crash outright. The single place that writes
  to stdout now treats a closed reader as success.

* Transparent pagination for `repo list`, `pr list` and `pipeline list`.
  `--limit` is now a total rather than a page size: fx walks pages at up to 100
  rows per request. Every list carries `truncated`, observed by asking for one
  row more than requested rather than guessed.
* Retries for transient failures — network errors, timeouts, 429, 502, 503,
  504 — with exponential backoff and jitter. `--retries` / `GITFOX_RETRIES`,
  default 2. A server's `Retry-After` is honoured up to five seconds. `POST`
  and `PATCH` are never retried; neither is `500`.
* `RATE_LIMITED` error code (exit 6), carrying `details.retry_after_secs`.
* [`docs/json-schema.md`](docs/json-schema.md): the machine contract, written
  down per command.

* `fx repo list | view | clone`, completing the twelve core commands.
  * `fx repo list` narrows to a named space, then `--org`, then the current
    checkout's space, and otherwise spans the instance. `-q` searches and
    `--sort` orders by name, creation or update.
  * `fx repo clone` clones into a directory named after the repository, not
    after the URL's last segment. The token is never spliced into the URL;
    `git` keeps its own progress output and credential prompt. `--ssh` selects
    the SSH clone URL.
  * `fx repo view` defaults to the current checkout's repository.
* `Repository` decodes both shapes GitFox returns. The instance-wide
  `GET /repos` omits `is_public`, so visibility there is `null` — unknown
  rather than wrongly reported as private — and the human table drops the
  column instead of showing a row of dashes.

* `fx pipeline list | view | logs | run | retry`, against endpoints verified
  from the instance's own `/openapi.yaml` (GitFox API v1.3.0).
  * `fx pipeline logs --failed` reads the run, walks its stages for steps that
    failed, and fetches only those. `--step` filters by name and composes with
    `--failed`. A green run answers with an empty list and exit 0.
  * `fx pipeline list` costs one request: `?latest=true` embeds each pipeline's
    most recent run. With `--pipeline` it lists that pipeline's runs instead,
    in the same shape.
  * The pipeline is inferred when a repository has only one, and the run
    number defaults to the most recent. Several pipelines refuse to guess and
    name the choices.
  * A step whose log is missing is still reported, with its status and exit
    code, rather than failing the command.
* `CiStatus` keeps the server's exact word rather than mapping onto a closed
  enum, so a status GitFox adds later neither fails to decode nor loses
  information on the way out.

* `fx pr list | view | create | merge`, against endpoints verified from the
  instance's own `/openapi.yaml` (GitFox API v1.3.0).
  * `fx pr view` and `fx pr merge` take no number inside a checkout: the current
    branch selects the pull request.
  * `fx pr create` defaults its base to the repository's default branch and its
    head to the current branch; `--fill` writes the title and body from the
    branch's commits.
  * `fx pr merge --dry-run` reports whether the merge would succeed.
    `--delete-branch` is a second request, because GitFox has no flag for it on
    merge.
  * `fx pr list --author` accepts a login and resolves it to the numeric
    principal id the API filters on.
* Git context detection (`crates/fx-cli/src/git.rs`): the repository, and for
  HTTP remotes the API host, are read from the git remote — the bottom tier of
  the configuration chain. It runs only when the flags, environment and config
  file left something unresolved.
* `REPO_NOT_FOUND` and `PR_NOT_FOUND` are now raised where a bare 404 would have
  been, so a caller can tell a missing repository from a missing pull request.

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
