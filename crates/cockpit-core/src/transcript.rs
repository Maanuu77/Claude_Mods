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
use crate::tasks::{
    self, Adapted, AgentState, ListItemView, TOOL_DESKTOP_PLAN, TOOL_TASK_UPDATE, TOOL_TODO_WRITE,
    TaskInputView,
};

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
    #[serde(rename = "toolUseResult", borrow, default)]
    tool_use_result: RawToolUseResult<'a>,
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

/// Argumentos de una herramienta: solo los campos que miran el `PrivacyFilter`
/// (§10) y los adaptadores de tareas (§8).
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
    // Task Monitor. `description`, `activeForm` y `metadata` no se leen nunca.
    subject: Option<Cow<'a, str>>,
    task_id: Option<Cow<'a, str>>,
    status: Option<Cow<'a, str>>,
    owner: Option<Cow<'a, str>>,
    add_blocked_by: Option<Vec<Cow<'a, str>>>,
    todos: Option<Vec<RawListItem<'a>>>,
    items: Option<Vec<RawListItem<'a>>>,
    advance: Option<bool>,
    done: Option<Vec<i64>>,
    start: Option<i64>,
    /// Hay claves fuera de las de `update_plan` (solo cuenta para A3, G6).
    beyond_plan_keys: bool,
}

/// Claves conocidas de `mcp__ccd_turn__update_plan` (fixtures de G6).
const PLAN_KEYS: &[&str] = &["items", "advance", "done", "start", "scope"];

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

    fn task_view(&self) -> TaskInputView<'_> {
        fn list<'s>(items: &'s Option<Vec<RawListItem<'_>>>) -> Option<Vec<ListItemView<'s>>> {
            items.as_ref().map(|items| {
                items
                    .iter()
                    .map(|i| ListItemView {
                        content: i.content.as_deref(),
                        status: i.status.as_deref(),
                    })
                    .collect()
            })
        }
        TaskInputView {
            subject: self.subject.as_deref(),
            task_id: self.task_id.as_deref(),
            status: self.status.as_deref(),
            owner: self.owner.as_deref(),
            add_blocked_by: self
                .add_blocked_by
                .as_ref()
                .map(|ids| ids.iter().map(|s| s.as_ref()).collect()),
            todos: list(&self.todos),
            items: list(&self.items),
            advance: self.advance,
            done: self.done.clone(),
            start: self.start,
            has_unknown_fields: self.beyond_plan_keys,
        }
    }
}

/// Elemento de `todos` (TodoWrite) o `items` (update_plan): `content` y `status`.
#[derive(Default)]
struct RawListItem<'a> {
    content: Option<Cow<'a, str>>,
    status: Option<Cow<'a, str>>,
}

impl<'de: 'a, 'a> Deserialize<'de> for RawListItem<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = RawListItem<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("un elemento de lista")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = RawListItem::default();
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match key.as_ref() {
                        "content" => out.content = map.next_value::<LenientStr<'a>>()?.0,
                        "status" => out.status = map.next_value::<LenientStr<'a>>()?.0,
                        // `activeForm` y cualquier otro campo: no se leen.
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawListItem::default())
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(RawListItem::default())
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(RawListItem::default())
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(RawListItem::default())
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(RawListItem::default())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(RawListItem::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(RawListItem::default())
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// Lista de `T`, o `None` si el valor no es una lista.
struct LenientList<T>(Option<Vec<T>>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for LenientList<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = LenientList<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(item) = seq.next_element::<T>()? {
                    // Más allá del tope no se guarda nada, pero se consume la lista.
                    if out.len() < crate::tasks::MAX_ITEMS + 1 {
                        out.push(item);
                    }
                }
                Ok(LenientList(Some(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LenientList(None))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(LenientList(None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(LenientList(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(LenientList(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(LenientList(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(LenientList(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(LenientList(None))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// Entero, o `None` con cualquier otro tipo. Un número no entero o negativo
/// cuenta como «no válido» (`i64::MIN`), para que el adaptador lo trate como deriva.
struct LenientInt(Option<i64>);

impl<'de> Deserialize<'de> for LenientInt {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LenientInt;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(LenientInt(Some(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(LenientInt(Some(i64::try_from(v).unwrap_or(i64::MIN))))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                let whole = v.fract() == 0.0 && v.abs() < 1e15;
                Ok(LenientInt(Some(if whole { v as i64 } else { i64::MIN })))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(LenientInt(Some(i64::MIN)))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(LenientInt(Some(i64::MIN)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(LenientInt(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LenientInt(Some(i64::MIN)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LenientInt(Some(i64::MIN)))
            }
        }
        d.deserialize_any(V)
    }
}

/// `toolUseResult` de una línea `user`: solo lo que necesitan A1 y A4.
///
/// Su forma depende de la herramienta (texto, lista u objeto). Del objeto solo se
/// leen `task.{id, subject}` (`TaskCreate`) y `agentId`, `agentType`, `status` y
/// `totalDurationMs` (`Agent`). El `prompt`, el contenido y el resto se saltan.
#[derive(Default)]
struct RawToolUseResult<'a> {
    task_id: Option<Cow<'a, str>>,
    task_subject: Option<Cow<'a, str>>,
    agent_id: Option<Cow<'a, str>>,
    agent_type: Option<Cow<'a, str>>,
    status: Option<Cow<'a, str>>,
    duration_ms: Option<i64>,
}

/// `task` del resultado de `TaskCreate`: `id` y `subject`; cualquier otra forma, vacío.
struct RawTaskRef<'a>(Option<Cow<'a, str>>, Option<Cow<'a, str>>);

impl<'de: 'a, 'a> Deserialize<'de> for RawTaskRef<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = RawTaskRef<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = RawTaskRef(None, None);
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match key.as_ref() {
                        "id" => out.0 = map.next_value::<LenientStr<'a>>()?.0,
                        "subject" => out.1 = map.next_value::<LenientStr<'a>>()?.0,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawTaskRef(None, None))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(RawTaskRef(None, None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(RawTaskRef(None, None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(RawTaskRef(None, None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(RawTaskRef(None, None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(RawTaskRef(None, None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(RawTaskRef(None, None))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for RawToolUseResult<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = RawToolUseResult<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("cualquier valor")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = RawToolUseResult::default();
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match key.as_ref() {
                        "task" => {
                            let task = map.next_value::<RawTaskRef<'a>>()?;
                            out.task_id = task.0;
                            out.task_subject = task.1;
                        }
                        "agentId" => out.agent_id = map.next_value::<LenientStr<'a>>()?.0,
                        "agentType" => out.agent_type = map.next_value::<LenientStr<'a>>()?.0,
                        "status" => out.status = map.next_value::<LenientStr<'a>>()?.0,
                        "totalDurationMs" => {
                            out.duration_ms = map.next_value::<LenientInt>()?.0;
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawToolUseResult::default())
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(RawToolUseResult::default())
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(RawToolUseResult::default())
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(RawToolUseResult::default())
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(RawToolUseResult::default())
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(RawToolUseResult::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(RawToolUseResult::default())
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
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
                    if !PLAN_KEYS.contains(&key.as_ref()) {
                        out.beyond_plan_keys = true;
                    }
                    match key.as_ref() {
                        "subject" => out.subject = map.next_value::<LenientStr<'a>>()?.0,
                        "taskId" => out.task_id = map.next_value::<LenientStr<'a>>()?.0,
                        "status" => out.status = map.next_value::<LenientStr<'a>>()?.0,
                        "owner" => out.owner = map.next_value::<LenientStr<'a>>()?.0,
                        "addBlockedBy" => {
                            out.add_blocked_by = map
                                .next_value::<LenientList<LenientStr<'a>>>()?
                                .0
                                .map(|ids| ids.into_iter().flat_map(|s| s.0).collect());
                        }
                        "todos" => out.todos = map.next_value::<LenientList<RawListItem<'a>>>()?.0,
                        "items" => out.items = map.next_value::<LenientList<RawListItem<'a>>>()?.0,
                        "advance" => out.advance = map.next_value::<LenientBool>()?.0,
                        "done" => {
                            out.done = map.next_value::<LenientList<LenientInt>>()?.0.map(|ns| {
                                ns.into_iter().map(|n| n.0.unwrap_or(i64::MIN)).collect()
                            });
                        }
                        "start" => out.start = map.next_value::<LenientInt>()?.0,
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
    /// Avisos de deriva de los adaptadores de tareas (`adapter:<fuente>`), sin contenido.
    notes: Vec<String>,
}

impl TranscriptParser {
    pub fn new(policy: PrivacyPolicy) -> Self {
        Self {
            policy,
            announced: None,
            announced_ts: i64::MIN,
            last: None,
            notes: Vec::new(),
        }
    }

    /// Avisos de deriva acumulados desde la última llamada. Una herramienta de
    /// tareas con una forma inesperada no rompe la línea: se ignora y se avisa (G6).
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
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
        // `toolUseResult` es de la línea: solo se asocia si hay un único resultado.
        let results = message
            .content
            .0
            .iter()
            .filter(|b| b.kind.as_deref() == Some("tool_result"))
            .count();
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
                        self.task_op(raw, &key, ts, id, name, &block.input, out);
                    }
                }
                (Some("tool_result"), false) => {
                    if let Some(id) = &block.tool_use_id {
                        let is_error = block.is_error.0 == Some(true);
                        let status = if is_error {
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
                        if results == 1
                            && let Some(kind) = self.result_meta(id, is_error, &raw.tool_use_result)
                        {
                            out.push(self.event(raw, &key, ts, kind));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(LineClass::Handled)
    }

    /// A1–A3: la operación que declara una llamada a una herramienta de tareas.
    #[allow(clippy::too_many_arguments)]
    fn task_op(
        &mut self,
        raw: &RawLine<'_>,
        key: &SessionKey,
        ts: i64,
        tool_use_id: &str,
        tool: &str,
        input: &RawToolInput<'_>,
        out: &mut Vec<CockpitEvent>,
    ) {
        if !matches!(tool, TOOL_TASK_UPDATE | TOOL_TODO_WRITE | TOOL_DESKTOP_PLAN) {
            return;
        }
        let view = input.task_view();
        match tasks::op_from_call(tool, &view, &self.policy, raw.agent_id.is_some()) {
            Adapted::Op(source, op) => {
                if let Some(op_key) = op.key(Some(tool_use_id)) {
                    out.push(self.event(raw, key, ts, EventKind::TaskOp { source, op_key, op }));
                }
            }
            Adapted::Drift(source) => self.notes.push(format!("adapter:{}", source.as_str())),
            Adapted::NotATaskCall => {}
        }
    }

    /// A1 (`TaskCreate`) y A4 (`Agent`): lo que trae el resultado, si trae algo.
    fn result_meta(
        &self,
        tool_use_id: &str,
        is_error: bool,
        result: &RawToolUseResult<'_>,
    ) -> Option<EventKind> {
        let task_id = result.task_id.as_deref().and_then(tasks::clean_id);
        let agent = result.agent_id.as_deref().and_then(tasks::clean_id);
        if task_id.is_none() && agent.is_none() {
            return None;
        }
        Some(EventKind::ToolResultMeta {
            tool_use_id: tool_use_id.to_owned(),
            is_error,
            task_subject: task_id.as_ref().and_then(|_| {
                result
                    .task_subject
                    .as_deref()
                    .and_then(|s| tasks::subject(s, &self.policy))
            }),
            task_id,
            agent_type: agent.as_ref().and_then(|_| {
                result
                    .agent_type
                    .as_deref()
                    .and_then(tasks::clean_agent_type)
            }),
            agent_status: agent
                .as_ref()
                .and_then(|_| AgentState::from_agent_result(result.status.as_deref(), is_error)),
            duration_ms: agent
                .as_ref()
                .and(result.duration_ms)
                .and_then(|d| u64::try_from(d).ok()),
            agent,
        })
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

    fn tool_use(agent: Option<&str>, name: &str, input: &str) -> String {
        let agent = agent.map_or(String::new(), |a| format!(r#""agentId":"{a}","#));
        format!(
            r#"{{"type":"assistant",{agent}"sessionId":"s","timestamp":"2026-11-10T10:00:00.000Z","message":{{"id":"m","model":"claude-x","usage":{{"input_tokens":1,"output_tokens":2}},"content":[{{"type":"tool_use","id":"t1","name":"{name}","input":{input}}}]}}}}"#
        )
    }

    fn task_ops(events: &[CockpitEvent]) -> Vec<(tasks::TaskSource, String, tasks::TaskOp)> {
        events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::TaskOp { source, op_key, op } => {
                    Some((*source, op_key.clone(), op.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn todo_write_becomes_a_full_list_without_active_form_or_extra_fields() {
        let (class, out) = feed(&tool_use(
            None,
            "TodoWrite",
            r#"{"todos":[{"content":"Leer","status":"completed","activeForm":"SECRETO-TRAMPA"},{"content":"Escribir","status":"in_progress","activeForm":"SECRETO-TRAMPA","extra":{"x":1}}]}"#,
        ));
        assert_eq!(class, Ok(LineClass::Handled));
        let ops = task_ops(&out);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].0, tasks::TaskSource::TodoWrite);
        assert_eq!(ops[0].1, "tool:t1");
        assert!(!format!("{out:?}").contains("SECRETO"));
        let tasks::TaskOp::Replace { items } = &ops[0].2 else {
            panic!("{:?}", ops[0].2);
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].subject.as_deref(), Some("Escribir"));
    }

    #[test]
    fn task_update_reads_only_whitelisted_fields() {
        let (_, out) = feed(&tool_use(
            Some("agente-1"),
            "TaskUpdate",
            r#"{"taskId":"2","status":"completed","description":"SECRETO-TRAMPA","metadata":{"k":"SECRETO-TRAMPA"},"addBlockedBy":["1",7,"no válido"]}"#,
        ));
        assert!(!format!("{out:?}").contains("SECRETO"));
        let ops = task_ops(&out);
        assert_eq!(
            ops[0].2,
            tasks::TaskOp::Update {
                task_id: "2".to_owned(),
                status: Some(tasks::TaskStatus::Completed),
                subject: None,
                owner: None,
                add_blocked_by: vec!["1".to_owned()],
            }
        );
        // Desde un subagente: la lista de tareas es de la sesión.
        let event = out
            .iter()
            .find(|e| matches!(e.kind, EventKind::TaskOp { .. }));
        assert_eq!(event.unwrap().agent_id.as_deref(), Some("agente-1"));
    }

    #[test]
    fn a_plan_with_an_unexpected_shape_is_noted_and_skipped() {
        let mut parser = TranscriptParser::new(PrivacyPolicy::APP_DEFAULT);
        let mut out = Vec::new();
        let line = tool_use(
            None,
            "mcp__ccd_turn__update_plan",
            r#"{"items":[{"content":"x","status":"blocked"}]}"#,
        );
        assert_eq!(
            parser.feed(line.as_bytes(), &mut out),
            Ok(LineClass::Handled)
        );
        assert!(task_ops(&out).is_empty());
        // La herramienta se sigue registrando: solo se pierde la operación.
        assert!(
            out.iter()
                .any(|e| matches!(e.kind, EventKind::ToolStarted { .. }))
        );
        assert_eq!(parser.take_notes(), ["adapter:desktop_plan"]);
        assert!(parser.take_notes().is_empty());

        let new_field = tool_use(
            None,
            "mcp__ccd_turn__update_plan",
            r#"{"advance":true,"mode":"x"}"#,
        );
        parser.feed(new_field.as_bytes(), &mut out).unwrap();
        assert_eq!(parser.take_notes(), ["adapter:desktop_plan"]);
    }

    #[test]
    fn subagent_todo_lists_are_not_the_session_list() {
        let (_, out) = feed(&tool_use(
            Some("agente-1"),
            "TodoWrite",
            r#"{"todos":[{"content":"x","status":"pending"}]}"#,
        ));
        assert!(task_ops(&out).is_empty());
    }

    fn tool_result(result: &str, is_error: bool) -> String {
        format!(
            r#"{{"type":"user","sessionId":"s","timestamp":"2026-11-10T10:00:01.000Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","is_error":{is_error},"content":"SECRETO-TRAMPA"}}]}},"toolUseResult":{result}}}"#
        )
    }

    #[test]
    fn task_create_result_carries_the_id_and_title() {
        let (class, out) = feed(&tool_result(
            r#"{"task":{"id":"3","subject":"Revisar la SPEC","description":"SECRETO-TRAMPA"}}"#,
            false,
        ));
        assert_eq!(class, Ok(LineClass::Handled));
        let meta = out.iter().find_map(|e| match &e.kind {
            EventKind::ToolResultMeta {
                task_id,
                task_subject,
                agent,
                ..
            } => Some((task_id.clone(), task_subject.clone(), agent.clone())),
            _ => None,
        });
        assert_eq!(
            meta,
            Some((
                Some("3".to_owned()),
                Some("Revisar la SPEC".to_owned()),
                None
            ))
        );
        assert!(!format!("{out:?}").contains("SECRETO"));
    }

    #[test]
    fn agent_result_reports_the_subagent_without_its_prompt() {
        let (_, out) = feed(&tool_result(
            r#"{"agentId":"a1b2","agentType":"Explore","status":"completed","totalDurationMs":4200,"prompt":"SECRETO-TRAMPA","content":[{"type":"text","text":"SECRETO-TRAMPA"}]}"#,
            false,
        ));
        let meta = out.iter().find_map(|e| match &e.kind {
            EventKind::ToolResultMeta {
                agent,
                agent_type,
                agent_status,
                duration_ms,
                ..
            } => Some((
                agent.clone(),
                agent_type.clone(),
                *agent_status,
                *duration_ms,
            )),
            _ => None,
        });
        assert_eq!(
            meta,
            Some((
                Some("a1b2".to_owned()),
                Some("Explore".to_owned()),
                Some(tasks::AgentState::Completed),
                Some(4200)
            ))
        );
        assert!(!format!("{out:?}").contains("SECRETO"));
    }

    #[test]
    fn results_of_other_tools_carry_no_task_meta() {
        for result in [
            r#""texto""#,
            r#"[{"type":"text","text":"x"}]"#,
            r#"{"stdout":"x","stderr":""}"#,
            r#"{"task":"no es un objeto"}"#,
            "null",
        ] {
            let (class, out) = feed(&tool_result(result, false));
            assert_eq!(class, Ok(LineClass::Handled), "{result}");
            assert!(
                !out.iter()
                    .any(|e| matches!(e.kind, EventKind::ToolResultMeta { .. })),
                "{result}"
            );
        }
    }

    #[test]
    fn empty_lines_are_ignored() {
        let (class, out) = feed("   \r");
        assert_eq!(class, Ok(LineClass::Ignored));
        assert!(out.is_empty());
    }
}
