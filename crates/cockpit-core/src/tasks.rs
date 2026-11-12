//! Task Monitor (SPEC §8): lo que cada sesión **declara** sobre su lista de tareas.
//!
//! El progreso nunca se infiere del número de herramientas. Sale de tres
//! adaptadores, cada uno con su lista:
//!
//! - **A1** `TaskCreate`/`TaskUpdate` y los hooks `TaskCreated`/`TaskCompleted`:
//!   tareas con id, estado, responsable y bloqueos. Compartidas por todos los
//!   bucles de la sesión (subagentes y compañeros de equipo incluidos).
//! - **A2** `TodoWrite`: cada llamada es la lista completa.
//! - **A3** `mcp__ccd_turn__update_plan` del Desktop: lista completa (`items`) u
//!   operaciones relativas (`advance`, `done`/`start`). Esquema no publicado [INT];
//!   la puerta G6 lo valida con fixtures de 2.1.299–2.1.321.
//!
//! Cada llamada produce una [`TaskOp`]. El estado de una lista es la reproducción
//! ordenada de sus operaciones ([`replay`]): así el mod y el transcript pueden
//! entregar la misma operación en cualquier orden sin duplicar un `advance`.
//!
//! De los argumentos solo se leen los campos de §10: id, estado, responsable,
//! bloqueos y el título truncado a 120 caracteres si `capture_task_titles` lo
//! permite (D1). Nunca `description`, `activeForm` ni `metadata`.

use serde::{Deserialize, Serialize};

use crate::privacy::PrivacyPolicy;

/// Longitud máxima de un título (D1).
pub const SUBJECT_MAX_CHARS: usize = 120;
/// Longitud máxima de un id de tarea o de un responsable.
pub const ID_MAX_CHARS: usize = 64;
/// Tope de elementos de una lista. Lo que pase de aquí no se guarda.
pub const MAX_ITEMS: usize = 200;
/// Tope de bloqueos por tarea.
pub const MAX_BLOCKERS: usize = 32;

pub const TOOL_TASK_CREATE: &str = "TaskCreate";
pub const TOOL_TASK_UPDATE: &str = "TaskUpdate";
pub const TOOL_TODO_WRITE: &str = "TodoWrite";
pub const TOOL_DESKTOP_PLAN: &str = "mcp__ccd_turn__update_plan";

/// Adaptador del que sale una lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    /// A1: `TaskCreate`/`TaskUpdate`.
    TaskTools,
    /// A2: `TodoWrite`.
    TodoWrite,
    /// A3: `update_plan` del Desktop.
    DesktopPlan,
}

impl TaskSource {
    pub const ALL: [TaskSource; 3] = [
        TaskSource::TaskTools,
        TaskSource::TodoWrite,
        TaskSource::DesktopPlan,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            TaskSource::TaskTools => "task_tools",
            TaskSource::TodoWrite => "todo_write",
            TaskSource::DesktopPlan => "desktop_plan",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Herramienta cuyo uso produce esta lista (para el aviso de deriva).
    pub const fn tool(self) -> &'static str {
        match self {
            TaskSource::TaskTools => TOOL_TASK_UPDATE,
            TaskSource::TodoWrite => TOOL_TODO_WRITE,
            TaskSource::DesktopPlan => TOOL_DESKTOP_PLAN,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    /// Solo A1: `TaskUpdate {status: "deleted"}`. No cuenta para el progreso.
    Deleted,
}

impl TaskStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(TaskStatus::Pending),
            "in_progress" => Some(TaskStatus::InProgress),
            "completed" => Some(TaskStatus::Completed),
            "deleted" => Some(TaskStatus::Deleted),
            _ => None,
        }
    }
}

/// Un elemento de una lista completa (A2, A3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub status: TaskStatus,
}

/// Una operación declarada sobre una lista.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskOp {
    /// A1: resultado de `TaskCreate` o hook `TaskCreated`.
    Create {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
    },
    /// A1: `TaskUpdate`. Solo cambia lo que trae.
    Update {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<TaskStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_blocked_by: Vec<String>,
    },
    /// A1: hook `TaskCompleted`.
    Complete { task_id: String },
    /// A2 y A3: la lista completa sustituye a la anterior.
    Replace { items: Vec<ListItem> },
    /// A3: termina el paso en curso y empieza el siguiente pendiente.
    Advance,
    /// A3: marca pasos (1…n) como terminados y, opcionalmente, empieza uno.
    Mark {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        done: Vec<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<u32>,
    },
}

impl TaskOp {
    /// Clave idempotente dentro de la sesión (SPEC §6.3: `(session_id, source, task_id)`).
    ///
    /// Una tarea se crea una sola vez, así que el resultado de `TaskCreate` y el hook
    /// `TaskCreated` comparten clave. El resto se identifica por su `tool_use_id`.
    pub fn key(&self, tool_use_id: Option<&str>) -> Option<String> {
        match self {
            TaskOp::Create { task_id, .. } => Some(format!("create:{task_id}")),
            TaskOp::Complete { task_id } => Some(format!("completed:{task_id}")),
            _ => tool_use_id
                .filter(|id| !id.is_empty())
                .map(|id| format!("tool:{id}")),
        }
    }

    /// Rellena los títulos que faltan con los de `other`, si es la misma operación.
    ///
    /// Lo capturado por el mod antes del primer `hello` no lleva títulos; el
    /// transcript los trae después con la política de la app (SPEC §5.4).
    pub fn fill_subjects(&mut self, other: &TaskOp) {
        match (self, other) {
            (
                TaskOp::Create { subject, .. },
                TaskOp::Create {
                    subject: theirs, ..
                },
            )
            | (
                TaskOp::Update { subject, .. },
                TaskOp::Update {
                    subject: theirs, ..
                },
            ) => {
                if subject.is_none() {
                    subject.clone_from(theirs);
                }
            }
            (TaskOp::Replace { items }, TaskOp::Replace { items: theirs })
                if items.len() == theirs.len() =>
            {
                for (mine, other) in items.iter_mut().zip(theirs) {
                    if mine.subject.is_none() {
                        mine.subject.clone_from(&other.subject);
                    }
                }
            }
            _ => {}
        }
    }

    /// Quita todos los títulos (`capture_task_titles = false`, SPEC §10).
    pub fn clear_subjects(&mut self) {
        match self {
            TaskOp::Create { subject, .. } | TaskOp::Update { subject, .. } => *subject = None,
            TaskOp::Replace { items } => items.iter_mut().for_each(|i| i.subject = None),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Extracción (lista blanca de §10)
// ---------------------------------------------------------------------------

/// Elemento de una lista tal como llega en los argumentos.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListItemView<'a> {
    pub content: Option<&'a str>,
    pub status: Option<&'a str>,
}

/// Campos de los argumentos que miran los adaptadores. Todo lo demás ni se lee.
#[derive(Debug, Clone, Default)]
pub struct TaskInputView<'a> {
    pub subject: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub add_blocked_by: Option<Vec<&'a str>>,
    /// `TodoWrite.todos`.
    pub todos: Option<Vec<ListItemView<'a>>>,
    /// `update_plan.items`.
    pub items: Option<Vec<ListItemView<'a>>>,
    pub advance: Option<bool>,
    pub done: Option<Vec<i64>>,
    pub start: Option<i64>,
    /// Hay campos que el adaptador no conoce (posible cambio de esquema, G6).
    pub has_unknown_fields: bool,
}

/// Qué produce una llamada a herramienta para el Task Monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adapted {
    /// No es una herramienta de tareas, o no aplica (p. ej. A2 en un subagente).
    NotATaskCall,
    /// Operación válida.
    Op(TaskSource, TaskOp),
    /// La forma no es la esperada: no se aplica nada y se avisa de la deriva.
    Drift(TaskSource),
}

/// Operación de una llamada a `TaskUpdate`, `TodoWrite` o `update_plan`.
/// `TaskCreate` sale de su resultado ([`create_op`]), que trae el id.
///
/// Si la llamada termina en error, la operación se retira (lo hace el store).
///
/// `is_subagent`: la llamada viene de un subagente. Su `TodoWrite` es una lista
/// propia que no es la de la sesión, y el Desktop rechaza su `update_plan`.
pub fn op_from_call(
    tool: &str,
    input: &TaskInputView<'_>,
    policy: &PrivacyPolicy,
    is_subagent: bool,
) -> Adapted {
    match tool {
        TOOL_TASK_UPDATE => update_op(input, policy),
        TOOL_TODO_WRITE if is_subagent => Adapted::NotATaskCall,
        TOOL_TODO_WRITE => match &input.todos {
            Some(todos) => list_op(TaskSource::TodoWrite, todos, policy, false),
            None => Adapted::Drift(TaskSource::TodoWrite),
        },
        TOOL_DESKTOP_PLAN if is_subagent => Adapted::NotATaskCall,
        TOOL_DESKTOP_PLAN => plan_op(input, policy),
        _ => Adapted::NotATaskCall,
    }
}

fn update_op(input: &TaskInputView<'_>, policy: &PrivacyPolicy) -> Adapted {
    let drift = Adapted::Drift(TaskSource::TaskTools);
    let Some(task_id) = input.task_id.and_then(clean_id) else {
        return drift;
    };
    let status = match input.status {
        Some(s) => match TaskStatus::parse(s) {
            Some(status) => Some(status),
            None => return drift,
        },
        None => None,
    };
    let mut add_blocked_by: Vec<String> = Vec::new();
    for id in input.add_blocked_by.iter().flatten() {
        if let Some(id) = clean_id(id)
            && !add_blocked_by.contains(&id)
            && add_blocked_by.len() < MAX_BLOCKERS
        {
            add_blocked_by.push(id);
        }
    }
    Adapted::Op(
        TaskSource::TaskTools,
        TaskOp::Update {
            task_id,
            status,
            subject: input.subject.and_then(|s| subject(s, policy)),
            owner: input.owner.and_then(clean_owner),
            add_blocked_by,
        },
    )
}

fn list_op(
    source: TaskSource,
    list: &[ListItemView<'_>],
    policy: &PrivacyPolicy,
    must_be_non_empty: bool,
) -> Adapted {
    if must_be_non_empty && list.is_empty() {
        return Adapted::Drift(source);
    }
    let mut items = Vec::with_capacity(list.len().min(MAX_ITEMS));
    for item in list.iter().take(MAX_ITEMS) {
        // Un estado desconocido es un cambio de esquema: no se adivina.
        let Some(status) = item.status.and_then(TaskStatus::parse) else {
            return Adapted::Drift(source);
        };
        if status == TaskStatus::Deleted {
            return Adapted::Drift(source);
        }
        items.push(ListItem {
            subject: item.content.and_then(|c| subject(c, policy)),
            status,
        });
    }
    Adapted::Op(source, TaskOp::Replace { items })
}

/// A3. Exactamente una forma: `items`, `advance: true` o `done`/`start` (G6).
fn plan_op(input: &TaskInputView<'_>, policy: &PrivacyPolicy) -> Adapted {
    let drift = Adapted::Drift(TaskSource::DesktopPlan);
    if input.has_unknown_fields {
        return drift;
    }
    let marks = input.done.is_some() || input.start.is_some();
    let forms = usize::from(input.items.is_some())
        + usize::from(input.advance.is_some())
        + usize::from(marks);
    if forms != 1 {
        return drift;
    }
    if let Some(items) = &input.items {
        return list_op(TaskSource::DesktopPlan, items, policy, true);
    }
    if let Some(advance) = input.advance {
        return if advance {
            Adapted::Op(TaskSource::DesktopPlan, TaskOp::Advance)
        } else {
            drift
        };
    }
    let step = |n: i64| u32::try_from(n).ok().filter(|n| *n >= 1);
    let mut done = Vec::new();
    for n in input.done.iter().flatten() {
        match step(*n) {
            Some(n) if !done.contains(&n) => done.push(n),
            Some(_) => {}
            None => return drift,
        }
    }
    let start = match input.start {
        Some(n) => match step(n) {
            Some(n) => Some(n),
            None => return drift,
        },
        None => None,
    };
    Adapted::Op(TaskSource::DesktopPlan, TaskOp::Mark { done, start })
}

/// A1: tarea creada, con el id que devolvió `TaskCreate` o que trae `TaskCreated`.
pub fn create_op(
    task_id: &str,
    subject_text: Option<&str>,
    policy: &PrivacyPolicy,
) -> Option<TaskOp> {
    Some(TaskOp::Create {
        task_id: clean_id(task_id)?,
        subject: subject_text.and_then(|s| subject(s, policy)),
    })
}

/// A1: hook `TaskCompleted`.
pub fn complete_op(task_id: &str) -> Option<TaskOp> {
    Some(TaskOp::Complete {
        task_id: clean_id(task_id)?,
    })
}

/// Título permitido por la política: una línea, como mucho 120 caracteres (D1).
pub fn subject(text: &str, policy: &PrivacyPolicy) -> Option<String> {
    if !policy.capture_task_titles {
        return None;
    }
    let mut out = String::new();
    let mut pending_space = false;
    for c in text.trim().chars() {
        if c.is_whitespace() || c.is_control() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    let out: String = out.chars().take(SUBJECT_MAX_CHARS).collect();
    (!out.is_empty()).then_some(out)
}

/// Id de tarea: un identificador corto, nunca texto libre.
pub fn clean_id(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = !s.is_empty()
        && s.chars().count() <= ID_MAX_CHARS
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'));
    ok.then(|| s.to_owned())
}

/// Responsable: nombre de un compañero de equipo (`nombre` o `nombre@equipo`).
pub fn clean_owner(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = !s.is_empty()
        && s.chars().count() <= ID_MAX_CHARS
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '@'));
    ok.then(|| s.to_owned())
}

// ---------------------------------------------------------------------------
// Reproducción
// ---------------------------------------------------------------------------

/// Estado de una tarea tras reproducir las operaciones de su lista.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskState {
    /// Id de `TaskCreate` en A1; posición (`1`…`n`) en A2 y A3.
    pub task_id: String,
    pub subject: Option<String>,
    pub status: TaskStatus,
    pub owner: Option<String>,
    pub blocked_by: Vec<String>,
    /// Orden de presentación, desde 0.
    pub position: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

impl TaskState {
    fn new(task_id: String, position: u32, ts: i64) -> Self {
        Self {
            task_id,
            subject: None,
            status: TaskStatus::Pending,
            owner: None,
            blocked_by: Vec::new(),
            position,
            created_at: ts,
            updated_at: ts,
            completed_at: None,
        }
    }

    fn set_status(&mut self, status: TaskStatus, ts: i64) {
        if status == TaskStatus::Completed {
            if self.status != TaskStatus::Completed {
                self.completed_at = Some(ts);
            }
        } else {
            self.completed_at = None;
        }
        self.status = status;
        self.updated_at = self.updated_at.max(ts);
    }
}

/// Estado de una lista a partir de sus operaciones, **ya ordenadas** por
/// `(ts, orden de llegada)`. Las operaciones de otro adaptador se ignoran.
pub fn replay(source: TaskSource, ops: &[(i64, TaskOp)]) -> Vec<TaskState> {
    match source {
        TaskSource::TaskTools => replay_tasks(ops),
        TaskSource::TodoWrite | TaskSource::DesktopPlan => replay_list(source, ops),
    }
}

fn replay_tasks(ops: &[(i64, TaskOp)]) -> Vec<TaskState> {
    let mut tasks: Vec<TaskState> = Vec::new();
    fn entry(tasks: &mut Vec<TaskState>, id: &str, ts: i64) -> usize {
        match tasks.iter().position(|t| t.task_id == id) {
            Some(i) => i,
            None => {
                let position = tasks.len() as u32;
                tasks.push(TaskState::new(id.to_owned(), position, ts));
                tasks.len() - 1
            }
        }
    }
    for (ts, op) in ops {
        match op {
            TaskOp::Create { task_id, subject } => {
                let i = entry(&mut tasks, task_id, *ts);
                let task = &mut tasks[i];
                task.created_at = task.created_at.min(*ts);
                if task.subject.is_none() {
                    task.subject.clone_from(subject);
                }
            }
            TaskOp::Update {
                task_id,
                status,
                subject,
                owner,
                add_blocked_by,
            } => {
                let i = entry(&mut tasks, task_id, *ts);
                let task = &mut tasks[i];
                if let Some(status) = status {
                    task.set_status(*status, *ts);
                }
                if subject.is_some() {
                    task.subject.clone_from(subject);
                }
                if owner.is_some() {
                    task.owner.clone_from(owner);
                }
                for id in add_blocked_by {
                    if !task.blocked_by.contains(id) && task.blocked_by.len() < MAX_BLOCKERS {
                        task.blocked_by.push(id.clone());
                    }
                }
                task.updated_at = task.updated_at.max(*ts);
            }
            TaskOp::Complete { task_id } => {
                let i = entry(&mut tasks, task_id, *ts);
                tasks[i].set_status(TaskStatus::Completed, *ts);
            }
            TaskOp::Replace { .. } | TaskOp::Advance | TaskOp::Mark { .. } => {}
        }
    }
    // Los ids de Claude Code son números correlativos: se ordenan como números.
    if tasks.iter().all(|t| t.task_id.parse::<u64>().is_ok()) {
        tasks.sort_by_key(|t| t.task_id.parse::<u64>().unwrap_or(u64::MAX));
    }
    for (i, task) in tasks.iter_mut().enumerate() {
        task.position = i as u32;
    }
    tasks
}

fn replay_list(source: TaskSource, ops: &[(i64, TaskOp)]) -> Vec<TaskState> {
    let mut list: Vec<TaskState> = Vec::new();
    for (ts, op) in ops {
        let ts = *ts;
        match op {
            TaskOp::Replace { items } => {
                let mut next = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    let mut task = match list.get(i) {
                        // La misma posición conserva su historia si sigue siendo el mismo paso.
                        Some(prev) if prev.subject == item.subject => prev.clone(),
                        _ => TaskState::new((i + 1).to_string(), i as u32, ts),
                    };
                    task.subject.clone_from(&item.subject);
                    if task.status != item.status {
                        task.set_status(item.status, ts);
                    }
                    task.updated_at = ts;
                    next.push(task);
                }
                list = next;
            }
            TaskOp::Advance if source == TaskSource::DesktopPlan => {
                let current = list.iter().position(|t| t.status == TaskStatus::InProgress);
                if let Some(i) = current {
                    list[i].set_status(TaskStatus::Completed, ts);
                }
                let from = current.map_or(0, |i| i + 1);
                let next = list
                    .iter()
                    .enumerate()
                    .position(|(i, t)| i >= from && t.status == TaskStatus::Pending)
                    .or_else(|| list.iter().position(|t| t.status == TaskStatus::Pending));
                if let Some(i) = next {
                    list[i].set_status(TaskStatus::InProgress, ts);
                }
            }
            TaskOp::Mark { done, start } if source == TaskSource::DesktopPlan => {
                let index = |n: u32| (n as usize).checked_sub(1);
                for n in done {
                    if let Some(task) = index(*n).and_then(|i| list.get_mut(i)) {
                        task.set_status(TaskStatus::Completed, ts);
                    }
                }
                if let Some(task) = start.and_then(index).and_then(|i| list.get_mut(i)) {
                    task.set_status(TaskStatus::InProgress, ts);
                }
            }
            _ => {}
        }
    }
    list
}

/// Resumen de progreso: lo que declara la lista, sin las tareas borradas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub total: u32,
    pub completed: u32,
    pub in_progress: u32,
}

pub fn progress(tasks: &[TaskState]) -> Progress {
    let mut p = Progress::default();
    for t in tasks {
        match t.status {
            TaskStatus::Deleted => continue,
            TaskStatus::Completed => p.completed += 1,
            TaskStatus::InProgress => p.in_progress += 1,
            TaskStatus::Pending => {}
        }
        p.total += 1;
    }
    p
}

// ---------------------------------------------------------------------------
// Subagentes (A4)
// ---------------------------------------------------------------------------

/// Estado de un subagente (`AgentStatus` del motor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Pending,
    Running,
    Waiting,
    Idle,
    Completed,
    Failed,
    Killed,
}

impl AgentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            AgentState::Pending => "pending",
            AgentState::Running => "running",
            AgentState::Waiting => "waiting",
            AgentState::Idle => "idle",
            AgentState::Completed => "completed",
            AgentState::Failed => "failed",
            AgentState::Killed => "killed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => AgentState::Pending,
            "running" => AgentState::Running,
            "waiting" => AgentState::Waiting,
            "idle" => AgentState::Idle,
            "completed" => AgentState::Completed,
            "failed" => AgentState::Failed,
            "killed" => AgentState::Killed,
            _ => return None,
        })
    }

    /// Estado del resultado de la herramienta `Agent` en el transcript.
    pub fn from_agent_result(status: Option<&str>, is_error: bool) -> Option<Self> {
        if is_error {
            return Some(AgentState::Failed);
        }
        match status? {
            "completed" => Some(AgentState::Completed),
            // En segundo plano: sigue en marcha cuando la herramienta ya ha vuelto.
            "async_launched" | "running" => Some(AgentState::Running),
            "failed" | "error" => Some(AgentState::Failed),
            "killed" | "cancelled" => Some(AgentState::Killed),
            other => AgentState::parse(other),
        }
    }
}

/// Tipo de agente (`Explore`, `general-purpose`, `plugin:nombre`…): un identificador.
pub fn clean_agent_type(s: &str) -> Option<String> {
    let s: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
        .take(ID_MAX_CHARS)
        .collect();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: PrivacyPolicy = PrivacyPolicy::APP_DEFAULT;
    const TRAP: &str = "SECRETO-TRAMPA";

    fn item<'a>(content: &'a str, status: &'a str) -> ListItemView<'a> {
        ListItemView {
            content: Some(content),
            status: Some(status),
        }
    }

    fn statuses(tasks: &[TaskState]) -> Vec<&'static str> {
        tasks.iter().map(|t| t.status.as_str()).collect()
    }

    #[test]
    fn titles_are_truncated_flattened_and_optional() {
        let long = "a".repeat(300);
        assert_eq!(subject(&long, &APP).map(|s| s.chars().count()), Some(120));
        assert_eq!(
            subject("  Primera línea\n\tsegunda  ", &APP).as_deref(),
            Some("Primera línea segunda")
        );
        assert_eq!(subject("   ", &APP), None);
        assert_eq!(subject("Título", &PrivacyPolicy::PRE_HANDSHAKE), None);
        let no_titles = PrivacyPolicy {
            capture_task_titles: false,
            ..APP
        };
        assert_eq!(subject("Título", &no_titles), None);
    }

    #[test]
    fn ids_and_owners_are_identifiers_not_text() {
        assert_eq!(clean_id("12").as_deref(), Some("12"));
        assert_eq!(clean_id("tarea con espacios"), None);
        assert_eq!(clean_id(&"9".repeat(65)), None);
        assert_eq!(clean_owner("ana@equipo").as_deref(), Some("ana@equipo"));
        assert_eq!(clean_owner("rm -rf /"), None);
    }

    #[test]
    fn task_update_keeps_only_whitelisted_fields() {
        let input = TaskInputView {
            task_id: Some("3"),
            status: Some("in_progress"),
            owner: Some("investigador"),
            add_blocked_by: Some(vec!["1", "2", "1", "no válido"]),
            subject: Some("Nuevo título"),
            ..Default::default()
        };
        let Adapted::Op(source, op) = op_from_call(TOOL_TASK_UPDATE, &input, &APP, false) else {
            panic!("debería adaptarse");
        };
        assert_eq!(source, TaskSource::TaskTools);
        assert_eq!(
            op,
            TaskOp::Update {
                task_id: "3".to_owned(),
                status: Some(TaskStatus::InProgress),
                subject: Some("Nuevo título".to_owned()),
                owner: Some("investigador".to_owned()),
                add_blocked_by: vec!["1".to_owned(), "2".to_owned()],
            }
        );
        // Funciona igual desde un subagente: la lista de tareas es de la sesión.
        assert!(matches!(
            op_from_call(TOOL_TASK_UPDATE, &input, &APP, true),
            Adapted::Op(..)
        ));
    }

    #[test]
    fn unexpected_shapes_are_drift_not_guesses() {
        let bad_status = TaskInputView {
            task_id: Some("1"),
            status: Some("archived"),
            ..Default::default()
        };
        assert_eq!(
            op_from_call(TOOL_TASK_UPDATE, &bad_status, &APP, false),
            Adapted::Drift(TaskSource::TaskTools)
        );
        let two_forms = TaskInputView {
            advance: Some(true),
            start: Some(2),
            ..Default::default()
        };
        assert_eq!(
            op_from_call(TOOL_DESKTOP_PLAN, &two_forms, &APP, false),
            Adapted::Drift(TaskSource::DesktopPlan)
        );
        let unknown = TaskInputView {
            advance: Some(true),
            has_unknown_fields: true,
            ..Default::default()
        };
        assert_eq!(
            op_from_call(TOOL_DESKTOP_PLAN, &unknown, &APP, false),
            Adapted::Drift(TaskSource::DesktopPlan)
        );
        let zero = TaskInputView {
            done: Some(vec![0]),
            ..Default::default()
        };
        assert_eq!(
            op_from_call(TOOL_DESKTOP_PLAN, &zero, &APP, false),
            Adapted::Drift(TaskSource::DesktopPlan)
        );
        let empty_plan = TaskInputView {
            items: Some(vec![]),
            ..Default::default()
        };
        assert_eq!(
            op_from_call(TOOL_DESKTOP_PLAN, &empty_plan, &APP, false),
            Adapted::Drift(TaskSource::DesktopPlan)
        );
    }

    #[test]
    fn subagent_lists_are_not_the_session_list() {
        let input = TaskInputView {
            todos: Some(vec![item("x", "pending")]),
            items: Some(vec![item("x", "pending")]),
            ..Default::default()
        };
        assert_eq!(
            op_from_call(TOOL_TODO_WRITE, &input, &APP, true),
            Adapted::NotATaskCall
        );
        assert_eq!(
            op_from_call(TOOL_DESKTOP_PLAN, &input, &APP, true),
            Adapted::NotATaskCall
        );
    }

    #[test]
    fn never_leaks_anything_but_the_title() {
        // Lo que no está en la vista no se puede filtrar: `description`,
        // `activeForm` y `metadata` ni siquiera tienen campo. Sin títulos, nada.
        let input = TaskInputView {
            todos: Some(vec![item(TRAP, "in_progress")]),
            ..Default::default()
        };
        let strict = PrivacyPolicy {
            capture_task_titles: false,
            ..APP
        };
        let Adapted::Op(_, op) = op_from_call(TOOL_TODO_WRITE, &input, &strict, false) else {
            panic!();
        };
        assert!(!format!("{op:?}").contains(TRAP));
        assert!(!serde_json::to_string(&op).unwrap().contains(TRAP));
    }

    #[test]
    fn task_tools_replay_creates_updates_and_completes() {
        let ops = vec![
            (10, create_op("1", Some("Leer la SPEC"), &APP).unwrap()),
            (
                11,
                create_op("2", Some("Escribir el adaptador"), &APP).unwrap(),
            ),
            (
                12,
                TaskOp::Update {
                    task_id: "2".to_owned(),
                    status: Some(TaskStatus::InProgress),
                    subject: None,
                    owner: Some("ana".to_owned()),
                    add_blocked_by: vec!["1".to_owned()],
                },
            ),
            (13, complete_op("1").unwrap()),
            // Un hook repetido no cambia nada.
            (14, create_op("1", Some("Otro título"), &APP).unwrap()),
        ];
        let tasks = replay(TaskSource::TaskTools, &ops);
        assert_eq!(statuses(&tasks), ["completed", "in_progress"]);
        assert_eq!(tasks[0].subject.as_deref(), Some("Leer la SPEC"));
        assert_eq!(tasks[0].completed_at, Some(13));
        assert_eq!(tasks[1].owner.as_deref(), Some("ana"));
        assert_eq!(tasks[1].blocked_by, ["1"]);
        assert_eq!(
            progress(&tasks),
            Progress {
                total: 2,
                completed: 1,
                in_progress: 1
            }
        );
    }

    #[test]
    fn an_update_before_its_create_still_lands() {
        // Orden invertido entre ficheros (un compañero en otro transcript).
        let ops = vec![
            (
                5,
                TaskOp::Update {
                    task_id: "10".to_owned(),
                    status: Some(TaskStatus::Deleted),
                    subject: None,
                    owner: None,
                    add_blocked_by: vec![],
                },
            ),
            (6, create_op("10", Some("Borrada"), &APP).unwrap()),
            (7, create_op("9", None, &APP).unwrap()),
        ];
        let tasks = replay(TaskSource::TaskTools, &ops);
        assert_eq!(
            tasks.iter().map(|t| t.task_id.as_str()).collect::<Vec<_>>(),
            ["9", "10"]
        );
        assert_eq!(tasks[1].status, TaskStatus::Deleted);
        assert_eq!(tasks[1].subject.as_deref(), Some("Borrada"));
        assert_eq!(progress(&tasks).total, 1);
    }

    fn plan(statuses: &[&'static str]) -> TaskOp {
        let views: Vec<_> = statuses
            .iter()
            .enumerate()
            .map(|(i, s)| ListItemView {
                content: Some(["Uno", "Dos", "Tres", "Cuatro"][i]),
                status: Some(s),
            })
            .collect();
        let input = TaskInputView {
            items: Some(views),
            ..Default::default()
        };
        match op_from_call(TOOL_DESKTOP_PLAN, &input, &APP, false) {
            Adapted::Op(_, op) => op,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn desktop_plan_advance_and_marks() {
        let ops = vec![
            (1, plan(&["in_progress", "pending", "pending", "pending"])),
            (2, TaskOp::Advance),
            (
                3,
                TaskOp::Mark {
                    done: vec![2, 3],
                    start: Some(4),
                },
            ),
        ];
        let tasks = replay(TaskSource::DesktopPlan, &ops[..2]);
        assert_eq!(
            statuses(&tasks),
            ["completed", "in_progress", "pending", "pending"]
        );
        let tasks = replay(TaskSource::DesktopPlan, &ops);
        assert_eq!(
            statuses(&tasks),
            ["completed", "completed", "completed", "in_progress"]
        );
        assert_eq!(tasks[0].completed_at, Some(2));
        // Un paso fuera de rango no hace nada.
        let more = [
            ops.clone(),
            vec![(
                4,
                TaskOp::Mark {
                    done: vec![9],
                    start: None,
                },
            )],
        ]
        .concat();
        assert_eq!(replay(TaskSource::DesktopPlan, &more), tasks);
    }

    #[test]
    fn advance_without_a_step_in_progress_starts_the_first_pending() {
        let ops = vec![
            (1, plan(&["completed", "pending", "pending"])),
            (2, TaskOp::Advance),
        ];
        let tasks = replay(TaskSource::DesktopPlan, &ops);
        assert_eq!(statuses(&tasks), ["completed", "in_progress", "pending"]);
    }

    #[test]
    fn replacing_a_list_keeps_the_history_of_unchanged_steps() {
        let ops = vec![
            (1, plan(&["in_progress", "pending"])),
            (5, plan(&["completed", "in_progress", "pending"])),
            (9, plan(&["completed", "completed", "in_progress"])),
        ];
        let tasks = replay(TaskSource::DesktopPlan, &ops);
        assert_eq!(tasks[0].created_at, 1);
        assert_eq!(tasks[0].completed_at, Some(5));
        assert_eq!(tasks[1].completed_at, Some(9));
        assert_eq!(tasks[2].created_at, 5);
    }

    #[test]
    fn todo_write_ignores_relative_plan_operations() {
        let input = TaskInputView {
            todos: Some(vec![item("a", "in_progress"), item("b", "pending")]),
            ..Default::default()
        };
        let Adapted::Op(source, op) = op_from_call(TOOL_TODO_WRITE, &input, &APP, false) else {
            panic!();
        };
        assert_eq!(source, TaskSource::TodoWrite);
        let tasks = replay(TaskSource::TodoWrite, &[(1, op), (2, TaskOp::Advance)]);
        assert_eq!(statuses(&tasks), ["in_progress", "pending"]);
    }

    #[test]
    fn missing_titles_are_filled_from_a_later_source() {
        let strict = PrivacyPolicy::PRE_HANDSHAKE;
        let input = TaskInputView {
            items: Some(vec![item("Uno", "in_progress"), item("Dos", "pending")]),
            ..Default::default()
        };
        let Adapted::Op(_, mut from_mod) = op_from_call(TOOL_DESKTOP_PLAN, &input, &strict, false)
        else {
            panic!();
        };
        let Adapted::Op(_, from_transcript) = op_from_call(TOOL_DESKTOP_PLAN, &input, &APP, false)
        else {
            panic!();
        };
        from_mod.fill_subjects(&from_transcript);
        assert_eq!(from_mod, from_transcript);
        from_mod.clear_subjects();
        assert!(
            matches!(&from_mod, TaskOp::Replace { items } if items.iter().all(|i| i.subject.is_none()))
        );
    }

    #[test]
    fn op_keys_are_shared_by_tool_and_hook() {
        let create = create_op("4", None, &APP).unwrap();
        assert_eq!(create.key(Some("toolu_1")).as_deref(), Some("create:4"));
        assert_eq!(create.key(None).as_deref(), Some("create:4"));
        assert_eq!(
            TaskOp::Advance.key(Some("toolu_2")).as_deref(),
            Some("tool:toolu_2")
        );
        assert_eq!(TaskOp::Advance.key(None), None);
    }

    #[test]
    fn ops_round_trip_through_json() {
        for op in [
            plan(&["in_progress", "pending"]),
            TaskOp::Advance,
            TaskOp::Mark {
                done: vec![1],
                start: Some(2),
            },
            create_op("1", Some("x"), &APP).unwrap(),
        ] {
            let json = serde_json::to_string(&op).unwrap();
            assert_eq!(serde_json::from_str::<TaskOp>(&json).unwrap(), op, "{json}");
        }
    }

    #[test]
    fn agent_states_from_the_agent_tool_result() {
        assert_eq!(
            AgentState::from_agent_result(Some("async_launched"), false),
            Some(AgentState::Running)
        );
        assert_eq!(
            AgentState::from_agent_result(Some("completed"), false),
            Some(AgentState::Completed)
        );
        assert_eq!(
            AgentState::from_agent_result(None, true),
            Some(AgentState::Failed)
        );
        assert_eq!(AgentState::from_agent_result(Some("raro"), false), None);
        assert_eq!(
            clean_agent_type("general-purpose").as_deref(),
            Some("general-purpose")
        );
    }
}
