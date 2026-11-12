//! Project Map (SPEC §9) de una o varias raíces, en solo lectura.
//!
//! ```text
//! cargo run -p cockpit-ingest --release --example project_map -- <raíz> [<raíz>…]
//! ```
//!
//! Imprime la estructura de primer nivel, los componentes, las relaciones internas
//! y los problemas, y cuánto tarda el escaneo. No escribe nada.

use std::path::Path;
use std::time::Instant;

use cockpit_core::project_map::DepScope;
use cockpit_ingest::git::GitReader;
use cockpit_ingest::project_scan::scan;

fn main() {
    let git = GitReader::new();
    for arg in std::env::args().skip(1) {
        let root = Path::new(&arg);
        let started = Instant::now();
        let s = scan(root, &git);
        let ms = started.elapsed().as_millis();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        println!(
            "== {name}: {:?}, {} ficheros, {} KiB, {} directorios{} · {ms} ms",
            s.source,
            s.tree.files,
            s.tree.bytes / 1024,
            s.tree.dirs.len(),
            if s.tree.truncated { " (cortado)" } else { "" },
        );
        if let Some(top) = s.tree.dirs.first() {
            let langs: Vec<String> = top
                .languages
                .iter()
                .take(6)
                .map(|l| format!("{} {}", l.language, l.files))
                .collect();
            println!("   lenguajes: {}", langs.join(", "));
        }
        for d in s.tree.dirs.iter().filter(|d| d.depth == 1).take(12) {
            println!(
                "   {:<28} {:>6} ficheros {:>8} KiB",
                d.rel_dir,
                d.files,
                d.bytes / 1024
            );
        }
        for c in &s.map.components {
            let external = s
                .map
                .deps
                .iter()
                .filter(|d| d.from == c.id && d.to.is_none() && d.scope != DepScope::Module)
                .count();
            println!(
                "   [{}] {}{} · {} · v{} · {external} dependencias externas{}",
                c.ecosystem.as_str(),
                c.name,
                if c.is_group { " (grupo)" } else { "" },
                if c.rel_dir.is_empty() {
                    "."
                } else {
                    &c.rel_dir
                },
                c.version.as_deref().unwrap_or("?"),
                if c.parts.is_empty() {
                    String::new()
                } else {
                    format!(" · partes: {}", c.parts.join(", "))
                },
            );
        }
        for d in s.map.deps.iter().filter(|d| d.to.is_some()) {
            println!(
                "   {} --{}--> {}",
                d.from,
                d.scope.as_str(),
                d.to.as_deref().unwrap_or("")
            );
        }
        for p in &s.map.problems {
            println!("   problema: {} ({})", p.manifest, p.reason);
        }
    }
}
