//! Background aider task: receives goals → calls LLM → applies edits → git commit.
//!
//! Uses std::sync::mpsc for sending messages back to the GUI (sync-safe).

use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;
use eframe::egui;
use tokio::sync::mpsc as tokio_mpsc;
use serde_json;

use editor::{apply_edits, parse_edits, EditFormat};
use git_ops::GitRepo;
use llm::{system_prompt_for_edit_format, LlmClient, LlmConfig, Message, OpenAiClient};

use crate::app::AppMsg;
use crate::kb;
use crate::matsux_log;

// ─── DuckDuckGo-sökning ───────────────────────────────────────────────────────

async fn web_search(query: &str) -> String {
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_redirect=1&no_html=1",
        urlencoding::encode(query)
    );
    let Ok(resp) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap()
        .get(&url)
        .header("User-Agent", "matsux-term/0.1")
        .send()
        .await
    else { return String::new() };

    let Ok(json) = resp.json::<serde_json::Value>().await else { return String::new() };

    let mut result = String::new();

    // AbstractText — kort sammanfattning
    if let Some(abs) = json["AbstractText"].as_str().filter(|s| !s.is_empty()) {
        result.push_str(&format!("**{abs}**\n\n"));
    }

    // RelatedTopics — de 5 första relevanta träffarna
    if let Some(topics) = json["RelatedTopics"].as_array() {
        for topic in topics.iter().take(5) {
            if let Some(text) = topic["Text"].as_str().filter(|s| !s.is_empty()) {
                result.push_str(&format!("- {text}\n"));
            }
        }
    }

    result
}

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
    log("aider::run() startad");
    while let Some(req) = rx.recv().await {
        log(&format!("aider::run() fick förfrågan: {}", req.goal));
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(req, &tx2, &ctx2).await {
                log(&format!("handle() fel: {e}"));
                let _ = matsux_log::append("(fel)", &[], &format!("fel: {e}"));
                send(&tx2, AppMsg::Error(e.to_string()), &ctx2);
            }
        });
    }
    log("aider::run() avslutad (kanal stängd)");
}

// ─── log() helper ────────────────────────────────────────────────────────────

fn log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/matsux-term.log") {
        let _ = writeln!(f, "{}", msg);
    }
}

// ─── handle() ────────────────────────────────────────────────────────────────

async fn handle(
    req: AiderRequest,
    tx: &mpsc::Sender<AppMsg>,
    ctx: &egui::Context,
) -> Result<()> {
    send(tx, AppMsg::StatusSet("Läser kontext…".into()), ctx);
    log("→ handle() start");

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
        max_tokens: 1024,
        temperature: 0.0,
        system_prompt: system_prompt_for_edit_format("wholefile").to_string(),
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

    // Sök i lokal kunskapsbas efter relevanta tidigare exempel.
    send(tx, AppMsg::StatusSet("Söker i kunskapsbas…".into()), ctx);
    let kb_context = kb::search(&req.goal, 3);
    if !kb_context.is_empty() {
        log(&format!("→ KB-träff hittad ({} tecken)", kb_context.len()));
    }

    // Hämta matsux-log så AI:n vet vad den gjort tidigare.
    let log_context = matsux_log::summary_for_context(15);

    // Webbsökning för programmerings-relaterade frågor.
    send(tx, AppMsg::StatusSet("Söker på webben…".into()), ctx);
    let search_query = format!("{} programming example", req.goal);
    let web_context = web_search(&search_query).await;
    if !web_context.is_empty() {
        log(&format!("→ Webbsökning: {} tecken", web_context.len()));
    }

    // Bygg slutlig kontext: logg + KB + webb + filer.
    let mut extra = String::new();
    if !log_context.is_empty() {
        extra.push_str(&log_context);
    }
    if !kb_context.is_empty() {
        extra.push_str(&kb_context);
    }
    if !web_context.is_empty() {
        extra.push_str("### Webbsökning\n\n");
        extra.push_str(&web_context);
        extra.push('\n');
    }

    let user_msg = if extra.is_empty() {
        user_msg
    } else {
        format!("{user_msg}\n\n{extra}")
    };

    send(tx, AppMsg::StatusSet("Väntar på AI…".into()), ctx);
    log("→ skickar till Ollama…");

    let response = client.chat(&[Message::user(user_msg)]).await?;
    log(&format!("→ svar mottaget ({} tecken)", response.len()));

    send(
        tx,
        AppMsg::ChatAppend { role: "ai".into(), text: response.clone() },
        ctx,
    );

    // Parse edits — if the model didn't follow the editblock format, show the
    // response as plain chat and finish gracefully.
    let mut edits = match parse_edits(&response, EditFormat::WholeFile) {
        Ok(e) => { log(&format!("→ parse_edits ok: {} edit(s)", e.len())); e }
        Err(e) => {
            log(&format!("→ parse_edits fel: {e}"));
            return Ok(());
        }
    };

    if edits.is_empty() {
        send(tx, AppMsg::StatusSet("Klar ✓ (inga kodfiler ändrades)".into()), ctx);
        return Ok(());
    }

    // Resolve paths — always place files inside repo_path (filer/).
    for edit in &mut edits {
        log(&format!("→ edit path (före): {:?}", edit.filename));
        let filename = edit.filename.clone();
        // Strip any leading absolute prefix so everything lands in filer/.
        let relative = if filename.is_absolute() {
            filename
                .file_name()
                .map(std::path::Path::new)
                .unwrap_or(&filename)
                .to_path_buf()
        } else {
            filename
        };
        edit.filename = req.repo_path.join(relative);
        log(&format!("→ edit path (efter): {:?}", edit.filename));
    }

    let n = edits.len();
    send(tx, AppMsg::StatusSet(format!("Applicerar {n} edit(s)…")), ctx);

    match apply_edits(&edits, false) {
        Ok(_) => log("→ apply_edits OK"),
        Err(e) => { log(&format!("→ apply_edits FEL: {e}")); return Err(e); }
    }

    // Spara lyckade kodfiler till kunskapsbasen så AI:n lär sig.
    let file_paths: Vec<&std::path::Path> = edits.iter().map(|e| e.filename.as_path()).collect();
    for edit in &edits {
        if let Ok(content) = std::fs::read_to_string(&edit.filename) {
            if let Err(e) = kb::save(&req.goal, &edit.filename, &content) {
                log(&format!("→ KB-sparning misslyckades: {e}"));
            } else {
                log(&format!("→ KB: sparade {:?}", edit.filename));
            }
        }
    }

    // Skriv till matsux-log.
    let _ = matsux_log::append(&req.goal, &file_paths, "ok");

    send(
        tx,
        AppMsg::ChatAppend {
            role: "sys".into(),
            text: format!("✅ Applicerade {n} edit(s)"),
        },
        ctx,
    );

    // Auto-compile .rs files with rustc — output binary goes to filer/.
    for edit in &edits {
        if edit.filename.extension().and_then(|e| e.to_str()) == Some("rs") {
            let stem = edit.filename.file_stem().unwrap_or_default();
            let out = req.repo_path.join(stem);
            log(&format!("→ kompilerar {:?} → {:?}", edit.filename, out));
            send(tx, AppMsg::StatusSet(format!("Kompilerar {}…", edit.filename.display())), ctx);
            let result = std::process::Command::new("rustc")
                .arg(&edit.filename)
                .arg("-o").arg(&out)
                .output();
            match result {
                Ok(out_result) if out_result.status.success() => {
                    log("→ rustc OK");
                    send(tx, AppMsg::ChatAppend {
                        role: "sys".into(),
                        text: format!("✅ Kompilerad → {}", out.display()),
                    }, ctx);
                }
                Ok(out_result) => {
                    let err = String::from_utf8_lossy(&out_result.stderr).to_string();
                    log(&format!("→ rustc FEL: {err}"));
                    send(tx, AppMsg::ChatAppend {
                        role: "sys".into(),
                        text: format!("❌ Kompileringsfel:\n{err}"),
                    }, ctx);
                }
                Err(e) => {
                    log(&format!("→ rustc kunde inte starta: {e}"));
                    send(tx, AppMsg::ChatAppend {
                        role: "sys".into(),
                        text: format!("❌ Kunde inte starta rustc: {e}"),
                    }, ctx);
                }
            }
        }
    }

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

// ─── send() helper ───────────────────────────────────────────────────────────

fn send(tx: &mpsc::Sender<AppMsg>, msg: AppMsg, ctx: &egui::Context) {
    let _ = tx.send(msg);
    // Coalesce multiple rapid messages into a single repaint to prevent flickering.
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}
