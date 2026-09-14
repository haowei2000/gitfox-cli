//! The parts of a command that talk to the person rather than to GitFox:
//! opening a browser, an editor, a confirmation, reading a body from a file.
//!
//! Each one knows it may be running with nobody there. A browser is not
//! launched for an agent, an editor is never opened non-interactively, and a
//! confirmation that cannot be asked is an error rather than an assumed "yes".

use std::io::{IsTerminal, Read};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::output::Render;

/// Open `url` in a browser, the way `--web` does in gh.
///
/// A machine caller gets the URL back instead of a browser window it cannot
/// see. The browser is the `browser` setting, then `$BROWSER`, then the
/// platform's opener.
pub fn open_in_browser(ctx: &Context, url: &str) -> Result<()> {
    let launch = !ctx.renderer.is_machine() && !ctx.config.agent;
    let opened = launch && launch_browser(ctx, url);
    if launch && !opened {
        ctx.warn(&format!("could not open a browser; visit {url}"));
    } else if opened {
        ctx.note(&format!("Opening {} in your browser.", display_url(url)));
    }
    ctx.renderer.emit(&WebPage {
        url: url.to_string(),
        opened,
    })
}

/// Report a URL without opening it — `fx browse --no-browser`.
pub fn print_url(ctx: &Context, url: &str) -> Result<()> {
    ctx.renderer.emit(&WebPage {
        url: url.to_string(),
        opened: false,
    })
}

fn display_url(url: &str) -> &str {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
}

fn launch_browser(ctx: &Context, url: &str) -> bool {
    let configured = ctx
        .config
        .browser
        .clone()
        .or_else(|| std::env::var("BROWSER").ok())
        .filter(|b| !b.trim().is_empty());

    let mut command = match configured {
        Some(browser) => {
            let words = crate::argv::split_words(&browser);
            let Some((program, args)) = words.split_first() else {
                return false;
            };
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        None if cfg!(target_os = "macos") => Command::new("open"),
        None if cfg!(windows) => {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        }
        None => Command::new("xdg-open"),
    };
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

struct WebPage {
    url: String,
    opened: bool,
}

impl Render for WebPage {
    fn to_json(&self) -> Value {
        json!({ "url": self.url, "opened": self.opened })
    }

    fn to_human(&self, _color: bool) -> String {
        // When the browser opened, stderr already said where; stdout stays
        // empty. When it did not, the URL is the result.
        if self.opened {
            String::new()
        } else {
            self.url.clone()
        }
    }
}

/// Edit `initial` in the user's editor and return what they saved.
///
/// The editor is the `editor` setting, then `$VISUAL`, then `$EDITOR`, then
/// `vi` (`notepad` on Windows).
pub fn edit_text(ctx: &Context, initial: &str, name: &str) -> Result<String> {
    ctx.require_interactive("an editor")?;
    let editor = ctx
        .config
        .editor
        .clone()
        .or_else(|| std::env::var("VISUAL").ok())
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("fx-{}-{nanos}-{name}", std::process::id()));
    std::fs::write(&path, initial)
        .map_err(|e| CliError::new(ErrorCode::Unexpected, format!("{}: {e}", path.display())))?;

    let words = crate::argv::split_words(&editor);
    let status = match words.split_first() {
        Some((program, args)) => Command::new(program).args(args).arg(&path).status(),
        None => {
            let _ = std::fs::remove_file(&path);
            return Err(CliError::config("the editor setting is empty"));
        }
    };
    let result = match status {
        Ok(status) if status.success() => std::fs::read_to_string(&path)
            .map_err(|e| CliError::new(ErrorCode::Unexpected, format!("{}: {e}", path.display()))),
        Ok(status) => Err(CliError::new(
            ErrorCode::Cancelled,
            format!("the editor exited with {status}; nothing was saved"),
        )),
        Err(e) => Err(CliError::config(format!("could not run `{editor}`: {e}"))),
    };
    let _ = std::fs::remove_file(&path);
    result
}

/// Ask a yes/no question. Not being able to ask is an error, never a "yes".
pub fn confirm(ctx: &Context, prompt: &str, flag: &str) -> Result<()> {
    if ctx.config.non_interactive {
        return Err(CliError::invalid_argument(format!(
            "{flag} is required when not running interactively"
        )));
    }
    let answer = dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .map_err(|e| CliError::invalid_argument(format!("could not read the answer: {e}")))?;
    if answer {
        Ok(())
    } else {
        Err(CliError::new(ErrorCode::Cancelled, "cancelled"))
    }
}

/// The text of `-F/--body-file`: a path, or `-` for stdin.
pub fn read_source(source: &str) -> Result<String> {
    if source == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::invalid_argument(format!("could not read stdin: {e}")))?;
        return Ok(buf);
    }
    std::fs::read_to_string(source)
        .map_err(|e| CliError::invalid_argument(format!("could not read {source}: {e}")))
}

/// `--body` or `--body-file`, whichever was given.
pub fn body_from(body: Option<&str>, body_file: Option<&str>) -> Result<Option<String>> {
    match (body, body_file) {
        (Some(body), _) => Ok(Some(body.to_string())),
        (None, Some(file)) => read_source(file).map(Some),
        (None, None) => Ok(None),
    }
}

/// Everything on stdin, when stdin is a pipe or a file rather than a terminal.
pub fn piped_stdin() -> Result<Option<String>> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| CliError::invalid_argument(format!("could not read stdin: {e}")))?;
    Ok(Some(buf))
}
