//! Criterio de F4c (SPEC §15): consultas de las vistas < 100 ms.
//!
//! ```text
//! cargo run -p cockpit-store --release --example f4c_queries -- <copia de cockpit.db>
//! ```
//!
//! Abre la base **en solo lectura** y cronometra cada consulta 20 veces (la primera
//! en frío). Usar siempre una copia, nunca la base real de la app.

use std::path::Path;
use std::time::Instant;

use cockpit_core::PATH_DETAIL_DAYS;
use cockpit_core::time::{closed_before, local_day_minus};
use cockpit_store::Store;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn time<T>(label: &str, mut f: impl FnMut() -> T) {
    let mut ms: Vec<f64> = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        std::hint::black_box(f());
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let cold = ms[0];
    let mut warm = ms[1..].to_vec();
    warm.sort_by(f64::total_cmp);
    let p50 = warm[warm.len() / 2];
    let max = warm[warm.len() - 1];
    println!("{label:<24} frío {cold:>7.1} ms · p50 {p50:>7.1} ms · máx {max:>7.1} ms");
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("ruta de una copia de cockpit.db");
    let store = Store::open_read_only(Path::new(&path)).expect("abrir la base");
    let now = now_ms();
    let closed = closed_before(now);
    let purge = local_day_minus(now, PATH_DETAIL_DAYS);
    time("diagnostics", || {
        store.diagnostics(&closed, &purge).unwrap()
    });
    for days in [7u32, 30, 90] {
        let since_day = local_day_minus(now, days - 1);
        let since_ms = now - i64::from(days) * 86_400_000;
        time(&format!("history {days} días"), || {
            store.history(&since_day, since_ms).unwrap()
        });
    }
    let h = store
        .history(&local_day_minus(now, 89), now - 90 * 86_400_000)
        .unwrap();
    println!(
        "(90 días: {} días con datos, {} herramientas, {} lecturas de límites)",
        h.days.len(),
        h.tools.len(),
        h.limits.len()
    );
}
