# Exit codes and error codes

Both are part of the public contract. They do not change within a major version;
new codes may be added, existing ones are never renumbered or renamed.

## Exit codes

| Exit | Meaning |
|---|---|
| `0` | success |
| `1` | unexpected internal error — or, as in gh, a failed check (`fx pr checks`) or run (`--exit-status`) |
| `2` | invalid arguments (also what clap returns for a bad command line), or a declined confirmation |
| `3` | authentication error |
| `4` | not found |
| `5` | API error |
| `6` | network error or timeout |
| `7` | configuration error |
| `8` | git context error — or, as in gh, checks still pending (`fx pr checks`) |
| `9` | not supported: a gh command or flag whose feature GitFox does not have |

Exits 1 and 8 each carry two meanings so that a script ported from gh keeps
working: `gh pr checks` exits 1 when a check failed and 8 while checks are
pending, and so does `fx pr checks`. `error.code` always tells them apart.

## Error codes

`error.code` in the JSON envelope. Each maps to exactly one exit code.

| `error.code` | Exit | Raised when |
|---|---|---|
| `AUTH_REQUIRED` | 3 | no token was found for the resolved host |
| `AUTH_FAILED` | 3 | the token was rejected (HTTP 401/403) |
| `NOT_FOUND` | 4 | the server answered 404 |
| `REPO_NOT_FOUND` | 4 | the named repository does not exist |
| `PR_NOT_FOUND` | 4 | the named pull request does not exist |
| `PIPELINE_NOT_FOUND` | 4 | the named pipeline or run does not exist |
| `INVALID_ARGUMENT` | 2 | a flag, field or body was malformed, or an unknown `--json` field was named |
| `CANCELLED` | 2 | a confirmation was declined, or an editor exited without saving |
| `API_ERROR` | 5 | any other non-success HTTP status, or an undecodable body |
| `NETWORK_ERROR` | 6 | DNS, TLS or connection failure |
| `TIMEOUT` | 6 | the request exceeded `--timeout` / `GITFOX_TIMEOUT` |
| `RATE_LIMITED` | 6 | the server answered 429; `details.retry_after_secs` when it said |
| `CONFIG_ERROR` | 7 | missing host, unreadable config, bad environment value |
| `GIT_CONTEXT_ERROR` | 8 | the current directory is not a usable GitFox checkout |
| `CHECKS_FAILED` | 1 | `fx pr checks`, human output: at least one check failed |
| `CHECKS_PENDING` | 8 | `fx pr checks`, human output: checks are still running |
| `RUN_FAILED` | 1 | `fx run view` / `fx run watch` / `fx pipeline view` with `--exit-status`, human output: the run failed |
| `UNSUPPORTED` | 9 | a gh command or flag GitFox has no feature for; the message says why |
| `NOT_IMPLEMENTED` | 9 | reserved; no command raises it today |
| `UNEXPECTED` | 1 | a bug in fx |

`CHECKS_FAILED`, `CHECKS_PENDING` and `RUN_FAILED` are raised only for human
output. With `--json`, `--output json`/`jsonl` or `--agent`, the checks or the
run are the result — exit 0, read `failed` / `status` from the data — because a
second, error document after the result would break "stdout is one JSON
document". gh behaves the same way with `--json`.

For a person, those three print nothing on stderr: the table already said it.

## Using them

```bash
fx --agent pr view 123
case $? in
  0) : ;;                      # got it
  3) echo "log in first" ;;
  4) echo "no such pull request" ;;
  6) echo "retry later" ;;
  9) echo "GitFox cannot do that" ;;
esac
```

```bash
# Gate a deploy on CI, exactly as with gh.
fx pr checks 123 && ./deploy.sh
```

An agent should branch on `error.code` rather than on the message, which is
written for people and may be reworded.

```bash
fx --agent pr view 123 | jq -r 'if .ok then .data.state else .error.code end'
```
