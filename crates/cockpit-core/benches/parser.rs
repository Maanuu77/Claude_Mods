//! B1: throughput del parser de transcripts sobre los fixtures sintéticos (SPEC §11.4).
//!
//! ```text
//! cargo bench -p cockpit-core
//! ```

use std::fs;
use std::hint::black_box;
use std::path::PathBuf;

use cockpit_core::privacy::PrivacyPolicy;
use cockpit_core::transcript::TranscriptParser;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    fs::read(path).unwrap()
}

fn parse(data: &[u8]) -> usize {
    let mut parser = TranscriptParser::new(PrivacyPolicy::APP_DEFAULT);
    let mut events = Vec::new();
    for line in data.split(|b| *b == b'\n') {
        let _ = parser.feed(line, &mut events);
    }
    parser.finish(&mut events);
    events.len()
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcript_parser");
    for name in ["session-2.1.321.jsonl", "session-2.1.128.jsonl"] {
        let data = fixture(name);
        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function(name, |b| b.iter(|| parse(black_box(&data))));
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
