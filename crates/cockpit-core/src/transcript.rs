//! Parser de transcripts JSONL de Claude Code (SPEC §6.1). Formato **[INT]**.
//!
//! - **Selectivo:** solo se deserializan los campos de la lista blanca. El texto de
//!   los mensajes, los argumentos y los resultados se saltan sin copiarlos.
//! - **Tolerante:** un tipo de línea desconocido no es un error, se cuenta como
//!   deriva de esquema. Un cambio de forma en un tipo que sí se usa se avisa aparte.
//! - Los errores no llevan el detalle de `serde_json`, que puede citar contenido.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::model::{CockpitEvent, EventKind, Source, TokenUsage, ToolStatus};
use crate::privacy::{PrivacyPolicy, ToolInputView, extract_tool_meta};

/// Tipos de línea conocidos que no aportan eventos (medidos en 2.1.128–2.1.321).
const KNOWN_IGNORED: &[&str] = &[
    "attachment",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "last-prompt",
    "custom-title",
    "ai-title",
    "agent-name",
    "atis-latch",
    "isolation-latch",
    "mode",
    "permission-mode",
    "file-history-delta",
    "file-history-snapshot",
    "queue-operation",
    "relocated",
    "pr-link",
    "tools-used",
    "started",
    "result",
    "failed",
    "launched",
    "summary",
];

const KNOWN_SYSTEM_SUBTYPES: &[&str] = &[
    "stop_hook_summary",
    "compact_boundary",
    "informational",
    "turn_duration",
    "local_command",
];

/// Modelo que usa el cliente para los mensajes que genera él mismo (errores).
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Qué se hizo con una línea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineClass {
    /// Tipo usado por la ingesta (haya producido eventos o no).
    Handled,
    /// Tipo conocido que no interesa.
    Ignored,
    /// Tipo que este parser no conoce: deriva de esquema.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LineError {
    #[error("línea JSON corrupta")]
    Corrupt,
    #[error("deriva de esquema en una línea de tipo `{0}`")]
    Drift(String),
}

// ---------------------------------------------------------------------------
// Deserialización selectiva
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Head<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct RawLine<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    subtype: Option<Cow<'a, str>>,
    #[serde(rename = "sessionId", borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(rename = "agentId", borrow)]
    agent_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(borrow)]
    version: Option<Cow<'a, str>>,
    #[serde(borrow)]
    cwd: Option<Cow<'a, str>>,
    #[serde(borrow)]
    entrypoint: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<RawMessage<'a>>,
    #[serde(rename = "apiErrorStatus")]
    api_error_status: Option<u16>,
    #[serde(default)]
    error: RawError,
    #[serde(rename = "totalCostUSD")]
    total_cost_usd: Option<f64>,
    #[serde(rename = "startTime")]
    start_time: Option<f64>,
    #[serde(rename = "modelUsage")]
    model_usage: Option<HashMap<String, RawModelUsage>>,
}

#[derive(Deserialize)]
struct RawMessage<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    #[serde(borrow)]
    stop_reason: Option<Cow<'a, str>>,
    usage: Option<RawUsage>,
    #[serde(borrow, default)]
    content: RawContent<'a>,
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RawModelUsage {
    #[serde(rename = "inputTokens")]
    input: Option<u64>,
    #[serde(rename = "outputTokens")]
    output: Option<u64>,
    #[serde(rename = "cacheReadInputTokens")]
    cache_read: Option<u64>,
    #[serde(rename = "cacheCreationInputTokens")]
    cache_creation: Option<u64>,
}

#[derive(Deserialize)]
struct RawBlock<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    name: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    input: RawToolInput<'a>,
    #[serde(borrow)]
    tool_use_id: Option<Cow<'a, str>>,
    #[serde(default)]
    is_error: LenientBool,
}

/// Argumentos de una herramienta: solo los campos que mira el `PrivacyFilter`.
///
/// Son indulgentes con el tipo, porque cualquier herramienta MCP puede reutilizar
/// estos nombres con otra forma y eso no debe invalidar la línea.
#[derive(Default)]
struct RawToolInput<'a> {
    file_path: Option<Cow<'a, str>>,
    notebook_path: Option<Cow<'a, str>>,
    url: Option<Cow<'a, str>>,
    subagent_type: Option<Cow<'a, str>>,
    command: Option<Cow<'a, str>>,
    replace_all: Option<bool>,
    run_in_background: Option<bool>,
}

impl<'a> RawToolInput<'a> {
    fn view(&self) -> ToolInputView<'_> {
        ToolInputView {
            file_path: self.file_path.as_deref().or(self.notebook_path.as_deref()),
            replace_all: self.replace_all,
            run_in_background: self.run_in_background,
            url: self.url.as_deref(),
            subagent_type: self.subagent_type.as_deref(),
            command: self.command.as_deref(),
        }
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for RawToolInput<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = RawToolInput<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("los argumentos de una herramienta")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = RawToolInput::default();
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match key.as_ref() {
                        "file_path" => out.file_path = map.next_value::<LenientStr<'a>>()?.0,
                        "notebook_path" => {
                            out.notebook_path = map.next_value::<LenientStr<'a>>()?.0;
                        }
                        "url" => out.url = map.next_value::<LenientStr<'a>>()?.0,
                        "subagent_type" => {
                            out.subagent_type = map.next_value::<LenientStr<'a>>()?.0;
                        }
                        "command" => out.command = map.next_value::<LenientStr<'a>>()?.0,
                        "replace_all" => out.replace_all = map.next_value::<LenientBool>()?.0,
                        "run_in_background" => {
                            out.run_in_background = map.next_value::<LenientBool>()?.0;
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
            // Cualquier otra forma (cadena, lista, null…) equivale a «sin argumentos».
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawToolInput::default())
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(RawToolInput::default())
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(RawToolInput::default())
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(RawToolInput::default())
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(RawToolInput::default())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(RawToolInput::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(RawToolInput::default())
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// Cadena, o `None` si el valor tiene otro tipo (que se salta sin fallar).
struct LenientStr<'a>(Option<Cow<'a, str>>);

impl<'de: 'a, 'a> Deserialize<'de> for LenientStr<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = LenientStr<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(LenientStr(Some(Cow::Borrowed(v))))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(LenientStr(Some(Cow::Owned(v.to_owned()))))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(LenientStr(Some(Cow::Owned(v))))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(LenientStr(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(LenientStr(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(LenientStr(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(LenientStr(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(LenientStr(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LenientStr(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LenientStr(None))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// Booleano (también `"true"`/`"false"`), o `None` si el valor tiene otro tipo.
#[derive(Default)]
struct LenientBool(Option<bool>);

impl<'de> Deserialize<'de> for LenientBool {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LenientBool;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(LenientBool(Some(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(LenientBool(match v {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                }))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(LenientBool(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(LenientBool(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(LenientBool(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(LenientBool(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LenientBool(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LenientBool(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// `message.content`: una lista de bloques, o una cadena que se salta.
#[derive(Default)]
struct RawContent<'a>(Vec<RawBlock<'a>>);

impl<'de: 'a, 'a> Deserialize<'de> for RawContent<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = RawContent<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("una cadena o una lista de bloques")
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(RawContent::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(RawContent::default())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut blocks = Vec::new();
                while let Some(block) = seq.next_element::<RawBlock<'a>>()? {
                    blocks.push(block);
                }
                Ok(RawContent(blocks))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// `error`: una cadena (`assistant`) o un objeto (`system:api_error`).
/// Solo se conserva un código corto, nunca el mensaje.
#[derive(Default)]
struct RawError(Option<String>);

impl<'de> Deserialize<'de> for RawError {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Conn<'a> {
            #[serde(borrow)]
            code: LenientStr<'a>,
        }
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RawError;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(RawError(error_code(v)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut code = None;
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    if key == "connection" {
                        code = map
                            .next_value::<Option<Conn<'de>>>()?
                            .and_then(|c| c.code.0)
                            .and_then(|c| error_code(c.as_ref()));
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(RawError(code))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawError(None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(RawError(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(RawError(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(RawError(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(RawError(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(RawError(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// Acepta solo identificadores cortos (`rate_limit`, `ECONNRESET`), no frases.
fn error_code(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 48
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    ok.then(|| s.to_owned())
}

// ---------------------------------------------------------------------------
// Parser por fichero
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq)]
struct SessionKey {
    session_id: String,
    cwd: Option<String>,
    entrypoint: Option<String>,
    version: Option<String>,
}

/// Parser con estado para **un** fichero de transcript, leído en orden.
pub struct TranscriptParser {
    policy: PrivacyPolicy,
    /// Lo último anunciado con `SessionSeen`.
    announced: Option<SessionKey>,
    announced_ts: i64,
    last: Option<(SessionKey, i64)>,
}

impl TranscriptParser {
    pub fn new(policy: PrivacyPolicy) -> Self {
        Self {
            policy,
            announced: None,
            announced_ts: i64::MIN,
            last: None,
        }
    }

    /// Procesa una línea (sin el salto final) y añade sus eventos a `out`.
    pub fn feed(
        &mut self,
        line: &[u8],
        out: &mut Vec<CockpitEvent>,
    ) -> Result<LineClass, LineError> {
        let line = line.trim_ascii();
        if line.is_empty() {
            return Ok(LineClass::Ignored);
        }
        let raw: RawLine<'_> = match serde_json::from_slice(line) {
            Ok(raw) => raw,
            Err(_) => {
                // ¿Es JSON válido con una forma que no esperábamos, o está roto?
                let head: Head<'_> =
                    serde_json::from_slice(line).map_err(|_| LineError::Corrupt)?;
                let kind = head.kind.unwrap_or(Cow::Borrowed(""));
                return if matches!(
                    kind.as_ref(),
                    "assistant" | "user" | "system" | "cost-state"
                ) {
                    Err(LineError::Drift(kind.into_owned()))
                } else {
                    Ok(classify_unhandled(&kind))
                };
            }
        };
        let kind = raw.kind.as_deref().unwrap_or("");
        match kind {
            "assistant" | "user" => self.conversation_line(&raw, kind == "assistant", out),
            "system" => match raw.subtype.as_deref() {
                Some("api_error") => {
                    let (key, ts) = self.header(&raw).ok_or_else(|| drift("system"))?;
                    self.announce(&key, ts, out);
                    out.push(self.event(
                        &raw,
                        &key,
                        ts,
                        EventKind::ApiError {
                            status: raw.api_error_status,
                            error_type: raw.error.0.clone(),
                        },
                    ));
                    Ok(LineClass::Handled)
                }
                Some(sub) if KNOWN_SYSTEM_SUBTYPES.contains(&sub) => Ok(LineClass::Ignored),
                Some(sub) => Ok(LineClass::Unknown(format!("system:{}", safe_kind(sub)))),
                None => Ok(LineClass::Unknown("system".to_owned())),
            },
            "cost-state" => self.cost_state(&raw, out),
            other => Ok(classify_unhandled(other)),
        }
    }

    /// Cierra el fichero: anuncia la última actividad si no se había anunciado.
    pub fn finish(&mut self, out: &mut Vec<CockpitEvent>) {
        if let Some((key, ts)) = self.last.clone()
            && ts > self.announced_ts
        {
            self.push_seen(&key, ts, out);
        }
    }

    fn header(&self, raw: &RawLine<'_>) -> Option<(SessionKey, i64)> {
        let session_id = raw.session_id.as_deref()?.to_owned();
        let ts = crate::time::parse_rfc3339_ms(raw.timestamp.as_deref()?)?;
        let cwd = match (&raw.cwd, self.policy.record_paths) {
            (Some(cwd), true) => Some(cwd.as_ref().to_owned()),
            _ => None,
        };
        let key = SessionKey {
            session_id,
            cwd,
            entrypoint: raw.entrypoint.as_deref().map(str::to_owned),
            version: raw.version.as_deref().map(str::to_owned),
        };
        Some((key, ts))
    }

    /// Emite `SessionSeen` solo cuando cambia la identidad de la sesión en el fichero.
    fn announce(&mut self, key: &SessionKey, ts: i64, out: &mut Vec<CockpitEvent>) {
        if self.announced.as_ref() != Some(key) {
            self.push_seen(key, ts, out);
        }
        self.last = Some((key.clone(), ts));
    }

    fn push_seen(&mut self, key: &SessionKey, ts: i64, out: &mut Vec<CockpitEvent>) {
        out.push(CockpitEvent {
            source: Source::Transcript,
            session_id: key.session_id.clone(),
            agent_id: None,
            ts_ms: ts,
            engine_version: key.version.clone(),
            kind: EventKind::SessionSeen {
                cwd: key.cwd.clone(),
                entrypoint: key.entrypoint.clone(),
            },
        });
        self.announced = Some(key.clone());
        self.announced_ts = ts;
    }

    fn event(&self, raw: &RawLine<'_>, key: &SessionKey, ts: i64, kind: EventKind) -> CockpitEvent {
        CockpitEvent {
            source: Source::Transcript,
            session_id: key.session_id.clone(),
            agent_id: raw.agent_id.as_deref().map(str::to_owned),
            ts_ms: ts,
            engine_version: key.version.clone(),
            kind,
        }
    }

    fn conversation_line(
        &mut self,
        raw: &RawLine<'_>,
        is_assistant: bool,
        out: &mut Vec<CockpitEvent>,
    ) -> Result<LineClass, LineError> {
        let kind_name = if is_assistant { "assistant" } else { "user" };
        let (key, ts) = self.header(raw).ok_or_else(|| drift(kind_name))?;
        // Las líneas de subagentes no cambian el `cwd` anunciado de la sesión.
        if raw.agent_id.is_none() || self.announced.is_none() {
            self.announce(&key, ts, out);
        } else {
            self.last = Some((self.announced.clone().unwrap_or(key.clone()), ts));
        }
        let Some(message) = &raw.message else {
            return Ok(LineClass::Handled);
        };

        if is_assistant {
            let synthetic = message.model.as_deref() == Some(SYNTHETIC_MODEL);
            if synthetic || raw.api_error_status.is_some() {
                out.push(self.event(
                    raw,
                    &key,
                    ts,
                    EventKind::ApiError {
                        status: raw.api_error_status,
                        error_type: raw.error.0.clone(),
                    },
                ));
            } else if let (Some(id), Some(model), Some(usage)) =
                (&message.id, &message.model, &message.usage)
            {
                out.push(self.event(
                    raw,
                    &key,
                    ts,
                    EventKind::ApiMessage {
                        message_id: id.as_ref().to_owned(),
                        model: model.as_ref().to_owned(),
                        usage: TokenUsage {
                            input: usage.input_tokens.unwrap_or(0),
                            output: usage.output_tokens.unwrap_or(0),
                            cache_read: usage.cache_read_input_tokens.unwrap_or(0),
                            cache_creation: usage.cache_creation_input_tokens.unwrap_or(0),
                        },
                        stop_reason: message.stop_reason.as_deref().and_then(error_code),
                    },
                ));
            }
        }

        // La raíz provisional para relativizar rutas es el `cwd` de la propia línea.
        let root = raw.cwd.as_deref();
        for block in &message.content.0 {
            match (block.kind.as_deref(), is_assistant) {
                (Some("tool_use"), true) => {
                    if let (Some(id), Some(name)) = (&block.id, &block.name) {
                        let meta = extract_tool_meta(name, &block.input.view(), &self.policy, root);
                        out.push(self.event(
                            raw,
                            &key,
                            ts,
                            EventKind::ToolStarted {
                                tool_use_id: id.as_ref().to_owned(),
                                tool: safe_kind(name),
                                meta,
                            },
                        ));
                    }
                }
                (Some("tool_result"), false) => {
                    if let Some(id) = &block.tool_use_id {
                        let status = if block.is_error.0 == Some(true) {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Ok
                        };
                        out.push(self.event(
                            raw,
                            &key,
                            ts,
                            EventKind::ToolFinished {
                                tool_use_id: id.as_ref().to_owned(),
                                status,
                            },
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(LineClass::Handled)
    }

    fn cost_state(
        &mut self,
        raw: &RawLine<'_>,
        out: &mut Vec<CockpitEvent>,
    ) -> Result<LineClass, LineError> {
        let (Some(session_id), Some(total), Some(models)) =
            (&raw.session_id, raw.total_cost_usd, &raw.model_usage)
        else {
            return Err(drift("cost-state"));
        };
        let mut usage = TokenUsage::default();
        for m in models.values() {
            usage.add(&TokenUsage {
                input: m.input.unwrap_or(0),
                output: m.output.unwrap_or(0),
                cache_read: m.cache_read.unwrap_or(0),
                cache_creation: m.cache_creation.unwrap_or(0),
            });
        }
        // La línea no lleva `timestamp`: se usa la última actividad vista en el fichero.
        let ts = match &self.last {
            Some((_, ts)) => *ts,
            None => raw.start_time.map(|t| t as i64).unwrap_or(0),
        };
        out.push(CockpitEvent {
            source: Source::Transcript,
            session_id: session_id.as_ref().to_owned(),
            agent_id: None,
            ts_ms: ts,
            engine_version: None,
            kind: EventKind::CostState {
                total_cost_usd: total,
                usage,
            },
        });
        Ok(LineClass::Handled)
    }
}

fn drift(kind: &str) -> LineError {
    LineError::Drift(kind.to_owned())
}

fn classify_unhandled(kind: &str) -> LineClass {
    if KNOWN_IGNORED.contains(&kind) {
        LineClass::Ignored
    } else if kind.is_empty() {
        LineClass::Unknown("<sin type>".to_owned())
    } else {
        LineClass::Unknown(safe_kind(kind))
    }
}

/// Un nombre de tipo o de herramienta es un identificador: se recorta y se limpia
/// para que nunca pueda arrastrar contenido.
fn safe_kind(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
        .take(96)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(line: &str) -> (Result<LineClass, LineError>, Vec<CockpitEvent>) {
        let mut parser = TranscriptParser::new(PrivacyPolicy::APP_DEFAULT);
        let mut out = Vec::new();
        let class = parser.feed(line.as_bytes(), &mut out);
        (class, out)
    }

    #[test]
    fn corrupt_line_is_reported_without_detail() {
        let (class, out) = feed(r#"{"type":"assistant","message":{"id":"m1", SECRETO"#);
        assert_eq!(class, Err(LineError::Corrupt));
        assert!(out.is_empty());
        assert!(!LineError::Corrupt.to_string().contains("SECRETO"));
    }

    #[test]
    fn unknown_type_is_drift_not_an_error() {
        let (class, out) = feed(r#"{"type":"tipo-del-futuro","payload":{"a":[1,2,3]}}"#);
        assert_eq!(class, Ok(LineClass::Unknown("tipo-del-futuro".to_owned())));
        assert!(out.is_empty());
    }

    #[test]
    fn known_irrelevant_types_are_ignored_even_with_odd_shapes() {
        // `message` es una cadena aquí: no debe contar como corrupta.
        let (class, _) = feed(r#"{"type":"queue-operation","message":"x","timestamp":5}"#);
        assert_eq!(class, Ok(LineClass::Ignored));
    }

    #[test]
    fn handled_type_with_broken_shape_is_drift() {
        let (class, _) = feed(
            r#"{"type":"assistant","sessionId":"s","timestamp":"2026-11-10T10:00:00.000Z","message":"texto"}"#,
        );
        assert_eq!(class, Err(LineError::Drift("assistant".to_owned())));
        let (class, _) = feed(r#"{"type":"assistant","message":{}}"#);
        assert_eq!(class, Err(LineError::Drift("assistant".to_owned())));
    }

    #[test]
    fn mcp_input_reusing_whitelisted_names_with_other_types_does_not_break_the_line() {
        let (class, out) = feed(
            r#"{"type":"assistant","sessionId":"s","timestamp":"2026-11-10T10:00:00.000Z","message":{"id":"m","model":"claude-x","usage":{"input_tokens":1,"output_tokens":2},"content":[{"type":"tool_use","id":"t1","name":"mcp__srv__do","input":{"url":{"a":1},"command":["x"],"file_path":7,"run_in_background":"true","replace_all":null}}]}}"#,
        );
        assert_eq!(class, Ok(LineClass::Handled));
        assert_eq!(out.len(), 3); // SessionSeen, ApiMessage, ToolStarted
    }

    #[test]
    fn empty_lines_are_ignored() {
        let (class, out) = feed("   \r");
        assert_eq!(class, Ok(LineClass::Ignored));
        assert!(out.is_empty());
    }
}
