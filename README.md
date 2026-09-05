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

**v0.3 — pull requests work.** `fx pr list|view|create|merge` are implemented
against a verified GitFox API v1.3.0, on top of the v0.1 foundation
(configuration chain, client, `fx api`, `fx auth`, `fx config`, output system,
error and exit-code contract) and git remote detection pulled forward from v0.2.

Still on the roadmap: `fx repo list|view|clone` (v0.2), `fx pipeline` (v0.4),
and `fx pr checkout|diff|checks` (v0.5). Each returns a structured
`NOT_IMPLEMENTED` error naming its version, and `fx api` reaches every endpoint
in the meantime.

## Install

```bash
cargo install --path crates/fx-cli
```

On Linux the OS keychain integration links against D-Bus:

```bash
sudo apt install -y libdbus-1-dev pkg-config
```

CI and agents do not need it — they pass `GITFOX_TOKEN` and never touch the
keychain.

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

## Pull requests

Inside a checkout, nothing needs to be spelled out — the repository comes from
the git remote and the pull request from the branch you are on:

```bash
cd ~/project
fx pr list                       # open pull requests in this repository
fx pr view                       # the one for the current branch
fx pr create --fill              # title and body from the branch's commits
fx pr merge --squash-ish -m squash
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

Two flags worth knowing:

* `fx pr merge --dry-run` answers "would this merge?" without merging, which is
  the question an agent actually wants before it does anything.
* `fx pr list --author whw` takes a login. GitFox filters by numeric principal
  id, so fx resolves the name for you.

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

| Format | Success | Failure |
|---|---|---|
| `table` (default) | human text on stdout | message on **stderr** |
| `json` | one `{"ok":true,"data":…}` document on stdout | one `{"ok":false,"error":…}` document on **stdout** |
| `jsonl` | one bare JSON value per line on stdout | one enveloped error object on stdout |

In machine modes the whole contract is: stdout is one JSON document, and the
exit code says whether it is a result or an error.

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
| v0.2 ◐ | git remote detection and `-R` ✅; `repo list/view/clone` still to come |
| **v0.3** ✅ | `pr list/view/create/merge` — the first genuinely daily-usable release |
| v0.4 | `pipeline list/view/logs/retry`, including `logs --failed` |
| v0.5 | `pr checkout/diff/checks`, `pr create --fill`, shell completion, nicer tables |
| v0.6 | agent hardening: pagination, retries, non-interactive edges, schema freeze |
| v0.7 | `fx-mcp`, reusing `gitfox-client` directly |
| v1.0 | CLI syntax, JSON schema, config format and exit codes all stable |

The twelve commands v0.1–v0.4 aim to make rock solid: `auth login/logout/status`,
`api`, `repo list/view`, `pr list/view/create/merge`, `pipeline list/logs`.
Nine are done.

### Where the endpoints come from

Every GitFox instance serves its own OpenAPI document at `/openapi.yaml`,
unauthenticated. The endpoints, parameter names and response shapes this CLI
targets were read from there rather than guessed, and the module docs in
`crates/gitfox-client/src/` record which version they were checked against.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

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
