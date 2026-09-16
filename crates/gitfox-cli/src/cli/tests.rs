use clap::{CommandFactory, Parser};

use super::*;

fn parse(argv: &[&str]) -> Cli {
    Cli::try_parse_from(argv).unwrap_or_else(|e| panic!("{argv:?} should parse: {e}"))
}

fn rejects(argv: &[&str]) -> clap::Error {
    match Cli::try_parse_from(argv) {
        Ok(cli) => panic!("{argv:?} should be rejected, parsed as {cli:?}"),
        Err(e) => e,
    }
}

#[test]
fn command_tree_is_well_formed() {
    Cli::command().debug_assert();
}

/// A local flag that reuses a global flag's id, long name or short letter
/// parses into the wrong type at runtime — clap's own checks miss it when the
/// clash is between a subcommand and a propagated global.
#[test]
fn no_subcommand_flag_shadows_a_global_one() {
    let mut root = Cli::command();
    root.build();
    let globals: Vec<&clap::Arg> = root.get_arguments().filter(|a| a.is_global_set()).collect();

    fn walk(
        command: &clap::Command,
        path: &str,
        globals: &[&clap::Arg],
        clashes: &mut Vec<String>,
    ) {
        for arg in command.get_arguments().filter(|a| !a.is_global_set()) {
            for global in globals {
                let same_id = arg.get_id() == global.get_id();
                let same_long = arg.get_long().is_some() && arg.get_long() == global.get_long();
                let same_short = arg.get_short().is_some() && arg.get_short() == global.get_short();
                if same_id || same_long || same_short {
                    clashes.push(format!(
                        "{path}: `{}` vs global `{}`",
                        arg.get_id(),
                        global.get_id()
                    ));
                }
            }
        }
        for sub in command.get_subcommands() {
            walk(sub, &format!("{path} {}", sub.get_name()), globals, clashes);
        }
    }

    let mut clashes = Vec::new();
    for sub in root.get_subcommands() {
        walk(
            sub,
            &format!("gf {}", sub.get_name()),
            &globals,
            &mut clashes,
        );
    }
    assert!(clashes.is_empty(), "{clashes:#?}");
}

#[test]
fn json_flag_is_shorthand_for_output_json() {
    let cli = parse(&["gf", "--json", "api", "/api/v1/user"]);
    assert_eq!(cli.global.overrides().output, Some(OutputFormat::Json));
}

#[test]
fn json_with_fields_is_an_export_that_leaves_the_format_alone() {
    let cli = parse(&["gf", "pr", "list", "--json=number,title"]);
    assert_eq!(cli.global.json.as_deref(), Some("number,title"));
    assert_eq!(cli.global.overrides().output, None);
}

#[test]
fn output_flag_and_json_flag_are_mutually_exclusive() {
    rejects(&["gf", "--json", "--output", "table", "api", "/x"]);
}

#[test]
fn global_flags_are_accepted_before_and_after_the_subcommand() {
    for argv in [
        vec![
            "gf",
            "--host",
            "https://git.example.com",
            "api",
            "/api/v1/user",
        ],
        vec![
            "gf",
            "api",
            "/api/v1/user",
            "--host",
            "https://git.example.com",
        ],
    ] {
        let cli = parse(&argv);
        assert_eq!(cli.global.host.as_deref(), Some("https://git.example.com"));
    }
}

#[test]
fn api_accepts_a_bare_path_or_a_method_and_a_path() {
    let bare = parse(&["gf", "api", "/api/v1/user"]);
    let Command::Api(args) = bare.command else {
        panic!("expected the api subcommand")
    };
    assert_eq!(args.method_or_path, "/api/v1/user");
    assert!(args.path.is_none());

    let explicit = parse(&["gf", "api", "POST", "/api/v1/foo"]);
    let Command::Api(args) = explicit.command else {
        panic!("expected the api subcommand")
    };
    assert_eq!(args.method_or_path, "POST");
    assert_eq!(args.path.as_deref(), Some("/api/v1/foo"));
}

#[test]
fn api_short_flags_mean_what_they_mean_in_gh() {
    // gh: -F/--field is typed, -f/--raw-field is a string, -X is the method.
    let cli = parse(&[
        "gf",
        "api",
        "-X",
        "POST",
        "/x",
        "-F",
        "count=3",
        "-f",
        "name=x",
        "--paginate",
        "-q",
        ".",
    ]);
    let Command::Api(args) = cli.command else {
        panic!("expected api")
    };
    assert_eq!(args.method.as_deref(), Some("POST"));
    assert_eq!(args.method_or_path, "/x");
    assert_eq!(args.fields, vec!["count=3"]);
    assert_eq!(args.raw_fields, vec!["name=x"]);
    assert!(args.paginate);
    assert_eq!(args.jq.as_deref(), Some("."));
}

#[test]
fn api_body_sources_are_mutually_exclusive() {
    rejects(&["gf", "api", "POST", "/x", "--body", "{}", "--input", "-"]);
    // Fields beside a body are the query string, as with gh's --input.
    parse(&["gf", "api", "POST", "/x", "--body", "{}", "--field", "a=b"]);
    parse(&["gf", "api", "POST", "/x", "--input", "-", "-f", "a=b"]);
}

#[test]
fn fast_forward_keeps_its_hyphen_on_the_command_line() {
    let cli = parse(&["gf", "pr", "merge", "12", "--method", "fast-forward"]);
    let Command::Pr(cmd) = cli.command else {
        panic!("expected pr")
    };
    let PrSubcommand::Merge(args) = cmd.command else {
        panic!("expected merge")
    };
    assert_eq!(args.strategy(), MergeMethod::FastForward);
    assert_eq!(
        gitfox_client::MergeMethod::from(args.strategy()).as_str(),
        "fast-forward"
    );
}

#[test]
fn pr_merge_takes_ghs_strategy_flags() {
    for (flag, expected) in [
        ("--squash", MergeMethod::Squash),
        ("-s", MergeMethod::Squash),
        ("--rebase", MergeMethod::Rebase),
        ("-r", MergeMethod::Rebase),
        ("--merge", MergeMethod::Merge),
        ("-m", MergeMethod::Merge),
    ] {
        let cli = parse(&["gf", "pr", "merge", "12", flag, "-d"]);
        let Command::Pr(cmd) = cli.command else {
            panic!("expected pr")
        };
        let PrSubcommand::Merge(args) = cmd.command else {
            panic!("expected merge")
        };
        assert_eq!(args.strategy(), expected, "{flag}");
        assert!(args.delete_branch, "-d is --delete-branch, as in gh");
    }
    // Two strategies at once is a contradiction, as in gh.
    rejects(&["gf", "pr", "merge", "12", "--squash", "--rebase"]);
    // -D still works for anyone used to gf 0.6.
    let cli = parse(&["gf", "pr", "merge", "12", "-D"]);
    let Command::Pr(cmd) = cli.command else {
        panic!("expected pr")
    };
    let PrSubcommand::Merge(args) = cmd.command else {
        panic!("expected merge")
    };
    assert!(args.delete_branch);
}

#[test]
fn state_all_expands_to_every_state() {
    assert_eq!(PrState::All.expand().len(), 3);
    assert_eq!(
        PrState::Open.expand(),
        vec![gitfox_client::PullRequestState::Open]
    );
}

#[test]
fn pr_create_follows_the_gh_short_flag_convention() {
    let cli = parse(&[
        "gf",
        "pr",
        "create",
        "-B",
        "main",
        "-H",
        "feat/oauth",
        "-t",
        "feat: add OAuth",
        "-b",
        "body",
        "-r",
        "alice,bob",
        "-l",
        "bug",
        "-d",
    ]);
    let Command::Pr(cmd) = cli.command else {
        panic!("expected the pr subcommand")
    };
    let PrSubcommand::Create(args) = cmd.command else {
        panic!("expected pr create")
    };
    assert_eq!(args.base.as_deref(), Some("main"));
    assert_eq!(args.head.as_deref(), Some("feat/oauth"));
    assert_eq!(args.title.as_deref(), Some("feat: add OAuth"));
    assert_eq!(args.body.as_deref(), Some("body"));
    assert_eq!(args.reviewers, vec!["alice", "bob"]);
    assert_eq!(args.labels, vec!["bug"]);
    assert!(args.draft);
    // -f is --fill, as in gh.
    let cli = parse(&["gf", "pr", "create", "-f"]);
    let Command::Pr(cmd) = cli.command else {
        panic!("expected pr")
    };
    let PrSubcommand::Create(args) = cmd.command else {
        panic!("expected create")
    };
    assert!(args.fill);
}

#[test]
fn pr_list_takes_ghs_filters() {
    let cli = parse(&[
        "gf", "pr", "list", "-A", "whw", "-B", "main", "-H", "feat", "-l", "bug", "-S", "oauth",
        "-d", "-s", "all", "-L", "5",
    ]);
    let Command::Pr(cmd) = cli.command else {
        panic!("expected pr")
    };
    let PrSubcommand::List(args) = cmd.command else {
        panic!("expected list")
    };
    assert_eq!(args.author.as_deref(), Some("whw"));
    assert_eq!(args.base.as_deref(), Some("main"));
    assert_eq!(args.head.as_deref(), Some("feat"));
    assert_eq!(args.labels, vec!["bug"]);
    assert_eq!(args.search.as_deref(), Some("oauth"));
    assert!(args.draft);
    assert_eq!(args.state, PrState::All);
    assert_eq!(args.limit, 5);
}

#[test]
fn a_pull_request_can_be_named_by_number_url_or_branch() {
    for selector in [
        "12",
        "#12",
        "https://git.example.com/ai/backend/pulls/12",
        "feat/oauth",
    ] {
        let cli = parse(&["gf", "pr", "view", selector]);
        let Command::Pr(cmd) = cli.command else {
            panic!("expected pr")
        };
        let PrSubcommand::View(args) = cmd.command else {
            panic!("expected view")
        };
        assert_eq!(args.target.selector.as_deref(), Some(selector));
    }
}

#[test]
fn pr_has_a_pull_request_alias_and_co_is_pr_checkout() {
    parse(&["gf", "pull-request", "list"]);
    parse(&["gf", "pr", "ls"]);
    let cli = parse(&["gf", "co", "12", "--detach"]);
    let Command::Co(args) = cli.command else {
        panic!("expected co")
    };
    assert_eq!(args.target.selector.as_deref(), Some("12"));
    assert!(args.detach);
}

#[test]
fn agent_flag_is_global() {
    let cli = parse(&["gf", "--agent", "pr", "list"]);
    assert!(cli.global.agent);
}

#[test]
fn dash_h_is_the_hostname_on_auth_commands_and_help_is_long_only() {
    for sub in ["login", "logout", "status", "token", "switch", "setup-git"] {
        let cli = parse(&["gf", "auth", sub, "-h", "git.example.com"]);
        let Command::Auth(cmd) = cli.command else {
            panic!("expected auth")
        };
        let hostname = match cmd.command {
            AuthSubcommand::Login(a) => a.hostname,
            AuthSubcommand::Logout(a) => a.hostname,
            AuthSubcommand::Status(a) => a.hostname,
            AuthSubcommand::Token(a) => a.hostname,
            AuthSubcommand::Switch(a) => a.hostname,
            AuthSubcommand::SetupGit(a) => a.hostname,
            AuthSubcommand::GitCredential(_) | AuthSubcommand::Refresh(_) => None,
        };
        assert_eq!(hostname.as_deref(), Some("git.example.com"), "{sub}");
    }
    let help = rejects(&["gf", "auth", "login", "--help"]);
    assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
}

#[test]
fn dash_h_is_the_host_scope_on_config_commands() {
    let cli = parse(&[
        "gf",
        "config",
        "get",
        "-h",
        "git.example.com",
        "git_protocol",
    ]);
    let Command::Config(cmd) = cli.command else {
        panic!("expected config")
    };
    let ConfigSubcommand::Get(args) = cmd.command else {
        panic!("expected get")
    };
    assert_eq!(args.scope.as_deref(), Some("git.example.com"));
    assert_eq!(args.key, "git_protocol");
    parse(&["gf", "config", "clear-cache"]);
}

#[test]
fn completion_takes_the_shell_positionally_or_with_dash_s() {
    parse(&["gf", "completion", "zsh"]);
    let cli = parse(&["gf", "completion", "-s", "bash"]);
    let Command::Completion(args) = cli.command else {
        panic!("expected completion")
    };
    assert_eq!(args.shell_flag, Some(clap_complete::Shell::Bash));
    rejects(&["gf", "completion"]);
}

#[test]
fn repo_clone_passes_git_flags_after_a_double_dash() {
    let cli = parse(&[
        "gf",
        "repo",
        "clone",
        "ai/backend",
        "dir",
        "--",
        "--depth",
        "1",
    ]);
    let Command::Repo(cmd) = cli.command else {
        panic!("expected repo")
    };
    let RepoSubcommand::Clone(args) = cmd.command else {
        panic!("expected clone")
    };
    assert_eq!(args.git_flags, vec!["--depth", "1"]);
}

#[test]
fn run_and_workflow_mirror_ghs_commands() {
    parse(&[
        "gf", "run", "list", "-w", "default", "-b", "main", "-s", "failure", "-L", "5",
    ]);
    parse(&["gf", "run", "view", "182", "--log-failed", "--exit-status"]);
    parse(&["gf", "run", "view", "default/182", "-j", "build"]);
    parse(&["gf", "run", "rerun", "182", "--failed"]);
    parse(&["gf", "run", "watch", "182", "--exit-status", "-i", "5"]);
    parse(&["gf", "run", "cancel", "182"]);
    parse(&["gf", "run", "delete", "182"]);
    parse(&["gf", "workflow", "list", "--all"]);
    parse(&["gf", "workflow", "view", "default", "--yaml", "-r", "main"]);
    parse(&["gf", "workflow", "run", "default", "--ref", "main"]);
    parse(&["gf", "workflow", "enable", "default"]);
    parse(&["gf", "workflow", "disable", "default"]);
}

#[test]
fn every_gh_command_gf_supports_parses() {
    for argv in [
        vec!["gf", "pr", "close", "12", "-c", "bye", "-d"],
        vec!["gf", "pr", "reopen", "12", "-c", "back"],
        vec!["gf", "pr", "ready", "12", "--undo"],
        vec![
            "gf",
            "pr",
            "edit",
            "12",
            "-t",
            "x",
            "--add-reviewer",
            "a",
            "--remove-label",
            "bug",
        ],
        vec!["gf", "pr", "comment", "12", "-b", "hi"],
        vec!["gf", "pr", "review", "12", "--approve", "-b", "LGTM"],
        vec!["gf", "pr", "status", "--json=number"],
        vec!["gf", "pr", "update-branch", "12", "--rebase"],
        vec![
            "gf",
            "pr",
            "checks",
            "12",
            "--watch",
            "--fail-fast",
            "-i",
            "5",
            "--required",
        ],
        vec![
            "gf", "pr", "diff", "12", "--color", "never", "--patch", "-e", "*.lock",
        ],
        vec![
            "gf",
            "pr",
            "checkout",
            "12",
            "-b",
            "local",
            "-f",
            "--recurse-submodules",
        ],
        vec![
            "gf",
            "repo",
            "create",
            "ai/new",
            "--private",
            "--add-readme",
            "-g",
            "Rust",
            "-c",
        ],
        vec!["gf", "repo", "delete", "ai/old", "--yes"],
        vec![
            "gf",
            "repo",
            "edit",
            "ai/x",
            "-d",
            "desc",
            "--visibility",
            "public",
            "--accept-visibility-change-consequences",
        ],
        vec!["gf", "repo", "rename", "new", "--yes"],
        vec!["gf", "repo", "fork", "ai/x", "--clone", "--fork-name", "y"],
        vec!["gf", "repo", "sync", "--force", "-b", "main"],
        vec!["gf", "repo", "set-default", "ai/x"],
        vec!["gf", "repo", "set-default", "--view"],
        vec![
            "gf",
            "repo",
            "read-file",
            "README.md",
            "--ref",
            "main",
            "-o",
            "out.md",
        ],
        vec!["gf", "repo", "read-dir", "src"],
        vec!["gf", "repo", "gitignore", "list"],
        vec!["gf", "repo", "license", "view", "mit"],
        vec![
            "gf",
            "repo",
            "list",
            "--visibility",
            "public",
            "--source",
            "-S",
            "back",
        ],
        vec!["gf", "repo", "view", "-w"],
        vec!["gf", "secret", "list", "-o", "ai"],
        vec!["gf", "secret", "set", "TOKEN", "-b", "value"],
        vec!["gf", "secret", "set", "-f", ".env"],
        vec!["gf", "secret", "delete", "TOKEN"],
        vec!["gf", "secret", "remove", "TOKEN"],
        vec!["gf", "label", "list", "-S", "bug", "--sort", "name"],
        vec![
            "gf",
            "label",
            "create",
            "bug",
            "-c",
            "d73a4a",
            "-d",
            "Something is broken",
        ],
        vec!["gf", "label", "edit", "bug", "-n", "defect"],
        vec!["gf", "label", "delete", "bug", "--yes"],
        vec!["gf", "label", "clone", "ai/other", "--force"],
        vec!["gf", "ssh-key", "list"],
        vec!["gf", "ssh-key", "add", "key.pub", "-t", "laptop"],
        vec!["gf", "ssh-key", "delete", "laptop", "-y"],
        vec!["gf", "org", "list", "-L", "5"],
        vec!["gf", "ruleset", "list"],
        vec!["gf", "rs", "view", "CI_Check"],
        vec!["gf", "codespace", "list"],
        vec!["gf", "cs", "stop", "-c", "dev"],
        vec!["gf", "browse", "12", "-n"],
        vec!["gf", "browse", "src/main.rs:10", "-b", "main"],
        vec!["gf", "browse", "--commit"],
        vec!["gf", "status", "-o", "ai"],
        vec!["gf", "alias", "set", "prs", "pr list"],
        vec!["gf", "alias", "list"],
        vec!["gf", "alias", "delete", "--all"],
        vec!["gf", "auth", "token"],
        vec!["gf", "auth", "status", "-t", "--json=hosts"],
    ] {
        parse(&argv);
    }
}

#[test]
fn a_gh_command_gitfox_lacks_parses_whatever_follows_it() {
    // `-h` included: it must reach the command, not print help and exit 0.
    for argv in [
        vec!["gf", "issue", "list", "--state", "open", "-h", "host"],
        vec!["gf", "release", "create", "v1.0", "--notes", "x"],
        vec!["gf", "search", "prs", "--author", "@me"],
        vec![
            "gf",
            "auth",
            "refresh",
            "-h",
            "git.example.com",
            "-s",
            "repo",
        ],
        vec!["gf", "pr", "lock", "12"],
        vec!["gf", "repo", "deploy-key", "list"],
        vec!["gf", "run", "download", "182", "-n", "artifact"],
        vec!["gf", "codespace", "ssh", "-c", "dev"],
        vec!["gf", "ruleset", "check", "main"],
        vec!["gf", "ext", "install", "owner/gh-thing"],
    ] {
        parse(&argv);
    }
    let cli = parse(&["gf", "issue", "view", "3", "-h", "x"]);
    let Command::GhOnly(GhOnlyCommand::Issue(args)) = cli.command else {
        panic!("expected issue")
    };
    assert_eq!(args.rest, vec!["view", "3", "-h", "x"]);
}

#[test]
fn jq_and_template_short_flags_are_per_command() {
    let cli = parse(&["gf", "pr", "list", "--json=number", "-q", ".[]"]);
    assert_eq!(cli.command.format().jq.as_deref(), Some(".[]"));
    let cli = parse(&["gf", "repo", "view", "--json=name", "-t", "{{.name}}"]);
    assert_eq!(cli.command.format().template.as_deref(), Some("{{.name}}"));
    // `pr create -t` is still the title.
    let cli = parse(&["gf", "pr", "create", "-t", "title"]);
    assert!(cli.command.format().template.is_none());
}
