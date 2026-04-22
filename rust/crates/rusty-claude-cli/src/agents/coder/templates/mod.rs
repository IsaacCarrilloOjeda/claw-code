//! Template scaffolds for the coder agent.
//!
//! Templates are pre-written skeletons for the ~6 repetitive file types in
//! GHOST (new migration, new daemon endpoint, new agent, etc.). The agent
//! stamps a template and feeds the stamped content through the same diff
//! queue a normal edit would — `stamp()` itself does not touch disk.
//!
//! Both the body (`<name>.tmpl`) and metadata (`<name>.meta.json`) live
//! next to this file and are baked into the binary with `include_str!`.
//! Runtime file I/O is deliberately avoided so deployed daemons have no
//! dependency on the original source tree.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Placeholder {
    pub name: String,
    pub description: String,
    pub example: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TemplateMeta {
    name: String,
    description: String,
    placeholders: Vec<Placeholder>,
    output_path_template: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[allow(clippy::struct_field_names)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub placeholders: Vec<Placeholder>,
    pub output_path_template: String,
    #[serde(skip)]
    pub body: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StampedOutput {
    pub path: String,
    pub content: String,
}

#[derive(Debug)]
pub enum StampError {
    UnknownTemplate(String),
    MissingPlaceholder(String),
    Io(String),
}

impl std::fmt::Display for StampError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTemplate(n) => write!(f, "unknown template: {n}"),
            Self::MissingPlaceholder(n) => write!(f, "missing placeholder: {n}"),
            Self::Io(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for StampError {}

// -- Baked-in bodies + metadata ---------------------------------------------

macro_rules! bundled {
    ($meta_path:expr, $body_path:expr) => {
        (include_str!($meta_path), include_str!($body_path))
    };
}

const BUNDLED: &[(&str, &str)] = &[
    bundled!("new_daemon_endpoint.meta.json", "new_daemon_endpoint.tmpl"),
    bundled!("new_migration.meta.json", "new_migration.tmpl"),
    bundled!("new_agent.meta.json", "new_agent.tmpl"),
    bundled!("new_dashboard_panel.meta.json", "new_dashboard_panel.tmpl"),
    bundled!("new_tool.meta.json", "new_tool.tmpl"),
    bundled!("new_db_helper.meta.json", "new_db_helper.tmpl"),
];

fn init_templates() -> Vec<Template> {
    let mut out = Vec::with_capacity(BUNDLED.len());
    for (meta_raw, body) in BUNDLED {
        let meta: TemplateMeta =
            serde_json::from_str(meta_raw).expect("bundled template metadata must be valid JSON");
        out.push(Template {
            name: meta.name,
            description: meta.description,
            placeholders: meta.placeholders,
            output_path_template: meta.output_path_template,
            body,
        });
    }
    out
}

fn templates() -> &'static [Template] {
    static CELL: OnceLock<Vec<Template>> = OnceLock::new();
    CELL.get_or_init(init_templates)
}

pub fn all() -> &'static [Template] {
    templates()
}

pub fn by_name(name: &str) -> Option<&'static Template> {
    templates().iter().find(|t| t.name == name)
}

// -- Stamping ---------------------------------------------------------------

/// Render `template.body` and `template.output_path_template` against the
/// supplied placeholder map. Special server-side placeholders
/// (`next_migration_number`, `today_date`) are filled in before user values.
pub fn stamp(
    template: &Template,
    values: &serde_json::Map<String, serde_json::Value>,
    migrations_dir: Option<&std::path::Path>,
) -> Result<StampedOutput, StampError> {
    let mut effective = values.clone();

    // Server-computed placeholders — only added if the template actually
    // references them, so we don't pay the fs scan on every stamp.
    let wants_next_mig = template.body.contains("{{next_migration_number}}")
        || template
            .output_path_template
            .contains("{{next_migration_number}}");
    if wants_next_mig {
        let next = next_migration_number(migrations_dir).map_err(StampError::Io)?;
        effective.insert(
            "next_migration_number".to_string(),
            serde_json::Value::String(format!("{next:03}")),
        );
    }

    let wants_today = template.body.contains("{{today_date}}")
        || template.output_path_template.contains("{{today_date}}");
    if wants_today {
        effective.insert(
            "today_date".to_string(),
            serde_json::Value::String(today_date_local()),
        );
    }

    let content = render(template.body, &effective)?;
    let path = render(&template.output_path_template, &effective)?;
    Ok(StampedOutput { path, content })
}

fn render(
    source: &str,
    values: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, StampError> {
    let mut out = source.to_string();
    for (k, v) in values {
        let marker = format!("{{{{{k}}}}}");
        let replacement = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out = out.replace(&marker, &replacement);
    }
    // Any surviving `{{...}}` markers mean the caller didn't supply a
    // required placeholder. Report the first one so the error is actionable.
    if let Some(idx) = out.find("{{") {
        if let Some(end) = out[idx..].find("}}") {
            let name = &out[idx + 2..idx + end];
            return Err(StampError::MissingPlaceholder(name.to_string()));
        }
    }
    Ok(out)
}

/// Scan `migrations_dir` (defaults to `rust/migrations/` relative to CWD),
/// find the highest 3-digit prefix, return `n+1`.
pub fn next_migration_number(dir: Option<&std::path::Path>) -> Result<u32, String> {
    let dir_owned: PathBuf;
    let dir: &std::path::Path = if let Some(d) = dir {
        d
    } else {
        dir_owned = std::env::current_dir()
            .map_err(|e| format!("cwd: {e}"))?
            .join("rust")
            .join("migrations");
        &dir_owned
    };

    if !dir.exists() {
        return Ok(1);
    }

    let mut max: u32 = 0;
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(prefix) = name.split('_').next() else {
            continue;
        };
        if prefix.len() < 3 {
            continue;
        }
        if let Ok(n) = prefix.parse::<u32>() {
            if n > max {
                max = n;
            }
        }
    }
    Ok(max + 1)
}

fn today_date_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_templates_parse() {
        let ts = all();
        assert_eq!(ts.len(), 6);
        let names: Vec<_> = ts.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"new_daemon_endpoint"));
        assert!(names.contains(&"new_migration"));
        assert!(names.contains(&"new_agent"));
        assert!(names.contains(&"new_dashboard_panel"));
        assert!(names.contains(&"new_tool"));
        assert!(names.contains(&"new_db_helper"));
    }

    #[test]
    fn stamp_renders_placeholders() {
        let tmpl = Template {
            name: "t".into(),
            description: "d".into(),
            placeholders: vec![],
            output_path_template: "out/{{name}}.txt".into(),
            body: "hello {{name}}, age {{age}}",
        };
        let mut values = serde_json::Map::new();
        values.insert("name".into(), serde_json::Value::String("world".into()));
        values.insert("age".into(), serde_json::Value::from(15));
        let out = stamp(&tmpl, &values, None).unwrap();
        assert_eq!(out.path, "out/world.txt");
        assert_eq!(out.content, "hello world, age 15");
    }

    #[test]
    fn stamp_reports_missing_placeholder() {
        let tmpl = Template {
            name: "t".into(),
            description: "d".into(),
            placeholders: vec![],
            output_path_template: "out/{{name}}.txt".into(),
            body: "hello {{name}}, age {{age}}",
        };
        let mut values = serde_json::Map::new();
        values.insert("name".into(), serde_json::Value::String("world".into()));
        let err = stamp(&tmpl, &values, None).unwrap_err();
        match err {
            StampError::MissingPlaceholder(n) => assert_eq!(n, "age"),
            other => panic!("expected MissingPlaceholder, got {other:?}"),
        }
    }

    #[test]
    fn next_migration_number_is_right() {
        let td = tempfile::tempdir().unwrap();
        for name in ["001_a.sql", "002_b.sql", "017_c.sql", "README.md"] {
            std::fs::write(td.path().join(name), "").unwrap();
        }
        assert_eq!(next_migration_number(Some(td.path())).unwrap(), 18);
    }

    #[test]
    fn next_migration_number_empty_dir_returns_one() {
        let td = tempfile::tempdir().unwrap();
        assert_eq!(next_migration_number(Some(td.path())).unwrap(), 1);
    }

    #[test]
    fn migration_template_resolves_next_number() {
        let td = tempfile::tempdir().unwrap();
        std::fs::write(td.path().join("020_x.sql"), "").unwrap();

        let tmpl = by_name("new_migration").expect("new_migration template missing");
        let mut values = serde_json::Map::new();
        values.insert(
            "name".into(),
            serde_json::Value::String("hello_world".into()),
        );
        values.insert(
            "description".into(),
            serde_json::Value::String("smoke test".into()),
        );
        let out = stamp(tmpl, &values, Some(td.path())).unwrap();
        assert!(out.path.contains("021_hello_world.sql"), "got {}", out.path);
        assert!(out.content.contains("smoke test"));
    }
}
