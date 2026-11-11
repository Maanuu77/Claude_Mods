//! `SessionRegistryWatch`: sesiones vivas según `~/.claude/sessions/<pid>.json`
//! (SPEC §3). Formato **[INT]**: es una señal auxiliar de liveness, y si falla la
//! app sigue funcionando solo con los transcripts. **Solo lectura.**

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

/// Lo que se usa de un fichero del registro. El nombre de la sesión y las rutas de
/// sockets no se leen.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RegistryEntry {
    pub pid: u32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: Option<String>,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub entrypoint: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "startedAt")]
    pub started_at: Option<i64>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<i64>,
}

/// Estado de liveness que ve la interfaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Liveness {
    pub pid: u32,
    /// `busy`, `idle`… tal como lo escribe Claude Code.
    pub status: Option<String>,
    pub kind: Option<String>,
    pub updated_at: Option<i64>,
}

impl From<&RegistryEntry> for Liveness {
    fn from(e: &RegistryEntry) -> Self {
        Self {
            pid: e.pid,
            status: e.status.clone(),
            kind: e.kind.clone(),
            updated_at: e.updated_at,
        }
    }
}

pub fn parse_entry(json: &[u8]) -> Option<RegistryEntry> {
    serde_json::from_slice(json).ok()
}

/// Comprueba qué procesos siguen vivos. Claude no borra su fichero si muere de golpe.
pub struct PidProbe {
    system: System,
}

impl Default for PidProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl PidProbe {
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }

    pub fn alive(&mut self, pids: &[u32]) -> Vec<u32> {
        let wanted: Vec<Pid> = pids.iter().map(|p| Pid::from_u32(*p)).collect();
        self.system
            .refresh_processes(ProcessesToUpdate::Some(&wanted), true);
        pids.iter()
            .copied()
            .filter(|p| self.system.process(Pid::from_u32(*p)).is_some())
            .collect()
    }
}

/// Lee el registro entero y devuelve las sesiones cuyo proceso sigue vivo, por id.
pub fn read_registry(dir: &Path, probe: &mut PidProbe) -> BTreeMap<String, RegistryEntry> {
    let mut entries = Vec::new();
    if let Ok(files) = fs::read_dir(dir) {
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(bytes) = fs::read(&path)
                && let Some(entry) = parse_entry(&bytes)
            {
                entries.push(entry);
            }
        }
    }
    let pids: Vec<u32> = entries.iter().map(|e| e.pid).collect();
    let alive = probe.alive(&pids);
    let mut by_session = BTreeMap::new();
    for entry in entries {
        if !alive.contains(&entry.pid) {
            continue;
        }
        // Si dos procesos declaran la misma sesión, gana el actualizado más tarde.
        match by_session.get(&entry.session_id) {
            Some(RegistryEntry { updated_at, .. }) if *updated_at >= entry.updated_at => {}
            _ => {
                by_session.insert(entry.session_id.clone(), entry);
            }
        }
    }
    by_session
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_fields_it_needs_and_ignores_the_rest() {
        let json = br#"{"pid":4242,"sessionId":"s1","cwd":"C:\\w","startedAt":1794304800000,
            "procStart":"x","version":"2.1.321","peerFeatures":[],"kind":"interactive",
            "entrypoint":"cli","name":"NOMBRE PRIVADO","messagingSocketPath":"\\\\.\\pipe\\x",
            "status":"busy","updatedAt":1794304801000}"#;
        let entry = parse_entry(json).unwrap();
        assert_eq!(entry.pid, 4242);
        assert_eq!(entry.session_id, "s1");
        assert_eq!(entry.status.as_deref(), Some("busy"));
        assert!(!format!("{entry:?}").contains("NOMBRE PRIVADO"));
        assert_eq!(parse_entry(b"{ roto"), None);
        assert_eq!(parse_entry(br#"{"pid":"no es un numero"}"#), None);
    }

    #[test]
    fn dead_processes_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        fs::write(
            dir.path().join(format!("{me}.json")),
            format!(r#"{{"pid":{me},"sessionId":"viva","status":"idle"}}"#),
        )
        .unwrap();
        // PID múltiplo de 4 muy alto: no existe.
        fs::write(
            dir.path().join("4294967292.json"),
            r#"{"pid":4294967292,"sessionId":"muerta"}"#,
        )
        .unwrap();
        fs::write(dir.path().join("basura.json"), "no es json").unwrap();

        let live = read_registry(dir.path(), &mut PidProbe::new());
        assert_eq!(live.keys().collect::<Vec<_>>(), ["viva"]);
        assert!(read_registry(&dir.path().join("no-existe"), &mut PidProbe::new()).is_empty());
    }
}
