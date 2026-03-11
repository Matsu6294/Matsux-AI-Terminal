//! Background aider task: receives goals → calls LLM → applies edits → git commit.
//!
//! Uses std::sync::mpsc for sending messages back to the GUI (sync-safe).

use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;
use eframe::egui;
use tokio::sync::mpsc as tokio_mpsc;

use editor::{apply_edits, parse_edits, EditFormat};
use git_ops::GitRepo;
use llm::{system_prompt_for_edit_format, LlmClient, LlmConfig, Message, OpenAiClient};

use crate::app::AppMsg;

// ─── Request type ─────────────────────────────────────────────────────────────

pub struct AiderRequest {
    pub goal: String,
    pub context_files: Vec<PathBuf>,
    pub repo_path: PathBuf,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
}

// ─── Background task loop ─────────────────────────────────────────────────────

pub async fn run(
    mut rx: tokio_mpsc::Receiver<AiderRequest>,
    tx: mpsc::Sender<AppMsg>,
    ctx: egui::Context,
) {
    while let Some(req) = rx.recv().await {
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(req, &tx2, &ctx2).await {
                send(&tx2, AppMsg::Error(e.to_string()), &ctx2);
            }
        });
    }
}

// ─── Single request ───────────────────────────────────────────────────────────

async fn handle(
    req: AiderRequest,
    tx: &mpsc::Sender<AppMsg>,
    ctx: &egui::Context,
) -> Result<()> {
    send(tx, AppMsg::StatusSet("Läser kontext…".into()), ctx);

    // Build file context.
    let mut context = String::new();
    for abs in &req.context_files {
        if let Ok(content) = std::fs::read_to_string(abs) {
            let rel = abs.strip_prefix(&req.repo_path).unwrap_or(abs.as_path());
            context.push_str(&format!("### {}\n```\n{content}\n```\n\n", rel.display()));
        }
    }

    // Build LLM client — uses api_key and base_url from app settings (no env vars).
    let config = LlmConfig {
        model_name: req.model.clone(),
        max_tokens: 4096,
        temperature: 0.0,
        system_prompt: system_prompt_for_edit_format("editblock").to_string(),
    };
    if req.api_key.is_empty() {
        return Err(anyhow::anyhow!(
            "API-nyckel saknas.\nÖppna ⚙ API-inställningar i sidofältet och ange din nyckel."
        ));
    }
    let client = OpenAiClient::new(config, req.base_url.clone(), req.api_key.clone());

    let user_msg = if context.is_empty() {
        req.goal.clone()
    } else {
        format!("{}\n\n{context}", req.goal)
    };

    send(tx, AppMsg::StatusSet("Väntar på AI…".into()), ctx);

    let response = client.chat(&[Message::user(user_msg)]).await?;

    send(
        tx,
        AppMsg::ChatAppend { role: "ai".into(), text: response.clone() },
        ctx,
    );

    // Parse edits.
    let mut edits = parse_edits(&response, EditFormat::EditBlock)?;

    if edits.is_empty() {
        send(tx, AppMsg::StatusSet("Klar ✓ (inga kodfiler ändrades)".into()), ctx);
        return Ok(());
    }

    // Resolve paths.
    for edit in &mut edits {
        if edit.filename.is_relative() {
            edit.filename = req.repo_path.join(&edit.filename);
        }
    }

    let n = edits.len();
    send(tx, AppMsg::StatusSet(format!("Applicerar {n} edit(s)…")), ctx);

    apply_edits(&edits, false)?;

    send(
        tx,
        AppMsg::ChatAppend {
            role: "sys".into(),
            text: format!("✅ Applicerade {n} edit(s)"),
        },
        ctx,
    );

    // Auto-commit.
    let changed: Vec<PathBuf> = edits.iter().map(|e| e.filename.clone()).collect();
    if let Ok(repo) = GitRepo::open(&req.repo_path) {
        let msg = format!("matsux: {}", &req.goal[..req.goal.len().min(72)]);
        match repo.commit(&msg, &changed) {
            Ok(sha) => {
                let short = &sha[..sha.len().min(8)];
                send(
                    tx,
                    AppMsg::ChatAppend {
                        role: "sys".into(),
                        text: format!("✅ git commit {short}"),
                    },
                    ctx,
                );

                if let Ok(diff) = repo.get_diff_since_commit(&sha) {
                    send(tx, AppMsg::DiffSet(diff), ctx);
                }

                if let Ok(log) = repo.log(10) {
                    let entries: Vec<String> = log
                        .iter()
                        .map(|(sha, msg)| format!("{} {}", &sha[..8.min(sha.len())], msg))
                        .collect();
                    send(tx, AppMsg::GitLogSet(entries), ctx);
                }
            }
            Err(e) => {
                send(
                    tx,
                    AppMsg::ChatAppend {
                        role: "sys".into(),
                        text: format!("⚠  git commit misslyckades: {e}"),
                    },
                    ctx,
                );
            }
        }
    }

    send(tx, AppMsg::StatusSet("Klar ✓".into()), ctx);
    Ok(())
}

// ─── Helper ───────────────────────────────────────────────────────────────────

fn send(tx: &mpsc::Sender<AppMsg>, msg: AppMsg, ctx: &egui::Context) {
    let _ = tx.send(msg);
    ctx.request_repaint(); // Wake up the GUI immediately.
}
