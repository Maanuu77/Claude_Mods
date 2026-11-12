//! Escaneo de una raíz para el Project Map (SPEC §9).
//!
//! - **Lista de ficheros:** `git ls-files` si la raíz tiene Git; si no, un recorrido
//!   que respeta `.gitignore` y las exclusiones fijas del Watcher. Como mucho
//!   [`MAX_ENTRIES`] ficheros.
//! - **De cada fichero solo se mira el tamaño** (`stat`) y, de los manifiestos,
//!   también la fecha de modificación.
//! - **Solo se leen los manifiestos** de la tabla de §9, hasta 1 MiB cada uno, y
//!   solo si su huella (ruta, bytes y fecha) no coincide con la del mapa guardado
//!   en la caché o si el Watcher vio cambiar alguno.
//!
//! Nada se escribe: ni en la raíz ni en Git (el `GitReader` es de solo lectura).
//! Guardar el mapa nuevo es cosa del escritor único de la base.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use cockpit_core::project_map::{
    self, CachedMap, MAX_ENTRIES, MAX_MANIFEST_BYTES, ManifestPrint, Problem, ProjectMap,
    TreeSummary,
};
use serde::Serialize;

use crate::git::GitReader;
use crate::watcher::EXCLUDED_DIRS;

/// De dónde sale la lista de ficheros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListSource {
    Git,
    Walk,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectScan {
    pub source: ListSource,
    pub tree: TreeSummary,
    pub map: ProjectMap,
    /// El mapa sale de la caché: ningún manifiesto ha cambiado desde que se guardó.
    pub map_cached: bool,
}

/// Escanea `root`. Nunca falla: lo que no se puede leer queda fuera o como problema.
pub fn scan(root: &Path, git: &GitReader) -> ProjectScan {
    scan_with_cache(root, git, None).0
}

/// Como [`scan`], pero reutiliza el mapa de `cached` si la huella de los manifiestos
/// es la misma y no está marcado como obsoleto. Si el mapa se ha calculado de
/// nuevo, devuelve también su huella, para guardarlo.
pub fn scan_with_cache(
    root: &Path,
    git: &GitReader,
    cached: Option<&CachedMap>,
) -> (ProjectScan, Option<ManifestPrint>) {
    let (paths, source, list_truncated) = match git.list_tracked(root) {
        Ok((paths, truncated)) if !paths.is_empty() => (paths, ListSource::Git, truncated),
        _ => {
            let (paths, truncated) = walk(root);
            (paths, ListSource::Walk, truncated)
        }
    };

    let sizes: Vec<(&str, u64)> = paths
        .iter()
        .take(MAX_ENTRIES)
        .map(|p| {
            let size = fs::metadata(root.join(native(p)))
                .map(|m| m.len())
                .unwrap_or(0);
            (p.as_str(), size)
        })
        .collect();
    let mut tree = project_map::summarize_tree(sizes.iter().copied());
    tree.truncated |= list_truncated;

    let mut print: Vec<(String, u64, i64)> = sizes
        .iter()
        .filter(|(p, _)| project_map::manifest_kind(p).is_some())
        .map(|(p, size)| ((*p).to_owned(), *size, modified_ms(&root.join(native(p)))))
        .collect();
    print.sort();
    let print = ManifestPrint(print);
    if let Some(c) = cached.filter(|c| !c.stale && c.print == print) {
        let scan = ProjectScan {
            source,
            tree,
            map: c.map.clone(),
            map_cached: true,
        };
        return (scan, None);
    }

    let mut too_large = Vec::new();
    let mut manifests: Vec<(String, String)> = Vec::new();
    for (path, size) in &sizes {
        if project_map::manifest_kind(path).is_none() {
            continue;
        }
        if *size as usize > MAX_MANIFEST_BYTES {
            too_large.push(Problem {
                manifest: (*path).to_owned(),
                reason: "too_large",
            });
            continue;
        }
        if let Ok(bytes) = fs::read(root.join(native(path))) {
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
            manifests.push(((*path).to_owned(), text));
        }
    }
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "proyecto".to_owned());
    let mut map = project_map::build_map(&root_name, &manifests);
    map.problems.extend(too_large);
    map.problems.sort_by(|a, b| a.manifest.cmp(&b.manifest));
    let scan = ProjectScan {
        source,
        tree,
        map,
        map_cached: false,
    };
    (scan, Some(print))
}

/// Fecha de modificación en ms, o 0 si no se puede leer.
fn modified_ms(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// Ficheros de `root` sin Git, con las reglas de ignorado. `true` si se cortó.
fn walk(root: &Path) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_some_and(|t| t.is_dir())
                && EXCLUDED_DIRS.iter().any(|x| name.eq_ignore_ascii_case(x)))
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        if out.len() >= MAX_ENTRIES {
            return (out, true);
        }
        out.push(rel.to_string_lossy().replace('\\', "/"));
    }
    out.sort();
    (out, false)
}

fn native(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(native(rel));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn a_folder_without_git_is_walked_with_its_ignore_rules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".gitignore", "secretos/\n");
        write(
            root,
            "package.json",
            "\u{feff}{\"name\":\"web\",\"dependencies\":{\"react\":\"^19\"}}",
        );
        write(root, "src/app.ts", "const a = 1;\n");
        write(root, "secretos/clave.txt", "x");
        write(
            root,
            "node_modules/react/package.json",
            "{\"name\":\"react\"}",
        );
        write(root, "dist/bundle.js", "x");
        let scan = scan(root, &GitReader::new());
        assert_eq!(scan.source, ListSource::Walk);
        let files: Vec<_> = scan.tree.dirs.iter().map(|d| d.rel_dir.as_str()).collect();
        assert_eq!(files, ["", "src"], "{:#?}", scan.tree);
        assert_eq!(scan.tree.files, 3, "gitignore, package.json y src/app.ts");
        assert_eq!(scan.map.components.len(), 1);
        assert_eq!(scan.map.components[0].name, "web");
        assert_eq!(scan.map.deps[0].name, "react");
    }

    #[test]
    fn the_cached_map_is_reused_until_a_manifest_changes_or_goes_stale() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "package.json", "{\"name\":\"web\"}");
        write(root, "src/app.ts", "const a = 1;\n");
        let git = GitReader::new();

        let (first, print) = scan_with_cache(root, &git, None);
        assert!(!first.map_cached);
        let print = print.expect("un mapa nuevo trae su huella");
        assert_eq!(print.0.len(), 1, "solo los manifiestos: {print:?}");
        assert_eq!(print.0[0].0, "package.json");

        // Un mapa guardado distinto prueba que se usa la caché y no los manifiestos.
        let mut saved = first.map.clone();
        saved.components[0].name = "de-la-cache".to_owned();
        let mut cached = CachedMap {
            map: saved,
            print: print.clone(),
            stale: false,
            scanned_at: 1,
        };
        // Cambiar otro fichero no invalida el mapa.
        write(root, "src/app.ts", "const a = 2; // más largo\n");
        let (again, none) = scan_with_cache(root, &git, Some(&cached));
        assert!(again.map_cached && none.is_none());
        assert_eq!(again.map.components[0].name, "de-la-cache");
        assert_eq!(again.tree.files, 2, "el árbol se calcula siempre");

        // Si el Watcher lo marcó, se recalcula aunque la huella coincida.
        cached.stale = true;
        let (fresh, _) = scan_with_cache(root, &git, Some(&cached));
        assert!(!fresh.map_cached);
        assert_eq!(fresh.map.components[0].name, "web");

        // Un manifiesto que cambia de tamaño, también.
        cached.stale = false;
        write(root, "package.json", "{\"name\":\"web-2\"}");
        let (changed, new_print) = scan_with_cache(root, &git, Some(&cached));
        assert!(!changed.map_cached);
        assert_eq!(changed.map.components[0].name, "web-2");
        assert_ne!(new_print.unwrap(), print);

        // Y uno nuevo, aunque los demás no cambien.
        cached.print = scan_with_cache(root, &git, None).1.unwrap();
        write(root, "pkg/package.json", "{\"name\":\"pkg\"}");
        assert!(!scan_with_cache(root, &git, Some(&cached)).0.map_cached);
    }
}
