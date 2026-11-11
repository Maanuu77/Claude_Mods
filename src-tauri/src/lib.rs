//! Shell Tauri de DEV COCKPIT: setup, comandos, Channels, bandeja y single-instance (SPEC §6.6).
//!
//! F0: abre una ventana vacía. Los comandos y los plugins llegan en F2.

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
