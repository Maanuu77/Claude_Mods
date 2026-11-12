//! Caché del Project Map (SPEC §9 y §6.4: `project_components` y `project_deps`).
//!
//! Guarda la última instantánea de cada raíz junto con la huella de sus manifiestos
//! (ruta, bytes y fecha de modificación). La vista la reutiliza mientras la huella
//! no cambie y el Watcher no haya visto cambiar un manifiesto (`stale`). No es un
//! histórico: cada escaneo nuevo sustituye al anterior.

use cockpit_core::project_map::{
    CachedMap, Component, DepScope, Dependency, Ecosystem, ManifestPrint, Problem, ProjectMap,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::{Result, Store};

/// Listas y tuplas de texto: serializarlas no falla. Si fallara, lo guardado no se
/// entendería al leerlo y el mapa se volvería a calcular.
fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

impl Store {
    /// Mapa guardado de una raíz. `None` si no hay, o si lo guardado ya no se
    /// entiende (otra versión): entonces se vuelve a calcular.
    pub fn project_map_cache(&self, root_id: i64) -> Result<Option<CachedMap>> {
        let conn = &self.conn;
        let Some((scanned_at, manifests, problems, stale)) = conn
            .query_row(
                "SELECT scanned_at, manifests_json, problems_json, stale
                   FROM project_map_state WHERE root_id = ?1",
                [root_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let Ok(print) = serde_json::from_str::<Vec<(String, u64, i64)>>(&manifests) else {
            return Ok(None);
        };
        let Ok(problems) = serde_json::from_str::<Vec<(String, String)>>(&problems) else {
            return Ok(None);
        };
        let mut map = ProjectMap::default();
        for (manifest, reason) in problems {
            let Some(reason) = Problem::known_reason(&reason) else {
                return Ok(None);
            };
            map.problems.push(Problem { manifest, reason });
        }

        let mut stmt = conn.prepare_cached(
            "SELECT component_id, kind, name, rel_dir, manifest_rel_path, version, is_group, parts_json
               FROM project_components WHERE root_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map([root_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, bool>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        for row in rows {
            let (id, kind, name, rel_dir, manifest, version, is_group, parts) = row?;
            let (Some(ecosystem), Ok(parts)) = (
                Ecosystem::parse(&kind),
                serde_json::from_str::<Vec<String>>(&parts),
            ) else {
                return Ok(None);
            };
            map.components.push(Component {
                id,
                ecosystem,
                name,
                rel_dir,
                manifest,
                version,
                is_group,
                parts,
            });
        }

        let mut stmt = conn.prepare_cached(
            "SELECT from_component, to_name, to_component, version_req, scope
               FROM project_deps WHERE root_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map([root_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (from, name, to, version_req, scope) = row?;
            let Some(scope) = DepScope::parse(&scope) else {
                return Ok(None);
            };
            map.deps.push(Dependency {
                from,
                name,
                version_req,
                scope,
                to,
            });
        }
        Ok(Some(CachedMap {
            map,
            print: ManifestPrint(print),
            stale,
            scanned_at,
        }))
    }

    /// Sustituye el mapa guardado de una raíz, en una transacción.
    pub fn save_project_map(
        &mut self,
        root_id: i64,
        map: &ProjectMap,
        print: &ManifestPrint,
        now_ms: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM project_components WHERE root_id = ?1",
            [root_id],
        )?;
        tx.execute("DELETE FROM project_deps WHERE root_id = ?1", [root_id])?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO project_components(root_id, component_id, kind, name, rel_dir,
                     manifest_rel_path, version, is_group, parts_json, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for (i, c) in map.components.iter().enumerate() {
                insert.execute(params![
                    root_id,
                    c.id,
                    c.ecosystem.as_str(),
                    c.name,
                    c.rel_dir,
                    c.manifest,
                    c.version,
                    c.is_group,
                    json(&c.parts),
                    i as i64,
                ])?;
            }
            let mut insert = tx.prepare_cached(
                "INSERT INTO project_deps(root_id, position, from_component, to_name, to_component,
                     version_req, scope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (i, d) in map.deps.iter().enumerate() {
                insert.execute(params![
                    root_id,
                    i as i64,
                    d.from,
                    d.name,
                    d.to,
                    d.version_req,
                    d.scope.as_str(),
                ])?;
            }
        }
        let problems: Vec<(&str, &str)> = map
            .problems
            .iter()
            .map(|p| (p.manifest.as_str(), p.reason))
            .collect();
        tx.execute(
            "INSERT INTO project_map_state(root_id, scanned_at, manifests_json, problems_json, stale)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(root_id) DO UPDATE SET scanned_at = excluded.scanned_at,
                 manifests_json = excluded.manifests_json,
                 problems_json = excluded.problems_json, stale = 0",
            params![
                root_id,
                now_ms,
                json(&print.0),
                json(&problems),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// El Watcher vio cambiar un manifiesto de la raíz: el mapa guardado ya no vale.
    pub fn mark_project_map_stale(&mut self, root_id: i64) -> Result<()> {
        self.conn
            .prepare_cached(
                "UPDATE project_map_state SET stale = 1 WHERE root_id = ?1 AND stale = 0",
            )?
            .execute([root_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use cockpit_core::project_map::build_map;

    use super::*;
    use crate::NewProject;

    fn project(store: &mut Store) -> i64 {
        store
            .upsert_project(
                &NewProject {
                    root: "C:/repos/web",
                    git_dir: None,
                    is_worktree: false,
                    main_root: None,
                },
                1,
            )
            .unwrap()
    }

    fn sample() -> ProjectMap {
        build_map(
            "web",
            &[
                (
                    "package.json".to_owned(),
                    r#"{"name":"web","workspaces":["packages/*"],"devDependencies":{"vite":"^7"}}"#
                        .to_owned(),
                ),
                (
                    "packages/ui/package.json".to_owned(),
                    r#"{"name":"ui","version":"1.0.0","dependencies":{"react":"^19","core":"workspace:*"}}"#
                        .to_owned(),
                ),
                (
                    "packages/core/package.json".to_owned(),
                    r#"{"name":"core","version":"0.2.0"}"#.to_owned(),
                ),
                ("default.project.json".to_owned(), "{".to_owned()),
            ],
        )
    }

    #[test]
    fn a_saved_map_reads_back_identical_and_replaces_the_previous_one() {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store);
        assert_eq!(store.project_map_cache(root).unwrap(), None);

        let map = sample();
        assert!(map.components.len() >= 3 && !map.deps.is_empty() && !map.problems.is_empty());
        let print = ManifestPrint(vec![("package.json".to_owned(), 120, 1_000)]);
        store.save_project_map(root, &map, &print, 50).unwrap();
        let cached = store.project_map_cache(root).unwrap().unwrap();
        assert_eq!(cached.map, map);
        assert_eq!(cached.print, print);
        assert_eq!((cached.stale, cached.scanned_at), (false, 50));

        // Un escaneo nuevo sustituye al anterior entero.
        let smaller = build_map(
            "web",
            &[("package.json".to_owned(), r#"{"name":"web"}"#.to_owned())],
        );
        store
            .save_project_map(root, &smaller, &ManifestPrint::default(), 60)
            .unwrap();
        let cached = store.project_map_cache(root).unwrap().unwrap();
        assert_eq!(cached.map, smaller);
        assert_eq!(cached.scanned_at, 60);
    }

    #[test]
    fn a_manifest_change_marks_the_map_stale_until_the_next_save() {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store);
        // Sin mapa guardado no hay nada que marcar.
        store.mark_project_map_stale(root).unwrap();
        assert_eq!(store.project_map_cache(root).unwrap(), None);

        store
            .save_project_map(root, &sample(), &ManifestPrint::default(), 50)
            .unwrap();
        store.mark_project_map_stale(root).unwrap();
        assert!(store.project_map_cache(root).unwrap().unwrap().stale);
        store
            .save_project_map(root, &sample(), &ManifestPrint::default(), 70)
            .unwrap();
        assert!(!store.project_map_cache(root).unwrap().unwrap().stale);
    }

    #[test]
    fn an_unreadable_cache_is_ignored() {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store);
        store
            .save_project_map(root, &sample(), &ManifestPrint::default(), 50)
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE project_deps SET scope = 'otro' WHERE root_id = ?1",
                [root],
            )
            .unwrap();
        assert_eq!(store.project_map_cache(root).unwrap(), None);
    }
}
