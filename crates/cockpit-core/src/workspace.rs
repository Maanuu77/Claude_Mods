//! Cambios de fichero del Workspace Watcher (SPEC §6.3 `FileChanged`, §7).
//!
//! Un cambio pertenece a una raíz de proyecto, no a una sesión: por eso no viaja
//! como `CockpitEvent`, que siempre lleva `session_id`. Nunca lleva contenido.

use serde::Serialize;

/// Tipo de cambio, tal como lo agrupa el debouncer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    /// El origen va en `from_rel_path`.
    Renamed,
}

impl ChangeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Created => "created",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Renamed => "renamed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "created" => Some(ChangeKind::Created),
            "modified" => Some(ChangeKind::Modified),
            "deleted" => Some(ChangeKind::Deleted),
            "renamed" => Some(ChangeKind::Renamed),
            _ => None,
        }
    }
}

/// Un cambio ya filtrado (exclusiones y `.gitignore`), relativo a su raíz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Ruta relativa a la raíz, con `/`.
    pub rel_path: String,
    pub change: ChangeKind,
    pub from_rel_path: Option<String>,
    /// Tamaño en bytes si el fichero existe al procesarlo.
    pub size: Option<u64>,
    /// Hora del primer evento crudo agrupado (reloj de pared, ms).
    pub ts_ms: i64,
}

/// Une el prefijo del `cwd` dentro de la raíz con una ruta relativa al `cwd`.
///
/// El mod y el parser relativizan las rutas al `cwd` de la sesión, y el Watcher a
/// la raíz de Git. `prefix` es lo que devuelve `git rev-parse --show-prefix`
/// (`sub/dir/` o vacío). Las rutas `<externo>` no se traducen.
pub fn root_relative(prefix: &str, cwd_relative: &str) -> Option<String> {
    if cwd_relative == crate::privacy::EXTERNAL_PATH {
        return None;
    }
    let prefix = prefix.replace('\\', "/");
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        Some(cwd_relative.to_owned())
    } else {
        Some(format!("{prefix}/{cwd_relative}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_kind_round_trips() {
        for k in [
            ChangeKind::Created,
            ChangeKind::Modified,
            ChangeKind::Deleted,
            ChangeKind::Renamed,
        ] {
            assert_eq!(ChangeKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(ChangeKind::parse("otro"), None);
    }

    #[test]
    fn cwd_paths_become_root_paths() {
        assert_eq!(root_relative("", "src/a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(
            root_relative("crates/core/", "src/a.rs").as_deref(),
            Some("crates/core/src/a.rs")
        );
        assert_eq!(root_relative("sub/", "<externo>"), None);
    }
}
