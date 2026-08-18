//! Integration test for `TmuxBackend` against REAL tmux.
//!
//! The unit suite exercises `SessionManager` through `FakeBackend`, so before
//! this file `TmuxBackend` itself had zero coverage — the `=name` / `=name:`
//! target conventions (verified against tmux 3.6b, see backend.rs) lived only
//! in a comment. This round-trip pins them against the installed tmux.
//!
//! Self-skips (prints a note and passes green) when tmux is not installed, so
//! a green suite on a tmux-less machine does NOT prove the backend works —
//! same convention as mirai-server's `pty::pruebas`.

use openmirai_engine::sessions::{SessionBackend, TmuxBackend};

fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Kills the test's tmux session even if an assertion panics.
struct SessionGuard<'a> {
    backend: &'a TmuxBackend,
    name: String,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        let _ = self.backend.kill(&self.name);
    }
}

#[test]
fn tmux_backend_round_trip() {
    if !tmux_available() {
        eprintln!("sin tmux en esta maquina — test saltado");
        return;
    }

    let backend = TmuxBackend::new();
    // Unique per run so parallel/aborted runs never collide; `mirai-` prefix
    // so `list()` (which filters by prefix) can see it.
    let name = format!(
        "mirai-it-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    let _guard = SessionGuard {
        backend: &backend,
        name: name.clone(),
    };

    // spawn: `cat` stays alive and echoes stdin — the same token-free stand-in
    // for a real agent that mirai-server's PRUEBA-RAPIDA uses.
    let env = vec![("OPENMIRAI_IT_MARKER".to_string(), "valor-123".to_string())];
    backend
        .spawn(&name, "/tmp", "cat", &env)
        .expect("spawn contra tmux real");

    assert!(backend.session_exists(&name), "la sesión debe existir tras spawn");
    assert!(
        backend.list().contains(&name),
        "list() debe incluir la sesión (prefijo mirai-)"
    );

    // get_env: the `-e` var set at new-session must be readable back — this is
    // what the startup adoption relies on for OPENMIRAI_SESSION_META.
    let got = backend
        .get_env(&name, "OPENMIRAI_IT_MARKER")
        .expect("show-environment contra tmux real");
    assert_eq!(got.as_deref(), Some("valor-123"));
    assert_eq!(
        backend.get_env(&name, "OPENMIRAI_IT_NO_EXISTE").unwrap(),
        None,
        "variable inexistente → Ok(None), no error"
    );

    // send_text + capture_output: the marker must appear in the pane (terminal
    // echo + cat's echo). The pane needs a moment — retry briefly.
    let marker = "marcador-integracion-tmux-777";
    backend.send_text(&name, marker).expect("send-keys");
    let mut seen = false;
    for _ in 0..20 {
        let pane = backend.capture_output(&name, 50).expect("capture-pane");
        if pane.iter().any(|l| l.contains(marker)) {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    assert!(seen, "el texto enviado debe aparecer en capture-pane");

    // kill: the session must be gone (the guard's kill then no-ops).
    backend.kill(&name).expect("kill-session");
    assert!(!backend.session_exists(&name), "tras kill la sesión no existe");
    assert!(!backend.list().contains(&name));
}
