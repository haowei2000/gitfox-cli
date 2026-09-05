# gitfox-cli — `fx`

A [GitFox](https://github.com/harness/gitness) client for humans, CI and AI
agents. Installs a single binary called `fx`.

```bash
cargo install gitfox-cli
```

```bash
fx pr list                                    # you
GITFOX_TOKEN=$TOKEN fx --agent pipeline list  # CI
fx --agent pr list                            # an agent
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
fx api GET /api/v1/user
```

Or log in interactively and let the token live in the OS keychain:

```bash
fx auth login --hostname git.example.com
```

Inside a checkout, nothing needs spelling out — the repository comes from the
git remote and the pull request from the branch you are on:

```bash
fx pr list
fx pr view
fx pr create --fill
fx pipeline logs --failed --tail 50
```

## Why it suits an agent

* One JSON envelope and one error-code table, both documented and stable.
* Lists report `truncated`, observed rather than guessed, so "did I see
  everything" always has an answer.
* `fx api` reaches any endpoint, so a missing command never blocks anything.
* `fx pipeline logs --failed` turns "why is CI red" from several requests and a
  wall of output into one command.

## Documentation

Full README, the JSON contract and the exit-code table live in the
[repository](https://github.com/haowei2000/gitfox-cli).

## License

MIT
