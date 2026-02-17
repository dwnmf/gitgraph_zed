use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{GitLgError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionScope {
    Global,
    BranchDrop,
    Commit,
    Commits,
    Stash,
    Tag,
    Branch,
}

impl ActionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::BranchDrop => "branch-drop",
            Self::Commit => "commit",
            Self::Commits => "commits",
            Self::Stash => "stash",
            Self::Tag => "tag",
            Self::Branch => "branch",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Global,
            Self::BranchDrop,
            Self::Commit,
            Self::Commits,
            Self::Stash,
            Self::Tag,
            Self::Branch,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOption {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub flag: String,
    #[serde(default)]
    pub default_active: bool,
    #[serde(default)]
    pub info: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionParam {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub default_value: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub multiline: bool,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTemplate {
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_action_scope")]
    pub scope: ActionScope,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub info: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub raw_args: String,
    #[serde(default)]
    pub shell_script: bool,
    #[serde(default)]
    pub params: Vec<ActionParam>,
    #[serde(default)]
    pub options: Vec<ActionOption>,
    #[serde(default)]
    pub immediate: bool,
    #[serde(default)]
    pub ignore_errors: bool,
    #[serde(default)]
    pub allow_non_zero_exit: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionContext {
    #[serde(default)]
    pub branch_display_name: Option<String>,
    #[serde(default)]
    pub branch_name: Option<String>,
    #[serde(default)]
    pub local_branch_name: Option<String>,
    #[serde(default)]
    pub branch_id: Option<String>,
    #[serde(default)]
    pub source_branch_name: Option<String>,
    #[serde(default)]
    pub target_branch_name: Option<String>,
    #[serde(default)]
    pub commit_hash: Option<String>,
    #[serde(default)]
    pub commit_hashes: Vec<String>,
    #[serde(default)]
    pub commit_body: Option<String>,
    #[serde(default)]
    pub stash_name: Option<String>,
    #[serde(default)]
    pub tag_name: Option<String>,
    #[serde(default)]
    pub remote_name: Option<String>,
    #[serde(default)]
    pub default_remote_name: Option<String>,
    #[serde(default)]
    pub additional_placeholders: HashMap<String, String>,
}

impl ActionContext {
    pub fn to_placeholder_map(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        if let Some(v) = &self.branch_display_name {
            out.insert("BRANCH_DISPLAY_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.branch_name {
            out.insert("BRANCH_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.local_branch_name {
            out.insert("LOCAL_BRANCH_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.branch_id {
            out.insert("BRANCH_ID".to_string(), v.clone());
        }
        if let Some(v) = &self.source_branch_name {
            out.insert("SOURCE_BRANCH_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.target_branch_name {
            out.insert("TARGET_BRANCH_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.commit_hash {
            out.insert("COMMIT_HASH".to_string(), v.clone());
        }
        if !self.commit_hashes.is_empty() {
            out.insert("COMMIT_HASHES".to_string(), self.commit_hashes.join(" "));
        }
        if let Some(v) = &self.commit_body {
            out.insert("COMMIT_BODY".to_string(), v.clone());
        }
        if let Some(v) = &self.stash_name {
            out.insert("STASH_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.tag_name {
            out.insert("TAG_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.remote_name {
            out.insert("REMOTE_NAME".to_string(), v.clone());
        }
        if let Some(v) = &self.default_remote_name {
            out.insert("DEFAULT_REMOTE_NAME".to_string(), v.clone());
        } else if let Some(v) = &self.remote_name {
            out.insert("DEFAULT_REMOTE_NAME".to_string(), v.clone());
        }
        out.extend(self.additional_placeholders.clone());
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub template_id: String,
    pub params: HashMap<String, String>,
    pub enabled_options: HashSet<String>,
    pub context: ActionContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAction {
    pub id: String,
    pub title: String,
    pub scope: ActionScope,
    pub args: Vec<String>,
    pub shell_script: Option<String>,
    pub command_line: String,
    pub allow_non_zero_exit: bool,
    pub ignore_errors: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCatalog {
    pub templates: Vec<ActionTemplate>,
}

impl ActionCatalog {
    pub fn with_defaults() -> Self {
        static BUILTIN: OnceLock<ActionCatalog> = OnceLock::new();
        BUILTIN
            .get_or_init(|| {
                let templates = parse_builtin_actions()
                    .expect("default-git-actions.json should be valid and parseable");
                ActionCatalog { templates }
            })
            .clone()
    }

    pub fn find(&self, id: &str) -> Option<&ActionTemplate> {
        if let Some(found) = self.templates.iter().find(|t| t.id == id) {
            return Some(found);
        }
        if id.contains(':') {
            return None;
        }
        let suffix = self
            .templates
            .iter()
            .filter(|t| t.id.ends_with(&format!(":{id}")))
            .min_by_key(|t| (t.shell_script, t.params.len(), t.args.len()));
        if suffix.is_some() {
            return suffix;
        }

        let title = self
            .templates
            .iter()
            .filter(|t| sanitize_id_fragment(&t.title) == id)
            .min_by_key(|t| (t.shell_script, t.params.len(), t.args.len()));
        if title.is_some() {
            return title;
        }

        self.templates
            .iter()
            .filter(|t| {
                t.args
                    .first()
                    .is_some_and(|command| command.eq_ignore_ascii_case(id))
            })
            .min_by_key(|t| (t.shell_script, t.params.len(), t.args.len()))
    }

    pub fn templates_for_scope(&self, scope: ActionScope) -> Vec<&ActionTemplate> {
        self.templates.iter().filter(|t| t.scope == scope).collect()
    }

    pub fn resolve(&self, request: ActionRequest) -> Result<ResolvedAction> {
        self.resolve_with_lookup(request, |_placeholder| Ok(None))
    }

    pub fn resolve_with_lookup<F>(
        &self,
        request: ActionRequest,
        lookup: F,
    ) -> Result<ResolvedAction>
    where
        F: Fn(&str) -> Result<Option<String>>,
    {
        let template = self.find(&request.template_id).ok_or_else(|| {
            GitLgError::State(format!(
                "unknown action template id: {}",
                request.template_id
            ))
        })?;

        let mut placeholders = request.context.to_placeholder_map();
        placeholders.extend(request.params);
        for param in &template.params {
            if placeholders.contains_key(&param.id)
                || placeholders.contains_key(&format!("${}", param.id))
            {
                continue;
            }
            let value = match expand_placeholders(&param.default_value, &placeholders, &lookup) {
                Ok(expanded) => expanded,
                Err(GitLgError::MissingPlaceholder(_)) => param.default_value.clone(),
                Err(e) => return Err(e),
            };
            placeholders.insert(param.id.clone(), value);
        }
        for (k, v) in numeric_placeholder_aliases(&placeholders) {
            placeholders.insert(k, v);
        }

        let mut args = Vec::new();
        for token in &template.args {
            args.push(expand_placeholders(token, &placeholders, &lookup)?);
        }
        for option in &template.options {
            if request.enabled_options.contains(&option.id)
                || request.enabled_options.contains(&option.flag)
                || option.default_active
            {
                for token in tokenize_args(&option.flag) {
                    args.push(expand_placeholders(&token, &placeholders, &lookup)?);
                }
            }
        }

        let mut command_line = if template.shell_script {
            expand_placeholders(&template.raw_args, &placeholders, &lookup)?
        } else {
            args.join(" ")
        };
        if template.shell_script {
            for option in &template.options {
                if request.enabled_options.contains(&option.id)
                    || request.enabled_options.contains(&option.flag)
                    || option.default_active
                {
                    let expanded = expand_placeholders(&option.flag, &placeholders, &lookup)?;
                    if !expanded.is_empty() {
                        command_line.push(' ');
                        command_line.push_str(&expanded);
                    }
                }
            }
        }

        Ok(ResolvedAction {
            id: template.id.clone(),
            title: template.title.clone(),
            scope: template.scope,
            args,
            shell_script: template.shell_script.then_some(command_line.clone()),
            command_line,
            allow_non_zero_exit: template.allow_non_zero_exit,
            ignore_errors: template.ignore_errors,
        })
    }
}

fn numeric_placeholder_aliases(values: &HashMap<String, String>) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    for (key, value) in values {
        if key.chars().all(|c| c.is_ascii_digit()) {
            aliases.insert(format!("${}", key), value.clone());
        }
    }
    aliases
}

pub fn expand_placeholders<F>(
    input: &str,
    placeholders: &HashMap<String, String>,
    lookup: &F,
) -> Result<String>
where
    F: Fn(&str) -> Result<Option<String>>,
{
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                key.push(next);
            }
            if !closed {
                return Err(GitLgError::Parse(format!(
                    "unterminated placeholder in token {:?}",
                    input
                )));
            }
            let value = if let Some(v) = placeholders.get(&key) {
                v.clone()
            } else if let Some(v) = lookup(&key)? {
                v
            } else {
                return Err(GitLgError::MissingPlaceholder(key));
            };
            out.push_str(&value);
            continue;
        }

        if ch == '$' {
            let mut numeric = String::new();
            while let Some(peek) = chars.peek() {
                if peek.is_ascii_digit() {
                    numeric.push(*peek);
                    chars.next();
                } else {
                    break;
                }
            }
            if numeric.is_empty() {
                out.push('$');
            } else {
                let key = format!("${}", numeric);
                let value = placeholders
                    .get(&key)
                    .ok_or_else(|| GitLgError::MissingPlaceholder(key.clone()))?;
                out.push_str(value);
            }
            continue;
        }
        out.push(ch);
    }
    Ok(out)
}

fn parse_builtin_actions() -> Result<Vec<ActionTemplate>> {
    let raw: RawActionsFile = serde_json::from_str(include_str!("../default-git-actions.json"))
        .map_err(|e| GitLgError::Parse(format!("invalid default actions json: {}", e)))?;

    let mut out = Vec::new();
    let groups = [
        (ActionScope::Global, raw.actions_global),
        (ActionScope::BranchDrop, raw.actions_branch_drop),
        (ActionScope::Commit, raw.actions_commit),
        (ActionScope::Commits, raw.actions_commits),
        (ActionScope::Stash, raw.actions_stash),
        (ActionScope::Tag, raw.actions_tag),
        (ActionScope::Branch, raw.actions_branch),
    ];

    for (scope, actions) in groups {
        for (index, raw_action) in actions.into_iter().enumerate() {
            out.push(convert_raw_action(scope, index, raw_action));
        }
    }
    Ok(out)
}

fn default_action_scope() -> ActionScope {
    ActionScope::Global
}

fn convert_raw_action(scope: ActionScope, index: usize, raw: RawAction) -> ActionTemplate {
    let raw_args = raw.args.unwrap_or_default();
    let args = tokenize_args(&raw_args);
    let title = choose_title(raw.title.as_deref(), raw.description.as_deref(), &args);
    let id = format!(
        "{}:{}:{}",
        scope.as_str(),
        index + 1,
        sanitize_id_fragment(&title)
    );
    let params = raw
        .params
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(idx, p)| convert_raw_param(idx, p))
        .collect::<Vec<_>>();
    let options = raw
        .options
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(idx, o)| convert_raw_option(idx, o))
        .collect::<Vec<_>>();

    ActionTemplate {
        id,
        scope,
        title,
        icon: raw.icon,
        description: raw.description.unwrap_or_default(),
        info: raw.info,
        args,
        raw_args: raw_args.clone(),
        shell_script: is_shell_script(&raw_args),
        params,
        options,
        immediate: raw.immediate.unwrap_or(false),
        ignore_errors: raw.ignore_errors.unwrap_or(false),
        allow_non_zero_exit: raw.ignore_errors.unwrap_or(false),
    }
}

fn convert_raw_param(index: usize, raw: RawParam) -> ActionParam {
    match raw {
        RawParam::Simple(value) => ActionParam {
            id: (index + 1).to_string(),
            default_value: value,
            placeholder: None,
            multiline: false,
            readonly: false,
        },
        RawParam::Detailed {
            value,
            multiline,
            placeholder,
            readonly,
        } => ActionParam {
            id: (index + 1).to_string(),
            default_value: value,
            placeholder,
            multiline: multiline.unwrap_or(false),
            readonly: readonly.unwrap_or(false),
        },
    }
}

fn convert_raw_option(index: usize, raw: RawOption) -> ActionOption {
    ActionOption {
        id: sanitize_id_fragment(&format!("{}-{}", raw.value, index + 1)),
        title: raw.value.clone(),
        flag: raw.value,
        default_active: raw.default_active.unwrap_or(false),
        info: raw.info,
    }
}

fn choose_title(title: Option<&str>, description: Option<&str>, args: &[String]) -> String {
    let title = title.unwrap_or("").trim();
    if !title.is_empty() {
        return title.to_string();
    }
    if let Some(desc) = description {
        let trimmed = desc.trim();
        if !trimmed.is_empty() {
            if let Some((prefix, _)) = trimmed.split_once('(') {
                return prefix.trim().to_string();
            }
            return trimmed.to_string();
        }
    }
    args.join(" ")
}

fn sanitize_id_fragment(text: &str) -> String {
    let lowered = text.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut prev_dash = false;
    for ch in lowered.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn tokenize_args(args: &str) -> Vec<String> {
    if args.trim().is_empty() {
        return Vec::new();
    }
    if let Some(tokens) = shlex::split(args) {
        return tokens;
    }
    args.split_whitespace().map(ToString::to_string).collect()
}

fn is_shell_script(raw_args: &str) -> bool {
    raw_args.contains("&&") || raw_args.contains("||") || raw_args.contains(';')
}

#[derive(Debug, Deserialize)]
struct RawActionsFile {
    #[serde(rename = "actions.global", default)]
    actions_global: Vec<RawAction>,
    #[serde(rename = "actions.branch-drop", default)]
    actions_branch_drop: Vec<RawAction>,
    #[serde(rename = "actions.commit", default)]
    actions_commit: Vec<RawAction>,
    #[serde(rename = "actions.commits", default)]
    actions_commits: Vec<RawAction>,
    #[serde(rename = "actions.stash", default)]
    actions_stash: Vec<RawAction>,
    #[serde(rename = "actions.tag", default)]
    actions_tag: Vec<RawAction>,
    #[serde(rename = "actions.branch", default)]
    actions_branch: Vec<RawAction>,
}

#[derive(Debug, Deserialize)]
struct RawAction {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    info: Option<String>,
    #[serde(default)]
    args: Option<String>,
    #[serde(default)]
    params: Option<Vec<RawParam>>,
    #[serde(default)]
    options: Option<Vec<RawOption>>,
    #[serde(default)]
    immediate: Option<bool>,
    #[serde(default)]
    ignore_errors: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawParam {
    Simple(String),
    Detailed {
        value: String,
        multiline: Option<bool>,
        placeholder: Option<String>,
        readonly: Option<bool>,
    },
}

#[derive(Debug, Deserialize)]
struct RawOption {
    value: String,
    default_active: Option<bool>,
    #[serde(default)]
    info: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{
        ActionCatalog, ActionContext, ActionRequest, ActionScope, ActionTemplate,
        expand_placeholders,
    };
    use crate::error::Result;

    #[test]
    fn expands_named_and_indexed_placeholders() {
        let mut values = HashMap::new();
        values.insert("BRANCH_NAME".to_string(), "main".to_string());
        values.insert("$1".to_string(), "feature".to_string());
        let expanded =
            expand_placeholders("merge {BRANCH_NAME} $1", &values, &|_| Ok(None)).expect("expands");
        assert_eq!(expanded, "merge main feature");
    }

    #[test]
    fn loads_builtin_scopes() {
        let catalog = ActionCatalog::with_defaults();
        assert!(!catalog.templates.is_empty());
        for scope in ActionScope::all() {
            assert!(
                !catalog.templates_for_scope(*scope).is_empty(),
                "scope {:?} should have templates",
                scope
            );
        }
    }

    #[test]
    fn resolves_dynamic_lookup_placeholder() {
        let mut catalog = ActionCatalog::default();
        catalog.templates.push(ActionTemplate {
            id: "test:dynamic".to_string(),
            scope: ActionScope::Global,
            title: "dynamic".to_string(),
            icon: None,
            description: String::new(),
            info: None,
            args: vec![
                "fetch".to_string(),
                "{GIT_CONFIG:remote.pushDefault}".to_string(),
            ],
            raw_args: "fetch {GIT_CONFIG:remote.pushDefault}".to_string(),
            shell_script: false,
            params: vec![],
            options: vec![],
            immediate: false,
            ignore_errors: false,
            allow_non_zero_exit: false,
        });
        let request = ActionRequest {
            template_id: "test:dynamic".to_string(),
            params: HashMap::new(),
            enabled_options: HashSet::new(),
            context: ActionContext::default(),
        };
        let resolved = catalog.resolve_with_lookup(request, |key| -> Result<Option<String>> {
            if key == "GIT_CONFIG:remote.pushDefault" {
                Ok(Some("origin".to_string()))
            } else {
                Ok(None)
            }
        });
        assert_eq!(resolved.expect("resolved").args, vec!["fetch", "origin"]);
    }
}
