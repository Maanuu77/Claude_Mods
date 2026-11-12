//! Caché del Project Map (SPEC §9) sobre una **copia** de la base, nunca la real.
//!
//! ```text
//! cargo run -p cockpit-ingest --release --example project_map_cache -- <copia de cockpit.db> <raíz> [<raíz>…]
//! ```
//!
//! Abre la copia con el `Store` normal (así aplica las migraciones pendientes) y,
//! por cada raíz: escanea 10 veces sin caché, guarda el mapa, escanea 10 veces
//! con la caché comprobando que el mapa es idéntico y, por último, lo marca como
//! obsoleto (lo que hace el Watcher al ver cambiar un manifiesto) y comprueba que
//! se recalcula. Las raíces solo se leen.

use std::path::Path;
use std::time::Instant;

use cockpit_ingest::git::GitReader;
use cockpit_ingest::project_scan::scan_with_cache;
use cockpit_store::{NewProject, SCHEMA_VERSION, Store};

const RUNS: usize = 10;

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let mut args = std::env::args().skip(1);
    let db = args
        .next()
        .expect("uso: project_map_cache <copia de cockpit.db> <raíz>…");
    let roots: Vec<String> = args.collect();
    assert!(!roots.is_empty(), "falta al menos una raíz");

    let mut store = Store::open(Path::new(&db)).expect("no se puede abrir la copia");
    let version: u32 = store
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    println!("esquema de la copia: {version} (binario: {SCHEMA_VERSION})");
    let git = GitReader::new();

    for root in &roots {
        let path = Path::new(root);
        let root_id = store
            .upsert_project(
                &NewProject {
                    root,
                    git_dir: None,
                    is_worktree: false,
                    main_root: None,
                },
                0,
            )
            .expect("proyecto");

        let (cold, print) = scan_with_cache(path, &git, None);
        let print = print.expect("un mapa recién calculado trae su huella");
        // Sin caché, con el disco ya caliente: la misma condición que con caché.
        let mut uncached = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let started = Instant::now();
            let (scan, _) = scan_with_cache(path, &git, None);
            uncached.push(ms(started));
            assert_eq!(scan.map, cold.map);
        }
        uncached.sort_by(f64::total_cmp);
        store
            .save_project_map(root_id, &cold.map, &print, 1)
            .expect("guardar");

        let mut warm = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let started = Instant::now();
            let cached = store.project_map_cache(root_id).expect("leer caché");
            let (scan, again) = scan_with_cache(path, &git, cached.as_ref());
            warm.push(ms(started));
            assert!(
                scan.map_cached && again.is_none(),
                "{root}: no usó la caché"
            );
            assert_eq!(
                scan.map, cold.map,
                "{root}: el mapa de la caché no es idéntico"
            );
        }
        warm.sort_by(f64::total_cmp);

        store.mark_project_map_stale(root_id).expect("marcar");
        let cached = store.project_map_cache(root_id).unwrap();
        let (fresh, fresh_print) = scan_with_cache(path, &git, cached.as_ref());
        assert!(
            !fresh.map_cached && fresh_print.is_some(),
            "{root}: obsoleto y aun así de la caché"
        );

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        println!(
            "{name}: {} ficheros, {} manifiestos, {} componentes, {} dependencias · sin caché p50 {:.1} ms · con caché p50 {:.1} ms (máx. {:.1}) · obsoleto → recalculado",
            cold.tree.files,
            print.0.len(),
            cold.map.components.len(),
            cold.map.deps.len(),
            uncached[RUNS / 2],
            warm[RUNS / 2],
            warm[RUNS - 1],
        );
    }
}
