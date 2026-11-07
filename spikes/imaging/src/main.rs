//! Pocket Forge large-image spike. Throwaway benchmark code, not production code.
//!
//! Every subcommand runs in its own process so that the peak memory it prints
//! belongs to that case alone. See README.md for the run steps.

mod anim;
mod codecs;
mod flatten;
mod mem;
mod pyramid;
mod region;
mod synth;

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Instant;

pub struct Args(HashMap<String, String>);

impl Args {
    pub fn get<T: FromStr>(&self, k: &str, default: T) -> T {
        self.0
            .get(k)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
    pub fn str(&self, k: &str, default: &str) -> String {
        self.0.get(k).cloned().unwrap_or_else(|| default.to_string())
    }
}

/// Run `f`, return its result and the wall time in milliseconds.
pub fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_secs_f64() * 1000.0)
}

/// FNV-1a 64-bit. Good enough to compare outputs for byte equality.
pub fn fnv(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Streaming FNV-1a.
pub struct Fnv(pub u64);
impl Fnv {
    pub fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

/// Print one result row. Rows are tab-separated `key=value` pairs.
pub fn row(case: &str, kv: &[(&str, String)]) {
    let mut s = format!("RESULT\tcase={case}");
    for (k, v) in kv {
        s.push('\t');
        s.push_str(k);
        s.push('=');
        s.push_str(v);
    }
    println!("{s}");
}

pub fn ms(v: f64) -> String {
    format!("{v:.0}")
}
pub fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1048576.0)
}

fn main() {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let rest: Vec<String> = it.collect();
    let mut map = HashMap::new();
    let mut i = 0;
    while i < rest.len() {
        let k = rest[i].trim_start_matches("--").to_string();
        let v = rest.get(i + 1).cloned().unwrap_or_default();
        map.insert(k, v);
        i += 2;
    }
    let a = Args(map);
    let t = Instant::now();
    match cmd.as_str() {
        "gen" => synth::cmd_gen(&a),
        "decode" => codecs::cmd_decode(&a),
        "pyramid" => pyramid::cmd_pyramid(&a),
        "region" => region::cmd_region(&a),
        "blend" => region::cmd_blend(&a),
        "flatten" => flatten::cmd_flatten(&a),
        "limits" => codecs::cmd_limits(&a),
        "determinism" => region::cmd_determinism(&a),
        "anim" => anim::cmd_anim(&a),
        "compare" => codecs::cmd_compare(&a),
        "animcheck" => anim::cmd_animcheck(&a),
        _ => {
            eprintln!(
                "usage: pf-imaging-spike <gen|decode|pyramid|region|blend|flatten|limits|determinism|anim|compare|animcheck> [--key value]..."
            );
            std::process::exit(2);
        }
    }
    let m = mem::peak();
    println!(
        "PEAK\tcmd={cmd}\twall_ms={:.0}\tpeak_ws_mib={:.0}\tpeak_commit_mib={:.0}\tthreads={}",
        t.elapsed().as_secs_f64() * 1000.0,
        m.0,
        m.1,
        rayon::current_num_threads()
    );
}
