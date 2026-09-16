# gf — GitFox CLI for humans, CI and AI agents

[![crates.io](https://img.shields.io/crates/v/gitfox-cli.svg)](https://crates.io/crates/gitfox-cli)
[![docs.rs](https://img.shields.io/docsrs/gitfox-client)](https://docs.rs/gitfox-client)
[![CI](https://github.com/haowei2000/gitfox-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/haowei2000/gitfox-cli/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/gitfox-cli.svg)](LICENSE)

`gf` is a single Rust binary that talks to a GitFox instance. It is built for
three callers at once, and it knows which one it is talking to:

```bash
gf pr list                                   # you
GITFOX_TOKEN=$TOKEN gf --agent pipeline list  # CI
gf --agent pr list                            # an AI agent
```

`--agent` is shorthand for `--output json --non-interactive --no-color`. In that
mode every command answers with a stable envelope and a stable exit code, so a
machine never has to parse prose:

```json
{ "ok": true, "data": { "id": 12, "title": "Add OAuth", "state": "open" } }
```

```json
{ "ok": false, "error": { "code": "AUTH_REQUIRED", "message": "…", "details": null } }
```

## Status

**gf speaks gh.** Every one of gh 2.100's 197 commands either works against
GitFox — `pr`, `repo`, `run`, `workflow`, `secret`, `label`, `ssh-key`, `org`,
`ruleset`, `codespace`, `browse`, `status`, `alias`, `auth`, `config`, `api` —
with gh's flags, `--json`/`--jq`/`--template` and exit codes, or answers
`UNSUPPORTED` with what GitFox lacks. Everything is verified against GitFox API
v1.3.0. Lists page transparently and say when they were truncated, transient
failures are retried, and the JSON contract is written down in
[docs/json-schema.md](docs/json-schema.md).

Next: `gf-mcp` (v0.8), reusing `gitfox-client` directly.

## Install

```bash
cargo install gitfox-cli
```

The crate is `gitfox-cli`; the binary it installs is `gf`. (`gf` on crates.io
belongs to an unrelated tool.)

### A prebuilt binary

No Rust toolchain needed. Download from [the latest release][releases]:

```bash
# macOS (Apple silicon)
BASE=https://github.com/haowei2000/gitfox-cli/releases/latest/download
curl -sSLO "$BASE/gf-darwin-aarch64.tar.gz"
curl -sSLO "$BASE/SHA256SUMS"

# Keep the published name so the checksum can be checked against it.
shasum -a 256 --ignore-missing -c SHA256SUMS   # sha256sum on Linux

tar -xzf gf-darwin-aarch64.tar.gz gf && chmod +x gf && sudo mv gf /usr/local/bin/
gf --version
```

Built for `darwin-aarch64`, `darwin-x86_64`, `linux-x86_64`, `linux-aarch64`
and `windows-x86_64`. `--ignore-missing` is what lets one line verify the one
archive you downloaded out of the five listed.

The Linux binaries link libdbus statically — their only shared-library
dependencies are `libc`, `libm` and `libgcc_s` — so they run in a slim
glibc-based image (`debian:*-slim`, `ubuntu`, `gcr.io/distroless/base`) with
nothing installed. They are **not** musl builds, so Alpine and
`distroless/static` need a source build.

### From a checkout

```bash
cargo install --path crates/gitfox-cli
```

A source build on Linux links against the system D-Bus for the OS keychain:

```bash
sudo apt install -y libdbus-1-dev pkg-config
```

CI and agents need none of that — they pass `GITFOX_TOKEN` and never touch the
keychain.

### Upgrading from `fx`

The binary was called `fx` up to 0.6. Nothing else changed: same crate, same
commands, same flags. `cargo install gitfox-cli` leaves the old binary behind,
so remove it once `gf` works.

```bash
rm -f "$(command -v fx)"
```

Your login and settings come along by themselves. A token in the OS keychain is
still found under its old entry, and a `~/.config/fx/config.toml` is used where
it lies until you move it to `~/.config/gf/` yourself — adopted rather than
copied, so there is never a second config file drifting from the first. The
same goes for a `fx.repo` left in a checkout by `fx repo set-default`.

One thing does need redoing: `fx auth setup-git` wrote git a credential helper
pointing at the old binary's path. Run `gf auth setup-git` again, and git will
call `gf` instead.

[releases]: https://github.com/haowei2000/gitfox-cli/releases/latest

## Quick start

```bash
export GITFOX_HOST=https://git.example.com
export GITFOX_TOKEN=xxxxxxxx

gf api GET /api/v1/user
gf auth status
```

Or log in interactively and let the token live in the OS keychain:

```bash
gf auth login --hostname git.example.com
gf auth setup-git    # and let git clone and push with it
```

## Coming from gh

Type what you would type to gh. `gf pr checks 12 && ./deploy.sh`,
`gf run view --log-failed`, `gf pr list --json number,title --jq '.[].title'`
and `gf api -X GET /api/v1/repos/{repo_ref}/pullreq -f state=merged` all do what
they do in gh.

What is different is what GitFox is:

* **Repositories are `space/name`**, and spaces nest. `-R` and `--org` take
  them; `gf org list` lists yours.
* **Runs belong to pipelines.** `gf run` and `gf workflow` are gh's names over
  GitFox pipelines, whose runs are numbered per pipeline: `gf run view 182` finds
  the run when only one pipeline has a #182, and otherwise wants
  `gf run view default/182`.
* **No issues, releases, gists, discussions, projects, search or variables.**
  Those commands answer exit 9, `UNSUPPORTED`, with the reason — and so does a
  gh flag for a feature GitFox lacks, such as `repo list --archived`.
* **Secrets live on spaces** (`gf secret set NAME -o SPACE`), codespaces are
  GitFox gitspaces, and `gf pr update-branch` needs `--rebase`, the only update
  GitFox performs.
* **Exit codes are finer than gh's.** Failed and pending checks are 1 and 8, as
  in gh, but authentication is 3 — gh's 4 means *not found* here. See
  [Exit codes](#exit-codes).
* **Bare `--json` is gf's envelope** (`gf --json pr list`); `--json FIELDS` is
  gh's. Environment variables are `GITFOX_*`, not `GH_*`.

## Repositories

```bash
gf repo list              # this space, or the whole instance from outside one
gf repo list ai -S back   # search within a space
gf repo view              # the current checkout's repository, with its README
gf repo clone ai/backend  # into ./backend
gf repo read-file src/main.rs --ref main
```

`create`, `delete`, `edit`, `rename`, `fork`, `sync` and `set-default` work as
in gh. `gf repo delete` asks you to type the name, and wants `--yes` where there
is nobody to ask.

```
REPOSITORY   VISIBILITY  DEFAULT  UPDATED  DESCRIPTION
ai/backend   private     main     3d ago   The backend
ai/docs      public      main     2w ago
```

The visibility column appears only when GitFox reported it. The instance-wide
listing (`GET /repos`) answers with a narrower shape than the space-scoped one,
so a repository there has *unknown* visibility rather than a guessed one — and
the column is dropped rather than filled with dashes.

`gf repo clone` hands the URL to `git`, which keeps its own progress output and
its own credential prompt. gf does not splice the token into the URL: that would
write it into `.git/config`, where it outlives the command and travels with the
checkout. Use `--ssh`, or `gf config set git_protocol ssh`, to clone over SSH
instead; `gf auth setup-git` lets git fetch the token over HTTPS itself.

## Pull requests

Inside a checkout, nothing needs to be spelled out — the repository comes from
the git remote and the pull request from the branch you are on:

```bash
cd ~/project
gf pr list                       # open pull requests in this repository
gf pr view                       # the one for the current branch
gf pr create --fill              # title and body from the branch's commits
gf pr merge --squash             # merge it, squashed
```

```
NUMBER  TITLE            STATE  BRANCHES           AUTHOR  UPDATED
#12     feat: add OAuth  open   feat/oauth → main  whw     3d ago
```

Explicitly, for CI and agents:

```bash
gf --agent pr list -R ai/backend --state all --limit 50
gf --agent pr view 12
gf --agent pr create -B main -H feat/oauth -t "feat: add OAuth" -b "Closes #4"
gf --agent pr merge 12 --squash --delete-branch
```

A pull request is named as in gh: `12`, `#12`, its URL, or its branch.

Reviewing one:

```bash
gf pr diff              # the raw patch, for your pager
gf pr diff --name-only  # just what changed
gf pr checks            # what CI says, and what is blocking the merge
gf pr checkout 12       # fetch the branch and switch to it, from a fork too
gf pr review 12 --approve -b "LGTM"
gf pr comment 12 --body "One question inline"
gf pr status            # yours, and the ones waiting on your review
```

`close`, `reopen`, `ready`, `edit` and `update-branch --rebase` round it out.

`gf pr diff` picks its form from the output mode: a person gets the unified
patch their pager and highlighter understand, `--agent` gets it split by file
with per-file patches (`--name-only` omits them). The endpoint content-
negotiates, so neither form is reassembled from the other.

Two flags worth knowing:

* `gf pr merge --dry-run` answers "would this merge?" without merging, which is
  the question an agent actually wants before it does anything.
* `gf pr list --author whw` takes a login. GitFox filters by numeric principal
  id, so gf resolves the name for you.

## CI

```bash
gf pipeline list            # every pipeline's latest run, one request
gf pipeline view            # the newest run, with its stage/step tree
gf pipeline logs --failed   # only the steps that failed
gf pipeline logs --failed --tail 50
gf pipeline retry           # run it again
```

```
RUN   PIPELINE  STATUS      BRANCH      MESSAGE          STARTED
#182  default   ✗ failure   main        feat: add OAuth  12m ago
#181  nightly   ✓ success   main        chore: bump      6h ago
```

`--failed` is the reason this exists. GitFox addresses logs per *step*, and only
the single-execution endpoint returns the stage tree, so answering "why is CI
red" by hand is: read the run, find the failed steps, fetch each one. Here:

```bash
gf --agent pipeline logs --failed
```

```json
{ "ok": true, "data": { "run": 182, "status": "failure", "count": 1,
  "steps": [ { "stage": "build", "step": "cargo test", "exit_code": 101,
               "lines": ["error[E0308]: mismatched types", "..."] } ] } }
```

Inside a checkout with one pipeline, nothing needs naming: the pipeline is
inferred, and the run defaults to the most recent. A green run answers with an
empty `steps` and exit 0 — "nothing failed" is a result, not an error.

gh's names work over the same runs:

```bash
gf run list --status failure --branch main
gf run view --log-failed
gf run watch --exit-status && ./deploy.sh
gf workflow run default --ref main
gf workflow view default --yaml
```

A failed build's log is mostly progress output and the reason is at the end, so
`--tail N` keeps that end. Each step reports `total_lines` alongside `lines`, so
nothing is dropped silently.

## Shell completion

```bash
gf completion zsh  > ~/.zfunc/_gf
gf completion bash > /usr/local/etc/bash_completion.d/gf
gf completion fish > ~/.config/fish/completions/gf.fish
```

## `gf api` — the escape hatch

Anything GitFox exposes is reachable on day one, whether or not a dedicated
command exists yet.

```bash
gf api /api/v1/user
gf api -X POST /api/v1/foo -F count=3 -f name=test
gf api -X GET /api/v1/repos/{repo_ref}/pullreq -f state=merged -F limit=5
gf api /api/v1/repos/{repo_ref}/pullreq --paginate --jq '.[].title'
cat payload.json | gf api -X POST /api/v1/foo --input -
gf api /api/v1/user --include              # status + headers too
```

The flags are gh's:

* The method is optional: `gf api /api/v1/user` is a `GET`, and sending anything
  without naming a method makes it a `POST`. `gf api POST /path` works too.
* `-F/--field` types its values — `count=3` is a number, `draft=true` a boolean,
  `parent=null` a null, `body=@notes.md` a file's contents — and `-f/--raw-field`
  always sends a string. `key[]=v`, `key[sub]=v` and `key[][sub]=v` build arrays
  and objects.
* They are the JSON body, except on a `GET`, and beside `--input` or `--body`,
  where they are the query string.
* `{owner}`, `{repo}`, `{branch}` and `{repo_ref}` — the `space%2Fname` GitFox's
  repository paths take — come from the current checkout.
* `--paginate` walks GitFox's `page`/`limit` pages into one array.

## Configuration

Resolved through one precedence chain, top to bottom:

```
CLI flag  >  environment variable  >  config file  >  git context  >  default
```

The git tier only speaks for remotes that point at the resolved GitFox host. A
checkout of some other host has a perfectly good `owner/name` that means nothing
to this instance, so gf says it could not infer a repository rather than asking
GitFox about a GitHub project. Several remotes are fine — the one matching the
host is the one that counts.

### Environment variables

| Variable | Meaning |
|---|---|
| `GITFOX_HOST` | Instance URL, e.g. `https://git.example.com` |
| `GITFOX_TOKEN` | API token |
| `GITFOX_REPO` | Repository as `space/name` |
| `GITFOX_ORG` | Space or organisation |
| `GITFOX_OUTPUT` | `table`, `json` or `jsonl` |
| `GITFOX_CONFIG` | Path to the config file |
| `GITFOX_TIMEOUT` | HTTP timeout in seconds (default `30`) |
| `GITFOX_RETRIES` | Retries for transient failures (default `2`) |
| `GITFOX_INSECURE` | Skip TLS verification — warns loudly when it does |
| `GITFOX_AGENT` | Turn on agent mode |
| `NO_COLOR` | Standard opt-out, honoured alongside `--no-color` |

Booleans accept `1`, `true`, `yes`, `y`, `on`; anything else is false, so a
typo cannot silently disable TLS verification.

### Config file

`$GITFOX_CONFIG`, else `$XDG_CONFIG_HOME/gf/config.toml`, else
`~/.config/gf/config.toml`. A pre-0.7 `fx/config.toml` is read and written in
place when the `gf` one does not exist yet; `gf config list` prints the file
actually in use.

```toml
default_host = "git.example.com"
git_protocol = "https"   # or ssh
editor = "vim"           # else $VISUAL, then $EDITOR
browser = "firefox"      # else $BROWSER
prompt = "enabled"       # disabled: never ask

[hosts."git.example.com"]
api_url = "https://git.example.com"
user = "whw"

[hosts."git.internal.local"]
api_url = "https://git.internal.local"
insecure = true
git_protocol = "ssh"

[aliases]
mine = "pr list --author @me"
```

`gf config get | set | list` read and write it, with `-h HOST` for a host's
keys as in gh. Tokens are never written here: `gf config set` cannot even
address a token key. `gf repo set-default` records a checkout's repository in
its own `.git/config`, which wins over the remotes.

### Where a token comes from

```
--token  >  GITFOX_TOKEN  >  OS keychain
```

`gf auth status` reports which of the three was used, and prints the value only
with `--show-token`.

## Output

The full contract — every command's `data` shape — is in
[docs/json-schema.md](docs/json-schema.md).

| Format | Success | Failure |
|---|---|---|
| `table` (default) | human text on stdout | message on **stderr** |
| `json` | one `{"ok":true,"data":…}` document on stdout | one `{"ok":false,"error":…}` document on **stdout** |
| `jsonl` | one bare JSON value per line on stdout | one enveloped error object on stdout |

In machine modes the whole contract is: stdout is one JSON document, and the
exit code says whether it is a result or an error.

### gh's `--json FIELDS`, `--jq` and `--template`

Where gh has them, so does gf, with gh's field names and value shapes — and no
envelope:

```bash
gf pr list --json number,title,headRefName
gf pr view 12 --json state,mergeable --jq .state
gf run list --json databaseId,conclusion --template '{{range .}}{{.databaseId}} {{.conclusion}}{{"\n"}}{{end}}'
```

jq is built in, and templates get gh's helpers (`tablerow`, `timeago`, `color`,
`truncate`, …). An unknown field fails with the list of the ones that exist.

### Lists never lie about being complete

Every list carries `count` and `truncated`:

```json
{ "ok": true, "data": { "count": 30, "truncated": true, "items": [ … ] } }
```

GitFox publishes no pagination headers, so gf asks for one row more than you
requested — receiving it is proof there are more. `truncated` is therefore an
observation, not a guess. Raise `--limit` and gf walks the pages for you, at up
to 100 rows per request.

### Transient failures are retried

Network errors, timeouts, `429`, `502`, `503` and `504` are retried with
exponential backoff (`--retries`, `GITFOX_RETRIES`, default 2). A server's
`Retry-After` is honoured up to five seconds.

`POST` and `PATCH` are **never** retried. A retried `POST /pullreq` that timed
out after the server accepted it would open a second pull request; reporting
that something might not have happened beats silently doing it twice. `500` is
not retried either — an internal error that repeats is usually a bug being hit
again, not a blip.

### Writes never follow a redirect

Following a redirect does not faithfully replay a write. On a `301` or `302` the
HTTP client resends a `POST` as a `GET` with no body, and on a `303` it does the
same to `PATCH`, `PUT` and `DELETE` — and that `GET` answers 200, so a redirected
`POST /api/v1/secrets` reported `ok: true` while nothing was created. gf follows
redirects for reads only. A request that changes data fails with `API_ERROR` and
names the `Location` the server pointed at, which is usually the fix: a trailing
slash, or `https://` for a host configured as `http://`.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | unexpected internal error — or, as in gh, a failed check or run |
| `2` | invalid arguments, or a declined confirmation |
| `3` | authentication error |
| `4` | not found |
| `5` | API error |
| `6` | network error or timeout |
| `7` | configuration error |
| `8` | git context error — or, as in gh, checks still pending |
| `9` | not supported: a gh command or flag for a feature GitFox lacks |

Full table with the matching `error.code` strings: [docs/exit-codes.md](docs/exit-codes.md).
The JSON each command returns: [docs/json-schema.md](docs/json-schema.md).

## Crates

| Crate | What it is |
|---|---|
| [`gitfox-cli`](https://crates.io/crates/gitfox-cli) | the `gf` binary |
| [`gitfox-client`](https://crates.io/crates/gitfox-client) · [docs](https://docs.rs/gitfox-client) | the GitFox API client, reusable on its own |

## Architecture

```
                       ┌──────────────┐
                       │  GitFox API  │
                       └──────▲───────┘
                              │
                       gitfox-client
                              ▲
                  ┌───────────┴───────────┐
                  │                       │
               gf CLI                  gf MCP  (v0.8)
                  ▲                       ▲
          ┌───────┼───────┐               │
          │       │       │               │
         you     CI     agent           agent
```

```
crates/
├── gitfox-client/    HTTP + GitFox. No terminals, no exit codes.
│   ├── client.rs     the one place a request is issued
│   ├── error.rs      typed API errors
│   ├── models/       domain models, deliberately not the raw API DTOs
│   └── auth.rs · repo.rs · pull_request.rs · pipeline.rs · principal.rs
│       labels.rs · spaces.rs · rules.rs
└── gitfox-cli/       the binary (installs as `gf`)
    ├── argv.rs       aliases and gh's `--json FIELDS`, before clap parses
    ├── cli.rs, cli/  the clap command tree
    ├── config.rs     the precedence chain (pure, heavily tested)
    ├── context.rs    resolved config + renderer + client
    ├── output.rs     Render trait, envelopes, tables
    ├── export.rs     gh's --json FIELDS, --jq and --template
    ├── error.rs      stable error codes and exit codes
    ├── git.rs        what the surrounding checkout says
    ├── interact.rs   prompts, editors, browsers
    ├── keychain.rs   OS keychain access
    ├── paginate.rs   walking page/limit endpoints
    └── commands/     one module per command, gh_only.rs for what GitFox lacks
```

Two rules keep this from rotting:

* **The client knows nothing about the CLI.** That is what lets `gf-mcp` reuse
  it in v0.8 rather than shelling out to this binary.
* **The CLI's JSON schema is its own.** Commands map API responses onto models
  the CLI owns, so a GitFox API change does not have to break an agent.

## Roadmap

| Version | Scope |
|---|---|
| **v0.1** ✅ | workspace, client, config chain, env vars, auth, `gf api`, JSON output, error model |
| **v0.2** ✅ | `repo list/view/clone`, git remote detection, `-R`, multi-host |
| **v0.3** ✅ | `pr list/view/create/merge` — the first genuinely daily-usable release |
| **v0.4** ✅ | `pipeline list/view/logs/run/retry`, including `logs --failed` |
| **v0.5** ✅ | `pr checkout/diff/checks`, `pr create --fill`, shell completion |
| **v0.6** ✅ | agent hardening: pagination, retries, non-interactive edges, schema freeze |
| **v0.7** ✅ | gh compatibility: gh's commands, flags, `--json`/`--jq`/`--template` and exit codes; the binary becomes `gf` |
| [v0.8](https://github.com/haowei2000/gitfox-cli/milestone/1) | `gf-mcp`, reusing `gitfox-client` directly |
| [v1.0](https://github.com/haowei2000/gitfox-cli/milestone/2) | CLI syntax, JSON schema, config format and exit codes all stable |

The twelve commands v0.1–v0.4 aim to make rock solid: `auth login/logout/status`,
`api`, `repo list/view`, `pr list/view/create/merge`, `pipeline list/logs`.
All twelve are done.

### Where the endpoints come from

Every GitFox instance serves its own OpenAPI document at `/openapi.yaml`,
unauthenticated. The endpoints, parameter names and response shapes this CLI
targets were read from there rather than guessed, and the module docs in
`crates/gitfox-client/src/` record which version they were checked against.

## Development

```bash
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
cargo test --workspace
```

CI runs clippy with `-D warnings` on the current stable, so a local toolchain
that has fallen behind can pass where CI does not. `rustup update` before
trusting a green local run.

Tests are layered:

* unit tests for the precedence chain, error mapping and rendering
  (`crates/gitfox-cli/src/*.rs`)
* HTTP tests against a mock GitFox covering 401/403/404/500, timeouts,
  non-JSON bodies and empty responses (`crates/gitfox-client/tests/http.rs`)
* end-to-end tests over the real binary asserting the JSON envelope, the exit
  codes, and that a token never reaches stdout or stderr — not even under
  `-vvv` (`crates/gitfox-cli/tests/cli.rs`)
* end-to-end tests typing gh's spellings and checking what reaches the server
  and what comes back, aimed at flags that parse but mean something else
  (`crates/gitfox-cli/tests/gh_compat.rs`)

## Security

* Tokens live in the OS keychain or in the environment, never in the config file.
* `Secret` redacts itself in `Debug` and `Display`; the `Authorization` header is
  marked sensitive so it stays out of logs.
* `gf auth status` reports `"token": "configured"` and its source. Two commands
  print a token, and only because that is what you asked them for:
  `gf auth token` and `gf auth status --show-token`, as in gh.
* `gf auth setup-git` makes git ask `gf auth git-credential` for credentials, so
  the token never lands in a remote URL or `.git/config`. It edits your global
  git config, and only when you run it.
* `gf secret set` never echoes a value; stdin or the hidden prompt keep it out of
  your shell history, which `--body` does not.
* `--insecure` works, and warns on stderr every time it does.

## License

MIT — see [LICENSE](LICENSE).
