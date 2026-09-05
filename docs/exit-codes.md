# Exit codes and error codes

Both are part of the public contract. They do not change within a major version;
new codes may be added, existing ones are never renumbered or renamed.

## Exit codes

| Exit | Meaning |
|---|---|
| `0` | success |
| `1` | unexpected internal error |
| `2` | invalid arguments (also what clap returns for a bad command line) |
| `3` | authentication error |
| `4` | not found |
| `5` | API error |
| `6` | network error or timeout |
| `7` | configuration error |
| `8` | git context error |
| `9` | not implemented yet |

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
| `INVALID_ARGUMENT` | 2 | a flag, field or body was malformed |
| `API_ERROR` | 5 | any other non-success HTTP status, or an undecodable body |
| `NETWORK_ERROR` | 6 | DNS, TLS or connection failure |
| `TIMEOUT` | 6 | the request exceeded `--timeout` / `GITFOX_TIMEOUT` |
| `RATE_LIMITED` | 6 | the server answered 429; `details.retry_after_secs` when it said |
| `CONFIG_ERROR` | 7 | missing host, unreadable config, bad environment value |
| `GIT_CONTEXT_ERROR` | 8 | the current directory is not a usable GitFox checkout |
| `NOT_IMPLEMENTED` | 9 | the command is on the roadmap; `details.planned_version` says when |
| `UNEXPECTED` | 1 | a bug in fx |

## Using them

```bash
fx --agent pr view 123
case $? in
  0) : ;;                      # got it
  3) echo "log in first" ;;
  4) echo "no such pull request" ;;
  6) echo "retry later" ;;
esac
```

An agent should branch on `error.code` rather than on the message, which is
written for people and may be reworded.

```bash
fx --agent pr view 123 | jq -r 'if .ok then .data.state else .error.code end'
```
