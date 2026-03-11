//! matsux-term — standalone AI-kodassistent med inbyggd aider.
//!
//! Öppnas med dubbelklick — startar ett eget GUI-fönster.
//!
//! Miljövariabler:
//!   OPENAI_API_KEY   — API-nyckel (OpenAI, Groq, Mistral AI, …)
//!   OPENAI_BASE_URL  — Valfri bas-URL, t.ex. http://localhost:11434/v1 (Ollama)
//!   MATSUX_MODEL     — Modellnamn (standard: gpt-4o)

mod app;
mod aider;
mod cargo_runner;
mod config;
mod kb;
mod matsux_log;

use std::path::PathBuf;
use eframe::egui;

fn main() -> eframe::Result<()> {
    // Log to file — never pollute the GUI.
    if let Ok(log) = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open("/tmp/matsux-term.log")
    {
        let _ = tracing_subscriber::fmt()
            .with_writer(log)
            .with_env_filter(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "matsux_term=debug,llm=info".into()),
            )
            .try_init();
    }

    // Always use the fixed files directory.
    let repo_path: PathBuf = PathBuf::from("/home/matsu/matsux-os/filer");

    // Build a multi-thread tokio runtime that lives as long as the app.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // req: GUI → aider task (tokio channel, aider uses .recv().await)
    let (req_tx, req_rx) = tokio::sync::mpsc::channel::<aider::AiderRequest>(16);
    // cargo: GUI → cargo runner task
    let (cargo_tx, cargo_rx) = tokio::sync::mpsc::channel::<cargo_runner::CargoCmd>(8);
    // msg: aider/cargo tasks → GUI (std channel, GUI uses .try_recv() in update())
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<app::AppMsg>();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("matsux-term")
            .with_inner_size([1300.0, 820.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "matsux-term",
        options,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            rt.spawn(aider::run(req_rx, msg_tx.clone(), ctx.clone()));
            rt.spawn(cargo_runner::run(cargo_rx, msg_tx, ctx.clone()));
            Ok(Box::new(app::App::new(repo_path, req_tx, msg_rx, cargo_tx, rt)))
        }),
    )
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());

    let mut style = (*ctx.style()).clone();
    // Use monospace for code, slightly larger default text.
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(13.0, egui::FontFamily::Monospace),
    );
    ctx.set_style(style);
}
