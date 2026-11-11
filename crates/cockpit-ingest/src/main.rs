//! CLI `cockpit-ingest`: ingesta de transcripts sin interfaz (SPEC §15, F1).
//!
//! Solo lee `~/.claude/projects`. Solo escribe en la base de datos indicada, que
//! por defecto es `%APPDATA%\dev-cockpit\cockpit.db`.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use cockpit_core::privacy::PrivacyPolicy;
use cockpit_ingest::tailer::{IngestStats, ingest_tree};
use cockpit_store::{DB_FILE_NAME, Store};

const USAGE: &str = "\
cockpit-ingest: ingesta de transcripts de Claude Code para DEV COCKPIT

USO:
    cockpit-ingest backfill [--dry-run] [--root <dir>] [--db <fichero>]
    cockpit-ingest verify [--db <fichero>]

ÓRDENES:
    backfill   Lee los transcripts nuevos o ampliados y los guarda en la base.
               Es incremental: repetirlo solo procesa lo que haya cambiado.
    verify     Compara los tokens deduplicados de cada sesión con su `cost-state`.

OPCIONES:
    --dry-run  Parsea todo desde cero y no escribe nada (medición B1).
    --root     Carpeta de transcripts. Por defecto: %USERPROFILE%\\.claude\\projects
    --db       Base de datos. Por defecto: %APPDATA%\\dev-cockpit\\cockpit.db
";

struct Args {
    command: String,
    dry_run: bool,
    root: Option<PathBuf>,
    db: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = env::args().skip(1);
    let command = it.next().ok_or("falta la orden")?;
    let mut args = Args {
        command,
        dry_run: false,
        root: None,
        db: None,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry-run" => args.dry_run = true,
            "--root" => args.root = Some(it.next().ok_or("--root necesita un valor")?.into()),
            "--db" => args.db = Some(it.next().ok_or("--db necesita un valor")?.into()),
            other => return Err(format!("opción desconocida: {other}")),
        }
    }
    Ok(args)
}

fn default_root() -> Result<PathBuf, String> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .ok_or("no se encuentra la carpeta del usuario")?;
    Ok(PathBuf::from(home).join(".claude").join("projects"))
}

fn default_db() -> Result<PathBuf, String> {
    let appdata = env::var_os("APPDATA").ok_or("no se encuentra %APPDATA%")?;
    Ok(PathBuf::from(appdata)
        .join("dev-cockpit")
        .join(DB_FILE_NAME))
}

fn open_store(db: Option<PathBuf>) -> Result<Store, String> {
    let path = match db {
        Some(path) => path,
        None => default_db()?,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("no se puede crear {}: {e}", dir.display()))?;
    }
    Store::open(&path).map_err(|e| format!("no se puede abrir {}: {e}", path.display()))
}

fn print_stats(stats: &IngestStats) {
    println!(
        "ficheros:          {} leídos de {}",
        stats.files_read, stats.files_seen
    );
    println!(
        "bytes leídos:      {:.1} MB",
        stats.bytes_read as f64 / 1_048_576.0
    );
    println!("líneas:            {}", stats.lines);
    println!("eventos:           {}", stats.events);
    println!("líneas corruptas:  {}", stats.corrupt_lines);
    println!(
        "deriva de esquema: {} líneas de tipos usados",
        stats.drift_lines
    );
    println!("tipos desconocidos: {} líneas", stats.unknown_lines);
    for (kind, n) in &stats.unknown_types {
        println!("    {kind}: {n}");
    }
    println!("ficheros truncados: {}", stats.truncated_files);
    println!("resultados huérfanos: {}", stats.orphan_results);
    println!(
        "tiempo:            {:.2} s ({:.0} MB/s)",
        stats.elapsed_secs,
        stats.megabytes_per_second()
    );
}

fn backfill(args: Args) -> Result<(), String> {
    let root = match args.root {
        Some(root) => root,
        None => default_root()?,
    };
    let mut store = if args.dry_run {
        None
    } else {
        Some(open_store(args.db)?)
    };
    let mut last_percent = u64::MAX;
    let stats = ingest_tree(
        &root,
        store.as_mut(),
        PrivacyPolicy::APP_DEFAULT,
        |stats, _| {
            let percent = stats.files_read * 100 / stats.files_seen.max(1);
            if percent / 10 != last_percent / 10 && stats.files_read > 0 {
                last_percent = percent;
                eprintln!("  {} / {} ficheros", stats.files_read, stats.files_seen);
            }
        },
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{}",
        if args.dry_run {
            "backfill (dry-run, sin escribir)"
        } else {
            "backfill"
        }
    );
    print_stats(&stats);
    if let Some(store) = &store {
        let t = store.totals().map_err(|e| e.to_string())?;
        println!("--- base de datos");
        println!("sesiones:          {}", t.sessions);
        println!(
            "mensajes de API:   {} (deduplicados por message_id)",
            t.api_messages
        );
        println!(
            "herramientas:      {} (+{} copias de historial)",
            t.tool_calls, t.tool_call_copies
        );
        println!("errores de API:    {}", t.api_errors);
        println!(
            "tokens:            entrada {} · salida {} · caché leída {} · caché creada {}",
            t.usage.input, t.usage.output, t.usage.cache_read, t.usage.cache_creation
        );
    }
    Ok(())
}

fn verify(args: Args) -> Result<(), String> {
    let store = open_store(args.db)?;
    let checks = store.cost_state_checks().map_err(|e| e.to_string())?;
    let matching = checks.iter().filter(|c| c.matches()).count();
    println!("sesiones con cost-state: {}", checks.len());
    println!("coinciden exactamente:   {matching}");
    println!("difieren:                {}", checks.len() - matching);
    for c in checks.iter().filter(|c| !c.matches()) {
        // Solo el principio del id, para poder localizar la sesión sin volcarlo entero.
        let id: String = c.session_id.chars().take(8).collect();
        println!(
            "    {id}…  salida {} / {}  entrada {} / {}  caché leída {} / {}  caché creada {} / {}",
            c.deduplicated.output,
            c.cost_state.output,
            c.deduplicated.input,
            c.cost_state.input,
            c.deduplicated.cache_read,
            c.cost_state.cache_read,
            c.deduplicated.cache_creation,
            c.cost_state.cache_creation
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match args.command.as_str() {
        "backfill" => backfill(args),
        "verify" => verify(args),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => {
            eprintln!("error: orden desconocida: {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
