//! End to end against a real haunt core. Copy into haunt's `cli/tests/` and run
//! `cargo test -p haunt-cli --features plugins --test pulse_lines` (see README).
#![cfg(feature = "plugins")]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use haunt_core::plugin_settings::PluginConfig;
use haunt_core::plugin_store::{Store, attach};
use haunt_core::plugins::PluginHost;
use haunt_core::{Core, SurfaceEnv};

const ID: &str = "dev.haunt.pulse-lines";

fn repo() -> PathBuf {
    std::env::var_os("HAUNT_PLUGINS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../haunt-plugins"))
        .join("haunt-pulse-lines")
}

fn until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send(sock: &mut UnixStream, env: &SurfaceEnv, nonce: bool, cwd: &str, rest: &str) {
    let nonce = if nonce { format!(r#","nonce":"{}""#, env.nonce) } else { String::new() };
    let line = format!(r#"{{"v":1,"agent":"claude","surface_id":"{}"{nonce},"session_id":"s1","cwd":"{cwd}",{rest}}}"#, env.surface_id);
    sock.write_all(format!("{line}\n").as_bytes()).unwrap();
}

#[test]
fn tool_calls_draw_a_sparkline_and_a_loop_turns_it_orange() {
    let dir = repo();
    if !dir.join("haunt-plugin.toml").is_file() {
        eprintln!("skipped: haunt-pulse-lines not found; set HAUNT_PLUGINS_DIR");
        return;
    }
    let home = PathBuf::from(format!("/tmp/haunt-fp-pulse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&dir)
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .unwrap();
    assert!(status.success());
    let o = Command::new(env!("CARGO_BIN_EXE_haunt")).args(["plugin", "install", dir.to_str().unwrap(), "--yes"]).env("HAUNT_HOME", &home).output().unwrap();
    assert!(o.status.success(), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));

    let core = Core::new(&home);
    core.start().unwrap();
    let (env, other) = (core.register_surface("/tmp").unwrap(), core.register_surface("/tmp").unwrap());
    let set = Store::open(&home).unwrap().load_all(&PluginHost::new().unwrap(), Some(&core.handle())).unwrap();
    assert!(set.errors().is_empty(), "{:?}", set.errors());
    let set = Arc::new(Mutex::new(set));
    attach(&core, &set);
    let decoration = |s: &str| core.snapshot().groups.iter().flat_map(|g| &g.rows).find(|r| r.surface_id == s).and_then(|r| r.decoration.clone());
    let call = |i: usize, cmd: &str| format!(r#""event":"tool_start","tool_name":"Bash","tool_use_id":"t{i}","tool_input":{{"command":"{cmd}"}}"#);

    let mut sock = UnixStream::connect(&env.socket_path).unwrap();
    send(&mut sock, &env, true, "/tmp/web", r#""event":"prompt_submit","query":"go""#);
    for i in 0..3 {
        send(&mut sock, &env, true, "/tmp/web", &call(i, &format!("ls {i}")));
        send(&mut sock, &other, false, "/tmp/web", &call(i, "ls"));
    }
    until("the sparkline", || decoration(&env.surface_id).and_then(|d| d.spark).is_some_and(|s| s.points.last() == Some(&255)));
    let d = decoration(&env.surface_id).unwrap();
    assert_eq!((d.spark.unwrap().points.len(), d.suffix), (30, None));
    assert_eq!(decoration(&other.surface_id), None, "forged calls draw nothing");

    for i in 3..12 {
        send(&mut sock, &env, true, "/tmp/web", &call(i, "pytest -x"));
    }
    until("the loop note", || decoration(&env.surface_id).and_then(|d| d.suffix).as_deref() == Some("same Bash call ×9"));
    assert_eq!(decoration(&env.surface_id).unwrap().suffix_tone, haunt_core::decorate::Tone::Warn);

    send(&mut sock, &env, true, "/tmp/web", r#""event":"permission_seen","tool_name":"Bash""#);
    until("haunt's status to own the needs-you row", || decoration(&env.surface_id).is_some_and(|d| d.spark.is_none() && d.suffix.is_none()));
    send(&mut sock, &env, true, "/tmp/web", r#""event":"prompt_submit","query":"go on""#);
    until("the row back to the plugin", || decoration(&env.surface_id).is_some_and(|d| d.spark.is_some()));

    set.lock().unwrap().set_config(&PluginConfig::parse(&format!("[plugins.\"{ID}\"]\nnotes = false\nminutes = 10\n")));
    let d = decoration(&env.surface_id).unwrap();
    assert_eq!((d.suffix, d.spark.unwrap().points.len()), (None, 10));

    drop(core);
    let _ = std::fs::remove_dir_all(&home);
}
