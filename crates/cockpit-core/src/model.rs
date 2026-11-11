//! Modelo canónico de eventos (SPEC §6.3).
//!
//! F1 define los eventos que produce el transcript. Los que vienen del mod, del
//! sistema de ficheros y de Git se añaden en sus fases (F3, F4a).

use serde::Serialize;

use crate::privacy::ToolMeta;

/// Origen de un evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Source {
    Mod,
    Transcript,
    Registry,
    Fs,
    Git,
}

impl Source {
    /// Bit de `tool_calls.source_mask` (SPEC §6.4).
    pub const fn mask(self) -> i64 {
        match self {
            Source::Mod => 1,
            Source::Transcript => 2,
            Source::Registry => 4,
            Source::Fs => 8,
            Source::Git => 16,
        }
    }
}

/// Consumo de tokens de una respuesta de la API o de un total de sesión.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_creation += other.cache_creation;
    }
}

/// Estado final de una herramienta visto desde el transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolStatus {
    Ok,
    Error,
}

impl ToolStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            ToolStatus::Ok => "ok",
            ToolStatus::Error => "error",
        }
    }
}

/// Evento normalizado y ya filtrado por la política de privacidad (SPEC §10).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CockpitEvent {
    pub source: Source,
    pub session_id: String,
    pub agent_id: Option<String>,
    /// Milisegundos desde la época Unix (reloj de pared).
    pub ts_ms: i64,
    pub engine_version: Option<String>,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum EventKind {
    /// La sesión existe y está activa en `ts_ms`. `cwd` solo va con `record_paths`.
    SessionSeen {
        cwd: Option<String>,
        entrypoint: Option<String>,
    },
    /// Fichero principal del transcript de la sesión. Su fecha de creación desempata
    /// qué sesión es la canónica cuando el historial está copiado (R-4).
    TranscriptFile { created_at_ms: i64 },
    /// Respuesta de la API. Identidad: `message_id` (SPEC §3, M7).
    ApiMessage {
        message_id: String,
        model: String,
        usage: TokenUsage,
        stop_reason: Option<String>,
    },
    /// Identidad: `(session_id, tool_use_id)` (R-4).
    ToolStarted {
        tool_use_id: String,
        tool: String,
        meta: ToolMeta,
    },
    ToolFinished {
        tool_use_id: String,
        status: ToolStatus,
    },
    /// Solo el estado HTTP y el tipo de error, nunca el cuerpo (SPEC §10).
    ApiError {
        status: Option<u16>,
        error_type: Option<String>,
    },
    /// Línea `cost-state` del transcript: totales que calcula el propio cliente.
    CostState {
        total_cost_usd: f64,
        usage: TokenUsage,
    },
}
