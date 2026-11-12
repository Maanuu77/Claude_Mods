//! Atribución de los cambios de fichero por intervalo completo (SPEC §7.3).
//!
//! Lógica pura: recibe el cambio observado y las herramientas candidatas de la
//! raíz, y decide quién lo hizo y con qué confianza. No lee nada del disco ni de
//! la base de datos; eso lo hace quien la llama.

use serde::Serialize;

/// Tolerancia de reloj entre el mod y Rust, que comparten máquina (§7.3, G5).
pub const EPSILON_MS: i64 = 250;
/// Retardo de escritura a disco, debounce de 200 ms y latencia del Watcher.
pub const DELTA_MS: i64 = 1_500;
/// La atribución es provisional durante este tiempo desde el cambio.
pub const SETTLE_MS: i64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Confidence::Low),
            "medium" => Some(Confidence::Medium),
            "high" => Some(Confidence::High),
            _ => None,
        }
    }

    /// Un nivel menos, sin bajar de `low` (desempates de §7.3).
    pub const fn lowered(self) -> Self {
        match self {
            Confidence::High => Confidence::Medium,
            Confidence::Medium | Confidence::Low => Confidence::Low,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Actor {
    /// Una herramienta de edición de Claude.
    Claude,
    /// Un `Bash`, `PowerShell` o MCP de Claude.
    ClaudeShell,
    /// Hay una sesión activa en la raíz, pero ninguna herramienta encaja.
    Unknown,
    /// No hay ninguna sesión activa en la raíz.
    External,
}

impl Actor {
    pub const fn as_str(self) -> &'static str {
        match self {
            Actor::Claude => "claude",
            Actor::ClaudeShell => "claude-shell",
            Actor::Unknown => "unknown",
            Actor::External => "external",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Actor::Claude),
            "claude-shell" => Some(Actor::ClaudeShell),
            "unknown" => Some(Actor::Unknown),
            "external" => Some(Actor::External),
            _ => None,
        }
    }
}

/// Regla de §7.3 que decidió la atribución, por orden de prioridad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Rule {
    R1,
    R2,
    R3,
    R4,
    R5,
}

/// Qué papel puede tener una herramienta en un cambio de fichero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    /// `Edit`, `Write`, `MultiEdit`, `NotebookEdit`: R1 y R2.
    Edit,
    /// `Bash`, `PowerShell` y las herramientas MCP: R3.
    Shell,
}

/// Clase de la herramienta, o `None` si no puede escribir ficheros (`Read`, `Grep`…).
pub fn tool_class(tool: &str) -> Option<ToolClass> {
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Some(ToolClass::Edit),
        "Bash" | "PowerShell" => Some(ToolClass::Shell),
        t if t.starts_with("mcp__") => Some(ToolClass::Shell),
        _ => None,
    }
}

/// Una herramienta de una sesión cuya raíz es la del cambio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub session_id: String,
    pub tool_use_id: String,
    pub class: ToolClass,
    pub started_at: i64,
    /// `None` mientras sigue en curso.
    pub finished_at: Option<i64>,
    /// Ruta relativa a la **raíz** (no al `cwd` de la sesión), con `/`.
    /// `None` si la herramienta no la lleva (`PRE_HANDSHAKE`, `record_paths=false`).
    pub rel_path: Option<String>,
}

impl Candidate {
    /// `t ∈ [started_at − ε, finished_at + δ]`, o `[started_at − ε, now]` en curso.
    pub fn covers(&self, t: i64, now: i64) -> bool {
        let end = self.finished_at.map_or(now, |f| f + DELTA_MS);
        self.started_at - EPSILON_MS <= t && t <= end
    }

    /// `t ∈ [started_at, finished_at]`: el intervalo real, sin las tolerancias.
    fn contains_strictly(&self, t: i64, now: i64) -> bool {
        self.started_at <= t && t <= self.finished_at.unwrap_or(now)
    }

    fn path_matches(&self, change: &Observed<'_>) -> bool {
        let Some(path) = self.rel_path.as_deref() else {
            return false;
        };
        change.rel_path.is_some_and(|p| same_path(p, path))
            || change.from_rel_path.is_some_and(|p| same_path(p, path))
    }
}

/// Windows no distingue mayúsculas en las rutas.
fn same_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

/// El cambio observado por el Watcher.
#[derive(Debug, Clone, Copy)]
pub struct Observed<'a> {
    /// Hora del primer evento crudo agrupado por el debouncer (reloj de pared).
    pub ts: i64,
    pub rel_path: Option<&'a str>,
    /// Ruta de origen de un renombrado.
    pub from_rel_path: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attribution {
    pub actor: Actor,
    pub session_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub confidence: Confidence,
    pub rule: Rule,
}

impl Attribution {
    fn nobody(actor: Actor, confidence: Confidence, rule: Rule) -> Self {
        Self {
            actor,
            session_id: None,
            tool_use_id: None,
            confidence,
            rule,
        }
    }

    fn tool(actor: Actor, c: &Candidate, confidence: Confidence, rule: Rule) -> Self {
        Self {
            actor,
            session_id: Some(c.session_id.clone()),
            tool_use_id: Some(c.tool_use_id.clone()),
            confidence,
            rule,
        }
    }
}

/// Aplica R1–R5 con los desempates de §7.3.
///
/// `candidates` son las herramientas de las sesiones de esta raíz (sin copias de
/// historial). `active_sessions` es el número de sesiones vivas en la raíz.
pub fn attribute(
    change: &Observed<'_>,
    candidates: &[Candidate],
    active_sessions: usize,
    now: i64,
) -> Attribution {
    let covering: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.covers(change.ts, now))
        .collect();
    let edits: Vec<&Candidate> = covering
        .iter()
        .copied()
        .filter(|c| c.class == ToolClass::Edit)
        .collect();

    // R1: herramienta de edición en el intervalo y con la misma ruta.
    let matching: Vec<&Candidate> = edits
        .iter()
        .copied()
        .filter(|c| c.path_matches(change))
        .collect();
    if !matching.is_empty() {
        return pick(
            &matching,
            change.ts,
            now,
            Actor::Claude,
            Confidence::High,
            Rule::R1,
            false,
        );
    }
    // R2: edición en el intervalo **sin ruta comparable** (`PRE_HANDSHAKE`,
    // `record_paths = false` o fuera del `cwd`). *Calibración de G5:* una edición
    // con ruta conocida que no coincide no es candidata, porque `Edit`/`Write`
    // solo escriben su propio fichero; atribuírsela convertía cambios externos
    // simultáneos en cambios de Claude.
    let pathless: Vec<&Candidate> = edits
        .iter()
        .copied()
        .filter(|c| c.rel_path.is_none())
        .collect();
    if !pathless.is_empty() {
        return pick(
            &pathless,
            change.ts,
            now,
            Actor::Claude,
            Confidence::Medium,
            Rule::R2,
            true,
        );
    }
    // R3: Bash, PowerShell o MCP en el intervalo.
    let shells: Vec<&Candidate> = covering
        .iter()
        .copied()
        .filter(|c| c.class == ToolClass::Shell)
        .collect();
    if !shells.is_empty() {
        return pick(
            &shells,
            change.ts,
            now,
            Actor::ClaudeShell,
            Confidence::Medium,
            Rule::R3,
            true,
        );
    }
    if active_sessions > 0 {
        Attribution::nobody(Actor::Unknown, Confidence::Low, Rule::R4)
    } else {
        Attribution::nobody(Actor::External, Confidence::High, Rule::R5)
    }
}

/// Desempate entre varias candidatas de la misma regla (§7.3):
/// - *calibración de G5:* si alguna contiene `t` en su intervalo real (sin ε ni
///   δ), solo cuentan esas. Así la cola δ de una herramienta ya terminada no
///   empata con la que está en curso;
/// - una sola: la regla con su confianza;
/// - de sesiones distintas y sin coincidencia de ruta: `unknown`, `low`;
/// - si no: la de inicio más reciente, con la confianza un nivel más baja.
fn pick(
    found: &[&Candidate],
    t: i64,
    now: i64,
    actor: Actor,
    confidence: Confidence,
    rule: Rule,
    without_path_match: bool,
) -> Attribution {
    let strict: Vec<&Candidate> = found
        .iter()
        .copied()
        .filter(|c| c.contains_strictly(t, now))
        .collect();
    let found = if strict.is_empty() { found } else { &strict };
    if let [only] = found {
        return Attribution::tool(actor, only, confidence, rule);
    }
    let first_session = &found[0].session_id;
    if without_path_match && found.iter().any(|c| &c.session_id != first_session) {
        return Attribution::nobody(Actor::Unknown, Confidence::Low, rule);
    }
    let latest = found
        .iter()
        .max_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then_with(|| b.tool_use_id.cmp(&a.tool_use_id))
        })
        .copied()
        .unwrap_or(found[0]);
    Attribution::tool(actor, latest, confidence.lowered(), rule)
}

/// ¿Sustituye `new` a la atribución guardada?
///
/// Durante la ventana de asentamiento manda siempre la última evaluación. Después
/// solo se admite subir un `unknown` a R1 o R2 (un `ToolFinished` tardío por una
/// reconexión), o *(calibración de G5)* subir a R1 con más confianza que la
/// guardada: una coincidencia de ruta es mejor prueba que un R2 o un R3. Nunca
/// se rebaja una atribución que ya era `high`.
pub fn should_replace(old: &Attribution, new: &Attribution, settled: bool) -> bool {
    if old == new {
        return false;
    }
    if !settled {
        return true;
    }
    (old.actor == Actor::Unknown && matches!(new.rule, Rule::R1 | Rule::R2))
        || (new.rule == Rule::R1 && new.confidence > old.confidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn tool(
        session: &str,
        id: &str,
        class: ToolClass,
        start: i64,
        end: Option<i64>,
        path: Option<&str>,
    ) -> Candidate {
        Candidate {
            session_id: session.to_owned(),
            tool_use_id: id.to_owned(),
            class,
            started_at: start,
            finished_at: end,
            rel_path: path.map(str::to_owned),
        }
    }

    fn at(ts: i64, path: &str) -> Observed<'_> {
        Observed {
            ts,
            rel_path: Some(path),
            from_rel_path: None,
        }
    }

    fn who(a: &Attribution) -> (Actor, Option<&str>, Confidence, Rule) {
        (a.actor, a.tool_use_id.as_deref(), a.confidence, a.rule)
    }

    #[test]
    fn classifies_tools() {
        assert_eq!(tool_class("Write"), Some(ToolClass::Edit));
        assert_eq!(tool_class("NotebookEdit"), Some(ToolClass::Edit));
        assert_eq!(tool_class("PowerShell"), Some(ToolClass::Shell));
        assert_eq!(tool_class("mcp__srv__run"), Some(ToolClass::Shell));
        assert_eq!(tool_class("Read"), None);
        assert_eq!(tool_class("Agent"), None);
    }

    #[test]
    fn interval_includes_epsilon_before_and_delta_after() {
        let t = tool("s", "t", ToolClass::Edit, 10_000, Some(11_000), None);
        assert!(t.covers(10_000 - EPSILON_MS, NOW));
        assert!(!t.covers(10_000 - EPSILON_MS - 1, NOW));
        assert!(t.covers(11_000 + DELTA_MS, NOW));
        assert!(!t.covers(11_000 + DELTA_MS + 1, NOW));
        // En curso: hasta ahora.
        let running = tool("s", "t", ToolClass::Edit, 10_000, None, None);
        assert!(running.covers(NOW, NOW));
    }

    #[test]
    fn r1_edit_with_same_path_is_high() {
        let c = [tool(
            "s1",
            "e1",
            ToolClass::Edit,
            100,
            Some(200),
            Some("src/a.rs"),
        )];
        let a = attribute(&at(150, "SRC/A.rs"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::Claude, Some("e1"), Confidence::High, Rule::R1)
        );
        assert_eq!(a.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn r1_matches_either_side_of_a_rename() {
        let c = [tool(
            "s1",
            "e1",
            ToolClass::Edit,
            100,
            Some(200),
            Some("old.rs"),
        )];
        let renamed = Observed {
            ts: 150,
            rel_path: Some("new.rs"),
            from_rel_path: Some("old.rs"),
        };
        assert_eq!(attribute(&renamed, &c, 1, NOW).rule, Rule::R1);
    }

    #[test]
    fn r2_is_an_edit_without_a_comparable_path() {
        let pathless = [tool("s1", "e1", ToolClass::Edit, 100, Some(200), None)];
        let a = attribute(&at(150, "a.rs"), &pathless, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::Claude, Some("e1"), Confidence::Medium, Rule::R2)
        );
    }

    /// Calibración de G5: un `Edit` con otra ruta no se lleva un cambio externo.
    #[test]
    fn an_edit_of_another_file_is_not_a_candidate() {
        let other = [tool(
            "s1",
            "e1",
            ToolClass::Edit,
            100,
            Some(200),
            Some("b.rs"),
        )];
        let a = attribute(&at(150, "a.rs"), &other, 1, NOW);
        assert_eq!(who(&a), (Actor::Unknown, None, Confidence::Low, Rule::R4));
    }

    #[test]
    fn r3_shell_in_interval_is_medium_claude_shell() {
        let c = [tool("s1", "b1", ToolClass::Shell, 100, Some(5_000), None)];
        let a = attribute(&at(4_000, "out/x.txt"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::ClaudeShell, Some("b1"), Confidence::Medium, Rule::R3)
        );
    }

    #[test]
    fn r4_and_r5_depend_on_active_sessions() {
        let late = [tool(
            "s1",
            "e1",
            ToolClass::Edit,
            100,
            Some(200),
            Some("a.rs"),
        )];
        let a = attribute(&at(200 + DELTA_MS + 1, "a.rs"), &late, 1, NOW);
        assert_eq!(who(&a), (Actor::Unknown, None, Confidence::Low, Rule::R4));
        let a = attribute(&at(200 + DELTA_MS + 1, "a.rs"), &late, 0, NOW);
        assert_eq!(who(&a), (Actor::External, None, Confidence::High, Rule::R5));
    }

    #[test]
    fn rules_apply_in_priority_order() {
        let c = [
            tool("s1", "b1", ToolClass::Shell, 100, Some(500), None),
            tool("s1", "e1", ToolClass::Edit, 120, Some(300), Some("x.rs")),
            tool("s1", "e2", ToolClass::Edit, 130, Some(300), Some("a.rs")),
        ];
        assert_eq!(
            attribute(&at(200, "a.rs"), &c, 1, NOW)
                .tool_use_id
                .as_deref(),
            Some("e2")
        );
        // Las ediciones de otros ficheros no cuentan: queda el Bash (R3).
        assert_eq!(attribute(&at(200, "z.rs"), &c, 1, NOW).rule, Rule::R3);
        let with_pathless = [
            tool("s1", "b1", ToolClass::Shell, 100, Some(500), None),
            tool("s1", "e3", ToolClass::Edit, 120, Some(300), None),
        ];
        assert_eq!(
            attribute(&at(200, "z.rs"), &with_pathless, 1, NOW).rule,
            Rule::R2
        );
    }

    /// Calibración de G5: la herramienta en curso gana a la cola δ de la anterior.
    #[test]
    fn the_real_interval_beats_the_tolerance_tail() {
        let c = [
            tool("s1", "b1", ToolClass::Shell, 100, Some(1_000), None),
            tool("s1", "b2", ToolClass::Shell, 1_400, Some(3_000), None),
        ];
        let a = attribute(&at(1_600, "out/x"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::ClaudeShell, Some("b2"), Confidence::Medium, Rule::R3)
        );
        // Solo en la cola δ de b2: una sola candidata.
        let a = attribute(&at(3_200, "out/x"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::ClaudeShell, Some("b2"), Confidence::Medium, Rule::R3)
        );
        // En la cola δ de b1 y en el ε de b2: empate, la más reciente y un nivel menos.
        let a = attribute(&at(1_200, "out/x"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::ClaudeShell, Some("b2"), Confidence::Low, Rule::R3)
        );
    }

    /// Desempate: la coincidencia de ruta decide entre llamadas en paralelo.
    #[test]
    fn parallel_edits_resolve_by_path() {
        let c = [
            tool("s1", "e1", ToolClass::Edit, 100, Some(300), Some("a.rs")),
            tool("s1", "e2", ToolClass::Edit, 110, Some(300), Some("b.rs")),
        ];
        let a = attribute(&at(200, "a.rs"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::Claude, Some("e1"), Confidence::High, Rule::R1)
        );
    }

    /// Desempate: si varias coinciden, la más reciente y un nivel menos.
    #[test]
    fn ties_pick_the_latest_start_and_lower_confidence() {
        let c = [
            tool("s1", "e1", ToolClass::Edit, 100, Some(300), Some("a.rs")),
            tool("s1", "e2", ToolClass::Edit, 150, Some(300), Some("a.rs")),
        ];
        let a = attribute(&at(200, "a.rs"), &c, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::Claude, Some("e2"), Confidence::Medium, Rule::R1)
        );

        let same_session = [
            tool("s1", "e1", ToolClass::Edit, 100, Some(300), None),
            tool("s1", "e2", ToolClass::Edit, 150, Some(300), None),
        ];
        let a = attribute(&at(200, "a.rs"), &same_session, 1, NOW);
        assert_eq!(
            who(&a),
            (Actor::Claude, Some("e2"), Confidence::Low, Rule::R2)
        );
    }

    /// Desempate: candidatas de sesiones distintas sin coincidencia de ruta.
    #[test]
    fn two_sessions_without_path_match_is_unknown() {
        let pathless = [
            tool("s1", "e1", ToolClass::Edit, 100, Some(300), None),
            tool("s2", "e2", ToolClass::Edit, 150, Some(300), None),
        ];
        let a = attribute(&at(200, "a.rs"), &pathless, 2, NOW);
        assert_eq!(who(&a), (Actor::Unknown, None, Confidence::Low, Rule::R2));
        // Con coincidencia de ruta en una de ellas, gana esa.
        let c = [
            tool("s1", "e1", ToolClass::Edit, 100, Some(300), Some("x.rs")),
            tool("s2", "e2", ToolClass::Edit, 150, Some(300), Some("y.rs")),
        ];
        let a = attribute(&at(200, "y.rs"), &c, 2, NOW);
        assert_eq!(
            who(&a),
            (Actor::Claude, Some("e2"), Confidence::High, Rule::R1)
        );
    }

    #[test]
    fn settled_attributions_only_go_up() {
        let unknown = Attribution::nobody(Actor::Unknown, Confidence::Low, Rule::R4);
        let external = Attribution::nobody(Actor::External, Confidence::High, Rule::R5);
        let c = tool("s1", "e1", ToolClass::Edit, 1, Some(2), Some("a"));
        let r1 = Attribution::tool(Actor::Claude, &c, Confidence::High, Rule::R1);
        let r2 = Attribution::tool(Actor::Claude, &c, Confidence::Medium, Rule::R2);
        let r3 = Attribution::tool(Actor::ClaudeShell, &c, Confidence::Medium, Rule::R3);

        // Provisional: siempre la última evaluación.
        assert!(should_replace(&r1, &unknown, false));
        assert!(!should_replace(&r1, &r1, false));
        // Definitiva: sube de unknown a R1/R2, o a R1 con más confianza.
        assert!(should_replace(&unknown, &r1, true));
        assert!(should_replace(&unknown, &r2, true));
        assert!(!should_replace(&unknown, &r3, true));
        assert!(should_replace(&r2, &r1, true));
        assert!(should_replace(&r3, &r1, true));
        // Nunca baja, ni cambia una `high` por otra.
        assert!(!should_replace(&r1, &unknown, true));
        assert!(!should_replace(&r1, &r2, true));
        assert!(!should_replace(&external, &r1, true));
    }
}
