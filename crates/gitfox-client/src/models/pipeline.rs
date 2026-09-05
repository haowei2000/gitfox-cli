use serde::{Deserialize, Serialize};

/// A CI status.
///
/// Deliberately a string rather than an enum. GitFox's set today is `blocked`,
/// `declined`, `error`, `failure`, `killed`, `pending`, `running`, `skipped`,
/// `success` and `waiting_on_dependencies`, but a CI system grows states over
/// time; a closed enum would either lose the server's exact word or fail to
/// decode. Everything `fx` needs is classification, and that is what the
/// predicates below provide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct CiStatus(pub String);

impl CiStatus {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Something went wrong and there is likely a log worth reading.
    ///
    /// `declined` and `skipped` are excluded on purpose: they never ran, so
    /// `fx pipeline logs --failed` would only show empty output for them.
    pub fn is_failed(&self) -> bool {
        matches!(self.0.as_str(), "failure" | "error" | "killed")
    }

    pub fn is_success(&self) -> bool {
        self.0 == "success"
    }

    /// Still going, or waiting to.
    pub fn is_pending(&self) -> bool {
        matches!(
            self.0.as_str(),
            "running" | "pending" | "blocked" | "waiting_on_dependencies"
        )
    }

    /// Ran to some conclusion, so it may have produced output.
    pub fn has_run(&self) -> bool {
        !matches!(
            self.0.as_str(),
            "pending" | "skipped" | "blocked" | "waiting_on_dependencies"
        )
    }
}

impl std::fmt::Display for CiStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for CiStatus {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// One step of a stage — the unit logs are addressed by.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: CiStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub stopped: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stage {
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: CiStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub stopped: Option<i64>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// One run of a pipeline.
///
/// `stages` is populated by the single-execution endpoint and empty in list
/// responses, which is why `fx pipeline logs` fetches the execution before it
/// can find anything to fetch logs for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Execution {
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub status: CiStatus,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author_login: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub pipeline_uid: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub finished: Option<i64>,
    #[serde(default)]
    pub stages: Vec<Stage>,
}

impl Execution {
    /// The branch this run is about.
    ///
    /// A push sets `target`; other events fall back to `source`, then to the
    /// raw ref with its `refs/heads/` prefix trimmed.
    pub fn branch(&self) -> Option<String> {
        self.target
            .clone()
            .filter(|b| !b.is_empty())
            .or_else(|| self.source.clone().filter(|b| !b.is_empty()))
            .or_else(|| {
                self.git_ref.as_deref().map(|r| {
                    r.strip_prefix("refs/heads/")
                        .or_else(|| r.strip_prefix("refs/tags/"))
                        .unwrap_or(r)
                        .to_string()
                })
            })
            .filter(|b| !b.is_empty())
    }

    /// The commit subject, or the run title, whichever is there.
    pub fn summary(&self) -> String {
        let raw = self
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| self.message.clone())
            .unwrap_or_default();
        raw.lines().next().unwrap_or_default().trim().to_string()
    }

    pub fn author(&self) -> String {
        self.author_login
            .clone()
            .filter(|a| !a.is_empty())
            .or_else(|| self.author_name.clone())
            .unwrap_or_default()
    }

    /// Every step in the run, tagged with the stage it belongs to.
    pub fn steps(&self) -> impl Iterator<Item = (&Stage, &Step)> {
        self.stages
            .iter()
            .flat_map(|stage| stage.steps.iter().map(move |step| (stage, step)))
    }
}

/// A pipeline definition. `execution` is its latest run, present only when the
/// list was asked for with `latest=true`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pipeline {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default)]
    pub config_path: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
    #[serde(default)]
    pub execution: Option<Execution>,
}

/// One line of step output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogLine {
    #[serde(default)]
    pub pos: i64,
    #[serde(default)]
    pub out: String,
    #[serde(default)]
    pub time: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_classification_matches_the_documented_set() {
        for failed in ["failure", "error", "killed"] {
            assert!(CiStatus::from(failed).is_failed(), "{failed}");
        }
        // Never ran, so `--failed` must not offer to show their empty logs.
        for not_failed in ["declined", "skipped", "success", "running", "pending"] {
            assert!(!CiStatus::from(not_failed).is_failed(), "{not_failed}");
        }
        assert!(CiStatus::from("running").is_pending());
        assert!(CiStatus::from("waiting_on_dependencies").is_pending());
        assert!(CiStatus::from("success").is_success());
        assert!(CiStatus::from("failure").has_run());
        assert!(!CiStatus::from("skipped").has_run());
    }

    #[test]
    fn an_unknown_status_survives_instead_of_failing_to_decode() {
        let status: CiStatus = serde_json::from_str("\"quarantined\"").unwrap();
        assert_eq!(status.as_str(), "quarantined");
        assert!(!status.is_failed() && !status.is_success());
        // And it round-trips back out unchanged.
        assert_eq!(serde_json::to_string(&status).unwrap(), "\"quarantined\"");
    }

    #[test]
    fn branch_prefers_target_then_source_then_the_ref() {
        let mut e = Execution {
            target: Some("main".into()),
            source: Some("feat/x".into()),
            git_ref: Some("refs/heads/other".into()),
            ..Default::default()
        };
        assert_eq!(e.branch().as_deref(), Some("main"));
        e.target = None;
        assert_eq!(e.branch().as_deref(), Some("feat/x"));
        e.source = None;
        assert_eq!(e.branch().as_deref(), Some("other"));
        e.git_ref = Some("refs/tags/v1.0".into());
        assert_eq!(e.branch().as_deref(), Some("v1.0"));
        e.git_ref = None;
        assert_eq!(e.branch(), None);
    }

    #[test]
    fn empty_strings_are_treated_as_absent() {
        let e = Execution {
            target: Some(String::new()),
            source: Some("feat/x".into()),
            ..Default::default()
        };
        assert_eq!(e.branch().as_deref(), Some("feat/x"));
    }

    #[test]
    fn summary_takes_the_first_line_only() {
        let e = Execution {
            message: Some("feat: add OAuth\n\nA long body.".into()),
            ..Default::default()
        };
        assert_eq!(e.summary(), "feat: add OAuth");
    }

    #[test]
    fn steps_are_walked_with_their_stage() {
        let execution: Execution = serde_json::from_value(json!({
            "number": 182,
            "status": "failure",
            "stages": [
                { "number": 1, "name": "build", "status": "failure", "steps": [
                    { "number": 1, "name": "clone", "status": "success" },
                    { "number": 2, "name": "cargo test", "status": "failure", "exit_code": 101 }
                ]},
                { "number": 2, "name": "deploy", "status": "skipped", "steps": [] }
            ]
        }))
        .unwrap();

        let all: Vec<_> = execution.steps().collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].0.name, "build");
        assert_eq!(all[1].1.name, "cargo test");

        let failed: Vec<_> = execution
            .steps()
            .filter(|(_, s)| s.status.is_failed())
            .collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].1.exit_code, Some(101));
    }

    #[test]
    fn a_list_response_without_stages_still_decodes() {
        let e: Execution =
            serde_json::from_value(json!({ "number": 1, "status": "success" })).unwrap();
        assert!(e.stages.is_empty());
        assert_eq!(e.steps().count(), 0);
    }
}
