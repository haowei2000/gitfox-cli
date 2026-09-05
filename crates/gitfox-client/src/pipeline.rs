//! Pipeline / CI endpoints.
//!
//! Lands in **v0.4** together with `fx pipeline list|view|logs|retry`. The
//! endpoints this module will wrap:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list pipelines  | `GET /api/v1/repos/{repo_ref}/pipelines` |
//! | list executions | `GET /api/v1/repos/{repo_ref}/pipelines/{pipeline}/executions` |
//! | view execution  | `GET /api/v1/repos/{repo_ref}/pipelines/{pipeline}/executions/{n}` |
//! | step logs       | `GET /api/v1/repos/{repo_ref}/pipelines/{pipeline}/executions/{n}/logs/{stage}/{step}` |
//!
//! `fx pipeline logs --failed` is the reason this is on the roadmap early: it is
//! the single most useful thing an agent can call while fixing a red build.
