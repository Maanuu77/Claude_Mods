//! Ajustes de privacidad y retención (SPEC §10), en `settings.json`, junto a la
//! base de datos (`%APPDATA%\dev-cockpit\`). Es el único fichero de ajustes de la
//! app y nunca está dentro de `~/.claude`.
//!
//! Se aplican al arrancar: la canalización y la política que reciben los mods en
//! `/v1/hello` se fijan entonces. Al reiniciar la app, cada mod vuelve a hacer
//! `hello` y adopta la política nueva (SPEC §5.4).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use cockpit_core::PATH_DETAIL_DAYS;
use cockpit_core::privacy::PrivacyPolicy;
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    /// Títulos de tarea, truncados a 120 caracteres (D1).
    pub capture_task_titles: bool,
    /// Nombre del ejecutable de Bash/PowerShell (D2).
    pub capture_shell_program: bool,
    /// `rel_path`, `cwd` y `host`. Sin ellas tampoco hay atribución por ruta.
    pub record_paths: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        let p = PrivacyPolicy::APP_DEFAULT;
        Self {
            capture_task_titles: p.capture_task_titles,
            capture_shell_program: p.capture_shell_program,
            record_paths: p.record_paths,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    /// Días de detalle de rutas (§6.4.1): de 1 a 30 (D3 fija el máximo).
    pub path_detail_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            path_detail_days: PATH_DETAIL_DAYS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Adapters {
    /// A3: `update_plan` del Desktop, con esquema no publicado (§8).
    pub desktop_plan: bool,
}

impl Default for Adapters {
    fn default() -> Self {
        Self { desktop_plan: true }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub privacy: Privacy,
    pub retention: Retention,
    pub adapters: Adapters,
}

impl Settings {
    pub fn policy(&self) -> PrivacyPolicy {
        PrivacyPolicy {
            capture_task_titles: self.privacy.capture_task_titles,
            capture_shell_program: self.privacy.capture_shell_program,
            record_paths: self.privacy.record_paths,
        }
    }

    /// Valores fuera de rango, a su límite.
    pub fn clamped(mut self) -> Self {
        self.retention.path_detail_days =
            self.retention.path_detail_days.clamp(1, PATH_DETAIL_DAYS);
        self
    }
}

pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

/// Los ajustes guardados, o los de por defecto si no hay fichero o no se entiende.
pub fn load(dir: &Path) -> Settings {
    fs::read(path_in(dir))
        .ok()
        .and_then(|raw| serde_json::from_slice::<Settings>(&raw).ok())
        .unwrap_or_default()
        .clamped()
}

/// Guarda los ajustes de forma atómica (fichero temporal y renombrado).
pub fn save(dir: &Path, settings: &Settings) -> io::Result<Settings> {
    let settings = settings.clamped();
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
    let json = serde_json::to_vec_pretty(&settings).map_err(io::Error::other)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path_in(dir))?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_spec_defaults() {
        let s = Settings::default();
        assert_eq!(s.policy(), PrivacyPolicy::APP_DEFAULT);
        assert_eq!(s.retention.path_detail_days, 30);
        assert!(s.adapters.desktop_plan);
    }

    #[test]
    fn missing_or_broken_files_fall_back_to_defaults_and_days_are_clamped() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        fs::write(path_in(dir.path()), "{ roto").unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        // Campos que faltan: por defecto. Días fuera de rango: a su límite.
        fs::write(
            path_in(dir.path()),
            r#"{"privacy":{"record_paths":false},"retention":{"path_detail_days":365}}"#,
        )
        .unwrap();
        let s = load(dir.path());
        assert!(!s.privacy.record_paths && s.privacy.capture_task_titles);
        assert_eq!(s.retention.path_detail_days, 30);
    }

    #[test]
    fn saving_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.privacy.capture_task_titles = false;
        s.retention.path_detail_days = 0;
        s.adapters.desktop_plan = false;
        let saved = save(dir.path(), &s).unwrap();
        assert_eq!(saved.retention.path_detail_days, 1);
        assert_eq!(load(dir.path()), saved);
        assert!(!dir.path().join("settings.json.tmp").exists());
    }
}
