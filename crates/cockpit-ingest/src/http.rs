//! `HttpIngest`: el endpoint local al que envía el mod (SPEC §6.2).
//!
//! Solo escucha en `127.0.0.1`, en un puerto aleatorio, con un Bearer de 256 bits
//! que se renueva en cada arranque. El mod descubre ambos en `endpoint.json`.
//! Las peticiones se entregan a la canalización, que es la única que escribe.

use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::oneshot;

use crate::wire::{Ack, EventsRequest, HelloRequest, HelloResponse, SCHEMA};

/// Cuerpo máximo de una petición.
const MAX_BODY: usize = 512 * 1024;
/// Tiempo máximo que una petición espera a la canalización.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Petición del mod, con el canal por el que la canalización responde.
pub enum ModRequest {
    Hello(Box<HelloRequest>, oneshot::Sender<HelloResponse>),
    Events(EventsRequest, oneshot::Sender<EventsOutcome>),
}

pub enum EventsOutcome {
    Ack(Ack),
    /// La instancia no ha hecho `hello`: el mod debe repetirlo (409).
    UnknownInstance,
}

/// Entrega una petición a la canalización. `false` si ya no está escuchando.
pub type Submit = Arc<dyn Fn(ModRequest) -> bool + Send + Sync>;

#[derive(Clone)]
struct Shared {
    token: Arc<str>,
    host: Arc<str>,
    submit: Submit,
}

pub struct HttpIngest {
    port: u16,
    endpoint_path: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl HttpIngest {
    /// Arranca el servidor y escribe `endpoint.json`.
    pub fn spawn(endpoint_path: PathBuf, submit: Submit) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = new_token()?;
        let shared = Shared {
            token: token.as_str().into(),
            host: format!("127.0.0.1:{port}").into(),
            submit,
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = thread::Builder::new()
            .name("cockpit-http".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                        return;
                    };
                    let _ = axum::serve(listener, router(shared))
                        .with_graceful_shutdown(async {
                            let _ = shutdown_rx.await;
                        })
                        .await;
                });
            })?;
        // El fichero se escribe cuando el puerto ya acepta conexiones.
        write_endpoint(&endpoint_path, port, &token)?;
        Ok(Self {
            port,
            endpoint_path,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for HttpIngest {
    fn drop(&mut self) {
        // Cierre limpio: sin `endpoint.json`, el mod pasa a vigilar (SPEC §5.4, S4).
        let _ = fs::remove_file(&self.endpoint_path);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Escritura atómica: el mod nunca ve un fichero a medias.
fn write_endpoint(path: &Path, port: u16, token: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = json!({ "port": port, "token": token, "pid": std::process::id(), "schema": SCHEMA });
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body.to_string())?;
    fs::rename(&tmp, path)
}

fn router(shared: Shared) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/hello", post(hello))
        .route("/v1/events", post(events))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(shared.clone(), guard))
        .with_state(shared)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Comprobaciones comunes a todas las rutas (SPEC §6.2.1).
async fn guard(State(shared): State<Shared>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    // Un navegador siempre manda `Origin`; el mod, nunca.
    if headers.contains_key(header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Protección contra DNS rebinding.
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h == &*shared.host);
    if !host_ok {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    }
    let token_ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .is_some_and(|t| constant_time_eq(t.as_bytes(), shared.token.as_bytes()));
    if !token_ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "app_version": APP_VERSION, "schema": [SCHEMA] }))
}

async fn hello(State(shared): State<Shared>, Json(request): Json<HelloRequest>) -> Response {
    if request.schema != SCHEMA {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    let (tx, rx) = oneshot::channel();
    if !(shared.submit)(ModRequest::Hello(Box::new(request), tx)) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match tokio::time::timeout(REPLY_TIMEOUT, rx).await {
        Ok(Ok(response)) => Json(response).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn events(State(shared): State<Shared>, Json(request): Json<EventsRequest>) -> Response {
    if request.schema != SCHEMA {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    let (tx, rx) = oneshot::channel();
    if !(shared.submit)(ModRequest::Events(request, tx)) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match tokio::time::timeout(REPLY_TIMEOUT, rx).await {
        Ok(Ok(EventsOutcome::Ack(ack))) => Json(ack).into_response(),
        Ok(Ok(EventsOutcome::UnknownInstance)) => StatusCode::CONFLICT.into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// Dirección del servidor, para los tests y el diagnóstico.
pub fn local_addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

#[cfg(test)]
pub(crate) mod testing {
    //! Cliente HTTP/1.1 mínimo para los tests: sin dependencias y con control total
    //! de las cabeceras (hace falta para probar `Host` y `Origin`).
    use std::io::{Read, Write};
    use std::net::TcpStream;

    pub struct Reply {
        pub status: u16,
        pub body: String,
    }

    pub fn request(
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Reply {
        let mut stream = TcpStream::connect(super::local_addr(port)).unwrap();
        let mut head = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            head.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
        }
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        let mut raw = String::new();
        let _ = stream.read_to_string(&mut raw);
        let status = raw
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = raw
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_owned())
            .unwrap_or_default();
        Reply { status, body }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::request;
    use super::*;

    fn endpoint(dir: &Path) -> (u16, String) {
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("endpoint.json")).unwrap()).unwrap();
        (
            v["port"].as_u64().unwrap() as u16,
            v["token"].as_str().unwrap().to_owned(),
        )
    }

    /// Servidor cuya «canalización» acepta todo.
    fn server(dir: &Path) -> HttpIngest {
        let submit: Submit = Arc::new(|request| {
            match request {
                ModRequest::Events(_, tx) => {
                    let _ = tx.send(EventsOutcome::Ack(Ack { ack_seq: 7 }));
                }
                ModRequest::Hello(_, _) => {}
            }
            true
        });
        HttpIngest::spawn(dir.join("endpoint.json"), submit).unwrap()
    }

    const EVENTS: &str = r#"{"schema":1,"instance_id":"i","session_id":"s","events":[]}"#;

    #[test]
    fn requests_are_rejected_before_reaching_the_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let http = server(dir.path());
        let (port, token) = endpoint(dir.path());
        assert_eq!(port, http.port());
        assert_eq!(token.len(), 64);
        let auth = format!("Bearer {token}");
        let json = ("Content-Type", "application/json");
        let post = |headers: &[(&str, &str)], body: &str| {
            request(port, "POST", "/v1/events", headers, body).status
        };

        assert_eq!(request(port, "GET", "/v1/health", &[], "").status, 401);
        assert_eq!(
            request(
                port,
                "GET",
                "/v1/health",
                &[("Authorization", "Bearer mal")],
                ""
            )
            .status,
            401
        );
        let ok = request(port, "GET", "/v1/health", &[("Authorization", &auth)], "");
        assert_eq!(ok.status, 200);
        assert!(ok.body.contains(r#""ok":true"#));

        assert_eq!(post(&[("Authorization", &auth), json], EVENTS), 200);
        // Origin: 403, aunque el token sea bueno.
        assert_eq!(
            post(
                &[
                    ("Authorization", &auth),
                    json,
                    ("Origin", "http://evil.example")
                ],
                EVENTS
            ),
            403
        );
        // Host ajeno: 421.
        assert_eq!(
            post(
                &[("Authorization", &auth), json, ("Host", "evil.example")],
                EVENTS
            ),
            421
        );
        // Sin Content-Type: 415.
        assert_eq!(post(&[("Authorization", &auth)], EVENTS), 415);
        // Campo fuera de la lista blanca: 422.
        let extra = EVENTS.replace(r#""events":[]"#, r#""events":[],"cwd":"C:\\secreto""#);
        assert_eq!(post(&[("Authorization", &auth), json], &extra), 422);
        // Esquema desconocido: 422.
        assert_eq!(
            post(
                &[("Authorization", &auth), json],
                &EVENTS.replace(r#""schema":1"#, r#""schema":2"#)
            ),
            422
        );
        // Cuerpo demasiado grande: 413.
        let big = format!(
            r#"{{"schema":1,"instance_id":"{}","session_id":"s","events":[]}}"#,
            "x".repeat(MAX_BODY)
        );
        assert_eq!(post(&[("Authorization", &auth), json], &big), 413);
    }

    #[test]
    fn endpoint_file_is_removed_on_clean_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let http = server(dir.path());
        let path = dir.path().join("endpoint.json");
        assert!(path.exists());
        let (port, _) = endpoint(dir.path());
        drop(http);
        assert!(!path.exists());
        assert!(std::net::TcpStream::connect(local_addr(port)).is_err());
    }

    #[test]
    fn every_start_uses_a_new_token() {
        let dir = tempfile::tempdir().unwrap();
        let first = server(dir.path());
        let (_, token_a) = endpoint(dir.path());
        drop(first);
        let _second = server(dir.path());
        let (_, token_b) = endpoint(dir.path());
        assert_ne!(token_a, token_b);
    }
}
