use prost::Message;
use std::{collections::HashMap, io::Cursor};

use crate::bazel_flags_proto::{FlagCollection, FlagInfo};

#[derive(Debug)]
pub struct BazelFlags {
    pub flags: Vec<FlagInfo>,
    pub flags_by_commands: HashMap<String, Vec<usize>>,
    pub flags_by_name: HashMap<String, usize>,
    pub flags_by_abbreviation: HashMap<String, usize>,
}

impl FlagInfo {
    pub fn get_documentation_markdown(&self) -> String {
        let mut result = String::new();

        // First line: Flag name and short hand (if any)
        result += format!("`--{}`", self.name).as_str();
        if let Some(abbr) = &self.abbreviation {
            result += format!(" [`-{}`]", abbr).as_str();
        }
        if self.has_negative_flag() {
            result += format!(", `--no{}`", self.name).as_str();
        }
        // Followed by the documentation text
        if let Some(doc) = &self.documentation {
            result += "\n\n";
            result += doc.as_str();
        }
        // And a list of tags
        result += "\n\n";
        if !self.effect_tags.is_empty() {
            result += "Effect tags: ";
            result += self
                .effect_tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
                .as_str();
            result += "\\\n";
        }
        if !self.metadata_tags.is_empty() {
            result += "Tags: ";
            result += self
                .metadata_tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
                .as_str();
            result += "\\\n";
        }
        if let Some(catgegory) = &self.documentation_category {
            result += format!("Category: {}\n", catgegory.to_lowercase()).as_str();
        }

        result
    }
}

impl BazelFlags {
    pub fn from_flags(flags: Vec<FlagInfo>) -> BazelFlags {
        let mut flags_by_commands = HashMap::<String, Vec<usize>>::new();
        let mut flags_by_name = HashMap::<String, usize>::new();
        let mut flags_by_abbreviation = HashMap::<String, usize>::new();
        for (i, f) in (&flags).iter().enumerate() {
            for c in &f.commands {
                let list = flags_by_commands
                    .entry(c.clone())
                    .or_insert_with(|| Default::default());
                list.push(i);
            }
            flags_by_name.insert(f.name.clone(), i);
            if let Some(abbreviation) = &f.abbreviation {
                flags_by_abbreviation.insert(abbreviation.clone(), i);
            }
        }
        return BazelFlags {
            flags: flags,
            flags_by_commands,
            flags_by_name,
            flags_by_abbreviation,
        };
    }

    pub fn get_by_invocation(&self, s: &str) -> Option<&FlagInfo> {
        let stripped = s.strip_suffix("=").unwrap_or(s);
        // Long names
        if let Some(long_name) = stripped.strip_prefix("--") {
            if long_name.starts_with("-") {
                return None;
            }
            return self
                .flags_by_name
                .get(long_name)
                .map(|i| self.flags.get(*i).unwrap());
        }
        // Short names
        if let Some(abbreviation) = stripped.strip_prefix("-") {
            if abbreviation.starts_with("-") {
                return None;
            }
            return self
                .flags_by_abbreviation
                .get(abbreviation)
                .map(|i| self.flags.get(*i).unwrap());
        }
        None
    }
}

pub fn load_bazel_flags() -> BazelFlags {
    let proto_bytes = include_bytes!("../flag-dumps/7.1.0.data");
    let flags = FlagCollection::decode(&mut Cursor::new(proto_bytes))
        .unwrap()
        .flag_infos;
    return BazelFlags::from_flags(flags);
}

#[test]
fn test_flags() {
    let flags = load_bazel_flags();
    let mut commands = flags
        .flags_by_commands
        .keys()
        .map(|s| s.clone())
        .collect::<Vec<_>>();
    assert!(commands.contains(&"build".to_string()));
    assert!(commands.contains(&"clean".to_string()));
    assert!(commands.contains(&"test".to_string()));

    // Can lookup a flag by its invocation
    let preemptible_info = flags.get_by_invocation("--preemptible");
    assert_eq!(
        preemptible_info
            .unwrap()
            .commands
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>(),
        vec!("startup")
    );

    // Supports both short and long forms
    assert_eq!(
        flags.get_by_invocation("-k"),
        flags.get_by_invocation("--keep_going")
    );
}
