//! Project Map v1 (SPEC §9): estructura del proyecto y componentes declarados.
//!
//! - **Estructura** ([`summarize_tree`]): a partir de la lista de ficheros (de
//!   `git ls-files` o del recorrido con `ignore`) y de sus tamaños: ficheros, bytes
//!   y lenguajes por directorio. Nunca se lee contenido.
//! - **Componentes** ([`build_map`]): solo de los manifiestos de la tabla de §9
//!   ([`manifest_kind`]). De cada uno se extraen nombre, versión, dependencias
//!   declaradas (nombre, versión pedida y ámbito) y relaciones internas. Nada más:
//!   ni scripts, ni descripciones, ni autores. No hay análisis de imports (v2).
//!
//! Todo trabaja con rutas relativas a la raíz y separadas por `/`. Una ruta que
//! salga de la raíz o sea absoluta no se resuelve ni se guarda, y las versiones que
//! son rutas o URL se reducen (pueden llevar el nombre del usuario o un token).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;
use serde_json::Value as Json;

/// Tope de entradas del árbol (SPEC §9: 50.000 sin Git).
pub const MAX_ENTRIES: usize = 50_000;
/// Un manifiesto más grande no se lee.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// Longitud máxima de un nombre o una versión.
const MAX_TEXT: usize = 200;
const MAX_DEPS_PER_MANIFEST: usize = 500;
const MAX_PARTS: usize = 64;
/// Directorios en los que un manifiesto nunca es del proyecto.
const VENDORED: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "bower_components",
    ".venv",
    "venv",
];

// ---------------------------------------------------------------------------
// Tipos públicos
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    Node,
    Rust,
    Java,
    Python,
    Dotnet,
    Unity,
    Rojo,
}

impl Ecosystem {
    pub const fn as_str(self) -> &'static str {
        match self {
            Ecosystem::Node => "node",
            Ecosystem::Rust => "rust",
            Ecosystem::Java => "java",
            Ecosystem::Python => "python",
            Ecosystem::Dotnet => "dotnet",
            Ecosystem::Unity => "unity",
            Ecosystem::Rojo => "rojo",
        }
    }
}

/// Manifiesto reconocido (tabla de §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    PackageJson,
    PnpmWorkspace,
    CargoToml,
    PomXml,
    GradleSettings,
    PyProject,
    Requirements,
    Sln,
    Csproj,
    UnityManifest,
    UnityVersion,
    RojoProject,
}

impl ManifestKind {
    pub const fn ecosystem(self) -> Ecosystem {
        match self {
            ManifestKind::PackageJson | ManifestKind::PnpmWorkspace => Ecosystem::Node,
            ManifestKind::CargoToml => Ecosystem::Rust,
            ManifestKind::PomXml | ManifestKind::GradleSettings => Ecosystem::Java,
            ManifestKind::PyProject | ManifestKind::Requirements => Ecosystem::Python,
            ManifestKind::Sln | ManifestKind::Csproj => Ecosystem::Dotnet,
            ManifestKind::UnityManifest | ManifestKind::UnityVersion => Ecosystem::Unity,
            ManifestKind::RojoProject => Ecosystem::Rojo,
        }
    }
}

/// Ámbito de una dependencia. `Module` es la pertenencia a un workspace, una
/// solución o un proyecto multimódulo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DepScope {
    Prod,
    Dev,
    Build,
    Peer,
    Optional,
    Module,
}

impl DepScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            DepScope::Prod => "prod",
            DepScope::Dev => "dev",
            DepScope::Build => "build",
            DepScope::Peer => "peer",
            DepScope::Optional => "optional",
            DepScope::Module => "module",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Component {
    /// Identificador estable: la ruta del manifiesto, o `ajustes#módulo` para un
    /// módulo de Gradle sin manifiesto propio.
    pub id: String,
    pub ecosystem: Ecosystem,
    pub name: String,
    /// Directorio del componente; `""` es la raíz.
    pub rel_dir: String,
    pub manifest: String,
    pub version: Option<String>,
    /// Agrupa a otros (workspace, solución, proyecto padre).
    pub is_group: bool,
    /// Rojo: servicios del árbol (`ReplicatedStorage`, `ServerScriptService`…).
    pub parts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dependency {
    /// `id` del componente que la declara.
    pub from: String,
    pub name: String,
    pub version_req: Option<String>,
    pub scope: DepScope,
    /// `id` del componente interno al que apunta, si lo hay.
    pub to: Option<String>,
}

/// Manifiesto que no se pudo leer. `reason` es un tipo de error, nunca contenido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub manifest: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProjectMap {
    pub components: Vec<Component>,
    pub deps: Vec<Dependency>,
    pub problems: Vec<Problem>,
}

// ---------------------------------------------------------------------------
// Detección
// ---------------------------------------------------------------------------

/// Tipo de manifiesto de una ruta relativa, o `None` si no es de la tabla de §9
/// o está en un directorio de dependencias de terceros.
pub fn manifest_kind(rel_path: &str) -> Option<ManifestKind> {
    let rel = slashes(rel_path);
    let segments: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let (name, dirs) = segments.split_last()?;
    if dirs
        .iter()
        .any(|d| VENDORED.iter().any(|v| d.eq_ignore_ascii_case(v)))
    {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let parent = dirs
        .last()
        .map(|d| d.to_ascii_lowercase())
        .unwrap_or_default();
    Some(match name.as_str() {
        "package.json" => ManifestKind::PackageJson,
        "pnpm-workspace.yaml" | "pnpm-workspace.yml" => ManifestKind::PnpmWorkspace,
        "cargo.toml" => ManifestKind::CargoToml,
        "pom.xml" => ManifestKind::PomXml,
        "settings.gradle" | "settings.gradle.kts" => ManifestKind::GradleSettings,
        "pyproject.toml" => ManifestKind::PyProject,
        n if n.starts_with("requirements") && n.ends_with(".txt") => ManifestKind::Requirements,
        n if n.ends_with(".sln") => ManifestKind::Sln,
        n if n.ends_with(".csproj") => ManifestKind::Csproj,
        "manifest.json" if parent == "packages" => ManifestKind::UnityManifest,
        "projectversion.txt" if parent == "projectsettings" => ManifestKind::UnityVersion,
        "default.project.json" => ManifestKind::RojoProject,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Construcción del mapa
// ---------------------------------------------------------------------------

/// Lo que dice un manifiesto antes de resolver nada.
#[derive(Debug, Default)]
struct Raw {
    ecosystem: Option<Ecosystem>,
    name: Option<String>,
    version: Option<String>,
    is_group: bool,
    /// Se suma al componente del mismo ecosistema en `attach_dir` (requirements,
    /// `ProjectVersion.txt`, pnpm) en vez de crear uno propio, si existe.
    attach_dir: Option<String>,
    /// Directorio del componente si no es el del manifiesto (Unity).
    rel_dir: Option<String>,
    deps: Vec<RawDep>,
    /// Miembros: patrones relativos al directorio del manifiesto.
    members: Vec<String>,
    /// Gradle: los miembros sin manifiesto se crean como módulos.
    synthesize_members: bool,
    parts: Vec<String>,
    /// Rust: `version.workspace = true`.
    inherits_version: bool,
    /// Rust: `[workspace.package] version` y `[workspace.dependencies]` con `path`.
    workspace_version: Option<String>,
    workspace_paths: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct RawDep {
    name: String,
    version_req: Option<String>,
    scope: DepScope,
    /// Ruta relativa al manifiesto (`path =`, `file:`, `ProjectReference`).
    path: Option<String>,
    /// Rust: `workspace = true`.
    from_workspace: bool,
    /// Node: `workspace:` (siempre interna).
    workspace_protocol: bool,
}

impl RawDep {
    fn new(name: &str, version_req: Option<String>, scope: DepScope) -> Option<Self> {
        Some(Self {
            name: clean_text(name)?,
            version_req,
            scope,
            path: None,
            from_workspace: false,
            workspace_protocol: false,
        })
    }
}

/// Construye el mapa de componentes a partir de los manifiestos `(ruta relativa,
/// texto)`. `root_name` nombra a los componentes de la raíz que no tienen nombre.
pub fn build_map(root_name: &str, manifests: &[(String, String)]) -> ProjectMap {
    let mut problems = Vec::new();
    let mut parsed: Vec<(String, ManifestKind, Raw)> = Vec::new();
    for (path, text) in manifests {
        let path = slashes(path);
        let Some(kind) = manifest_kind(&path) else {
            continue;
        };
        if text.len() > MAX_MANIFEST_BYTES {
            problems.push(Problem {
                manifest: path,
                reason: "too_large",
            });
            continue;
        }
        match parse(kind, &path, text) {
            Ok(mut raw) => {
                raw.ecosystem = Some(kind.ecosystem());
                raw.deps.truncate(MAX_DEPS_PER_MANIFEST);
                parsed.push((path, kind, raw));
            }
            Err(reason) => problems.push(Problem {
                manifest: path,
                reason,
            }),
        }
    }
    // Los que crean componente primero, y después los que se suman a otro.
    parsed.sort_by(|a, b| (a.2.attach_dir.is_some(), &a.0).cmp(&(b.2.attach_dir.is_some(), &b.0)));

    let mut map = Builder::default();
    for (path, kind, raw) in parsed {
        map.add(root_name, &path, kind, raw);
    }
    map.finish(problems)
}

#[derive(Default)]
struct Builder {
    components: BTreeMap<String, Component>,
    /// Lo que falta por resolver de cada componente.
    pending: Vec<(String, Raw)>,
}

impl Builder {
    fn add(&mut self, root_name: &str, path: &str, kind: ManifestKind, mut raw: Raw) {
        let ecosystem = kind.ecosystem();
        if let Some(dir) = raw.attach_dir.clone()
            && let Some(id) = self.find(ecosystem, |c| c.rel_dir == dir)
        {
            let component = self.components.get_mut(&id).expect("existe");
            if component.version.is_none() {
                component.version = raw.version.take();
            }
            component.is_group |= raw.is_group;
            self.pending.push((id, raw));
            return;
        }
        let rel_dir = raw
            .rel_dir
            .clone()
            .or_else(|| raw.attach_dir.clone())
            .unwrap_or_else(|| parent(path).to_owned());
        let name = raw
            .name
            .clone()
            .or_else(|| {
                file_stem(path)
                    .filter(|_| kind == ManifestKind::Csproj || kind == ManifestKind::Sln)
            })
            .unwrap_or_else(|| dir_name(&rel_dir, root_name));
        let id = path.to_owned();
        self.components.insert(
            id.clone(),
            Component {
                id: id.clone(),
                ecosystem,
                name,
                rel_dir,
                manifest: path.to_owned(),
                version: raw.version.clone(),
                is_group: raw.is_group,
                parts: std::mem::take(&mut raw.parts),
            },
        );
        self.pending.push((id, raw));
    }

    fn find(&self, ecosystem: Ecosystem, pred: impl Fn(&Component) -> bool) -> Option<String> {
        self.components
            .values()
            .find(|c| c.ecosystem == ecosystem && pred(c))
            .map(|c| c.id.clone())
    }

    fn finish(mut self, mut problems: Vec<Problem>) -> ProjectMap {
        /// `(desde, ámbito, nombre, versión, hacia)`: también da el orden de salida.
        type DepKey = (String, DepScope, String, Option<String>, Option<String>);
        let pending = std::mem::take(&mut self.pending);
        let mut deps: BTreeSet<DepKey> = BTreeSet::new();

        // Miembros de grupos: primero, porque Gradle puede crear componentes.
        for (id, raw) in &pending {
            let Some(group) = self.components.get(id).cloned() else {
                continue;
            };
            let base = parent(&group.manifest).to_owned();
            for pattern in &raw.members {
                let Some(target) = join(&base, pattern) else {
                    continue;
                };
                let mut matched: Vec<String> = self
                    .components
                    .values()
                    .filter(|c| {
                        c.ecosystem == group.ecosystem
                            && c.id != group.id
                            && (glob_match(&target, &c.rel_dir) || glob_match(&target, &c.manifest))
                    })
                    .map(|c| c.id.clone())
                    .collect();
                if matched.is_empty() && raw.synthesize_members && !target.contains('*') {
                    let module_id = format!("{}#{target}", group.manifest);
                    self.components.insert(
                        module_id.clone(),
                        Component {
                            id: module_id.clone(),
                            ecosystem: group.ecosystem,
                            name: dir_name(&target, &target),
                            rel_dir: target.clone(),
                            manifest: group.manifest.clone(),
                            version: None,
                            is_group: false,
                            parts: Vec::new(),
                        },
                    );
                    matched.push(module_id);
                }
                for member in matched {
                    let name = self.components[&member].name.clone();
                    deps.insert((group.id.clone(), DepScope::Module, name, None, Some(member)));
                }
            }
        }

        // Versión heredada del workspace de Rust: el que lo lista como miembro o, si
        // no, el más cercano por encima (como lo busca Cargo).
        let workspaces: Vec<(String, String, String)> = pending
            .iter()
            .filter_map(|(id, raw)| {
                let version = raw.workspace_version.clone()?;
                let dir = self.components.get(id)?.rel_dir.clone();
                Some((id.clone(), dir, version))
            })
            .collect();
        for (id, raw) in &pending {
            if !raw.inherits_version {
                continue;
            }
            let Some(member_dir) = self.components.get(id).map(|c| c.rel_dir.clone()) else {
                continue;
            };
            let listed = workspaces.iter().find(|(ws, _, _)| {
                deps.iter().any(|(from, scope, _, _, to)| {
                    from == ws && *scope == DepScope::Module && to.as_deref() == Some(id.as_str())
                })
            });
            let nearest = workspaces
                .iter()
                .filter(|(_, dir, _)| is_ancestor(dir, &member_dir))
                .max_by_key(|(_, dir, _)| dir.len());
            if let Some((_, _, version)) = listed.or(nearest)
                && let Some(c) = self.components.get_mut(id)
                && c.version.is_none()
            {
                c.version = Some(version.clone());
            }
        }

        let workspace_paths: HashMap<String, String> = pending
            .iter()
            .flat_map(|(id, raw)| {
                let base = parent(id).to_owned();
                raw.workspace_paths
                    .iter()
                    .filter_map(move |(name, path)| join(&base, path).map(|p| (name.clone(), p)))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Dependencias declaradas.
        for (id, raw) in &pending {
            let Some(from) = self.components.get(id).cloned() else {
                continue;
            };
            let base = parent(&from.manifest).to_owned();
            for dep in &raw.deps {
                let target = if let Some(path) = &dep.path {
                    join(&base, path)
                } else if dep.from_workspace {
                    workspace_paths.get(&dep.name).cloned()
                } else {
                    None
                };
                let to = match target {
                    Some(target) => self.find(from.ecosystem, |c| {
                        c.id != from.id && (c.rel_dir == target || c.manifest == target)
                    }),
                    None => None,
                }
                .or_else(|| {
                    // Por nombre (Maven: el `artifactId` de `grupo:artefacto`).
                    let wanted = dep.name.rsplit(':').next().unwrap_or(&dep.name);
                    self.find(from.ecosystem, |c| {
                        c.id != from.id && !c.is_group && c.name.eq_ignore_ascii_case(wanted)
                    })
                });
                deps.insert((
                    from.id.clone(),
                    dep.scope,
                    dep.name.clone(),
                    dep.version_req.clone(),
                    to,
                ));
            }
        }

        problems.sort_by(|a, b| a.manifest.cmp(&b.manifest));
        ProjectMap {
            components: self.components.into_values().collect(),
            deps: deps
                .into_iter()
                .map(|(from, scope, name, version_req, to)| Dependency {
                    from,
                    name,
                    version_req,
                    scope,
                    to,
                })
                .collect(),
            problems,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsers por manifiesto
// ---------------------------------------------------------------------------

fn parse(kind: ManifestKind, path: &str, text: &str) -> Result<Raw, &'static str> {
    match kind {
        ManifestKind::PackageJson => package_json(text),
        ManifestKind::PnpmWorkspace => Ok(pnpm_workspace(path, text)),
        ManifestKind::CargoToml => cargo_toml(text),
        ManifestKind::PomXml => pom_xml(text),
        ManifestKind::GradleSettings => Ok(gradle_settings(text)),
        ManifestKind::PyProject => pyproject(text),
        ManifestKind::Requirements => Ok(requirements(path, text)),
        ManifestKind::Sln => Ok(sln(text)),
        ManifestKind::Csproj => csproj(text),
        ManifestKind::UnityManifest => unity_manifest(path, text),
        ManifestKind::UnityVersion => Ok(unity_version(path, text)),
        ManifestKind::RojoProject => rojo(text),
    }
}

fn package_json(text: &str) -> Result<Raw, &'static str> {
    let json: Json = serde_json::from_str(text).map_err(|_| "invalid_json")?;
    let obj = json.as_object().ok_or("invalid_json")?;
    let mut raw = Raw {
        name: obj.get("name").and_then(Json::as_str).and_then(clean_text),
        version: obj
            .get("version")
            .and_then(Json::as_str)
            .and_then(clean_version),
        ..Default::default()
    };
    let workspaces = match obj.get("workspaces") {
        Some(Json::Array(list)) => list.as_slice(),
        Some(Json::Object(o)) => o
            .get("packages")
            .and_then(Json::as_array)
            .map_or(&[][..], Vec::as_slice),
        _ => &[],
    };
    raw.members = workspaces
        .iter()
        .filter_map(Json::as_str)
        .filter(|p| !p.starts_with('!'))
        .map(str::to_owned)
        .collect();
    raw.is_group = !raw.members.is_empty();
    for (section, scope) in [
        ("dependencies", DepScope::Prod),
        ("devDependencies", DepScope::Dev),
        ("peerDependencies", DepScope::Peer),
        ("optionalDependencies", DepScope::Optional),
    ] {
        let Some(table) = obj.get(section).and_then(Json::as_object) else {
            continue;
        };
        for (name, spec) in table {
            let spec = spec.as_str().unwrap_or("");
            let mut path = None;
            let mut workspace_protocol = false;
            if let Some(p) = spec
                .strip_prefix("file:")
                .or_else(|| spec.strip_prefix("link:"))
            {
                path = Some(p.to_owned());
            } else if spec.starts_with("workspace:") {
                workspace_protocol = true;
            }
            if let Some(mut dep) = RawDep::new(name, clean_version(spec), scope) {
                dep.path = path;
                dep.workspace_protocol = workspace_protocol;
                raw.deps.push(dep);
            }
        }
    }
    Ok(raw)
}

/// `pnpm-workspace.yaml`: solo la lista `packages`, sin un parser de YAML completo.
fn pnpm_workspace(path: &str, text: &str) -> Raw {
    let mut members = Vec::new();
    let mut in_packages = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) && !trimmed.starts_with('-') {
            in_packages = trimmed.trim_end_matches(':') == "packages";
            continue;
        }
        if in_packages && let Some(item) = trimmed.strip_prefix('-') {
            let item = item.trim().trim_matches(|c| c == '\'' || c == '"');
            if !item.is_empty() && !item.starts_with('!') {
                members.push(item.to_owned());
            }
        }
    }
    Raw {
        is_group: !members.is_empty(),
        members,
        attach_dir: Some(parent(path).to_owned()),
        ..Default::default()
    }
}

fn cargo_toml(text: &str) -> Result<Raw, &'static str> {
    let table: toml::Table = text.parse().map_err(|_| "invalid_toml")?;
    let mut raw = Raw::default();
    if let Some(package) = table.get("package").and_then(toml::Value::as_table) {
        raw.name = package
            .get("name")
            .and_then(toml::Value::as_str)
            .and_then(clean_text);
        match package.get("version") {
            Some(toml::Value::String(v)) => raw.version = clean_version(v),
            Some(toml::Value::Table(t)) => {
                raw.inherits_version =
                    t.get("workspace").and_then(toml::Value::as_bool) == Some(true);
            }
            _ => {}
        }
    }
    if let Some(workspace) = table.get("workspace").and_then(toml::Value::as_table) {
        raw.is_group = true;
        raw.members = workspace
            .get("members")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        raw.workspace_version = workspace
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(toml::Value::as_str)
            .and_then(clean_version);
        if let Some(wdeps) = workspace
            .get("dependencies")
            .and_then(toml::Value::as_table)
        {
            for (name, spec) in wdeps {
                if let Some(path) = spec.get("path").and_then(toml::Value::as_str) {
                    raw.workspace_paths.insert(name.clone(), path.to_owned());
                }
            }
        }
    }
    fn push_sections<'t>(t: &'t toml::Table, out: &mut Vec<(&'t toml::Table, DepScope)>) {
        for (key, scope) in [
            ("dependencies", DepScope::Prod),
            ("dev-dependencies", DepScope::Dev),
            ("build-dependencies", DepScope::Build),
        ] {
            if let Some(deps) = t.get(key).and_then(toml::Value::as_table) {
                out.push((deps, scope));
            }
        }
    }
    let mut sections: Vec<(&toml::Table, DepScope)> = Vec::new();
    push_sections(&table, &mut sections);
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for target in targets.values().filter_map(toml::Value::as_table) {
            push_sections(target, &mut sections);
        }
    }
    for (deps, scope) in sections {
        for (key, spec) in deps {
            let (version, path, from_workspace, name) = match spec {
                toml::Value::String(v) => (clean_version(v), None, false, key.as_str()),
                toml::Value::Table(t) => (
                    t.get("version")
                        .and_then(toml::Value::as_str)
                        .and_then(clean_version),
                    t.get("path")
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned),
                    t.get("workspace").and_then(toml::Value::as_bool) == Some(true),
                    t.get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(key.as_str()),
                ),
                _ => (None, None, false, key.as_str()),
            };
            if let Some(mut dep) = RawDep::new(name, version, scope) {
                dep.path = path;
                dep.from_workspace = from_workspace;
                raw.deps.push(dep);
            }
        }
    }
    Ok(raw)
}

/// Lo que ve [`walk_xml`]: `path` es la ruta de elementos abiertos, el propio
/// incluido en `Start` y `End`.
enum Xml<'e, 'a> {
    Start(&'e [String], &'e BytesStart<'a>),
    Text(&'e [String], &'e str),
    End(&'e [String]),
}

/// Recorre un XML con un solo callback.
fn walk_xml(text: &str, mut on: impl FnMut(Xml<'_, '_>)) -> Result<(), &'static str> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<String> = Vec::new();
    loop {
        match reader.read_event().map_err(|_| "invalid_xml")? {
            Event::Start(e) => {
                stack.push(e.local_name().as_ref().to_owned());
                if stack.len() > 64 {
                    return Err("invalid_xml");
                }
                on(Xml::Start(&stack, &e));
            }
            Event::Empty(e) => {
                stack.push(e.local_name().as_ref().to_owned());
                on(Xml::Start(&stack, &e));
                on(Xml::End(&stack));
                stack.pop();
            }
            Event::End(_) => {
                on(Xml::End(&stack));
                stack.pop();
            }
            Event::Text(t) => on(Xml::Text(&stack, &t.xml10_content())),
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn path_is(stack: &[String], path: &[&str]) -> bool {
    stack.len() == path.len() && stack.iter().zip(path).all(|(a, b)| a == b)
}

fn pom_xml(text: &str) -> Result<Raw, &'static str> {
    let mut raw = Raw::default();
    let mut parent_version = None;
    let mut packaging_pom = false;
    let mut current: HashMap<&'static str, String> = HashMap::new();
    walk_xml(text, |event| match event {
        Xml::Text(stack, value) => {
            if path_is(stack, &["project", "artifactId"]) {
                raw.name = clean_text(value);
            } else if path_is(stack, &["project", "version"]) {
                raw.version = clean_version(value);
            } else if path_is(stack, &["project", "parent", "version"]) {
                parent_version = clean_version(value);
            } else if path_is(stack, &["project", "packaging"]) {
                packaging_pom = value.trim() == "pom";
            } else if path_is(stack, &["project", "modules", "module"]) {
                raw.members.push(value.trim().to_owned());
            } else if stack.len() == 4
                && path_is(&stack[..3], &["project", "dependencies", "dependency"])
            {
                let key = match stack[3].as_str() {
                    "groupId" => "group",
                    "artifactId" => "artifact",
                    "version" => "version",
                    "scope" => "scope",
                    _ => return,
                };
                current.insert(key, value.trim().to_owned());
            }
        }
        Xml::End(stack) if path_is(stack, &["project", "dependencies", "dependency"]) => {
            let fields = std::mem::take(&mut current);
            if let Some(artifact) = fields.get("artifact") {
                let name = match fields.get("group") {
                    Some(g) => format!("{g}:{artifact}"),
                    None => artifact.clone(),
                };
                let scope = match fields.get("scope").map(String::as_str) {
                    Some("test") => DepScope::Dev,
                    Some("provided") => DepScope::Build,
                    _ => DepScope::Prod,
                };
                let version = fields.get("version").and_then(|v| clean_version(v));
                raw.deps.extend(RawDep::new(&name, version, scope));
            }
        }
        _ => {}
    })?;
    raw.version = raw.version.or(parent_version);
    raw.is_group = packaging_pom || !raw.members.is_empty();
    Ok(raw)
}

/// `settings.gradle(.kts)`: `rootProject.name` y los `include`, sin evaluar el script.
fn gradle_settings(text: &str) -> Raw {
    let mut raw = Raw {
        is_group: true,
        synthesize_members: true,
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.starts_with("rootProject.name") {
            raw.name = quoted(line).into_iter().next().and_then(|s| clean_text(&s));
        } else if line.starts_with("include") {
            for module in quoted(line) {
                let path = module.trim_start_matches(':').replace(':', "/");
                if !path.is_empty() {
                    raw.members.push(path);
                }
            }
        }
    }
    raw
}

fn pyproject(text: &str) -> Result<Raw, &'static str> {
    let table: toml::Table = text.parse().map_err(|_| "invalid_toml")?;
    let mut raw = Raw::default();
    let pep508 = |raw: &mut Raw, spec: &str, scope| {
        if let Some((name, version)) = requirement(spec) {
            raw.deps.extend(RawDep::new(&name, version, scope));
        }
    };
    if let Some(project) = table.get("project").and_then(toml::Value::as_table) {
        raw.name = project
            .get("name")
            .and_then(toml::Value::as_str)
            .and_then(clean_text);
        raw.version = project
            .get("version")
            .and_then(toml::Value::as_str)
            .and_then(clean_version);
        for spec in project
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
        {
            pep508(&mut raw, spec, DepScope::Prod);
        }
        for group in project
            .get("optional-dependencies")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|t| t.values())
        {
            for spec in group
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
            {
                pep508(&mut raw, spec, DepScope::Optional);
            }
        }
    }
    if let Some(groups) = table
        .get("dependency-groups")
        .and_then(toml::Value::as_table)
    {
        for group in groups.values() {
            for spec in group
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
            {
                pep508(&mut raw, spec, DepScope::Dev);
            }
        }
    }
    if let Some(poetry) = table
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(toml::Value::as_table)
    {
        if raw.name.is_none() {
            raw.name = poetry
                .get("name")
                .and_then(toml::Value::as_str)
                .and_then(clean_text);
        }
        if raw.version.is_none() {
            raw.version = poetry
                .get("version")
                .and_then(toml::Value::as_str)
                .and_then(clean_version);
        }
        let mut poetry_deps = |t: Option<&toml::Value>, scope| {
            for (name, spec) in t.and_then(toml::Value::as_table).into_iter().flatten() {
                if name == "python" {
                    continue;
                }
                let version = match spec {
                    toml::Value::String(v) => clean_version(v),
                    toml::Value::Table(t) => t
                        .get("version")
                        .and_then(toml::Value::as_str)
                        .and_then(clean_version),
                    _ => None,
                };
                raw.deps.extend(RawDep::new(name, version, scope));
            }
        };
        poetry_deps(poetry.get("dependencies"), DepScope::Prod);
        poetry_deps(poetry.get("dev-dependencies"), DepScope::Dev);
        for group in poetry
            .get("group")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|t| t.values())
        {
            poetry_deps(group.get("dependencies"), DepScope::Dev);
        }
    }
    Ok(raw)
}

fn requirements(path: &str, text: &str) -> Raw {
    let file = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let scope = if file.contains("dev") || file.contains("test") {
        DepScope::Dev
    } else {
        DepScope::Prod
    };
    let mut raw = Raw {
        attach_dir: Some(parent(path).to_owned()),
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('-') || line.contains("://") {
            continue;
        }
        if let Some((name, version)) = requirement(line) {
            raw.deps.extend(RawDep::new(&name, version, scope));
        }
    }
    raw
}

/// Nombre y versión pedida de un requisito PEP 508 (`requests[socks]>=2; python_version<"3.8"`).
fn requirement(spec: &str) -> Option<(String, Option<String>)> {
    let spec = spec.split(';').next()?.trim();
    let end = spec
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
        .unwrap_or(spec.len());
    let name = &spec[..end];
    if name.is_empty() {
        return None;
    }
    let mut rest = spec[end..].trim();
    if rest.starts_with('[') {
        rest = rest.split_once(']').map_or("", |(_, r)| r).trim();
    }
    let version = if rest.starts_with('@') {
        Some("url".to_owned())
    } else {
        clean_version(rest)
    };
    Some((name.to_owned(), version))
}

fn sln(text: &str) -> Raw {
    let mut raw = Raw {
        is_group: true,
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("Project(") {
            continue;
        }
        let fields = quoted(line);
        // Project("{tipo}") = "Nombre", "ruta\Nombre.csproj", "{guid}"
        if let Some(path) = fields.get(2)
            && path.to_ascii_lowercase().ends_with("proj")
        {
            raw.members.push(path.replace('\\', "/"));
        }
    }
    raw
}

fn csproj(text: &str) -> Result<Raw, &'static str> {
    fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
        e.attributes()
            .flatten()
            .find(|a| a.key.local_name().as_ref().eq_ignore_ascii_case(key))
            .and_then(|a| {
                a.normalized_value(XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
    }
    /// Elemento y su padre: `(padre, elemento)`.
    fn last_two(stack: &[String]) -> (&str, &str) {
        match stack {
            [.., p, e] => (p.as_str(), e.as_str()),
            [e] => ("", e.as_str()),
            [] => ("", ""),
        }
    }
    let mut raw = Raw::default();
    let mut pending_package: Option<(String, Option<String>)> = None;
    walk_xml(text, |event| match event {
        Xml::Start(stack, e) => match last_two(stack).1 {
            "PackageReference" => {
                if let Some(include) = attr(e, "Include") {
                    pending_package = Some((include, attr(e, "Version")));
                }
            }
            "ProjectReference" => {
                if let Some(include) = attr(e, "Include") {
                    let include = include.replace('\\', "/");
                    let name = file_stem(&include).unwrap_or_else(|| include.clone());
                    if let Some(mut dep) = RawDep::new(&name, None, DepScope::Prod) {
                        dep.path = Some(include);
                        raw.deps.push(dep);
                    }
                }
            }
            _ => {}
        },
        Xml::Text(stack, value) => match last_two(stack) {
            ("PackageReference", "Version") => {
                if let Some((_, v)) = pending_package.as_mut() {
                    *v = Some(value.trim().to_owned());
                }
            }
            ("PropertyGroup", "Version") => raw.version = clean_version(value),
            ("PropertyGroup", "AssemblyName") => raw.name = clean_text(value),
            _ => {}
        },
        Xml::End(stack) => {
            if last_two(stack).1 == "PackageReference"
                && let Some((include, v)) = pending_package.take()
            {
                let version = v.as_deref().and_then(clean_version);
                raw.deps
                    .extend(RawDep::new(&include, version, DepScope::Prod));
            }
        }
    })?;
    Ok(raw)
}

fn unity_manifest(path: &str, text: &str) -> Result<Raw, &'static str> {
    let json: Json = serde_json::from_str(text).map_err(|_| "invalid_json")?;
    let packages_dir = parent(path);
    let project_dir = parent(packages_dir).to_owned();
    let mut raw = Raw {
        rel_dir: Some(project_dir),
        ..Default::default()
    };
    for (name, spec) in json
        .get("dependencies")
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        let spec = spec.as_str().unwrap_or("");
        let mut path_dep = None;
        if let Some(p) = spec.strip_prefix("file:") {
            // Relativa a `Packages/`; el builder la resuelve desde el manifiesto.
            path_dep = Some(p.to_owned());
        }
        if let Some(mut dep) = RawDep::new(name, clean_version(spec), DepScope::Prod) {
            dep.path = path_dep;
            raw.deps.push(dep);
        }
    }
    Ok(raw)
}

fn unity_version(path: &str, text: &str) -> Raw {
    let project_dir = parent(parent(path)).to_owned();
    let version = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("m_EditorVersion:"))
        .and_then(|v| clean_version(v.trim()));
    Raw {
        version,
        attach_dir: Some(project_dir.clone()),
        rel_dir: Some(project_dir),
        ..Default::default()
    }
}

fn rojo(text: &str) -> Result<Raw, &'static str> {
    let json: Json = serde_json::from_str(text).map_err(|_| "invalid_json")?;
    let parts = json
        .get("tree")
        .and_then(Json::as_object)
        .map(|tree| {
            tree.keys()
                .filter(|k| !k.starts_with('$'))
                .filter_map(|k| clean_text(k))
                .take(MAX_PARTS)
                .collect()
        })
        .unwrap_or_default();
    Ok(Raw {
        name: json.get("name").and_then(Json::as_str).and_then(clean_text),
        parts,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Estructura del árbol
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LanguageCount {
    pub language: &'static str,
    pub files: u64,
}

/// Un directorio con lo que hay debajo de él, subdirectorios incluidos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirSummary {
    /// `""` es la raíz.
    pub rel_dir: String,
    pub depth: u32,
    pub files: u64,
    pub bytes: u64,
    /// De más a menos ficheros.
    pub languages: Vec<LanguageCount>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TreeSummary {
    /// Ordenados por ruta; el primero es la raíz.
    pub dirs: Vec<DirSummary>,
    pub files: u64,
    pub bytes: u64,
    /// Había más de [`MAX_ENTRIES`] ficheros y el resto no se contó.
    pub truncated: bool,
}

/// Resume una lista de ficheros `(ruta relativa, bytes)`. Solo cuenta: no lee nada.
pub fn summarize_tree<'a>(entries: impl IntoIterator<Item = (&'a str, u64)>) -> TreeSummary {
    #[derive(Default)]
    struct Acc {
        files: u64,
        bytes: u64,
        languages: HashMap<&'static str, u64>,
    }
    let mut dirs: BTreeMap<String, Acc> = BTreeMap::new();
    let mut summary = TreeSummary::default();
    for (count, (path, size)) in entries.into_iter().enumerate() {
        if count >= MAX_ENTRIES {
            summary.truncated = true;
            break;
        }
        let path = slashes(path);
        let language = language_of(&path);
        summary.files += 1;
        summary.bytes += size;
        let mut dir = parent(&path).to_owned();
        loop {
            let acc = dirs.entry(dir.clone()).or_default();
            acc.files += 1;
            acc.bytes += size;
            if let Some(language) = language {
                *acc.languages.entry(language).or_default() += 1;
            }
            if dir.is_empty() {
                break;
            }
            dir = parent(&dir).to_owned();
        }
    }
    summary.dirs = dirs
        .into_iter()
        .map(|(rel_dir, acc)| {
            let mut languages: Vec<LanguageCount> = acc
                .languages
                .into_iter()
                .map(|(language, files)| LanguageCount { language, files })
                .collect();
            languages.sort_by(|a, b| b.files.cmp(&a.files).then(a.language.cmp(b.language)));
            DirSummary {
                depth: if rel_dir.is_empty() {
                    0
                } else {
                    rel_dir.matches('/').count() as u32 + 1
                },
                rel_dir,
                files: acc.files,
                bytes: acc.bytes,
                languages,
            }
        })
        .collect();
    summary
}

/// Lenguaje de un fichero según su extensión.
pub fn language_of(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next()?;
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" | "pyi" => "Python",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "cs" => "C#",
        "lua" | "luau" => "Luau",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "C++",
        "go" => "Go",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "dart" => "Dart",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" | "sass" => "SCSS",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "sql" => "SQL",
        "sh" | "bash" => "Shell",
        "ps1" | "psm1" => "PowerShell",
        "json" | "jsonl" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "xml" | "csproj" | "sln" => "XML",
        "md" | "mdx" => "Markdown",
        "gradle" => "Gradle",
        "unity" | "prefab" | "asset" | "mat" => "Unity",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Utilidades de rutas y texto
// ---------------------------------------------------------------------------

fn slashes(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_owned()
}

/// Directorio padre (`""` para la raíz).
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn file_stem(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    clean_text(stem)
}

fn dir_name(rel_dir: &str, root_name: &str) -> String {
    let name = rel_dir.rsplit('/').next().unwrap_or("");
    clean_text(if name.is_empty() { root_name } else { name })
        .unwrap_or_else(|| "proyecto".to_owned())
}

/// `dir` es `child` o un directorio por encima de él (`""` lo es de todo).
fn is_ancestor(dir: &str, child: &str) -> bool {
    dir.is_empty() || child == dir || child.starts_with(&format!("{dir}/"))
}

/// `base` + `rel`, normalizada. `None` si es absoluta o sale de la raíz.
pub fn join(base: &str, rel: &str) -> Option<String> {
    let rel = rel.replace('\\', "/");
    if rel.starts_with('/') || rel.contains(':') {
        return None;
    }
    let mut out: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for segment in rel.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    Some(out.join("/"))
}

/// `*` dentro de un segmento y `**` para cualquier número de segmentos. Sin
/// distinguir mayúsculas, como el sistema de ficheros de Windows.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    fn segments(s: &str) -> Vec<&str> {
        s.split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect()
    }
    fn go(p: &[&str], s: &[&str]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((&"**", rest)) => (0..=s.len()).any(|i| go(rest, &s[i..])),
            Some((seg, rest)) => !s.is_empty() && seg_match(seg, s[0]) && go(rest, &s[1..]),
        }
    }
    fn seg_match(p: &str, s: &str) -> bool {
        let p: Vec<char> = p.to_lowercase().chars().collect();
        let s: Vec<char> = s.to_lowercase().chars().collect();
        fn m(p: &[char], s: &[char]) -> bool {
            match p.split_first() {
                None => s.is_empty(),
                Some(('*', rest)) => (0..=s.len()).any(|i| m(rest, &s[i..])),
                Some((c, rest)) => s.first() == Some(c) && m(rest, &s[1..]),
            }
        }
        m(&p, &s)
    }
    go(&segments(pattern), &segments(path))
}

/// Cadenas entre comillas simples o dobles de una línea, en orden.
fn quoted(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let s: String = chars.by_ref().take_while(|&d| d != c).collect();
            out.push(s);
        }
    }
    out
}

/// Nombre: una línea, como mucho 200 caracteres, sin vacíos.
fn clean_text(s: &str) -> Option<String> {
    let line: String = s
        .split(|c: char| c.is_control())
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_TEXT)
        .collect();
    (!line.is_empty()).then_some(line)
}

/// Versión pedida. Las rutas no se guardan (pueden ser absolutas) y las URL se
/// reducen a `url` (pueden llevar un token).
fn clean_version(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty()
        || s.starts_with("file:")
        || s.starts_with("link:")
        || s.starts_with("portal:")
        || s.starts_with('.')
        || s.starts_with('/')
    {
        return None;
    }
    if s.contains("://")
        || s.starts_with("git")
        || s.starts_with("github:")
        || (s.contains('@') && s.contains(':'))
    {
        return Some("url".to_owned());
    }
    clean_text(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(files: &[(&str, &str)]) -> ProjectMap {
        let manifests: Vec<(String, String)> = files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect();
        build_map("raiz", &manifests)
    }

    fn component<'a>(map: &'a ProjectMap, id: &str) -> &'a Component {
        map.components
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("falta {id}: {:#?}", map.components))
    }

    fn deps_of<'a>(map: &'a ProjectMap, from: &str) -> Vec<&'a Dependency> {
        map.deps.iter().filter(|d| d.from == from).collect()
    }

    fn edge(map: &ProjectMap, from: &str, scope: DepScope) -> Vec<(String, Option<String>)> {
        deps_of(map, from)
            .into_iter()
            .filter(|d| d.scope == scope)
            .map(|d| (d.name.clone(), d.to.clone()))
            .collect()
    }

    #[test]
    fn manifests_are_recognised_and_vendored_ones_skipped() {
        assert_eq!(
            manifest_kind("package.json"),
            Some(ManifestKind::PackageJson)
        );
        assert_eq!(
            manifest_kind("crates\\core\\Cargo.toml"),
            Some(ManifestKind::CargoToml)
        );
        assert_eq!(
            manifest_kind("app/requirements-dev.txt"),
            Some(ManifestKind::Requirements)
        );
        assert_eq!(
            manifest_kind("Game/Packages/manifest.json"),
            Some(ManifestKind::UnityManifest)
        );
        assert_eq!(manifest_kind("manifest.json"), None);
        assert_eq!(
            manifest_kind("Game/ProjectSettings/ProjectVersion.txt"),
            Some(ManifestKind::UnityVersion)
        );
        assert_eq!(
            manifest_kind("src/Api/Api.csproj"),
            Some(ManifestKind::Csproj)
        );
        assert_eq!(manifest_kind("node_modules/react/package.json"), None);
        assert_eq!(manifest_kind("target/debug/build/x/Cargo.toml"), None);
        assert_eq!(manifest_kind("src/main.rs"), None);
    }

    #[test]
    fn cargo_workspace_like_this_repository() {
        let m = map(&[
            (
                "Cargo.toml",
                r#"[workspace]
                members = ["crates/*", "src-tauri"]
                [workspace.package]
                version = "0.1.0"
                [workspace.dependencies]
                cockpit-core = { path = "crates/cockpit-core" }
                serde = { version = "1", features = ["derive"] }"#,
            ),
            (
                "crates/cockpit-core/Cargo.toml",
                r#"[package]
                name = "cockpit-core"
                version.workspace = true
                [dependencies]
                serde.workspace = true
                [dev-dependencies]
                criterion = "0.8""#,
            ),
            (
                "crates/cockpit-store/Cargo.toml",
                r#"[package]
                name = "cockpit-store"
                version.workspace = true
                [dependencies]
                cockpit-core.workspace = true
                rusqlite = { version = "0.41", features = ["bundled"] }"#,
            ),
            (
                "src-tauri/Cargo.toml",
                r#"[package]
                name = "dev-cockpit"
                version = "0.1.0"
                [dependencies]
                cockpit-store = { path = "../crates/cockpit-store" }
                [target.'cfg(windows)'.dependencies]
                webview2-com = "0.39"
                [build-dependencies]
                tauri-build = "2""#,
            ),
        ]);
        let root = component(&m, "Cargo.toml");
        assert!(root.is_group);
        assert_eq!(root.name, "raiz");
        let mut members = edge(&m, "Cargo.toml", DepScope::Module);
        members.sort();
        assert_eq!(
            members,
            [
                (
                    "cockpit-core".to_owned(),
                    Some("crates/cockpit-core/Cargo.toml".to_owned())
                ),
                (
                    "cockpit-store".to_owned(),
                    Some("crates/cockpit-store/Cargo.toml".to_owned())
                ),
                (
                    "dev-cockpit".to_owned(),
                    Some("src-tauri/Cargo.toml".to_owned())
                ),
            ]
        );
        // Versión heredada del workspace.
        assert_eq!(
            component(&m, "crates/cockpit-core/Cargo.toml")
                .version
                .as_deref(),
            Some("0.1.0")
        );
        // `workspace = true` con `path` en el workspace y `path =` directo: internas.
        assert_eq!(
            edge(&m, "crates/cockpit-store/Cargo.toml", DepScope::Prod),
            [
                (
                    "cockpit-core".to_owned(),
                    Some("crates/cockpit-core/Cargo.toml".to_owned())
                ),
                ("rusqlite".to_owned(), None),
            ]
        );
        let tauri = deps_of(&m, "src-tauri/Cargo.toml");
        assert!(tauri.iter().any(|d| d.name == "cockpit-store"
            && d.to.as_deref() == Some("crates/cockpit-store/Cargo.toml")));
        assert!(
            tauri
                .iter()
                .any(|d| d.name == "webview2-com" && d.scope == DepScope::Prod)
        );
        assert!(
            tauri
                .iter()
                .any(|d| d.name == "tauri-build" && d.scope == DepScope::Build)
        );
        assert!(
            edge(&m, "crates/cockpit-core/Cargo.toml", DepScope::Dev)
                .contains(&("criterion".to_owned(), None))
        );
    }

    #[test]
    fn node_workspaces_resolve_internal_packages_without_storing_paths() {
        let m = map(&[
            (
                "package.json",
                r#"{"name":"mono","private":true,"workspaces":["packages/*"],"devDependencies":{"typescript":"^5.6"},"scripts":{"build":"SECRETO"}}"#,
            ),
            (
                "packages/ui/package.json",
                r#"{"name":"@mono/ui","version":"1.2.0","dependencies":{"react":"^19.0.0","@mono/core":"workspace:*"},"peerDependencies":{"react-dom":"^19"}}"#,
            ),
            (
                "packages/core/package.json",
                r#"{"name":"@mono/core","version":"1.0.0","dependencies":{"local":"file:C:\\Users\\alguien\\x","priv":"git+https://token@github.com/a/b.git"}}"#,
            ),
        ]);
        assert!(component(&m, "package.json").is_group);
        assert_eq!(edge(&m, "package.json", DepScope::Module).len(), 2);
        let ui = deps_of(&m, "packages/ui/package.json");
        assert!(ui.iter().any(
            |d| d.name == "@mono/core" && d.to.as_deref() == Some("packages/core/package.json")
        ));
        assert!(
            ui.iter()
                .any(|d| d.name == "react-dom" && d.scope == DepScope::Peer)
        );
        let core = deps_of(&m, "packages/core/package.json");
        let local = core.iter().find(|d| d.name == "local").unwrap();
        assert_eq!(local.version_req, None, "una ruta no se guarda");
        assert_eq!(
            core.iter()
                .find(|d| d.name == "priv")
                .unwrap()
                .version_req
                .as_deref(),
            Some("url")
        );
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("SECRETO") && !json.contains("alguien") && !json.contains("token"),
            "{json}"
        );
    }

    #[test]
    fn pnpm_workspace_is_added_to_the_root_package() {
        let m = map(&[
            ("package.json", r#"{"name":"root"}"#),
            (
                "pnpm-workspace.yaml",
                "packages:\n  - 'apps/*'\n  - \"!apps/old\"\ncatalog:\n  react: ^19\n",
            ),
            ("apps/web/package.json", r#"{"name":"web"}"#),
        ]);
        assert_eq!(m.components.len(), 2);
        assert!(component(&m, "package.json").is_group);
        assert_eq!(
            edge(&m, "package.json", DepScope::Module),
            [("web".to_owned(), Some("apps/web/package.json".to_owned()))]
        );
    }

    #[test]
    fn maven_parent_with_modules_and_scopes() {
        let m = map(&[
            (
                "pom.xml",
                "<project><groupId>com.benchify</groupId><artifactId>benchify</artifactId><version>1.0</version><packaging>pom</packaging><modules><module>backend</module></modules></project>",
            ),
            (
                "backend/pom.xml",
                "<project><parent><version>1.0</version></parent><artifactId>backend</artifactId><dependencies><dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-web</artifactId></dependency><dependency><groupId>org.junit</groupId><artifactId>junit</artifactId><version>5.10</version><scope>test</scope></dependency></dependencies><build><plugins><plugin><artifactId>no-es-dependencia</artifactId></plugin></plugins></build></project>",
            ),
        ]);
        assert!(component(&m, "pom.xml").is_group);
        assert_eq!(
            edge(&m, "pom.xml", DepScope::Module),
            [("backend".to_owned(), Some("backend/pom.xml".to_owned()))]
        );
        let backend = component(&m, "backend/pom.xml");
        assert_eq!(backend.version.as_deref(), Some("1.0"));
        let deps = deps_of(&m, "backend/pom.xml");
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "org.junit:junit"
            && d.scope == DepScope::Dev
            && d.version_req.as_deref() == Some("5.10")));
    }

    #[test]
    fn gradle_modules_without_manifest_are_created() {
        let m = map(&[(
            "settings.gradle.kts",
            "rootProject.name = \"agenda\"\ninclude(\":app\", \":core:model\") // comentario\n",
        )]);
        let root = component(&m, "settings.gradle.kts");
        assert_eq!(root.name, "agenda");
        let mut modules = edge(&m, "settings.gradle.kts", DepScope::Module);
        modules.sort();
        assert_eq!(
            modules,
            [
                ("app".to_owned(), Some("settings.gradle.kts#app".to_owned())),
                (
                    "model".to_owned(),
                    Some("settings.gradle.kts#core/model".to_owned())
                ),
            ]
        );
    }

    #[test]
    fn python_project_and_requirements_join() {
        let m = map(&[
            (
                "pyproject.toml",
                "[project]\nname = \"herramienta\"\nversion = \"0.3\"\ndependencies = [\"requests[socks]>=2.31; python_version>'3.8'\", \"pkg @ https://x/y.whl\"]\n[project.optional-dependencies]\nplot = [\"matplotlib\"]\n",
            ),
            (
                "requirements-dev.txt",
                "# comentario\npytest==8.3\n-r requirements.txt\n-e .\n",
            ),
            ("scripts/requirements.txt", "numpy>=2\n"),
        ]);
        assert_eq!(m.components.len(), 2, "{:#?}", m.components);
        let deps = deps_of(&m, "pyproject.toml");
        assert!(
            deps.iter()
                .any(|d| d.name == "requests" && d.version_req.as_deref() == Some(">=2.31"))
        );
        assert!(
            deps.iter()
                .any(|d| d.name == "pkg" && d.version_req.as_deref() == Some("url"))
        );
        assert!(
            deps.iter()
                .any(|d| d.name == "matplotlib" && d.scope == DepScope::Optional)
        );
        assert!(
            deps.iter()
                .any(|d| d.name == "pytest" && d.scope == DepScope::Dev)
        );
        assert_eq!(component(&m, "scripts/requirements.txt").name, "scripts");
    }

    #[test]
    fn dotnet_solution_projects_and_references() {
        let m = map(&[
            (
                "App.sln",
                "Microsoft Visual Studio Solution File\nProject(\"{FAE04EC0}\") = \"Api\", \"src\\Api\\Api.csproj\", \"{1}\"\nProject(\"{2150E333}\") = \"Docs\", \"Docs\", \"{2}\"\n",
            ),
            (
                "src/Api/Api.csproj",
                "<Project Sdk=\"Microsoft.NET.Sdk.Web\"><PropertyGroup><Version>2.1.0</Version></PropertyGroup><ItemGroup><PackageReference Include=\"Serilog\" Version=\"4.0\" /><PackageReference Include=\"Dapper\"><Version>2.1</Version></PackageReference><ProjectReference Include=\"..\\Domain\\Domain.csproj\" /></ItemGroup></Project>",
            ),
            (
                "src/Domain/Domain.csproj",
                "<Project Sdk=\"Microsoft.NET.Sdk\"/>",
            ),
        ]);
        assert_eq!(
            edge(&m, "App.sln", DepScope::Module),
            [("Api".to_owned(), Some("src/Api/Api.csproj".to_owned()))]
        );
        let api = component(&m, "src/Api/Api.csproj");
        assert_eq!(
            (api.name.as_str(), api.version.as_deref()),
            ("Api", Some("2.1.0"))
        );
        let mut deps = edge(&m, "src/Api/Api.csproj", DepScope::Prod);
        deps.sort();
        assert_eq!(
            deps,
            [
                ("Dapper".to_owned(), None),
                (
                    "Domain".to_owned(),
                    Some("src/Domain/Domain.csproj".to_owned())
                ),
                ("Serilog".to_owned(), None),
            ]
        );
        assert!(
            deps_of(&m, "src/Api/Api.csproj")
                .iter()
                .any(|d| d.name == "Dapper" && d.version_req.as_deref() == Some("2.1"))
        );
    }

    #[test]
    fn unity_project_packages_and_editor_version() {
        let m = map(&[
            (
                "Game/Packages/manifest.json",
                r#"{"dependencies":{"com.unity.inputsystem":"1.11.2","com.studio.tools":"file:../LocalPackages/tools"}}"#,
            ),
            (
                "Game/ProjectSettings/ProjectVersion.txt",
                "m_EditorVersion: 6000.0.23f1\nm_EditorVersionWithRevision: x\n",
            ),
        ]);
        assert_eq!(m.components.len(), 1);
        let game = component(&m, "Game/Packages/manifest.json");
        assert_eq!(
            (
                game.name.as_str(),
                game.rel_dir.as_str(),
                game.version.as_deref()
            ),
            ("Game", "Game", Some("6000.0.23f1"))
        );
        let deps = deps_of(&m, "Game/Packages/manifest.json");
        assert!(deps.iter().any(
            |d| d.name == "com.unity.inputsystem" && d.version_req.as_deref() == Some("1.11.2")
        ));
        assert!(
            deps.iter()
                .any(|d| d.name == "com.studio.tools" && d.version_req.is_none())
        );
    }

    #[test]
    fn rojo_services_are_parts() {
        let m = map(&[(
            "default.project.json",
            r#"{"name":"StealAGalaxy","tree":{"$className":"DataModel","ReplicatedStorage":{"$path":"src/shared"},"ServerScriptService":{"$path":"src/server"}}}"#,
        )]);
        let c = component(&m, "default.project.json");
        assert_eq!(c.name, "StealAGalaxy");
        assert_eq!(c.parts, ["ReplicatedStorage", "ServerScriptService"]);
    }

    #[test]
    fn broken_manifests_are_problems_without_content() {
        let m = map(&[
            ("package.json", "{ SECRETO"),
            ("Cargo.toml", "[package\nname = SECRETO"),
            ("pom.xml", "<project><a></b></project>"),
        ]);
        assert!(m.components.is_empty());
        let reasons: Vec<_> = m
            .problems
            .iter()
            .map(|p| (p.manifest.as_str(), p.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                ("Cargo.toml", "invalid_toml"),
                ("package.json", "invalid_json"),
                ("pom.xml", "invalid_xml")
            ]
        );
        assert!(!serde_json::to_string(&m).unwrap().contains("SECRETO"));
        let big = "x".repeat(MAX_MANIFEST_BYTES + 1);
        assert_eq!(
            build_map("r", &[("package.json".to_owned(), big)]).problems[0].reason,
            "too_large"
        );
    }

    #[test]
    fn tree_summary_counts_nested_files_bytes_and_languages() {
        let s = summarize_tree([
            ("src/main.rs", 100),
            ("src/live/store.ts", 50),
            ("src/live/types.ts", 30),
            ("README.md", 10),
            ("LICENSE", 5),
        ]);
        assert_eq!((s.files, s.bytes, s.truncated), (5, 195, false));
        let dirs: Vec<_> = s
            .dirs
            .iter()
            .map(|d| (d.rel_dir.as_str(), d.depth, d.files, d.bytes))
            .collect();
        assert_eq!(
            dirs,
            [("", 0, 5, 195), ("src", 1, 3, 180), ("src/live", 2, 2, 80)]
        );
        assert_eq!(
            s.dirs[1].languages,
            [
                LanguageCount {
                    language: "TypeScript",
                    files: 2
                },
                LanguageCount {
                    language: "Rust",
                    files: 1
                },
            ]
        );
    }

    #[test]
    fn tree_summary_stops_at_the_cap() {
        let names: Vec<String> = (0..MAX_ENTRIES + 3).map(|i| format!("f{i}.rs")).collect();
        let s = summarize_tree(names.iter().map(|n| (n.as_str(), 1)));
        assert!(s.truncated);
        assert_eq!(s.files, MAX_ENTRIES as u64);
    }

    #[test]
    fn paths_never_leave_the_root() {
        assert_eq!(join("a/b", "../c").as_deref(), Some("a/c"));
        assert_eq!(join("", "./x/./y").as_deref(), Some("x/y"));
        assert_eq!(join("a", "../../etc"), None);
        assert_eq!(join("a", "C:/Users/x"), None);
        assert_eq!(join("a", "/abs"), None);
        assert!(glob_match("crates/*", "crates/cockpit-core"));
        assert!(!glob_match("crates/*", "crates/a/b"));
        assert!(glob_match("apps/**", "apps/a/b"));
        assert!(glob_match("packages/pkg-*", "Packages/PKG-ui"));
        assert!(glob_match("src-tauri", "src-tauri"));
    }
}
