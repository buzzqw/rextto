//! User-configurable event hooks.
//!
//! The same idea appears in every reference project: qBittorrent's "Run
//! external program on torrent added/finished", Sonarr's "Custom Script"
//! notification, BiglyBT's tag exec-on-assign actions and autobrr's exec
//! actions. Rextto already emits a rich event stream through
//! [`crate::notifier::Notifier`]; this module lets a user attach an external
//! program to those events with a stable variable contract:
//!
//! * `{event}`, `{title}`, `{hash}`, `{path}`, `{series}`, `{season}`,
//!   `{episode}`, `{kind}`, `{source}`, `{quality_score}`, ... — every scalar
//!   field of the notification payload plus its flattened `a.b` forms.
//! * The same values are exported as environment variables prefixed with
//!   `REXTTO_` (dots become underscores), e.g. `REXTTO_EVENT`,
//!   `REXTTO_TITLE`, `REXTTO_HASH`.
//!
//! Programs are executed directly (never through a shell), with a timeout and
//! captured output so a misbehaving hook cannot wedge the daemon.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

fn default_true() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    60
}

/// One external program bound to a set of events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventHook {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Event names to react to; empty means every event.
    #[serde(default)]
    pub events: Vec<String>,
    /// Executable path (no shell interpretation).
    #[serde(default)]
    pub program: String,
    /// Argument template; supports `{placeholder}` expansion and quotes.
    #[serde(default)]
    pub args: String,
    /// Kill the process after this many seconds (default 60, max 86400).
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for EventHook {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            events: Vec::new(),
            program: String::new(),
            args: String::new(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

/// Loads hooks from the `event_hooks` settings key (a JSON array).
pub fn load_hooks(settings: &BTreeMap<String, String>) -> Vec<EventHook> {
    settings
        .get("event_hooks")
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}

/// Persists hooks into the settings table.
pub fn save_hooks(data_dir: &Path, hooks: &[EventHook]) -> Result<()> {
    let json = serde_json::to_string(hooks)?;
    crate::config::Config::save_setting(data_dir, "event_hooks", &json)
}

/// Validates a hook list before it is persisted.
pub fn validate_hooks(hooks: &[EventHook]) -> Option<String> {
    if hooks.len() > 100 {
        return Some("too many event hooks (max 100)".into());
    }
    for hook in hooks {
        if hook.program.trim().is_empty() {
            return Some(format!(
                "hook '{}' has no program",
                if hook.name.trim().is_empty() {
                    "unnamed"
                } else {
                    hook.name.trim()
                }
            ));
        }
        if hook.program.len() > 4096 || hook.args.len() > 8192 || hook.name.len() > 200 {
            return Some("an event hook is too large".into());
        }
        if hook.timeout_secs > 86_400 {
            return Some("an event hook timeout exceeds 24h".into());
        }
    }
    None
}

/// Expands `{key}` placeholders. Unknown keys are left untouched so a typo is
/// visible in the executed arguments instead of silently vanishing.
pub fn expand(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let key = &after[..close];
            if let Some(value) = vars.get(key) {
                out.push_str(value);
                rest = &after[close + 1..];
                continue;
            }
        }
        // Not a known placeholder: emit the brace literally and continue.
        out.push('{');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Splits an argument template honoring single/double quotes and backslash
/// escapes, without invoking a shell.
pub fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut has_token = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            has_token = true;
            continue;
        }
        match quote {
            Some(delimiter) => {
                if character == delimiter {
                    quote = None;
                } else if character == '\\' && delimiter == '"' {
                    escaped = true;
                } else {
                    current.push(character);
                }
                has_token = true;
            }
            None => {
                if character == '\\' {
                    escaped = true;
                } else if character == '\'' || character == '"' {
                    quote = Some(character);
                    has_token = true;
                } else if character.is_whitespace() {
                    if has_token {
                        args.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                } else {
                    current.push(character);
                    has_token = true;
                }
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// Flattens a JSON payload into `key` and `a.b` string variables and adds the
/// event/time fields every hook can rely on.
pub fn variables(event: &str, payload: &serde_json::Value) -> BTreeMap<String, String> {
    fn walk(prefix: &str, value: &serde_json::Value, vars: &mut BTreeMap<String, String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    let next = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    walk(&next, value, vars);
                }
            }
            serde_json::Value::Array(_) => {
                if let Ok(serialized) = serde_json::to_string(value) {
                    vars.insert(prefix.to_string(), serialized);
                }
            }
            serde_json::Value::Null => {}
            other => {
                if let Some(text) = other.as_str() {
                    vars.insert(prefix.to_string(), text.to_string());
                } else {
                    vars.insert(prefix.to_string(), other.to_string());
                }
            }
        }
    }
    let mut vars = BTreeMap::new();
    vars.insert("event".into(), event.to_string());
    vars.insert("date".into(), chrono::Utc::now().to_rfc3339());
    // Top-level convenience keys (the payloads emitted by Rextto are flat).
    if let serde_json::Value::Object(map) = payload {
        for (key, value) in map {
            match value {
                serde_json::Value::String(text) => {
                    vars.insert(key.clone(), text.clone());
                }
                serde_json::Value::Number(number) => {
                    vars.insert(key.clone(), number.to_string());
                }
                serde_json::Value::Bool(flag) => {
                    vars.insert(key.clone(), flag.to_string());
                }
                _ => {}
            }
        }
    }
    walk("", payload, &mut vars);
    vars
}

/// Result of executing one hook.
#[derive(Debug, Clone, Serialize)]
pub struct HookRun {
    pub hook: String,
    pub program: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

fn truncate(text: &[u8]) -> String {
    const LIMIT: usize = 4096;
    let slice = if text.len() > LIMIT { &text[..LIMIT] } else { text };
    String::from_utf8_lossy(slice).trim().to_string()
}

/// Runs a hook, expanding its program and arguments and exporting the variables
/// as environment. Returns an error only when the process cannot be spawned.
pub async fn run_hook(
    hook: &EventHook,
    event: &str,
    payload: &serde_json::Value,
) -> Result<HookRun, String> {
    let vars = variables(event, payload);
    let program = expand(hook.program.trim(), &vars);
    let args = split_args(&expand(&hook.args, &vars));
    // `0` was the serde default in older configurations. Treat it as the
    // documented default too, so upgrading does not silently turn hooks into
    // one-second processes.
    let timeout_secs = if hook.timeout_secs == 0 {
        default_timeout_secs()
    } else {
        hook.timeout_secs
    };
    let timeout = Duration::from_secs(timeout_secs.clamp(1, 86_400));
    let mut command = tokio::process::Command::new(&program);
    command.args(&args);
    command.kill_on_drop(true);
    for (key, value) in &vars {
        let env_key = format!(
            "REXTTO_{}",
            key.to_ascii_uppercase()
                .replace('.', "_")
                .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_")
        );
        command.env(env_key, value);
    }
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("could not run '{program}': {error}")),
        Err(_) => return Err(format!("'{program}' timed out after {}s", timeout.as_secs())),
    };
    Ok(HookRun {
        hook: hook.name.clone(),
        program,
        exit_code: output.status.code(),
        success: output.status.success(),
        stdout: truncate(&output.stdout),
        stderr: truncate(&output.stderr),
    })
}

/// Events a hook subscribes to; an empty list means all events.
pub fn hook_matches_event(hook: &EventHook, event: &str) -> bool {
    hook.events.is_empty() || hook.events.iter().any(|name| name == event)
}

/// Dispatches hooks for one event on a detached task. Never blocks the caller
/// (notification delivery) and logs each outcome.
pub fn dispatch(hooks: Vec<EventHook>, event: String, payload: serde_json::Value) {
    let selected: Vec<EventHook> = hooks
        .into_iter()
        .filter(|hook| hook.enabled && !hook.program.trim().is_empty())
        .collect();
    if selected.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for hook in selected {
            if !hook_matches_event(&hook, &event) {
                continue;
            }
            match run_hook(&hook, &event, &payload).await {
                Ok(run) => {
                    if run.success {
                        tracing::info!(
                            hook = %run.hook,
                            program = %run.program,
                            event = %event,
                            stdout = %run.stdout,
                            "🪝 event hook completed"
                        );
                    } else {
                        tracing::warn!(
                            hook = %run.hook,
                            program = %run.program,
                            event = %event,
                            exit_code = run.exit_code,
                            stderr = %run.stderr,
                            "🪝 event hook failed"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    hook = %hook.name,
                    program = %hook.program,
                    event = %event,
                    %error,
                    "🪝 event hook could not run"
                ),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expands_known_placeholders_and_keeps_unknown_ones() {
        let mut vars = BTreeMap::new();
        vars.insert("title".into(), "Movie.2020".into());
        vars.insert("hash".into(), "abc123".into());
        assert_eq!(
            expand("{title} [{hash}]", &vars),
            "Movie.2020 [abc123]"
        );
        // Unknown placeholders survive verbatim so a typo is visible.
        assert_eq!(expand("{nope}", &vars), "{nope}");
        // A lone brace is literal.
        assert_eq!(expand("a { b", &vars), "a { b");
    }

    #[test]
    fn splits_arguments_with_quotes_and_escapes() {
        assert_eq!(
            split_args("--path /media/film.mkv --title \"The Film\""),
            vec!["--path", "/media/film.mkv", "--title", "The Film"]
        );
        assert_eq!(
            split_args("'single quoted' plain \"with \\\"escape\\\"\""),
            vec!["single quoted", "plain", "with \"escape\""]
        );
        assert_eq!(split_args("  "), Vec::<String>::new());
    }

    #[test]
    fn flatten_variables_includes_nested_and_scalar_fields() {
        let payload = json!({
            "title": "Movie.2020",
            "hash": "abc",
            "season": 2,
            "meta": {"group": "NTb"}
        });
        let vars = variables("torrent_completed", &payload);
        assert_eq!(vars["event"], "torrent_completed");
        assert_eq!(vars["title"], "Movie.2020");
        assert_eq!(vars["season"], "2");
        assert_eq!(vars["meta.group"], "NTb");
    }

    #[test]
    fn event_filter_respects_subscriptions() {
        let hook = EventHook {
            name: "test".into(),
            enabled: true,
            events: vec!["torrent_completed".into()],
            program: "/bin/true".into(),
            args: String::new(),
            timeout_secs: 5,
        };
        assert!(hook_matches_event(&hook, "torrent_completed"));
        assert!(!hook_matches_event(&hook, "download_started"));
        let all = EventHook {
            events: Vec::new(),
            ..hook.clone()
        };
        assert!(hook_matches_event(&all, "anything"));
    }

    #[test]
    fn validation_rejects_missing_program() {
        let hooks = vec![EventHook {
            name: "broken".into(),
            program: "  ".into(),
            ..Default::default()
        }];
        assert!(validate_hooks(&hooks).is_some());
    }

    #[test]
    fn missing_timeout_uses_the_documented_default() {
        let hook: EventHook = serde_json::from_str(
            r#"{"name":"legacy","program":"/bin/true"}"#,
        )
        .unwrap();
        assert_eq!(hook.timeout_secs, 60);
    }

    #[tokio::test]
    async fn runs_a_program_and_captures_output() {
        let hook = EventHook {
            name: "echo".into(),
            enabled: true,
            events: vec!["download_started".into()],
            program: "/bin/echo".into(),
            args: "{title} {hash}".into(),
            timeout_secs: 5,
        };
        let payload = json!({"title": "Movie.2020", "hash": "abc"});
        let run = run_hook(&hook, "download_started", &payload)
            .await
            .expect("echo must run");
        assert!(run.success);
        assert_eq!(run.stdout, "Movie.2020 abc");
    }

    #[tokio::test]
    async fn expands_environment_variables_for_the_child() {
        let hook = EventHook {
            name: "env".into(),
            enabled: true,
            events: Vec::new(),
            // The shell is used only *inside* the test child, not by Rextto.
            program: "/bin/sh".into(),
            args: "-c \"printf %s \\\"$REXTTO_TITLE\\\"\"".into(),
            timeout_secs: 5,
        };
        let payload = json!({"title": "Env Movie"});
        let run = run_hook(&hook, "torrent_completed", &payload)
            .await
            .expect("sh must run");
        assert!(run.success, "stderr: {}", run.stderr);
        assert_eq!(run.stdout, "Env Movie");
    }
}
