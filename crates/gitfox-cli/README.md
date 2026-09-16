# gitfox-cli — `gf`

A [GitFox](https://github.com/harness/gitness) client for humans, CI and AI
agents. Installs a single binary called `gf` — named `fx` up to 0.6; upgrading
keeps your login, config and per-checkout default, and only needs `gf auth
setup-git` re-run.

```bash
cargo install gitfox-cli
```

```bash
gf pr list                                    # you
GITFOX_TOKEN=$TOKEN gf --agent pipeline list  # CI
gf --agent pr list                            # an agent
```

`--agent` is shorthand for `--output json --non-interactive --no-color`. In that
mode every command answers with a stable envelope and a stable exit code, so
nothing has to be parsed out of prose:

```json
{ "ok": true, "data": { "count": 30, "truncated": true, "items": [] } }
```

## Quick start

```bash
export GITFOX_HOST=https://git.example.com
export GITFOX_TOKEN=xxxxxxxx
gf api GET /api/v1/user
```

Or log in interactively and let the token live in the OS keychain:

```bash
gf auth login --hostname git.example.com
```

Inside a checkout, nothing needs spelling out — the repository comes from the
git remote and the pull request from the branch you are on:

```bash
gf pr list
gf pr view
gf pr create --fill
gf pipeline logs --failed --tail 50
```

## Coming from gh

gf takes gh's commands, flags and exit codes: `gf pr checks && ./deploy.sh`,
`gf run view --log-failed`, `gf pr list --json number,title --jq '.[].title'`.
A gh command for something GitFox does not have — issues, releases, gists —
answers exit 9 with the reason rather than failing to parse.

## Why it suits an agent

* One JSON envelope and one error-code table, both documented and stable.
* Lists report `truncated`, observed rather than guessed, so "did I see
  everything" always has an answer.
* `gf api` reaches any endpoint, so a missing command never blocks anything.
* `gf pipeline logs --failed` turns "why is CI red" from several requests and a
  wall of output into one command.

## Documentation

Full README, the JSON contract and the exit-code table live in the
[repository](https://github.com/haowei2000/gitfox-cli).

## License

MIT
