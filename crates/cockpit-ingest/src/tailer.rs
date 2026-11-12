//! `TranscriptTailer`: lectura incremental de los JSONL de `~/.claude/projects`
//! (SPEC §6.1). **Solo lectura:** nunca se escribe en esos ficheros.
//!
//! F1 cubre el descubrimiento, la lectura por offsets y el backfill. La vigilancia
//! en vivo con `notify` llega en F2, sobre estas mismas funciones.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::model::{CockpitEvent, EventKind, Source};
use cockpit_core::privacy::PrivacyPolicy;
use cockpit_core::tasks::{self, AgentState};
use cockpit_core::transcript::{
    LineClass, LineError, PARSER_VERSION, TranscriptParser, introduced_in,
};
use cockpit_store::{Batch, FileOffset, IngestError, Store, StoreError};

/// Eventos por transacción durante el backfill.
const FLUSH_EVENTS: usize = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum IngestFailure {
    #[error("error de E/S: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Un JSONL de transcript encontrado en disco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFile {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub created_ms: Option<i64>,
    /// `<sesión>/subagents/agent-*.jsonl`.
    pub is_subagent: bool,
}

impl TranscriptFile {
    fn from_metadata(path: PathBuf, meta: &fs::Metadata) -> Self {
        // `subagents/agent-*.jsonl` y también `subagents/workflows/<wf>/agent-*.jsonl`.
        let is_subagent = subagents_dir(&path).is_some();
        Self {
            size: meta.len(),
            mtime_ms: millis(meta.modified()).unwrap_or(0),
            created_ms: millis(meta.created()),
            is_subagent,
            path,
        }
    }

    /// Estado actual de un JSONL, o `None` si no existe o no es un transcript.
    pub fn from_path(path: &Path) -> Option<Self> {
        if path.extension().is_none_or(|e| e != "jsonl") {
            return None;
        }
        let meta = fs::metadata(path).ok()?;
        meta.is_file()
            .then(|| Self::from_metadata(path.to_path_buf(), &meta))
    }

    /// El fichero principal de una sesión se llama `<session_id>.jsonl`.
    pub fn session_id(&self) -> Option<&str> {
        if self.is_subagent {
            return None;
        }
        self.path.file_stem()?.to_str()
    }
}

/// La carpeta `subagents` que contiene `path`, si la hay (como mucho tres niveles).
fn subagents_dir(path: &Path) -> Option<&Path> {
    path.ancestors()
        .skip(1)
        .take(3)
        .find(|a| a.file_name().is_some_and(|n| n == "subagents"))
}

/// Metadatos de un subagente que se pueden leer: nunca su `description`.
#[derive(serde::Deserialize)]
struct SubagentMeta {
    #[serde(rename = "agentType", default)]
    agent_type: Option<String>,
    #[serde(default)]
    finished: Option<bool>,
}

/// `.meta.json` de un subagente (A4): `<sesión>/subagents/…/agent-<id>.meta.json`.
pub fn is_subagent_meta(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("agent-") && n.ends_with(".meta.json"))
        && subagents_dir(path).is_some()
}

/// Tipo y fin de un subagente según su `.meta.json` [INT]. Solo se leen `agentType`
/// y `finished`; la sesión sale de la carpeta y el agente, del nombre del fichero.
pub fn subagent_meta_event(meta_path: &Path, ts_ms: i64) -> Option<CockpitEvent> {
    let name = meta_path.file_name()?.to_str()?;
    let agent = tasks::clean_id(name.strip_prefix("agent-")?.strip_suffix(".meta.json")?)?;
    let session_id = subagents_dir(meta_path)?
        .parent()?
        .file_name()?
        .to_str()?
        .to_owned();
    let meta: SubagentMeta = serde_json::from_slice(&fs::read(meta_path).ok()?).ok()?;
    let agent_type = meta.agent_type.as_deref().and_then(tasks::clean_agent_type);
    let status = (meta.finished == Some(true)).then_some(AgentState::Completed);
    if agent_type.is_none() && status.is_none() {
        return None;
    }
    Some(CockpitEvent {
        source: Source::Transcript,
        session_id,
        agent_id: None,
        ts_ms,
        engine_version: None,
        kind: EventKind::AgentStatus {
            agent,
            agent_type,
            status,
            duration_ms: None,
        },
    })
}

fn meta_path_of(jsonl: &Path) -> PathBuf {
    let mut name = jsonl.file_stem().unwrap_or_default().to_os_string();
    name.push(".meta.json");
    jsonl.with_file_name(name)
}

fn millis(t: io::Result<SystemTime>) -> Option<i64> {
    let d = t.ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

/// Busca todos los `.jsonl` bajo `root`, del más reciente al más antiguo.
pub fn scan_transcripts(root: &Path) -> io::Result<Vec<TranscriptFile>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            // Un directorio que desaparece a mitad del recorrido no es un error.
            Err(e) if e.kind() == io::ErrorKind::NotFound && dir != root => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let Ok(meta) = entry.metadata() else { continue };
            let path = entry.path();
            if meta.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                files.push(TranscriptFile::from_metadata(path, &meta));
            }
        }
    }
    files.sort_by(|a, b| {
        b.mtime_ms
            .cmp(&a.mtime_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(files)
}

/// Lee las líneas **completas** a partir de `start` y devuelve la nueva posición.
///
/// Una última línea sin salto final se deja sin consumir: Claude puede estar a
/// mitad de escribirla, y se leerá entera en la siguiente pasada.
pub fn read_complete_lines(
    file: &mut File,
    start: u64,
    mut on_line: impl FnMut(&[u8], u64) -> Result<(), IngestFailure>,
) -> Result<u64, IngestFailure> {
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut buf = Vec::with_capacity(1 << 16);
    let mut offset = start;
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            return Ok(offset);
        }
        offset += n as u64;
        on_line(&buf[..n - 1], offset)?;
    }
}

/// Contadores de una pasada de ingesta. No contienen rutas ni contenido.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct IngestStats {
    pub files_seen: u64,
    pub files_read: u64,
    pub bytes_read: u64,
    pub lines: u64,
    pub events: u64,
    pub corrupt_lines: u64,
    pub drift_lines: u64,
    /// Llamadas a herramientas de tareas con una forma que el adaptador no conoce.
    pub adapter_drift: u64,
    pub unknown_lines: u64,
    pub truncated_files: u64,
    /// Ficheros leídos con una versión anterior del parser que se releen para
    /// recuperar solo los eventos nuevos ([`PARSER_VERSION`]).
    pub reread_files: u64,
    pub orphan_results: u64,
    /// Tipos de línea desconocidos y cuántas veces aparecen.
    pub unknown_types: Vec<(String, u64)>,
    /// Sesiones con eventos en esta pasada.
    pub touched_sessions: BTreeSet<String>,
    pub elapsed_secs: f64,
}

impl IngestStats {
    pub fn megabytes_per_second(&self) -> f64 {
        if self.elapsed_secs > 0.0 {
            self.bytes_read as f64 / 1_048_576.0 / self.elapsed_secs
        } else {
            0.0
        }
    }

    fn unknown(&mut self, kind: String) {
        self.unknown_lines += 1;
        match self.unknown_types.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => self.unknown_types.push((kind, 1)),
        }
    }
}

fn now_ms() -> i64 {
    millis(Ok(SystemTime::now())).unwrap_or(0)
}

/// Ingesta incremental de todo `root`.
///
/// Con `store`, cada fichero continúa desde su offset guardado, y los eventos y el
/// nuevo offset se escriben en la misma transacción: repetir la pasada no duplica
/// nada. Sin `store` (`--dry-run`) se parsea todo desde cero y no se escribe nada.
pub fn ingest_tree(
    root: &Path,
    mut store: Option<&mut Store>,
    policy: PrivacyPolicy,
    mut progress: impl FnMut(&IngestStats, u64),
) -> Result<IngestStats, IngestFailure> {
    let started = Instant::now();
    let files = scan_transcripts(root)?;
    let total_bytes: u64 = files.iter().map(|f| f.size).sum();
    let mut stats = IngestStats {
        files_seen: files.len() as u64,
        ..Default::default()
    };
    for file in &files {
        ingest_file(file, store.as_deref_mut(), policy, &mut stats)?;
        progress(&stats, total_bytes);
    }
    stats
        .unknown_types
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    stats.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(stats)
}

/// Ingesta incremental de un solo fichero. Ver [`ingest_tree`].
pub fn ingest_file(
    file: &TranscriptFile,
    mut store: Option<&mut Store>,
    policy: PrivacyPolicy,
    stats: &mut IngestStats,
) -> Result<(), IngestFailure> {
    let path_key = file.path.to_string_lossy().into_owned();
    let mut errors: Vec<IngestError> = Vec::new();
    let mut start = 0;
    // Lo leído hasta `until` con un parser anterior (`old`): se relee entero, pero de
    // esa parte solo se aplican los eventos que la versión `old` no producía.
    let mut catch_up: Option<CatchUp> = None;
    if let Some(store) = store.as_deref()
        && let Some(prev) = store.file_offset(&path_key)?
    {
        if file.size < prev.offset {
            // El fichero se ha truncado o reemplazado: se vuelve a leer entero.
            stats.truncated_files += 1;
            errors.push(error(&path_key, "truncated", ""));
        } else if prev.parser_version < PARSER_VERSION {
            stats.reread_files += 1;
            catch_up = Some(CatchUp {
                until: prev.offset,
                old: prev.parser_version,
            });
        } else if file.size == prev.offset {
            return Ok(());
        } else {
            start = prev.offset;
        }
    }

    // En Windows, `File::open` comparte lectura, escritura y borrado: no bloquea a Claude.
    let mut handle = match File::open(&file.path) {
        Ok(handle) => handle,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    stats.files_read += 1;

    let mut parser = TranscriptParser::new(policy);
    let mut events: Vec<CockpitEvent> = Vec::new();
    if start == 0
        && catch_up.is_none()
        && let (Some(session_id), Some(created)) = (file.session_id(), file.created_ms)
    {
        events.push(CockpitEvent {
            source: Source::Transcript,
            session_id: session_id.to_owned(),
            agent_id: None,
            ts_ms: created,
            engine_version: None,
            kind: EventKind::TranscriptFile {
                created_at_ms: created,
            },
        });
    }

    let end = read_complete_lines(&mut handle, start, |line, offset| {
        stats.lines += 1;
        // Parte ya ingerida con un parser anterior: sus incidencias ya constan.
        let old_part = catch_up.filter(|c| offset <= c.until);
        let before = events.len();
        match parser.feed(line, &mut events) {
            Ok(LineClass::Handled | LineClass::Ignored) => {}
            Ok(LineClass::Unknown(_)) if old_part.is_some() => {}
            Ok(LineClass::Unknown(kind)) => {
                push_error(&mut errors, "", "unknown_type", &kind);
                stats.unknown(kind);
            }
            Err(_) if old_part.is_some() => {}
            Err(LineError::Corrupt) => {
                stats.corrupt_lines += 1;
                push_error(&mut errors, &path_key, "corrupt", "");
            }
            Err(LineError::Drift(kind)) => {
                stats.drift_lines += 1;
                push_error(&mut errors, &path_key, "drift", &kind);
            }
        }
        if let Some(c) = old_part {
            c.keep_new(&mut events, before);
        }
        // Una herramienta de tareas con forma inesperada: la línea vale, la operación no.
        // Los adaptadores son de la versión 2: también cuentan en la parte releída.
        for note in parser.take_notes() {
            if old_part.is_some_and(|c| c.old >= 2) {
                continue;
            }
            stats.adapter_drift += 1;
            push_error(&mut errors, &path_key, "drift", &note);
        }
        if events.len() >= FLUSH_EVENTS {
            flush(
                store.as_deref_mut(),
                file,
                &path_key,
                recorded(offset, catch_up),
                &mut events,
                &mut errors,
                stats,
            )?;
        }
        Ok(())
    })?;
    let before = events.len();
    parser.finish(&mut events);
    if let Some(c) = catch_up.filter(|c| end <= c.until) {
        c.keep_new(&mut events, before);
    }
    if file.is_subagent
        && end > start
        && let Some(event) = subagent_meta_event(&meta_path_of(&file.path), file.mtime_ms)
    {
        events.push(event);
    }
    stats.bytes_read += end - start;
    flush(
        store,
        file,
        &path_key,
        recorded(end, catch_up),
        &mut events,
        &mut errors,
        stats,
    )
}

/// Relectura de lo ingerido con un parser anterior (ver [`PARSER_VERSION`]).
#[derive(Debug, Clone, Copy)]
struct CatchUp {
    /// Hasta dónde se había leído.
    until: u64,
    /// Versión del parser con la que se leyó.
    old: u32,
}

impl CatchUp {
    /// Quita de `events[from..]` lo que la versión anterior ya producía.
    fn keep_new(self, events: &mut Vec<CockpitEvent>, from: usize) {
        let mut tail = events.split_off(from);
        tail.retain(|e| introduced_in(&e.kind) > self.old);
        events.append(&mut tail);
    }
}

/// Offset y versión que se guardan tras leer hasta `offset`. Mientras quede parte
/// antigua por releer se conserva lo que había: si la relectura se interrumpe,
/// vuelve a empezar y nunca aplica dos veces los eventos antiguos.
fn recorded(offset: u64, catch_up: Option<CatchUp>) -> (u64, u32) {
    match catch_up {
        Some(c) if offset < c.until => (c.until, c.old),
        _ => (offset, PARSER_VERSION),
    }
}

fn error(path: &str, kind: &'static str, detail: &str) -> IngestError {
    IngestError {
        path: path.to_owned(),
        kind,
        detail: detail.to_owned(),
        count: 1,
    }
}

fn push_error(errors: &mut Vec<IngestError>, path: &str, kind: &'static str, detail: &str) {
    match errors
        .iter_mut()
        .find(|e| e.kind == kind && e.detail == detail && e.path == path)
    {
        Some(e) => e.count += 1,
        None => errors.push(error(path, kind, detail)),
    }
}

fn flush(
    store: Option<&mut Store>,
    file: &TranscriptFile,
    path_key: &str,
    (offset, parser_version): (u64, u32),
    events: &mut Vec<CockpitEvent>,
    errors: &mut Vec<IngestError>,
    stats: &mut IngestStats,
) -> Result<(), IngestFailure> {
    stats.events += events.len() as u64;
    for e in events.iter() {
        if !stats.touched_sessions.contains(&e.session_id) {
            stats.touched_sessions.insert(e.session_id.clone());
        }
    }
    if let Some(store) = store {
        let offsets = [FileOffset {
            path: path_key.to_owned(),
            size: file.size,
            offset,
            mtime_ms: file.mtime_ms,
            parser_version,
        }];
        let batch_stats = store.apply(&Batch {
            events,
            offsets: &offsets,
            errors,
            now_ms: now_ms(),
            mod_ack: None,
        })?;
        stats.orphan_results += batch_stats.orphan_results;
    }
    events.clear();
    errors.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SESSION: &str = "11111111-1111-4111-8111-111111111111";

    fn assistant(n: u32, output: u32) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{SESSION}","timestamp":"2026-11-10T10:00:{n:02}.000Z","version":"2.1.321","cwd":"C:\\w\\repo","message":{{"id":"msg_{n}","model":"claude-test","usage":{{"input_tokens":1,"output_tokens":{output}}},"content":[{{"type":"text","text":"hola"}}]}}}}"#
        )
    }

    fn transcript(dir: &Path) -> PathBuf {
        let project = dir.join("C--w-repo");
        fs::create_dir_all(&project).unwrap();
        project.join(format!("{SESSION}.jsonl"))
    }

    fn append(path: &Path, text: &str) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn run(root: &Path, store: &mut Store) -> IngestStats {
        ingest_tree(root, Some(store), PrivacyPolicy::APP_DEFAULT, |_, _| {}).unwrap()
    }

    #[test]
    fn an_incomplete_last_line_waits_for_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path());
        let second = assistant(2, 20);
        let (head, tail) = second.split_at(60);
        append(&path, &format!("{}\n{head}", assistant(1, 10)));

        let mut store = Store::open_in_memory().unwrap();
        let stats = run(dir.path(), &mut store);
        assert_eq!(stats.lines, 1);
        assert_eq!(stats.corrupt_lines, 0);
        assert_eq!(store.totals().unwrap().api_messages, 1);

        append(&path, &format!("{tail}\n"));
        let stats = run(dir.path(), &mut store);
        assert_eq!(stats.lines, 1);
        let totals = store.totals().unwrap();
        assert_eq!(totals.api_messages, 2);
        assert_eq!(totals.usage.output, 30);
    }

    #[test]
    fn a_second_pass_without_changes_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path());
        append(
            &path,
            &format!("{}\n{}\n", assistant(1, 10), assistant(2, 20)),
        );

        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(run(dir.path(), &mut store).files_read, 1);
        let again = run(dir.path(), &mut store);
        assert_eq!((again.files_seen, again.files_read, again.lines), (1, 0, 0));
        assert_eq!(store.totals().unwrap().api_messages, 2);
    }

    #[test]
    fn a_truncated_file_is_read_again_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path());
        append(
            &path,
            &format!("{}\n{}\n", assistant(1, 10), assistant(2, 20)),
        );
        let mut store = Store::open_in_memory().unwrap();
        run(dir.path(), &mut store);

        fs::write(&path, format!("{}\n", assistant(3, 5))).unwrap();
        let stats = run(dir.path(), &mut store);
        assert_eq!(stats.truncated_files, 1);
        assert_eq!(stats.lines, 1);
        assert_eq!(store.totals().unwrap().api_messages, 3);
    }

    #[test]
    fn corrupt_and_unknown_lines_are_counted_and_do_not_stop_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path());
        append(
            &path,
            &format!(
                "{}\n{{\"type\":\"assistant\", roto\n{{\"type\":\"tipo-nuevo\"}}\n{}\n",
                assistant(1, 10),
                assistant(2, 20)
            ),
        );
        let mut store = Store::open_in_memory().unwrap();
        let stats = run(dir.path(), &mut store);
        assert_eq!(stats.corrupt_lines, 1);
        assert_eq!(stats.unknown_types, [("tipo-nuevo".to_owned(), 1)]);
        assert_eq!(store.totals().unwrap().api_messages, 2);
        let kinds: Vec<String> = store
            .connection()
            .prepare("SELECT kind FROM ingest_errors ORDER BY kind")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(kinds, ["corrupt", "unknown_type"]);
    }

    #[test]
    fn subagent_files_are_found_and_never_named_as_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let main = transcript(dir.path());
        append(&main, &format!("{}\n", assistant(1, 10)));
        let sub = main.with_extension("").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        append(
            &sub.join("agent-a1.jsonl"),
            &format!("{}\n", assistant(2, 20)),
        );

        let files = scan_transcripts(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
        let sessions: Vec<_> = files.iter().filter_map(|f| f.session_id()).collect();
        assert_eq!(sessions, [SESSION]);
    }

    #[test]
    fn workflow_subagents_are_subagents_and_their_meta_is_read_without_description() {
        let dir = tempfile::tempdir().unwrap();
        let session = "33333333-3333-4333-8333-333333333333";
        let wf = dir
            .path()
            .join("proyecto")
            .join(session)
            .join("subagents/workflows/wf_1");
        fs::create_dir_all(&wf).unwrap();
        let jsonl = wf.join("agent-a1b2c3.jsonl");
        append(&jsonl, &format!("{}\n", assistant(1, 10)));
        fs::write(
            wf.join("agent-a1b2c3.meta.json"),
            r#"{"agentType":"workflow-subagent","description":"SECRETO-TRAMPA","finished":true,"spawnDepth":1}"#,
        )
        .unwrap();

        let file = TranscriptFile::from_path(&jsonl).unwrap();
        assert!(file.is_subagent);
        assert_eq!(file.session_id(), None);

        let meta = wf.join("agent-a1b2c3.meta.json");
        assert!(is_subagent_meta(&meta));
        let event = subagent_meta_event(&meta, 7).unwrap();
        assert_eq!(event.session_id, session);
        assert!(!format!("{event:?}").contains("SECRETO"));
        assert!(matches!(
            event.kind,
            EventKind::AgentStatus { ref agent, status: Some(AgentState::Completed), ref agent_type, .. }
                if agent == "a1b2c3" && agent_type.as_deref() == Some("workflow-subagent")
        ));

        // Ninguna sesión llamada `agent-*`: solo la real.
        let mut store = Store::open_in_memory().unwrap();
        run(dir.path(), &mut store);
        let ids: Vec<String> = store
            .connection()
            .prepare("SELECT id FROM sessions")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(ids.iter().all(|id| !id.starts_with("agent-")), "{ids:?}");
        let agents: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE agent_type = 'workflow-subagent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(agents, 1);
    }

    fn tool_line(n: u32, tool_id: &str, name: &str, input: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{SESSION}","timestamp":"2026-11-10T10:01:{n:02}.000Z","version":"2.1.321","cwd":"C:\\w\\repo","message":{{"id":"msg_t{n}","model":"claude-test","usage":{{"input_tokens":1,"output_tokens":1}},"content":[{{"type":"tool_use","id":"{tool_id}","name":"{name}","input":{input}}}]}}}}"#
        )
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store.connection().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn history_read_by_an_older_parser_gets_only_the_new_events() {
        // Base ingerida hasta F4a (parser 1): sin tareas, y con la ruta de una
        // herramienta ya purgada por la retención (D3).
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path());
        append(
            &path,
            &format!(
                "{}\n{}\n{}\n",
                assistant(1, 10),
                tool_line(
                    2,
                    "t_edit",
                    "Edit",
                    r#"{"file_path":"C:\\w\\repo\\src\\a.rs","old_string":"x","new_string":"y"}"#
                ),
                tool_line(
                    3,
                    "t_todo",
                    "TodoWrite",
                    r#"{"todos":[{"content":"Leer","status":"completed"},{"content":"Escribir","status":"in_progress"}]}"#
                ),
            ),
        );
        let mut store = Store::open_in_memory().unwrap();
        run(dir.path(), &mut store);
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM tool_calls WHERE rel_path IS NOT NULL"
            ),
            1
        );
        store
            .connection()
            .execute_batch(
                "DELETE FROM task_ops; DELETE FROM tasks;
                 UPDATE tool_calls SET rel_path = NULL;
                 UPDATE file_offsets SET parser_version = 1;",
            )
            .unwrap();
        let messages = store.totals().unwrap().api_messages;
        let errors = count(&store, "SELECT COALESCE(SUM(count), 0) FROM ingest_errors");

        // El fichero ha crecido mientras tanto.
        append(&path, &format!("{}\n", assistant(4, 5)));
        let stats = run(dir.path(), &mut store);
        assert_eq!(stats.reread_files, 1);
        // Las tareas vuelven; lo demás no se repite.
        assert_eq!(count(&store, "SELECT COUNT(*) FROM tasks"), 2);
        assert_eq!(store.totals().unwrap().api_messages, messages + 1);
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM tool_calls WHERE rel_path IS NOT NULL"
            ),
            0,
            "la relectura no recupera rutas purgadas"
        );
        assert_eq!(
            count(&store, "SELECT COALESCE(SUM(count), 0) FROM ingest_errors"),
            errors
        );
        let offset = store.file_offset(&path.to_string_lossy()).unwrap().unwrap();
        assert_eq!(offset.parser_version, PARSER_VERSION);
        assert_eq!(offset.offset, fs::metadata(&path).unwrap().len());

        let again = run(dir.path(), &mut store);
        assert_eq!((again.reread_files, again.files_read), (0, 0));
    }

    #[test]
    fn an_interrupted_reread_keeps_the_old_offset_and_version() {
        let c = Some(CatchUp { until: 50, old: 1 });
        assert_eq!(recorded(10, c), (50, 1));
        assert_eq!(recorded(50, c), (50, PARSER_VERSION));
        assert_eq!(recorded(60, c), (60, PARSER_VERSION));
        assert_eq!(recorded(5, None), (5, PARSER_VERSION));
    }

    #[test]
    fn dry_run_parses_everything_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path());
        append(
            &path,
            &format!("{}\n{}\n", assistant(1, 10), assistant(2, 20)),
        );
        let stats = ingest_tree(dir.path(), None, PrivacyPolicy::APP_DEFAULT, |_, _| {}).unwrap();
        assert_eq!(stats.lines, 2);
        // TranscriptFile + SessionSeen + 2 ApiMessage + SessionSeen de cierre.
        assert_eq!(stats.events, 5);
    }
}
