//! `PrivacyFilter`: lista blanca de metadatos por herramienta (SPEC §10).
//!
//! Los argumentos de una herramienta nunca se guardan completos. De cada llamada
//! solo salen los campos derivados de este módulo: una ruta relativa, un host o
//! un puñado de valores cerrados.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Política de captura. La app decide la suya; el mod empieza en `PRE_HANDSHAKE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyPolicy {
    pub capture_task_titles: bool,
    pub capture_shell_program: bool,
    pub record_paths: bool,
}

impl PrivacyPolicy {
    /// Valores por defecto de la app (D1, D2).
    pub const APP_DEFAULT: Self = Self {
        capture_task_titles: true,
        capture_shell_program: false,
        record_paths: true,
    };

    /// La política más restrictiva: sin títulos y sin rutas de ningún tipo (SPEC §5.4).
    pub const PRE_HANDSHAKE: Self = Self {
        capture_task_titles: false,
        capture_shell_program: false,
        record_paths: false,
    };
}

impl Default for PrivacyPolicy {
    fn default() -> Self {
        Self::APP_DEFAULT
    }
}

/// Valor de un metadato: solo booleanos y cadenas cortas ya derivadas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum MetaValue {
    Bool(bool),
    Str(String),
}

/// Metadatos permitidos de una llamada a herramienta.
///
/// Las rutas y el host solo viven en `rel_path` y `host`, que tienen retención de
/// 30 días (SPEC §6.4.1). `meta` nunca contiene rutas.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ToolMeta {
    pub rel_path: Option<String>,
    pub host: Option<String>,
    pub meta: BTreeMap<String, MetaValue>,
}

impl ToolMeta {
    /// JSON de `meta` para `tool_calls.meta_json`, o `None` si está vacío.
    pub fn meta_json(&self) -> Option<String> {
        if self.meta.is_empty() {
            return None;
        }
        debug_assert!(meta_is_clean(&self.meta));
        serde_json::to_string(&self.meta).ok()
    }
}

/// Marca de `rel_path` para un fichero fuera de la raíz del proyecto.
pub const EXTERNAL_PATH: &str = "<externo>";

/// Claves que `meta` no puede contener nunca (SPEC §6.4).
pub const FORBIDDEN_META_KEYS: [&str; 8] = [
    "path",
    "file_path",
    "notebook_path",
    "rel_path",
    "url",
    "host",
    "cwd",
    "command",
];

/// `true` si `meta` no contiene ninguna clave de ruta.
pub fn meta_is_clean(meta: &BTreeMap<String, MetaValue>) -> bool {
    meta.keys()
        .all(|k| !FORBIDDEN_META_KEYS.contains(&k.as_str()))
}

/// Campos de los argumentos que el filtro puede mirar. Todo lo demás ni se lee.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolInputView<'a> {
    pub file_path: Option<&'a str>,
    pub replace_all: Option<bool>,
    pub run_in_background: Option<bool>,
    pub url: Option<&'a str>,
    pub subagent_type: Option<&'a str>,
    pub command: Option<&'a str>,
}

/// Aplica la lista blanca de §10 a una llamada.
///
/// `root` es la raíz respecto a la que se relativizan las rutas. Hasta que el
/// `GitReader` de F4a resuelva la raíz real, se usa el `cwd` de la sesión.
pub fn extract_tool_meta(
    tool: &str,
    input: &ToolInputView<'_>,
    policy: &PrivacyPolicy,
    root: Option<&str>,
) -> ToolMeta {
    let mut out = ToolMeta::default();
    match tool {
        "Read" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            if policy.record_paths
                && let Some(path) = input.file_path
            {
                match relativize(path, root) {
                    Some(rel) => out.rel_path = Some(rel),
                    None => {
                        out.rel_path = Some(EXTERNAL_PATH.to_owned());
                        out.meta
                            .insert("ext".to_owned(), MetaValue::Str(extension(path)));
                    }
                }
            }
            if let Some(v) = input.replace_all {
                out.meta
                    .insert("replace_all".to_owned(), MetaValue::Bool(v));
            }
        }
        "Bash" | "PowerShell" => {
            if let Some(v) = input.run_in_background {
                out.meta
                    .insert("run_in_background".to_owned(), MetaValue::Bool(v));
            }
            if policy.capture_shell_program
                && let Some(program) = input.command.and_then(shell_program)
            {
                out.meta
                    .insert("program".to_owned(), MetaValue::Str(program));
            }
        }
        "WebFetch" => {
            if policy.record_paths {
                out.host = input.url.and_then(url_host);
            }
        }
        "Agent" => {
            if let Some(t) = input.subagent_type {
                out.meta
                    .insert("subagent_type".to_owned(), MetaValue::Str(truncate(t, 64)));
            }
            if let Some(v) = input.run_in_background {
                out.meta
                    .insert("run_in_background".to_owned(), MetaValue::Bool(v));
            }
        }
        _ => {
            // MCP: solo el servidor y la herramienta, que ya van en el nombre.
            if let Some((server, mcp_tool)) = split_mcp(tool) {
                out.meta
                    .insert("server".to_owned(), MetaValue::Str(server.to_owned()));
                out.meta
                    .insert("tool".to_owned(), MetaValue::Str(mcp_tool.to_owned()));
            }
            // Grep, Glob, WebSearch, tareas (F4b) y desconocidas: solo `tool`.
        }
    }
    out
}

/// `mcp__<servidor>__<herramienta>` → `(servidor, herramienta)`.
pub fn split_mcp(tool: &str) -> Option<(&str, &str)> {
    let rest = tool.strip_prefix("mcp__")?;
    let (server, mcp_tool) = rest.split_once("__")?;
    (!server.is_empty() && !mcp_tool.is_empty()).then_some((server, mcp_tool))
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn normalize_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// Ruta relativa a `root` con `/`, o `None` si queda fuera o no se puede saber.
fn relativize(path: &str, root: Option<&str>) -> Option<String> {
    let path = normalize_slashes(path);
    let rel = if is_absolute(&path) {
        let root = normalize_slashes(root?);
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            return None;
        }
        // Windows no distingue mayúsculas: se compara sin ellas y se conserva la original.
        let head = path.get(..root.len())?;
        if !head.eq_ignore_ascii_case(root) && head.to_lowercase() != root.to_lowercase() {
            return None;
        }
        path[root.len()..].strip_prefix('/')?.to_owned()
    } else {
        path.trim_start_matches("./").to_owned()
    };
    if rel.is_empty() || rel.split('/').any(|seg| seg == "..") {
        return None;
    }
    Some(rel)
}

/// Extensión en minúsculas con el punto (`.rs`), o `""`. No es una ruta.
pub fn extension(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 12
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            format!(".{}", ext.to_ascii_lowercase())
        }
        _ => String::new(),
    }
}

/// Nombre del ejecutable: el primer token, sin directorio ni argumentos.
fn shell_program(command: &str) -> Option<String> {
    let token = command.split_whitespace().next()?;
    // `VAR=valor cmd` empieza por una asignación, que puede llevar un secreto.
    if token.contains('=') {
        return None;
    }
    let token = token.trim_matches(|c| matches!(c, '"' | '\'' | '&' | '(' | ')'));
    let name = token.rsplit(['/', '\\']).next()?;
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    ok.then(|| name.to_owned())
}

/// Host de una URL en minúsculas, sin credenciales, puerto, ruta ni consulta.
fn url_host(url: &str) -> Option<String> {
    let (_, rest) = url.trim().split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        host_port.split(':').next()?
    };
    let ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    ok.then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAP: &str = "SECRETO-TRAMPA";
    const ROOT: Option<&str> = Some(r"C:\Users\dev\proyecto");

    fn all_strings(meta: &ToolMeta) -> String {
        format!("{meta:?}")
    }

    #[test]
    fn edit_keeps_only_relative_path_and_flag() {
        let input = ToolInputView {
            file_path: Some(r"C:\Users\dev\Proyecto\src\main.rs"),
            replace_all: Some(true),
            command: Some(TRAP),
            ..Default::default()
        };
        let meta = extract_tool_meta("Edit", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(meta.rel_path.as_deref(), Some("src/main.rs"));
        assert_eq!(meta.meta_json().as_deref(), Some(r#"{"replace_all":true}"#));
        assert!(!all_strings(&meta).contains(TRAP));
    }

    #[test]
    fn path_outside_root_is_marked_external_with_extension() {
        let input = ToolInputView {
            file_path: Some(r"C:\Users\dev\otro\notas.SECRETAS.MD"),
            ..Default::default()
        };
        let meta = extract_tool_meta("Read", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(meta.rel_path.as_deref(), Some(EXTERNAL_PATH));
        assert_eq!(meta.meta_json().as_deref(), Some(r#"{"ext":".md"}"#));
        assert!(!all_strings(&meta).contains("otro"));
    }

    #[test]
    fn sibling_directory_with_same_prefix_is_external() {
        let input = ToolInputView {
            file_path: Some(r"C:\Users\dev\proyecto-2\a.rs"),
            ..Default::default()
        };
        let meta = extract_tool_meta("Write", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(meta.rel_path.as_deref(), Some(EXTERNAL_PATH));
    }

    #[test]
    fn parent_traversal_is_external() {
        let input = ToolInputView {
            file_path: Some("../fuera/a.rs"),
            ..Default::default()
        };
        let meta = extract_tool_meta("Read", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(meta.rel_path.as_deref(), Some(EXTERNAL_PATH));
    }

    #[test]
    fn pre_handshake_records_no_path_and_no_host() {
        let edit = ToolInputView {
            file_path: Some(r"C:\Users\dev\proyecto\src\main.rs"),
            ..Default::default()
        };
        let meta = extract_tool_meta("Edit", &edit, &PrivacyPolicy::PRE_HANDSHAKE, ROOT);
        assert_eq!(meta, ToolMeta::default());

        let fetch = ToolInputView {
            url: Some("https://example.com/a"),
            ..Default::default()
        };
        let meta = extract_tool_meta("WebFetch", &fetch, &PrivacyPolicy::PRE_HANDSHAKE, ROOT);
        assert_eq!(meta, ToolMeta::default());
    }

    #[test]
    fn bash_never_leaks_the_command() {
        let input = ToolInputView {
            command: Some("curl -H 'Authorization: SECRETO-TRAMPA' https://example.com"),
            run_in_background: Some(false),
            ..Default::default()
        };
        let meta = extract_tool_meta("Bash", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(
            meta.meta_json().as_deref(),
            Some(r#"{"run_in_background":false}"#)
        );
        assert!(!all_strings(&meta).contains(TRAP));
        assert!(!all_strings(&meta).contains("curl"));
    }

    #[test]
    fn shell_program_is_opt_in_and_only_the_first_token() {
        let policy = PrivacyPolicy {
            capture_shell_program: true,
            ..PrivacyPolicy::APP_DEFAULT
        };
        let input = ToolInputView {
            command: Some(r#""C:\Program Files\Git\bin\git.exe" push SECRETO-TRAMPA"#),
            ..Default::default()
        };
        // La ruta entre comillas con espacios se parte en el primer espacio: no se adivina.
        let meta = extract_tool_meta("Bash", &input, &policy, ROOT);
        assert!(!all_strings(&meta).contains(TRAP));

        let input = ToolInputView {
            command: Some("/usr/bin/cargo test --workspace"),
            ..Default::default()
        };
        let meta = extract_tool_meta("Bash", &input, &policy, ROOT);
        assert_eq!(meta.meta_json().as_deref(), Some(r#"{"program":"cargo"}"#));

        let input = ToolInputView {
            command: Some("TOKEN=SECRETO-TRAMPA deploy"),
            ..Default::default()
        };
        let meta = extract_tool_meta("Bash", &input, &policy, ROOT);
        assert_eq!(meta.meta_json(), None);
    }

    #[test]
    fn webfetch_keeps_only_the_host() {
        let input = ToolInputView {
            url: Some("https://user:SECRETO-TRAMPA@Docs.Example.com:8443/a/b?q=SECRETO-TRAMPA#x"),
            ..Default::default()
        };
        let meta = extract_tool_meta("WebFetch", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(meta.host.as_deref(), Some("docs.example.com"));
        assert_eq!(meta.meta_json(), None);
        assert!(!all_strings(&meta).contains(TRAP));
    }

    #[test]
    fn grep_glob_websearch_and_unknown_tools_keep_nothing() {
        let input = ToolInputView {
            file_path: Some(r"C:\Users\dev\proyecto\x"),
            url: Some("https://example.com"),
            command: Some(TRAP),
            subagent_type: Some(TRAP),
            replace_all: Some(true),
            run_in_background: Some(true),
        };
        for tool in ["Grep", "Glob", "WebSearch", "TodoWrite", "HerramientaNueva"] {
            let meta = extract_tool_meta(tool, &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
            assert_eq!(meta, ToolMeta::default(), "{tool}");
        }
    }

    #[test]
    fn mcp_keeps_server_and_tool_only() {
        let input = ToolInputView {
            url: Some("https://example.com/SECRETO-TRAMPA"),
            ..Default::default()
        };
        let meta = extract_tool_meta(
            "mcp__ccd_turn__update_plan",
            &input,
            &PrivacyPolicy::APP_DEFAULT,
            ROOT,
        );
        assert_eq!(
            meta.meta_json().as_deref(),
            Some(r#"{"server":"ccd_turn","tool":"update_plan"}"#)
        );
        assert_eq!(meta.host, None);
    }

    #[test]
    fn agent_keeps_type_and_background_flag() {
        let input = ToolInputView {
            subagent_type: Some("Explore"),
            run_in_background: Some(true),
            command: Some(TRAP),
            ..Default::default()
        };
        let meta = extract_tool_meta("Agent", &input, &PrivacyPolicy::APP_DEFAULT, ROOT);
        assert_eq!(
            meta.meta_json().as_deref(),
            Some(r#"{"run_in_background":true,"subagent_type":"Explore"}"#)
        );
    }

    #[test]
    fn produced_meta_never_has_path_keys() {
        let input = ToolInputView {
            file_path: Some(r"C:\Users\dev\proyecto\a.rs"),
            url: Some("https://example.com"),
            command: Some("git status"),
            subagent_type: Some("x"),
            replace_all: Some(true),
            run_in_background: Some(true),
        };
        let policy = PrivacyPolicy {
            capture_shell_program: true,
            ..PrivacyPolicy::APP_DEFAULT
        };
        for tool in [
            "Read",
            "Edit",
            "Write",
            "Bash",
            "PowerShell",
            "WebFetch",
            "Agent",
            "Grep",
            "mcp__a__b",
            "Otra",
        ] {
            let meta = extract_tool_meta(tool, &input, &policy, ROOT);
            assert!(meta_is_clean(&meta.meta), "{tool}");
        }
    }
}
