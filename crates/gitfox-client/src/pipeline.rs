//! Pipeline / CI endpoints.
//!
//! Lands in **v0.4** together with `fx pipeline list|view|logs|retry`. Verified
//! against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list pipelines  | `GET /api/v1/repos/{repo_ref}/pipelines` |
//! | list in space   | `GET /api/v1/spaces/{space_ref}/pipelines` |
//! | list executions | `GET /api/v1/repos/{repo_ref}/pipelines/{pipeline_identifier}/executions` |
//! | view execution  | `GET …/executions/{execution_number}` |
//! | step logs       | `GET …/executions/{execution_number}/logs/{stage_number}/{step_number}` |
//! | retry           | `POST …/executions/{execution_number}/retry` |
//! | cancel          | `POST …/executions/{execution_number}/cancel` |
//! | trigger a run   | `POST /api/v1/repos/{repo_ref}/pipelines/{pipeline_identifier}/executions` |
//!
//! Note the log endpoint is per *step*, addressed by stage and step number.
//! `fx pipeline logs --failed` therefore has to read the execution first, walk
//! its stages for steps whose status is not success, and fetch only those —
//! which is exactly why the flag is worth having: it turns an agent's "why is
//! CI red" from N requests plus a wall of output into one command.
