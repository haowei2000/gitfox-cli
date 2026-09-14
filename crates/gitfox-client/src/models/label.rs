use serde::{Deserialize, Serialize};

/// A label defined on a repository or a space.
///
/// GitFox labels are keyed rather than named, and a label may carry a set of
/// values (`priority:high`). Colours come from a fixed palette of names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// `static` or `dynamic`.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Set for a repository label; a space label has `space_id` instead.
    #[serde(default)]
    pub repo_id: Option<i64>,
    #[serde(default)]
    pub space_id: Option<i64>,
    #[serde(default)]
    pub scope: Option<i64>,
    #[serde(default)]
    pub value_count: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelValue {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub label_id: Option<i64>,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// The body for defining or updating a label. Every field is optional on an
/// update; `key` renames the label.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LabelInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// `GET …/pullreq/{n}/labels`: the labels that can be, or are, attached.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequestLabels {
    #[serde(default)]
    pub label_data: Vec<LabelAssignment>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelAssignment {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub assigned: Option<bool>,
    #[serde(default)]
    pub assigned_value: Option<LabelValueInfo>,
    #[serde(default)]
    pub values: Vec<LabelValueInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelValueInfo {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

/// The colours GitFox accepts, with the RGB each one is drawn as — used to
/// map a hex colour from a GitHub-style command onto the nearest name.
pub const LABEL_COLORS: [(&str, [u8; 3]); 13] = [
    ("blue", [0x3b, 0x82, 0xf6]),
    ("brown", [0x92, 0x40, 0x0e]),
    ("cyan", [0x06, 0xb6, 0xd4]),
    ("green", [0x22, 0xc5, 0x5e]),
    ("indigo", [0x63, 0x66, 0xf1]),
    ("lime", [0x84, 0xcc, 0x16]),
    ("mint", [0x34, 0xd3, 0x99]),
    ("orange", [0xf9, 0x73, 0x16]),
    ("pink", [0xec, 0x48, 0x99]),
    ("purple", [0xa8, 0x55, 0xf7]),
    ("red", [0xef, 0x44, 0x44]),
    ("violet", [0x8b, 0x5c, 0xf6]),
    ("yellow", [0xea, 0xb3, 0x08]),
];

/// A colour name GitFox accepts, from either a name or a hex triplet.
///
/// A hex colour becomes the nearest palette entry, so a `d73a4a` copied from
/// a GitHub label still lands on `red`. `None` when the input is neither.
pub fn label_color(input: &str) -> Option<&'static str> {
    let trimmed = input.trim().trim_start_matches('#').to_ascii_lowercase();
    if let Some((name, _)) = LABEL_COLORS.iter().find(|(name, _)| *name == trimmed) {
        return Some(name);
    }
    let rgb = parse_hex(&trimmed)?;
    LABEL_COLORS
        .iter()
        .min_by_key(|(_, c)| {
            c.iter()
                .zip(rgb.iter())
                .map(|(a, b)| (*a as i32 - *b as i32).pow(2))
                .sum::<i32>()
        })
        .map(|(name, _)| *name)
}

/// The hex triplet a palette name is drawn as, without `#`.
pub fn label_color_hex(name: &str) -> Option<String> {
    LABEL_COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, [r, g, b])| format!("{r:02x}{g:02x}{b:02x}"))
}

fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let expanded: String = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(expanded.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_palette_name_passes_through_and_hex_lands_on_the_nearest_name() {
        assert_eq!(label_color("red"), Some("red"));
        assert_eq!(label_color("  Blue "), Some("blue"));
        // GitHub's default `bug` colour.
        assert_eq!(label_color("d73a4a"), Some("red"));
        assert_eq!(label_color("#0e8a16"), Some("green"));
        // Shorthand hex is expanded before matching.
        assert_eq!(label_color("f44"), Some("red"));
        assert_eq!(label_color("nonsense"), None);
        assert_eq!(label_color_hex("red").as_deref(), Some("ef4444"));
    }
}
