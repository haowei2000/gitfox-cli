//! `gf org list` — the spaces you belong to. A GitFox space is the nearest
//! thing to a GitHub organisation.

use gitfox_client::Membership;
use serde_json::{Value, json};

use crate::cli::{OrgCommand, OrgListArgs, OrgSubcommand};
use crate::context::Context;
use crate::error::Result;
use crate::output::Render;
use crate::paginate;

pub async fn run(cmd: OrgCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        OrgSubcommand::List(args) => list(args, ctx).await,
    }
}

async fn list(args: OrgListArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let client = &client;
    let paged = paginate::collect(args.limit, move |page, limit| async move {
        client.spaces().memberships(page, limit).await
    })
    .await?;
    ctx.renderer.emit(&SpaceList {
        memberships: paged.items,
        truncated: paged.truncated,
    })
}

struct SpaceList {
    memberships: Vec<Membership>,
    truncated: bool,
}

impl Render for SpaceList {
    fn to_json(&self) -> Value {
        json!({
            "count": self.memberships.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.memberships
            .iter()
            .map(|m| {
                json!({
                    "space": m.space.reference(),
                    "identifier": m.space.identifier,
                    "description": m.space.description,
                    "is_public": m.space.is_public,
                    "role": m.role,
                })
            })
            .collect()
    }

    fn to_human(&self, _color: bool) -> String {
        if self.memberships.is_empty() {
            return "You are not a member of any spaces".to_string();
        }
        // One per line, as gh prints organisations, so it pipes cleanly.
        let mut out = self
            .memberships
            .iter()
            .map(|m| m.space.reference())
            .collect::<Vec<_>>()
            .join("\n");
        if self.truncated {
            out.push_str(&format!(
                "\n\nShowing {} of more; raise --limit to see the rest.",
                self.memberships.len()
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_print_one_per_line() {
        let list = SpaceList {
            memberships: serde_json::from_value(json!([
                { "space": { "identifier": "ai-repos", "path": "ai-repos" }, "role": "space_owner" },
                { "space": { "identifier": "team", "path": "org/team" }, "role": "reader" }
            ]))
            .unwrap(),
            truncated: false,
        };
        assert_eq!(list.to_human(false), "ai-repos\norg/team");
        assert_eq!(list.to_json()["items"][1]["role"], "reader");
    }
}
