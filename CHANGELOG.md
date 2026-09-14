# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0 the CLI syntax, JSON schema, config format and exit codes may still
change; from 1.0 they are covered by the version promise. See
[docs/json-schema.md](docs/json-schema.md) and
[docs/exit-codes.md](docs/exit-codes.md).

## [Unreleased]

gh compatibility. Every one of gh 2.100's 197 commands now either works against
GitFox with gh's spelling, flags and exit codes, or answers `UNSUPPORTED`
(exit 9) naming the feature GitFox lacks, instead of clap's "unrecognized
subcommand".

### Changed — breaking

Each of these was a place where a gh habit did something different in fx, most
of them silently.

* **`fx api -F` / `-f` are gh's.** `-F` is `--field` (typed) and `-f` is
  `--raw-field` (always a string); 0.6 had the letters the other way round, so
  `-f count=3` sent a number where gh sends a string. The long names keep their
  meaning. `--field` also follows gh more closely: it converts integers but not
  decimals (`1.10` stays the string `"1.10"`), a value is sent as written rather
  than trimmed, and naming a key twice is an error instead of the last one
  winning.
* **`fx api` sends parameters as the query string on a GET.** 0.6 sent them as
  a JSON body, which GitFox ignores, so `fx api -X GET …/pullreq -f state=merged`
  answered with every pull request. On a GET or HEAD, and beside `--input` or
  `--body` (which may now be combined with fields), parameters are the query
  string; an array repeats its key.
* **`fx pr merge -m` is `--merge`**, gh's flag for the merge strategy, not
  `--method METHOD`. `-m squash` is now an error: write `--squash` (`-s`),
  `--rebase` (`-r`) or `--method squash`. `--delete-branch` is `-d` as in gh;
  `-D` still works.
* **`fx repo list -q TEXT` is `-S TEXT`.** `-q` is `--jq`, as everywhere in gh.
* **`-h` is the host on `fx auth` and `fx config` subcommands**, as in gh:
  `--hostname` on `auth login | logout | status | token | switch | setup-git`,
  the host scope on `config get | set | list`. A gh user's `fx auth status -h
  host` printed help and exited 0. `--help` still works.
* **`fx pr checks` exits 1 when a check failed and 8 while checks are pending**,
  as gh does, so `fx pr checks && deploy` gates on CI. 0.6 exited 0 with failing
  checks. Only for human output: with `--json`, `--output json` or `--agent` the
  checks are the result, exit 0, and `failed` says what happened.
* **`--json` followed by a field list selects fields**, gh's `--json FIELDS`, on
  the commands that export fields (listed in
  [docs/json-schema.md](docs/json-schema.md)). On those, `fx repo list --json
  ai` now asks for a field called `ai`; write `fx --json repo list ai`. Bare
  `--json`, and `--json` on any other command, keep their 0.6 meaning.

### Added

* **Pull requests:** `fx pr close | reopen | ready | edit | comment | review |
  status | update-branch`, and `fx co` for `fx pr checkout`. Every `fx pr`
  command takes gh's selectors: a number, `#12`, a pull request URL, or a
  branch name.
  * `list`: `--base`, `--head`, `--label`, `--search`, `--draft`, `--web`, and
    `-A` for `--author`.
  * `create`: `--body-file`, `--fill-first`, `--fill-verbose`, `--reviewer`,
    `--label`, `--editor`, `--template`, `--web`, `--dry-run`.
  * `merge`: `--admin` (bypass rules), `--subject`, `--body`, `--body-file`,
    `--match-head-commit`.
  * `diff`: `--patch`, `--exclude`, `--color`, `--web`. `checks`: `--watch`,
    `--fail-fast`, `--interval`, `--required`, `--web`. `view`: `--comments`,
    `--web`.
  * `checkout`: `--branch`, `--detach`, `--force`, `--worktree`,
    `--recurse-submodules` — and pull requests from forks, which 0.6 refused.
  * `status` and `fx status` fall back to asking each open pull request for its
    reviewers, because GitFox 1.3.0 answers HTTP 500 to the reviewer filter.
* **Repositories:** `fx repo create | delete | edit | rename | fork | sync |
  set-default | read-file | read-dir | gitignore list | license list`.
  `repo list` gains `--visibility`, `--fork` and `--source`; `repo view` shows
  the README and takes `--branch` and `--web`; `repo clone` sets up an
  `upstream` remote for a fork and passes git flags after `--`. `repo delete`
  needs `--yes` when nobody can confirm, and reports `deleted_at` because GitFox
  deletes softly.
* **CI in gh's words:** `fx run list | view | rerun | watch | cancel | delete`
  and `fx workflow list | view | run | enable | disable`, over the same
  pipelines and executions as `fx pipeline`. `run list` filters by `--branch`,
  `--status` (gh's vocabulary or GitFox's), `--user`, `--commit`, `--event` and
  `--created`; `run view --log-failed` is `fx pipeline logs --failed`;
  `workflow view --yaml` prints the pipeline definition. `fx pipeline list
  --workflow`, `fx pipeline run --ref` and `fx pipeline view --exit-status`
  accept gh's names too.
* **Space resources:** `fx secret list | set | delete` (from stdin, a hidden
  prompt, `--body` or a dotenv `--env-file`), `fx label list | create | edit |
  delete | clone`, `fx ssh-key list | add | delete`, `fx org list` (the spaces
  you belong to), `fx ruleset list | view`, and `fx codespace list | view |
  create | stop | delete | logs` over GitFox gitspaces.
* **Everyday gh commands:** `fx browse` (repository, file, commit, pull request
  and pipeline pages; `-n` prints the URL), `fx status`, `fx alias set | list |
  delete | import` (with `--shell` aliases), `fx auth token | switch |
  setup-git`, `fx config clear-cache`, and `fx completion -s SHELL`.
* **gh-style output:** `--json FIELDS`, `--jq` and `--template` on every
  command gh has them for, with gh's field names and value shapes (`OPEN`, ISO
  8601 times, `{"login": …}`). jq runs built in; templates get gh's helpers
  (`tablerow`, `timeago`, `color`, …).
* **`fx api`:** `-X/--method`, `--paginate` (over GitFox's `page`/`limit`) with
  `--slurp`, `--jq`, `--template`, `--silent`, `--hostname`; `key[]=v`,
  `key[sub]=v` and `key[][sub]=v` parameters; `{owner}`, `{repo}`, `{repo_ref}`
  and `{branch}` placeholders in the path and in `-F` values.
* **Config:** `git_protocol` (per host too), `editor`, `browser`, `prompt`
  (`disabled` never prompts) and `pager`. fx does not page its output, and says
  so when `pager` is set.
* **gh commands for features GitFox does not have** — issues, releases, gists,
  discussions, projects, search, variables, caches, attestations, GPG keys,
  deploy keys, extensions and the Copilot commands — and gh flags for missing
  features, such as `repo list --archived` or `pr edit --milestone`, answer
  `UNSUPPORTED` (exit 9) with the reason and, where there is one, what to use
  instead.
* Exit codes: `CHECKS_FAILED` (1), `CHECKS_PENDING` (8), `RUN_FAILED` (1, with
  `--exit-status`), `CANCELLED` (2, a declined confirmation) and `UNSUPPORTED`
  (9). See [docs/exit-codes.md](docs/exit-codes.md).
* `gitfox-client`: labels, spaces, secrets, public keys, rules and gitspaces
  APIs; pull request edits, comments, reviews, reviewers and labels; repository
  create, delete, rename, fork, sync, content and templates; pipeline get,
  enable/disable, definition and execution delete.

### Changed

* `gitfox-client`: `PullRequest`, `Repository`, `Execution`, `Pipeline`, `User`
  and `PullRequestFilter` have new public fields, so code that builds one with a
  struct literal needs to name them.

### Security

* `fx auth token` prints the token, and `fx auth status --show-token` includes
  it — both only when asked, as in gh, because printing it is what they are
  for. No other command outputs a token, in any mode; the end-to-end test that
  checks every stream still covers the rest.
* `fx auth setup-git` makes git ask `fx auth git-credential` for the configured
  hosts' credentials, so a clone authenticates without the token ever being
  written into a remote URL or `.git/config`. It edits your global git config,
  and only when you run it.
* `fx secret set` never echoes a value. `--body` puts the value in your shell
  history and the process list, as it does in gh; prefer stdin or the prompt.

## [0.6.2] — 2026-09-11

### Fixed

* A write the server redirected no longer reports success. Following a `301`
  or `302` resent a `POST` as a `GET` with no body, and a `303` did the same to
  `PATCH`, `PUT` and `DELETE`. The `GET` answered 200, so creating a secret with
  `fx api POST /api/v1/secrets` could say `ok: true` while nothing was created.
  Redirects are now followed for `GET` and `HEAD` only; any other method fails
  with `API_ERROR`, naming the `Location` the server sent.

## [0.6.1] — 2026-09-05

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
