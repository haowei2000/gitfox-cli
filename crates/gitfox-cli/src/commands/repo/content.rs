//! Reading a repository without cloning it: `read-file`, `read-dir`, and the
//! gitignore and license templates GitFox offers at creation.

use gitfox_client::{Content, RepoRef, encode_base64};
use serde_json::{Value, json};

use crate::cli::{
    RepoGitignoreCommand, RepoLicenseCommand, RepoReadDirArgs, RepoReadFileArgs, TemplateSubcommand,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export;
use crate::interact;
use crate::output::{Render, plain_table};

// ---------------------------------------------------------------------------
// read-file
// ---------------------------------------------------------------------------

pub async fn read_file(args: RepoReadFileArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let content = fetch_content(ctx, &repo, &args.path, args.git_ref.as_deref()).await?;
    if content.is_dir() {
        return Err(
            CliError::invalid_argument(format!("`{}` is a directory", args.path))
                .with_hint("use `fx repo read-dir` to list it"),
        );
    }
    let bytes = content.bytes().ok_or_else(|| {
        CliError::new(
            ErrorCode::ApiError,
            format!(
                "`{}` is a {}, which has no contents to read",
                args.path, content.kind
            ),
        )
    })?;

    if let Some(output) = &args.output {
        if output.exists() && !args.clobber {
            return Err(
                CliError::invalid_argument(format!("{} already exists", output.display()))
                    .with_hint("pass --clobber to overwrite it"),
            );
        }
        std::fs::write(output, &bytes).map_err(|e| {
            CliError::new(ErrorCode::Unexpected, format!("{}: {e}", output.display()))
        })?;
        return ctx.renderer.emit(&FileWritten {
            path: content.path.clone(),
            destination: output.display().to_string(),
            bytes: bytes.len(),
        });
    }

    let file = FileRead {
        api_url: client
            .resolve(&format!(
                "/api/v1/repos/{}/content/{}",
                repo.encoded(),
                gitfox_client::repo::encode_path(&content.path)
            ))
            .map(|u| u.to_string())
            .unwrap_or_default(),
        raw_url: client
            .resolve(&format!(
                "/api/v1/repos/{}/raw/{}",
                repo.encoded(),
                gitfox_client::repo::encode_path(&content.path)
            ))
            .map(|u| u.to_string())
            .unwrap_or_default(),
        html_url: ctx
            .web_url(&format!(
                "{}/files/{}/~/{}",
                repo.full(),
                args.git_ref.as_deref().unwrap_or("HEAD"),
                content.path
            ))
            .unwrap_or_default(),
        git_ref: args.git_ref.clone(),
        content,
        bytes,
    };
    if ctx.renderer.is_machine() {
        return ctx.renderer.emit(&file);
    }
    // For a person, the file itself — byte for byte, so it can be redirected.
    ctx.renderer.write_bytes(&file.bytes)
}

async fn fetch_content(
    ctx: &Context,
    repo: &RepoRef,
    path: &str,
    git_ref: Option<&str>,
) -> Result<Content> {
    let client = ctx.client()?;
    client
        .repos()
        .content(repo, path, git_ref)
        .await
        .map_err(|e| match e {
            gitfox_client::Error::NotFound { .. } => CliError::new(
                ErrorCode::NotFound,
                format!(
                    "no `{}` in {repo}{}",
                    if path.is_empty() { "/" } else { path },
                    git_ref.map(|r| format!(" at {r}")).unwrap_or_default()
                ),
            ),
            other => CliError::from(other),
        })
}

struct FileRead {
    content: Content,
    bytes: Vec<u8>,
    git_ref: Option<String>,
    api_url: String,
    raw_url: String,
    html_url: String,
}

const FILE_FIELDS: &[&str] = &[
    "content",
    "downloadUrl",
    "encoding",
    "gitSHA",
    "gitUrl",
    "htmlUrl",
    "name",
    "path",
    "size",
    "type",
    "url",
];

impl Render for FileRead {
    fn to_json(&self) -> Value {
        let (encoding, text) = match std::str::from_utf8(&self.bytes) {
            Ok(text) => ("utf-8", text.to_string()),
            Err(_) => ("base64", encode_base64(&self.bytes)),
        };
        json!({
            "path": self.content.path,
            "name": self.content.name,
            "ref": self.git_ref,
            "sha": self.content.sha,
            "size": self.bytes.len(),
            "encoding": encoding,
            "content": text,
        })
    }

    fn to_human(&self, _color: bool) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        FILE_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        export::select(fields, |field| match field {
            // As the contents API serves it: base64, with the encoding named.
            "content" => json!(encode_base64(&self.bytes)),
            "encoding" => json!("base64"),
            "downloadUrl" => json!(self.raw_url),
            "gitSHA" => json!(self.content.sha),
            "gitUrl" => json!(""),
            "htmlUrl" => json!(self.html_url),
            "name" => json!(self.content.name),
            "path" => json!(self.content.path),
            "size" => json!(self.bytes.len()),
            "type" => json!(self.content.kind),
            "url" => json!(self.api_url),
            _ => Value::Null,
        })
    }
}

struct FileWritten {
    path: String,
    destination: String,
    bytes: usize,
}

impl Render for FileWritten {
    fn to_json(&self) -> Value {
        json!({ "path": self.path, "written_to": self.destination, "size": self.bytes })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Wrote {} ({} bytes) to {}",
            self.path, self.bytes, self.destination
        )
    }
}

// ---------------------------------------------------------------------------
// read-dir
// ---------------------------------------------------------------------------

pub async fn read_dir(args: RepoReadDirArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let path = args.path.clone().unwrap_or_default();
    let content = fetch_content(ctx, &repo, &path, args.git_ref.as_deref()).await?;
    if !content.is_dir() {
        return Err(
            CliError::invalid_argument(format!("`{path}` is not a directory"))
                .with_hint("use `fx repo read-file` to print it"),
        );
    }
    ctx.renderer.emit(&DirListing {
        path: content.path.clone(),
        git_ref: args.git_ref.clone(),
        content,
    })
}

struct DirListing {
    path: String,
    git_ref: Option<String>,
    content: Content,
}

const DIR_FIELDS: &[&str] = &[
    "gitSHA",
    "gitType",
    "mode",
    "modeOctal",
    "name",
    "nameRaw",
    "path",
    "pathRaw",
    "size",
    "submodule",
    "type",
];

impl Render for DirListing {
    fn to_json(&self) -> Value {
        let entries = self.content.entries();
        json!({
            "path": self.path,
            "ref": self.git_ref,
            "count": entries.len(),
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.content
            .entries()
            .iter()
            .map(|e| json!({ "name": e.name, "path": e.path, "type": e.kind, "sha": e.sha }))
            .collect()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        DIR_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.content
                .entries()
                .iter()
                .map(|e| {
                    export::select(fields, |field| match field {
                        "gitSHA" => json!(e.sha),
                        "gitType" => json!(match e.kind.as_str() {
                            "dir" => "tree",
                            "submodule" => "commit",
                            _ => "blob",
                        }),
                        "mode" | "modeOctal" => json!(""),
                        "name" | "nameRaw" => json!(e.name),
                        "path" | "pathRaw" => json!(e.path),
                        "size" => Value::Null,
                        "submodule" => json!(e.kind == "submodule"),
                        "type" => json!(e.kind),
                        _ => Value::Null,
                    })
                })
                .collect(),
        )
    }

    fn to_human(&self, _color: bool) -> String {
        let mut entries = self.content.entries();
        if entries.is_empty() {
            return format!(
                "{} is empty",
                if self.path.is_empty() {
                    "/"
                } else {
                    &self.path
                }
            );
        }
        // Directories first, as a file browser lists them.
        entries.sort_by(|a, b| (a.kind != "dir", &a.name).cmp(&(b.kind != "dir", &b.name)));
        let rows: Vec<Vec<String>> = entries
            .iter()
            .map(|e| {
                vec![
                    e.kind.clone(),
                    if e.kind == "dir" {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    },
                ]
            })
            .collect();
        plain_table(&["type", "name"], &rows)
    }
}

// ---------------------------------------------------------------------------
// gitignore / license templates
// ---------------------------------------------------------------------------

pub async fn gitignore(cmd: RepoGitignoreCommand, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let names = client.repos().gitignore_templates().await?;
    match cmd.command {
        TemplateSubcommand::List => ctx.renderer.emit(&Templates {
            items: names.into_iter().map(|n| (n.clone(), n)).collect(),
            kind: "gitignore",
        }),
        TemplateSubcommand::View(view) => {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&view.name)) {
                return Err(CliError::new(
                    ErrorCode::NotFound,
                    format!("no gitignore template `{}`", view.name),
                )
                .with_hint("see `fx repo gitignore list`"));
            }
            Err(CliError::unsupported(
                "viewing a gitignore template",
                "GitFox lists its templates but does not serve their contents",
            )
            .with_hint(format!(
                "`fx repo create NAME --gitignore {}` applies it to a new repository",
                view.name
            )))
        }
    }
}

pub async fn license(cmd: RepoLicenseCommand, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let licenses = client.repos().license_templates().await?;
    match cmd.command {
        TemplateSubcommand::List => ctx.renderer.emit(&Templates {
            items: licenses
                .into_iter()
                .filter(|l| l.value != "none")
                .map(|l| (l.value, l.label))
                .collect(),
            kind: "license",
        }),
        TemplateSubcommand::View(view) => {
            let found = licenses
                .iter()
                .find(|l| l.value.eq_ignore_ascii_case(&view.name))
                .ok_or_else(|| {
                    CliError::new(ErrorCode::NotFound, format!("no license `{}`", view.name))
                        .with_hint("see `fx repo license list`")
                })?;
            if view.web {
                return interact::open_in_browser(
                    ctx,
                    &format!("https://choosealicense.com/licenses/{}/", found.value),
                );
            }
            Err(CliError::unsupported(
                "viewing a license's text",
                "GitFox lists its licenses but does not serve their text",
            )
            .with_hint(format!(
                "`fx repo license view {} --web`, or `fx repo create NAME --license {}`",
                found.value, found.value
            )))
        }
    }
}

struct Templates {
    /// (key, display name)
    items: Vec<(String, String)>,
    kind: &'static str,
}

impl Render for Templates {
    fn to_json(&self) -> Value {
        json!({
            "count": self.items.len(),
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.items
            .iter()
            .map(|(key, name)| json!({ "key": key, "name": name }))
            .collect()
    }

    fn to_human(&self, _color: bool) -> String {
        if self.items.is_empty() {
            return format!("GitFox offers no {} templates", self.kind);
        }
        if self.kind == "gitignore" {
            return self
                .items
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>()
                .join("\n");
        }
        let rows: Vec<Vec<String>> = self
            .items
            .iter()
            .map(|(key, name)| vec![key.clone(), name.clone()])
            .collect();
        plain_table(&["license key", "name"], &rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> Content {
        serde_json::from_value(json!({
            "type": "dir", "path": "",
            "content": { "entries": [
                { "type": "file", "name": "README.md", "path": "README.md", "sha": "1" },
                { "type": "dir", "name": "src", "path": "src", "sha": "2" }
            ] }
        }))
        .unwrap()
    }

    #[test]
    fn a_directory_lists_directories_first() {
        let listing = DirListing {
            path: String::new(),
            git_ref: None,
            content: dir(),
        };
        let text = listing.to_human(false);
        let src = text.find("src/").unwrap();
        let readme = text.find("README.md").unwrap();
        assert!(src < readme, "{text}");
        assert_eq!(listing.to_json()["count"], 2);
        let exported = listing.export(&["name".into(), "gitType".into()]);
        assert_eq!(exported[1], json!({ "gitType": "tree", "name": "src" }));
    }

    #[test]
    fn a_file_reads_as_text_or_base64() {
        let content: Content = serde_json::from_value(json!({
            "type": "file", "name": "a.txt", "path": "a.txt", "sha": "3",
            "content": { "data": "aGk=", "encoding": "base64" }
        }))
        .unwrap();
        let read = FileRead {
            bytes: content.bytes().unwrap(),
            content,
            git_ref: Some("main".into()),
            api_url: "u".into(),
            raw_url: "r".into(),
            html_url: "h".into(),
        };
        assert_eq!(read.to_json()["content"], "hi");
        assert_eq!(read.to_json()["encoding"], "utf-8");
        assert_eq!(
            read.export(&["content".into(), "encoding".into()]),
            json!({ "content": "aGk=", "encoding": "base64" })
        );
        assert_eq!(read.to_human(false), "hi");
    }
}
