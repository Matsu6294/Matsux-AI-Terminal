//! Cargo and rustup integration for matsux-term.
//!
//! Handles:
//! - Reading / writing rust-toolchain.toml
//! - Listing installed rustup toolchains
//! - Running cargo commands and streaming output to the GUI

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eframe::egui;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc as tokio_mpsc;

use crate::app::AppMsg;

// ─── Command type ─────────────────────────────────────────────────────────────

pub struct CargoCmd {
    pub repo_path: PathBuf,
    /// Arguments passed to `cargo`, e.g. ["build", "--release"]
    pub args: Vec<String>,
}

// ─── Background task ──────────────────────────────────────────────────────────

pub async fn run(
    mut rx: tokio_mpsc::Receiver<CargoCmd>,
    tx: mpsc::Sender<AppMsg>,
    ctx: egui::Context,
) {
    // Load installed toolchains once at startup.
    let toolchains = list_toolchains().await;
    notify(&tx, AppMsg::ToolchainsLoaded(toolchains), &ctx);

    while let Some(cmd) = rx.recv().await {
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            run_cargo(cmd.repo_path, cmd.args, tx2, ctx2).await;
        });
    }
}

// ─── rust-toolchain.toml ──────────────────────────────────────────────────────

/// Read the `channel` from rust-toolchain.toml (simple line scan, no full TOML parser).
pub fn read_toolchain(repo_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(repo_path.join("rust-toolchain.toml")).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("channel") {
            if let Some(val) = line.splitn(2, '=').nth(1) {
                return Some(val.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Write (or overwrite) rust-toolchain.toml with the given channel.
pub fn write_toolchain(repo_path: &Path, channel: &str) -> std::io::Result<()> {
    let content = format!(
        "[toolchain]\nchannel = \"{channel}\"\ncomponents = [\"rustfmt\", \"clippy\"]\ntargets = [\"x86_64-unknown-linux-gnu\"]\n"
    );
    std::fs::write(repo_path.join("rust-toolchain.toml"), content)
}

// ─── rustup toolchain list ────────────────────────────────────────────────────

/// Return all installed toolchain names (stripped of target triple and "(default)").
pub async fn list_toolchains() -> Vec<String> {
    let Ok(out) = Command::new("rustup")
        .args(["toolchain", "list"])
        .output()
        .await
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let name = l.split_whitespace().next()?;
            // Strip target triple: "1.94.0-x86_64-unknown-linux-gnu" → "1.94.0"
            let short = name.split('-').next().unwrap_or(name);
            if short.is_empty() { None } else { Some(short.to_string()) }
        })
        .collect()
}

// ─── cargo command runner ─────────────────────────────────────────────────────

async fn run_cargo(
    repo_path: PathBuf,
    args: Vec<String>,
    tx: mpsc::Sender<AppMsg>,
    ctx: egui::Context,
) {
    let cmd_str = format!("cargo {}", args.join(" "));

    notify(&tx, AppMsg::ChatAppend { role: "sys".into(), text: format!("$ {cmd_str}") }, &ctx);
    notify(&tx, AppMsg::StatusSet(format!("Kör {cmd_str}…")), &ctx);
    notify(&tx, AppMsg::CargoRunning(true), &ctx);

    let result = Command::new("cargo")
        .args(&args)
        .current_dir(&repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = result else {
        notify(&tx, AppMsg::Error("Kunde inte starta cargo — är Rust installerat?".into()), &ctx);
        notify(&tx, AppMsg::CargoRunning(false), &ctx);
        return;
    };

    // cargo writes build output to stderr; stream it line by line.
    if let Some(stderr) = child.stderr.take() {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let colored = colorize_cargo_line(&line);
            notify(&tx, AppMsg::ChatAppend { role: "sys".into(), text: colored }, &ctx);
        }
    }

    // Also capture stdout (e.g. for `cargo test`).
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            notify(&tx, AppMsg::ChatAppend { role: "sys".into(), text: line }, &ctx);
        }
    }

    let success = child.wait().await.map(|s| s.success()).unwrap_or(false);

    let summary = if success {
        format!("✅ {cmd_str} klar")
    } else {
        format!("❌ {cmd_str} misslyckades")
    };

    notify(&tx, AppMsg::ChatAppend { role: "sys".into(), text: summary }, &ctx);
    notify(&tx, AppMsg::StatusSet(if success { "Klar ✓".into() } else { "Byggfel ✗".into() }), &ctx);
    notify(&tx, AppMsg::CargoRunning(false), &ctx);
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn notify(tx: &mpsc::Sender<AppMsg>, msg: AppMsg, ctx: &egui::Context) {
    let _ = tx.send(msg);
    // Coalesce rapid line-by-line output into a single repaint to prevent flickering.
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}

/// Prefix cargo output lines with a visual hint based on content.
fn colorize_cargo_line(line: &str) -> String {
    if line.trim_start().starts_with("error") {
        format!("🔴 {line}")
    } else if line.trim_start().starts_with("warning") {
        format!("🟡 {line}")
    } else if line.contains("Compiling") || line.contains("Finished") || line.contains("Running") {
        format!("   {line}")
    } else {
        line.to_string()
    }
}
