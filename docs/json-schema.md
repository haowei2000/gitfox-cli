# The machine contract

What `--output json` and `--output jsonl` promise, and what gh-style
`--json FIELDS` prints. Everything here is stable within a major version:
fields may be **added**, but existing field names, types and meanings do not
change, and neither do the strings in `error.code` or the process exit codes.

Turn the envelope on with `--json`, `--output json`, `--agent`, or
`GITFOX_AGENT=1`.

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

## gh-style output: `--json FIELDS`, `--jq`, `--template`

Given field names, `--json` prints what gh prints: the resource, or an array of
them, restricted to those fields — **no envelope**:

```bash
fx pr list --json number,title,headRefName
```

```json
[{"headRefName":"feat/oauth","number":12,"title":"feat: add OAuth"}]
```

Output is one line when stdout is not a terminal and indented when it is, as
with gh. With `--output jsonl`, an array streams one object per line.

* **Field names** are gh's (`headRefName`, `createdAt`, `isDraft`, …) with gh's
  value shapes: uppercase states (`OPEN`), ISO 8601 times, `{"login": …}`
  actors. Each command also accepts the snake_case keys of its own envelope
  (`source_branch`, `created`, …) where gh has no name for the same thing. A gh
  field GitFox has no data for exports gh's empty value — `[]`, `""`, `false` or
  `null` — so a script selecting it keeps working.
* **An unknown field** fails with `INVALID_ARGUMENT` and
  `details.available` listing every field the command exports.
* **`--jq EXPR`** runs a jq program over the selected fields (jq is built in;
  none is needed on PATH). Strings print raw, everything else as JSON.
* **`--template TMPL`** renders a Go template with gh's helpers: `autocolor`,
  `color`, `join`, `pluck`, `tablerow`, `tablerender`, `timeago`, `timefmt`,
  `truncate`, `hyperlink`.
* `--jq` and `--template` need `--json FIELDS`, and exclude each other.
* Errors still follow the resolved format: with `--agent` they are the error
  envelope on stdout; otherwise a message on stderr.

Bare `--json` keeps its original meaning — `fx --json pr list` is the envelope —
and so does `--json` on a command without the table below: a word after it
stays an argument, so `fx pipeline run --json default` runs `default`.

| Command | Fields |
|---|---|
| `fx pr list` / `view` | gh's pull request fields (`additions` … `url`) plus `description`, `is_draft`, `source_branch`, `target_branch`, `created`, `updated`, `merged`, `web_url`, `stats`, `merge_check_status`, `merge_conflicts`, `merge_method`, `source_sha`, `merger`, `check_summary` |
| `fx pr status` | the pull request fields, as `{"currentBranch": …, "createdBy": […], "needsReview": […]}` |
| `fx pr checks` | `bucket`, `completedAt`, `description`, `event`, `link`, `name`, `startedAt`, `state`, `workflow`, plus `status`, `required`, `summary` |
| `fx repo list` / `view` | gh's repository fields (`archivedAt` … `watchers`) plus `repository`, `default_branch`, `is_public`, `is_empty`, `open_pull_requests`, `size_kib`, `git_url`, `git_ssh_url`, `created`, `updated` |
| `fx repo read-file` | `content` (base64), `downloadUrl`, `encoding`, `gitSHA`, `gitUrl`, `htmlUrl`, `name`, `path`, `size`, `type`, `url` |
| `fx repo read-dir` | `gitSHA`, `gitType`, `mode`, `modeOctal`, `name`, `nameRaw`, `path`, `pathRaw`, `size`, `submodule`, `type` |
| `fx run list` / `view` | `attempt`, `conclusion`, `createdAt`, `databaseId`, `displayTitle`, `event`, `headBranch`, `headSha`, `jobs` (the stages, which `view` fills in), `name`, `number`, `startedAt`, `status`, `updatedAt`, `url`, `workflowDatabaseId`, `workflowName`, plus `pipeline`, `branch`, `message`, `author`, `commit`, `created`, `started`, `finished` |
| `fx workflow list` | `id`, `name`, `path`, `state`, plus `identifier`, `description`, `disabled`, `config_path`, `default_branch`, `created`, `updated` |
| `fx secret list` | `name`, `numSelectedRepos`, `selectedReposURL`, `updatedAt`, `visibility`, plus `identifier`, `description`, `created`, `updated` |
| `fx label list` | `color` (hex), `createdAt`, `description`, `id`, `isDefault`, `name`, `updatedAt`, `url`, plus `key`, `type`, `scope`, `value_count` |
| `fx codespace list` / `view` | `createdAt`, `displayName`, `gitStatus`, `lastUsedAt`, `machineName`, `name`, `owner`, `repository`, `state`, `vscsTarget`, plus `identifier`, `branch`, `ide`, `url`; `view` also takes gh's `billableOwner`, `devcontainerPath`, `environmentId`, `idleTimeoutMinutes`, `location`, `machineDisplayName`, `prebuild`, `recentFolders`, `retentionExpiresAt`, `retentionPeriodDays` |
| `fx auth status` | `hosts` |

A few gh vocabularies, mapped:

* **Run `status` / `conclusion`**: GitFox `running` → `in_progress`,
  `pending` → `queued`, `blocked`/`waiting_on_dependencies` → `waiting`, anything
  finished → `completed`. `conclusion` is `success`, `failure` (from `failure` or
  `error`), `cancelled` (from `killed`) or `skipped` (from `skipped` or
  `declined`), and `""` until the run finishes.
* **Check `state` / `bucket`**: `success` → `SUCCESS`/`pass`; `failure` →
  `FAILURE`/`fail`; `error` → `ERROR`/`fail`; `running` → `IN_PROGRESS`/`pending`;
  `pending`, `blocked`, `waiting_on_dependencies` → `PENDING`/`pending`;
  `skipped`, `declined` → `SKIPPED`/`skipping`; `killed` → `CANCELLED`/`cancel`.
* **Run `event`**: GitFox `manual` → `workflow_dispatch`, `cron` → `schedule`.
* **Pull request `mergeable`**: `mergeable` → `MERGEABLE`, `conflict` →
  `CONFLICTING`, otherwise `UNKNOWN`.

## Lists

Every list command returns the same outer shape:

| Field | Type | Meaning |
|---|---|---|
| `count` | integer | How many items are in `items` |
| `truncated` | boolean | Whether the server had **more** than `--limit` allowed through |
| `items` | array | The rows |

`truncated` is observed, not guessed: fx asks for one more item than you
requested, so receiving it is proof more exist. When it is `true`, raise
`--limit` (fx pages transparently, at up to 100 rows per request). A filter the
server cannot apply — `fx pr list --draft`, `fx run list --status` — reads pages
until enough rows match, so `truncated` stays an observation.

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
  "is_fork": false,
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

### `fx repo create` / `edit` / `rename` / `fork` / `delete`

```json
{
  "action": "created",
  "repository": "ai/new",
  "web_url": "…", "git_url": "…", "git_ssh_url": "…",
  "visibility": "private", "default_branch": "main", "description": "…"
}
```

`action` is `created`, `edited`, `renamed`, `forked` or `deleted`. Extra keys by
action: `cloned_to` and `remote_added` (create, fork), `previous` and
`remote_updated` (rename), `source` (fork), `deleted_at` (delete — GitFox
deletes softly, and this is what a restore needs).

### `fx repo sync` / `set-default`

```json
{ "synced": "origin", "branch": "main", "detail": "…" }
{ "default_repository": "ai/backend", "changed": true }
```

`sync` with a repository argument syncs it on the server and `branch` is `null`.

### `fx repo read-file` / `read-dir`

```json
{ "path": "src/main.rs", "name": "main.rs", "ref": "main", "sha": "…",
  "size": 120, "encoding": "utf-8", "content": "fn main() {}" }
```

`encoding` is `utf-8` with the text in `content`, or `base64` for a binary
file. With `-o PATH` the file is written there instead and `data` is
`{ "path", "written_to", "size" }`. For a person, `read-file` prints the bytes
and nothing else.

`read-dir`: `{ "path", "ref", "count", "items": [ { "name", "path", "type", "sha" } ] }`.

### `fx repo gitignore list` / `license list`

`{ "count", "items": [ { "key", "name" } ] }`.

### `fx pr list` / `view` / `create` / `edit`

`items[]`, and the whole of `data` for `view`, `create` and `edit`:

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
  "source_sha": "…",
  "created": 1756000000000, "updated": 1756000000000,
  "merged": null, "closed": null,
  "web_url": "…",
  "stats": { "commits": 3, "files_changed": 5, "additions": 120, "deletions": 8 },
  "labels": [ { "id": 3, "key": "priority", "value": "high", "value_id": 9,
                "color": "red", "value_color": "orange", "scope": 0 } ],
  "check_summary": null,
  "merge_check_status": "mergeable",
  "merge_conflicts": [],
  "merge_method": null
}
```

`state` is `open`, `closed` or `merged`. A draft is `state: "open"` with
`is_draft: true`; the human table shows it as `draft`.

`fx pr list` also carries `repository`. `fx pr view --comments` adds
`comments: [ { "id", "author", "text", "created", "path" } ]`, `author` in the
shape above and `path` set only for a code comment. `fx pr create --dry-run` creates nothing and answers
`{ "dry_run": true, "repository", "title", "description", "source_branch",
"target_branch", "is_draft", "reviewers", "labels" }`.

### `fx pr close` / `reopen` / `ready`

```json
{ "number": 12, "title": "…", "state": "closed", "is_draft": false,
  "changed": true, "branch_deleted": false, "web_url": "…" }
```

`changed` is `false` when the pull request was already in that state — which is
not an error, as with gh.

### `fx pr comment` / `review` / `update-branch`

```json
{ "action": "added", "number": 12, "comment_id": 5, "text": "…", "web_url": "…" }
{ "number": 12, "decision": "approved", "commit_sha": "…", "commented": false }
{ "number": 12, "source_branch": "feat/oauth", "target_branch": "main",
  "already_up_to_date": false, "sha": "…" }
```

`action` is `added`, `edited` or `deleted`. `decision` is GitFox's word:
`approved`, `changereq` or `reviewed`.

### `fx pr status`

```json
{ "repository": "ai/backend", "current_branch": { … } ,
  "created_by": [ { … } ], "needs_review": [ { … } ] }
```

Each pull request has the `fx pr view` shape; `current_branch` is `null` off a
branch or without a pull request.

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

### `fx pr checkout` (and `fx co`)

```json
{ "number": 12, "title": "…", "branch": "feat/oauth", "remote": "origin",
  "created_branch": true, "detached": false, "worktree": null }
```

`branch` is `null` with `--detach`. For a pull request from a fork, `remote`
names the fork's repository, which is where the branch was fetched from.

### `fx pr diff`

```json
{ "number": 12, "source_branch": "…", "target_branch": "…", "count": 1,
  "files": [ { "path", "old_path", "status", "additions", "deletions",
               "changes", "is_binary", "is_submodule", "patch" } ] }
```

`--name-only` leaves `patch` null. `--patch` answers `{ "number", "patch" }`,
the pull request as `git format-patch` mails.

### `fx pr checks`

```json
{ "number": 12, "commit_sha": "…", "count": 2, "failed": true, "blocking": 1,
  "checks": [ { "name", "status", "required", "bypassable", "summary", "link",
                "started", "ended" } ] }
```

Exit 0 in every machine mode; see [exit-codes.md](exit-codes.md) for what a
person gets.

### `fx pipeline list` / `view` / `run` / `retry`, `fx run list` / `view` / `rerun` / `cancel`, `fx workflow run`

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

`fx pipeline view`, `fx run view` and `fx run watch` add `stages`; `fx run view`
and `fx run watch` also add `url`, the run's page:

```json
{ "stages": [ { "number": 1, "name": "build", "status": "failure", "error": null,
  "steps": [ { "number": 2, "name": "cargo test", "status": "failure",
               "exit_code": 101, "error": null } ] } ] }
```

`fx run delete` answers `{ "pipeline", "number", "deleted": true }`.

### `fx pipeline logs`, `fx run view --log` / `--log-failed`

```json
{
  "pipeline": "default", "run": 182, "status": "failure",
  "only_failed": true, "count": 1,
  "steps": [
    { "stage": "build", "stage_number": 1,
      "step": "cargo test", "step_number": 2,
      "status": "failure", "exit_code": 101, "error": null,
      "live": false, "log_available": true,
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

### `fx workflow list` / `view` / `enable` / `disable`

`items[]` for `list` (beside `repository`, `count` and `truncated`), and the
base of `view`:

```json
{ "identifier": "default", "id": 24, "description": "", "config_path": ".gitfox/default.yaml",
  "default_branch": "main", "disabled": false, "created": …, "updated": …, "web_url": "…" }
```

`view` adds `recent_runs`, each in the run shape above. `view --yaml` answers
`{ "pipeline", "path", "ref", "content" }` with the definition decoded.
`enable`/`disable` answer `{ "identifier", "disabled", "changed" }`.

### `fx secret list` / `set` / `delete`

```json
{ "space": "ai", "count": 1, "truncated": false,
  "items": [ { "identifier": "DEPLOY_TOKEN", "description": "…", "created": …, "updated": … } ] }
{ "space": "ai", "count": 1, "items": [ { "name": "DEPLOY_TOKEN", "action": "created" } ] }
{ "space": "ai", "name": "DEPLOY_TOKEN", "deleted": true }
```

A secret's value appears nowhere, in any mode: GitFox never returns one and fx
never echoes the one it sent. `action` is `created` or `updated`.

### `fx label list` / `create` / `edit` / `delete` / `clone`

```json
{ "id": 3, "key": "bug", "description": "…", "color": "red", "type": "static",
  "scope": "repository", "value_count": 0, "created": …, "updated": … }
```

`list` wraps these in `items`, beside `repository`, `count` and `truncated`.
`create` and `edit` add `action` — `created`, or `updated` for `edit` and for
`create --force` over an existing label — and `repository`. `color` is
GitFox's palette name here and hex in `--json color`. `scope` is `repository`
or `space`. `delete`: `{ "repository", "name", "deleted": true }`; `clone`:
`{ "source", "destination", "created", "updated", "skipped" }`, each of the last
three a list of label names.

### `fx ssh-key list` / `add` / `delete`

```json
{ "id": "laptop", "title": "laptop", "fingerprint": "SHA256:…", "type": "ssh-ed25519",
  "usage": "auth", "comment": "me@laptop", "created": …, "verified": … }
```

`list` wraps these as `{ "count", "items" }`; `add` answers the one key;
`delete` answers `{ "id", "deleted": true }`.

### `fx org list`

`{ "count", "truncated", "items": [ { "space", "identifier", "description", "is_public", "role" } ] }`.

### `fx ruleset list` / `view`

```json
{ "identifier": "CI_Check", "description": "", "type": "branch", "state": "disabled",
  "targets": ["default branch", "develop"], "pattern": { … }, "definition": { … },
  "created_by": "whw", "created": …, "updated": … }
```

`pattern` and `definition` are GitFox's own, passed through. `list` wraps
these in `items`, beside `repository`, `count` and `truncated`; `view` adds
`web_url`.

### `fx codespace list` / `view` / `create` / `stop` / `delete` / `logs`

```json
{ "identifier": "backend-1757000000", "name": "backend", "state": "running",
  "repository": "ai/backend", "branch": "main", "ide": "vs_code_web", "url": "…",
  "created": …, "updated": … }
```

`list` wraps these as `{ "count", "truncated", "items" }`. `create` and `stop`
add `action` (`created`, `stopped`); `delete` answers `{ "deleted": [ids] }`;
`logs` answers `{ "identifier", "count", "lines" }`. On an instance with
gitspaces switched off, every one of these fails with `UNSUPPORTED`.

### `fx status`

```json
{ "space": "ai", "repositories_scanned": 3,
  "review_requests": [ { "repository", "number", "title", "is_draft", "updated", "web_url" } ],
  "pull_requests": [ … ] }
```

### `fx browse`, and `--web` anywhere

`{ "url": "…", "opened": false }` — a machine gets the URL; a browser is only
opened for a person.

### `fx api`

`data` is the endpoint's response body, untouched. With `--include` it becomes
`{ "status": 200, "headers": { … }, "body": … }`. With `--paginate`, the pages'
arrays are merged into one (`--slurp`: an array of the pages). `--silent`
answers `null`. `--jq` and `--template` print their result directly, without
the envelope.

A response that is not JSON (a diff, a log) arrives as a JSON string.

Redirects are followed for `GET` and `HEAD` only. Any other method that the
server redirects fails with `API_ERROR`: `details.status` is the 3xx status and
`message` names where the server pointed. Following it could resend the write
as a `GET`, which changes nothing and still answers 200.

### `fx auth status` / `token` / `switch` / `setup-git`

```json
{ "host": "…", "host_key": "git.example.com", "user": "Haowei", "login": "whw",
  "authenticated": true, "token": "configured",
  "token_source": "env", "insecure": false, "git_protocol": "https" }
```

`token` is the literal `"configured"` — unless `--show-token` asked for the
value. `token_source` is `flag`, `env` or `keyring`.

`fx auth token` answers `{ "host_key", "token" }` (and prints only the token for
a person). These two are the only places a token is ever output, because that
is what they are for; see the README's security section.

`switch`: `{ "default_host", "previous" }`. `setup-git`: `{ "configured": [origins] }`.

### `fx alias`, `fx config`

`alias list`: `{ "count", "items": [ { "name", "expansion" } ] }`;
`alias set`: `{ "name", "expansion", "replaced" }`; `alias delete`:
`{ "deleted": [names] }`; `alias import`: `{ "imported", "skipped" }`.

`config get`/`set`: `{ "key", "value" }`. `config list` adds `git_protocol`,
`editor`, `browser`, `pager` and `prompt` beside `hosts` and `resolved`.
`config clear-cache`: `{ "cleared": true, "entries": 0 }`.

## Timestamps

Epoch integers, exactly as GitFox sends them — this CLI does not reinterpret
them. Instances have been observed sending milliseconds; if you need a date,
treat a value beyond ~1e11 as milliseconds. Human output does this for you, and
so does gh-style `--json FIELDS`, whose time fields are ISO 8601 strings.
