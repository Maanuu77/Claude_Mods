//! Aislamiento de procesos WebView2 (C2, SPEC §11.4; puerta G9).
//!
//! Otras aplicaciones también usan WebView2, así que para medir memoria hay que
//! saber qué procesos son **los nuestros**. La fuente autoritativa es
//! `ICoreWebView2Environment8::GetProcessInfos` del entorno de la propia app.

#![allow(unsafe_code)] // Llamadas COM a WebView2: no hay API segura para esto.

use std::sync::mpsc;

use serde::Serialize;
use tauri::WebviewWindow;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PROCESS_KIND, COREWEBVIEW2_PROCESS_KIND_BROWSER, COREWEBVIEW2_PROCESS_KIND_GPU,
    COREWEBVIEW2_PROCESS_KIND_PPAPI_BROKER, COREWEBVIEW2_PROCESS_KIND_PPAPI_PLUGIN,
    COREWEBVIEW2_PROCESS_KIND_RENDERER, COREWEBVIEW2_PROCESS_KIND_SANDBOX_HELPER,
    COREWEBVIEW2_PROCESS_KIND_UTILITY, ICoreWebView2_2, ICoreWebView2Environment8,
};
use windows_core::Interface;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub kind: &'static str,
}

fn kind_name(kind: COREWEBVIEW2_PROCESS_KIND) -> &'static str {
    match kind {
        COREWEBVIEW2_PROCESS_KIND_BROWSER => "browser",
        COREWEBVIEW2_PROCESS_KIND_RENDERER => "renderer",
        COREWEBVIEW2_PROCESS_KIND_UTILITY => "utility",
        COREWEBVIEW2_PROCESS_KIND_SANDBOX_HELPER => "sandbox_helper",
        COREWEBVIEW2_PROCESS_KIND_GPU => "gpu",
        COREWEBVIEW2_PROCESS_KIND_PPAPI_PLUGIN => "ppapi_plugin",
        COREWEBVIEW2_PROCESS_KIND_PPAPI_BROKER => "ppapi_broker",
        _ => "other",
    }
}

/// Procesos del grupo WebView2 de esta app, más el proceso Rust (`host`).
pub async fn webview_processes(window: &WebviewWindow) -> Result<Vec<ProcessInfo>, String> {
    let (tx, rx) = mpsc::channel();
    window
        .with_webview(move |webview| {
            let _ = tx.send(read_process_infos(&webview));
        })
        .map_err(|e| e.to_string())?;
    let mut processes = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())??;
    processes.push(ProcessInfo {
        pid: std::process::id(),
        kind: "host",
    });
    processes.sort_by_key(|p| p.pid);
    Ok(processes)
}

/// Se ejecuta en el hilo de la interfaz, que es el dueño de los objetos COM.
fn read_process_infos(
    webview: &tauri::webview::PlatformWebview,
) -> Result<Vec<ProcessInfo>, String> {
    let err = |e: windows_core::Error| format!("WebView2: {e}");
    // SAFETY: los punteros COM vienen del controlador vivo de la ventana y todas
    // las llamadas se hacen en su hilo. Los parámetros de salida son locales.
    unsafe {
        let core = webview.controller().CoreWebView2().map_err(err)?;
        let environment = core
            .cast::<ICoreWebView2_2>()
            .map_err(err)?
            .Environment()
            .map_err(err)?
            .cast::<ICoreWebView2Environment8>()
            .map_err(err)?;
        let infos = environment.GetProcessInfos().map_err(err)?;
        let mut count = 0u32;
        infos.Count(&mut count).map_err(err)?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let info = infos.GetValueAtIndex(i).map_err(err)?;
            let mut pid = 0i32;
            info.ProcessId(&mut pid).map_err(err)?;
            let mut kind = COREWEBVIEW2_PROCESS_KIND::default();
            info.Kind(&mut kind).map_err(err)?;
            out.push(ProcessInfo {
                pid: pid as u32,
                kind: kind_name(kind),
            });
        }
        Ok(out)
    }
}
