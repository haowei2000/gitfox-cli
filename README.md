# fx — GitFox CLI for humans, CI and AI agents

`fx` is a single Rust binary that talks to a GitFox instance. It is built for
three callers at once, and it knows which one it is talking to:

```bash
fx pr list                                   # you
GITFOX_TOKEN=$TOKEN fx --agent pipeline list  # CI
fx --agent pr list                            # an AI agent
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

**Every command in the surface is implemented.** `repo`, `pr`, `pipeline`,
`auth`, `config`, `api` and `completion` all work against a verified GitFox API
v1.3.0. Lists page transparently and say when they were truncated, transient
failures are retried, and the JSON contract is written down in
[docs/json-schema.md](docs/json-schema.md).

Next: `fx-mcp` (v0.7), reusing `gitfox-client` directly.

## Install

### A prebuilt binary

Download it from [the latest release][releases], make it executable, and put it
on your `PATH`:

```bash
# macOS (Apple silicon)
BASE=https://github.com/haowei2000/gitfox-cli/releases/latest/download
curl -sSLO "$BASE/fx-darwin-aarch64.tar.gz"
curl -sSLO "$BASE/SHA256SUMS"

# Keep the published name so the checksum can be checked against it.
shasum -a 256 --ignore-missing -c SHA256SUMS   # sha256sum on Linux

tar -xzf fx-darwin-aarch64.tar.gz fx && chmod +x fx && sudo mv fx /usr/local/bin/
fx --version
```

Built for `darwin-aarch64`, `darwin-x86_64`, `linux-x86_64`, `linux-aarch64`
and `windows-x86_64`. `--ignore-missing` is what lets one line verify the one
archive you downloaded out of the five listed.

The Linux binaries link libdbus statically — their only shared-library
dependencies are `libc`, `libm` and `libgcc_s` — so they run in a slim
glibc-based image (`debian:*-slim`, `ubuntu`, `gcr.io/distroless/base`) with
nothing installed. They are **not** musl builds, so Alpine and
`distroless/static` need a source build instead.

### From source

```bash
cargo install --path crates/fx-cli
```

A source build on Linux links against the system D-Bus for the OS keychain:

```bash
sudo apt install -y libdbus-1-dev pkg-config
```

CI and agents need none of that — they pass `GITFOX_TOKEN` and never touch the
keychain.

[releases]: https://github.com/haowei2000/gitfox-cli/releases/latest

## Quick start

```bash
export GITFOX_HOST=https://git.example.com
export GITFOX_TOKEN=xxxxxxxx

fx api GET /api/v1/user
fx auth status
```

Or log in interactively and let the token live in the OS keychain:

```bash
fx auth login --hostname git.example.com
```

## Repositories

```bash
fx repo list              # this space, or the whole instance from outside one
fx repo list ai -q back   # search within a space
fx repo view              # the current checkout's repository
fx repo clone ai/backend  # into ./backend
```

```
REPOSITORY   VISIBILITY  DEFAULT  UPDATED  DESCRIPTION
ai/backend   private     main     3d ago   The backend
ai/docs      public      main     2w ago
```

The visibility column appears only when GitFox reported it. The instance-wide
listing (`GET /repos`) answers with a narrower shape than the space-scoped one,
so a repository there has *unknown* visibility rather than a guessed one — and
the column is dropped rather than filled with dashes.

`fx repo clone` hands the URL to `git`, which keeps its own progress output and
its own credential prompt. fx does not splice the token into the URL: that would
write it into `.git/config`, where it outlives the command and travels with the
checkout. Use `--ssh` to clone over SSH instead.

## Pull requests

Inside a checkout, nothing needs to be spelled out — the repository comes from
the git remote and the pull request from the branch you are on:

```bash
cd ~/project
fx pr list                       # open pull requests in this repository
fx pr view                       # the one for the current branch
fx pr create --fill              # title and body from the branch's commits
fx pr merge -m squash            # merge it, squashed
```

```
NUMBER  TITLE            STATE  BRANCHES           AUTHOR  UPDATED
#12     feat: add OAuth  open   feat/oauth → main  whw     3d ago
```

Explicitly, for CI and agents:

```bash
fx --agent pr list -R ai/backend --state all --limit 50
fx --agent pr view 12
fx --agent pr create -B main -H feat/oauth -t "feat: add OAuth" -b "Closes #4"
fx --agent pr merge 12 -m squash --delete-branch
```

Reviewing one:

```bash
fx pr diff              # the raw patch, for your pager
fx pr diff --name-only  # just what changed
fx pr checks            # what CI says, and what is blocking the merge
fx pr checkout 12       # fetch the branch and switch to it
```

`fx pr diff` picks its form from the output mode: a person gets the unified
patch their pager and highlighter understand, `--agent` gets it split by file
with per-file patches (`--name-only` omits them). The endpoint content-
negotiates, so neither form is reassembled from the other.

Two flags worth knowing:

* `fx pr merge --dry-run` answers "would this merge?" without merging, which is
  the question an agent actually wants before it does anything.
* `fx pr list --author whw` takes a login. GitFox filters by numeric principal
  id, so fx resolves the name for you.

## CI

```bash
fx pipeline list            # every pipeline's latest run, one request
fx pipeline view            # the newest run, with its stage/step tree
fx pipeline logs --failed   # only the steps that failed
fx pipeline logs --failed --tail 50
fx pipeline retry           # run it again
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
fx --agent pipeline logs --failed
```

```json
{ "ok": true, "data": { "run": 182, "status": "failure", "count": 1,
  "steps": [ { "stage": "build", "step": "cargo test", "exit_code": 101,
               "lines": ["error[E0308]: mismatched types", "..."] } ] } }
```

Inside a checkout with one pipeline, nothing needs naming: the pipeline is
inferred, and the run defaults to the most recent. A green run answers with an
empty `steps` and exit 0 — "nothing failed" is a result, not an error.

A failed build's log is mostly progress output and the reason is at the end, so
`--tail N` keeps that end. Each step reports `total_lines` alongside `lines`, so
nothing is dropped silently.

## Shell completion

```bash
fx completion zsh  > ~/.zfunc/_fx
fx completion bash > /usr/local/etc/bash_completion.d/fx
fx completion fish > ~/.config/fish/completions/fx.fish
```

## `fx api` — the escape hatch

Anything GitFox exposes is reachable on day one, whether or not a dedicated
command exists yet.

```bash
fx api GET /api/v1/user
fx api POST /api/v1/foo --field name=test --field count=3
fx api POST /api/v1/foo --body '{"name":"test"}'
cat payload.json | fx api POST /api/v1/foo --input -
fx api GET /api/v1/user --include          # status + headers too
```

* The method is optional: `fx api /api/v1/user` is a `GET`, and adding a body
  without naming a method makes it a `POST`.
* `--field` types its values (`count=3` is a number, `draft=true` a boolean,
  `parent=null` a null); `--raw-field` always sends a string.

## Configuration

Resolved through one precedence chain, top to bottom:

```
CLI flag  >  environment variable  >  config file  >  git context  >  default
```

The git tier only speaks for remotes that point at the resolved GitFox host. A
checkout of some other host has a perfectly good `owner/name` that means nothing
to this instance, so fx says it could not infer a repository rather than asking
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

`$GITFOX_CONFIG`, else `$XDG_CONFIG_HOME/fx/config.toml`, else
`~/.config/fx/config.toml`.

```toml
default_host = "git.example.com"

[hosts."git.example.com"]
api_url = "https://git.example.com"
user = "whw"

[hosts."git.internal.local"]
api_url = "https://git.internal.local"
insecure = true
```

Tokens are never written here. `fx config set` cannot even address a token key.

### Where a token comes from

```
--token  >  GITFOX_TOKEN  >  OS keychain
```

`fx auth status` reports which of the three was used — never the value.

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

### Lists never lie about being complete

Every list carries `count` and `truncated`:

```json
{ "ok": true, "data": { "count": 30, "truncated": true, "items": [ … ] } }
```

GitFox publishes no pagination headers, so fx asks for one row more than you
requested — receiving it is proof there are more. `truncated` is therefore an
observation, not a guess. Raise `--limit` and fx walks the pages for you, at up
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

## Exit codes

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | unexpected internal error |
| `2` | invalid arguments |
| `3` | authentication error |
| `4` | not found |
| `5` | API error |
| `6` | network error or timeout |
| `7` | configuration error |
| `8` | git context error |
| `9` | not implemented yet |

Full table with the matching `error.code` strings: [docs/exit-codes.md](docs/exit-codes.md).
The JSON each command returns: [docs/json-schema.md](docs/json-schema.md).

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
               fx CLI                  fx MCP  (v0.7)
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
│   └── auth.rs · repo.rs · pull_request.rs · principal.rs · pipeline.rs
└── fx-cli/           the binary
    ├── cli.rs        the clap command tree
    ├── config.rs     the precedence chain (pure, heavily tested)
    ├── context.rs    resolved config + renderer + client
    ├── output.rs     Render trait, envelopes, tables
    ├── error.rs      stable error codes and exit codes
    ├── git.rs        what the surrounding checkout says
    ├── keychain.rs   OS keychain access
    ├── paginate.rs   walking page/limit endpoints
    └── commands/     one module per command
```

Two rules keep this from rotting:

* **The client knows nothing about the CLI.** That is what lets `fx-mcp` reuse
  it in v0.7 rather than shelling out to this binary.
* **The CLI's JSON schema is its own.** Commands map API responses onto models
  the CLI owns, so a GitFox API change does not have to break an agent.

## Roadmap

| Version | Scope |
|---|---|
| **v0.1** ✅ | workspace, client, config chain, env vars, auth, `fx api`, JSON output, error model |
| **v0.2** ✅ | `repo list/view/clone`, git remote detection, `-R`, multi-host |
| **v0.3** ✅ | `pr list/view/create/merge` — the first genuinely daily-usable release |
| **v0.4** ✅ | `pipeline list/view/logs/run/retry`, including `logs --failed` |
| **v0.5** ✅ | `pr checkout/diff/checks`, `pr create --fill`, shell completion |
| **v0.6** ✅ | agent hardening: pagination, retries, non-interactive edges, schema freeze |
| [v0.7](https://github.com/haowei2000/gitfox-cli/milestone/1) | `fx-mcp`, reusing `gitfox-client` directly |
| [v1.0](https://github.com/haowei2000/gitfox-cli/milestone/2) | CLI syntax, JSON schema, config format and exit codes all stable; published to crates.io |

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
  (`crates/fx-cli/src/*.rs`)
* HTTP tests against a mock GitFox covering 401/403/404/500, timeouts,
  non-JSON bodies and empty responses (`crates/gitfox-client/tests/http.rs`)
* end-to-end tests over the real binary asserting the JSON envelope, the exit
  codes, and that a token never reaches stdout or stderr — not even under
  `-vvv` (`crates/fx-cli/tests/cli.rs`)

## Security

* Tokens live in the OS keychain or in the environment, never in the config file.
* `Secret` redacts itself in `Debug` and `Display`; the `Authorization` header is
  marked sensitive so it stays out of logs.
* `fx auth status` reports `"token": "configured"` and its source, never the value.
* `--insecure` works, and warns on stderr every time it does.

## License

MIT — see [LICENSE](LICENSE).
