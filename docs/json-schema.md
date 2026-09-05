# The machine contract

What `--output json` and `--output jsonl` promise. Everything here is stable
within a major version: fields may be **added**, but existing field names,
types and meanings do not change, and neither do the strings in `error.code` or
the process exit codes.

Turn it on with `--json`, `--output json`, `--agent`, or `GITFOX_AGENT=1`.

## The envelope

Success — exactly one JSON document on stdout, exit code 0:

```json
{ "ok": true, "data": { } }
```

Failure — exactly one JSON document on **stdout**, non-zero exit code:

```json
{ "ok": false, "error": { "code": "AUTH_REQUIRED", "message": "…", "details": null } }
```

`error.details` is always present and may be `null`. Human-facing hints are
written to stderr and are deliberately **not** part of this contract.

Branch on `error.code`, not on `error.message`: messages are written for people
and get reworded. The full table is in [exit-codes.md](exit-codes.md).

### `jsonl`

`--output jsonl` writes one bare JSON value per line — the elements of `items`,
without the envelope. Built for `jq`, `xargs` and line-oriented tooling. A
failure still emits a single enveloped error object.

## Lists

Every list command returns the same outer shape:

| Field | Type | Meaning |
|---|---|---|
| `count` | integer | How many items are in `items` |
| `truncated` | boolean | Whether the server had **more** than `--limit` allowed through |
| `items` | array | The rows |

`truncated` is observed, not guessed: fx asks for one more item than you
requested, so receiving it is proof more exist. When it is `true`, raise
`--limit` (fx pages transparently, at up to 100 rows per request).

`fx pipeline list` without `--pipeline` pages over *pipelines*, each
contributing its most recent run, so `count` can be lower than `--limit`
without anything being hidden — a pipeline that has never run has no row.

## `data` by command

### `fx repo list` / `fx repo view`

`items[]`, and the whole of `data` for `view`:

```json
{
  "repository": "ai/backend",
  "name": "backend",
  "description": "…",
  "default_branch": "main",
  "visibility": "private",
  "is_public": false,
  "is_empty": false,
  "open_pull_requests": 2,
  "size_kib": 1024,
  "git_url": "…", "git_ssh_url": "…",
  "created": 1756000000000, "updated": 1756000000000
}
```

`visibility` and `is_public` are `null` when the endpoint did not report them —
which is the case for the instance-wide `fx repo list`. Unknown, not private.
`fx repo list <space>` and `fx repo view` always report it.

`fx repo list` also carries `space`, which is `null` when the listing spanned
the whole instance.

### `fx pr list` / `view` / `create`

`items[]`, and the whole of `data` for `view` and `create`:

```json
{
  "number": 12,
  "title": "feat: add OAuth",
  "description": "…",
  "state": "open",
  "is_draft": false,
  "author": { "id": 7, "uid": "whw", "display_name": "Haowei", "email": "…" },
  "source_branch": "feat/oauth",
  "target_branch": "main",
  "created": 1756000000000, "updated": 1756000000000,
  "merged": null, "closed": null,
  "web_url": "…",
  "stats": { "commits": 3, "files_changed": 5, "additions": 120, "deletions": 8 },
  "merge_check_status": "mergeable",
  "merge_conflicts": [],
  "merge_method": null
}
```

`state` is `open`, `closed` or `merged`. A draft is `state: "open"` with
`is_draft: true`; the human table shows it as `draft`.

`fx pr list` also carries `repository`.

### `fx pr merge`

```json
{
  "number": 12, "title": "…",
  "source_branch": "feat/oauth", "target_branch": "main",
  "dry_run": false, "merged": true, "mergeable": null,
  "sha": "0123456789abcdef", "branch_deleted": true,
  "conflict_files": [], "allowed_methods": ["merge", "squash"]
}
```

With `--dry-run`, `merged` is `false` and `mergeable` answers the question.

### `fx pipeline list` / `view` / `run` / `retry`

`items[]`, and the base of `data` for the others:

```json
{
  "pipeline": "default",
  "number": 182,
  "status": "failure",
  "branch": "main",
  "message": "feat: add OAuth",
  "author": "whw",
  "event": "push",
  "commit": "0123456789abcdef",
  "created": 1756000000000, "started": 1756000000000, "finished": null,
  "error": null, "link": "…"
}
```

`status` is GitFox's own word, passed through unchanged. Today's set is
`blocked`, `declined`, `error`, `failure`, `killed`, `pending`, `running`,
`skipped`, `success`, `waiting_on_dependencies` — **treat it as an open set**
and match on the values you care about rather than assuming these are all of
them. `error`, `failure` and `killed` are the ones fx treats as failed.

`fx pipeline view` adds `stages`:

```json
{ "stages": [ { "number": 1, "name": "build", "status": "failure", "error": null,
  "steps": [ { "number": 2, "name": "cargo test", "status": "failure",
               "exit_code": 101, "error": null } ] } ] }
```

### `fx pipeline logs`

```json
{
  "pipeline": "default", "run": 182, "status": "failure",
  "only_failed": true, "count": 1,
  "steps": [
    { "stage": "build", "stage_number": 1,
      "step": "cargo test", "step_number": 2,
      "status": "failure", "exit_code": 101, "error": null,
      "total_lines": 1658,
      "lines": ["error[E0308]: mismatched types", "error: aborting"] }
  ]
}
```

`total_lines` is what the step produced; `lines` is what came back after
`--tail`. They differ only when `--tail` was given, so truncation is never
silent. `lines` have their trailing newlines stripped. A step whose log could not be
fetched still appears, with its status and exit code and an empty `lines`.

A green run with `--failed` is **not** an error: `steps` is empty, `count` is
`0`, and the exit code is `0`. Check `count`.

### `fx api`

`data` is the endpoint's response body, untouched. With `--include` it becomes
`{ "status": 200, "headers": { … }, "body": … }`.

A response that is not JSON (a diff, a log) arrives as a JSON string.

### `fx auth status`

```json
{ "host": "…", "host_key": "git.example.com", "user": "whw",
  "authenticated": true, "token": "configured",
  "token_source": "env", "insecure": false }
```

`token` is always the literal `"configured"`. The value is never emitted
anywhere, in any mode. `token_source` is `flag`, `env` or `keyring`.

## Timestamps

Epoch integers, exactly as GitFox sends them — this CLI does not reinterpret
them. Instances have been observed sending milliseconds; if you need a date,
treat a value beyond ~1e11 as milliseconds. Human output does this for you.
