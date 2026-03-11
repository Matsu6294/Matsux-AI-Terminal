//! App state + egui rendering for matsux-term.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eframe::egui;
use tokio::sync::mpsc as tokio_mpsc;

use crate::aider::AiderRequest;
use crate::cargo_runner::{self, CargoCmd};
use crate::config::{self, Config, PROVIDERS};

// ─── Messages from aider task → GUI ──────────────────────────────────────────

pub enum AppMsg {
    ChatAppend { role: String, text: String },
    StatusSet(String),
    DiffSet(String),
    GitLogSet(Vec<String>),
    Error(String),
    /// Cargo command started or finished.
    CargoRunning(bool),
    /// Installed rustup toolchains loaded.
    ToolchainsLoaded(Vec<String>),
}

// ─── Chat entry ───────────────────────────────────────────────────────────────

struct ChatLine {
    role: String, // "du" | "ai" | "sys"
    text: String,
}

// ─── Bottom panel selection ───────────────────────────────────────────────────

#[derive(PartialEq)]
enum Panel { None, Diff, GitLog, Help }

// ─── App state ────────────────────────────────────────────────────────────────

pub struct App {
    repo_path: PathBuf,
    model: String,

    // File browser
    files: Vec<PathBuf>,
    context_files: Vec<PathBuf>,

    // Chat
    chat: Vec<ChatLine>,
    input: String,
    request_focus: bool,

    // State
    busy: bool,
    status: String,

    // Panels
    diff: String,
    git_log: Vec<String>,
    panel: Panel,

    // Channels
    req_tx: tokio_mpsc::Sender<AiderRequest>,
    msg_rx: mpsc::Receiver<AppMsg>,
    cargo_tx: tokio_mpsc::Sender<CargoCmd>,

    // Cargo / toolchain
    pub toolchain_channel: String,
    toolchain_input: String,
    installed_toolchains: Vec<String>,
    cargo_busy: bool,

    // API settings
    cfg: Config,
    show_key: bool,      // toggle password visibility
    selected_provider: usize,
}

impl App {
    pub fn new(
        repo_path: PathBuf,
        req_tx: tokio_mpsc::Sender<AiderRequest>,
        msg_rx: mpsc::Receiver<AppMsg>,
        cargo_tx: tokio_mpsc::Sender<CargoCmd>,
    ) -> Self {
        let model = std::env::var("MATSUX_MODEL")
            .or_else(|_| std::env::var("OPENAI_MODEL"))
            .unwrap_or_else(|_| "gpt-4o".to_string());

        let welcome = format!(
            "matsux-term  •  {}  •  modell: {}",
            repo_path.display(),
            model
        );

        let toolchain_channel = cargo_runner::read_toolchain(&repo_path)
            .unwrap_or_else(|| "stable".to_string());
        let toolchain_input = toolchain_channel.clone();

        let cfg = config::load();
        // Derive initial model from cfg (overrides MATSUX_MODEL env)
        let model = cfg.model.clone();

        let mut app = App {
            repo_path,
            model,
            files: Vec::new(),
            context_files: Vec::new(),
            chat: vec![ChatLine { role: "sys".into(), text: welcome }],
            input: String::new(),
            request_focus: true,
            busy: false,
            status: "Klar".into(),
            diff: String::new(),
            git_log: Vec::new(),
            panel: Panel::None,
            req_tx,
            msg_rx,
            cargo_tx,
            toolchain_channel,
            toolchain_input,
            installed_toolchains: Vec::new(),
            cargo_busy: false,
            cfg,
            show_key: false,
            selected_provider: 0,
        };
        app.scan_files();
        app
    }

    fn scan_files(&mut self) {
        let mut files = Vec::new();
        walk_dir(&self.repo_path, &self.repo_path, &mut files, 0);
        files.sort();
        self.files = files;
    }

    fn drain_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                AppMsg::ChatAppend { role, text } => {
                    if let Some(last) = self.chat.last_mut() {
                        if last.role == role {
                            last.text.push('\n');
                            last.text.push_str(&text);
                            continue;
                        }
                    }
                    self.chat.push(ChatLine { role, text });
                }
                AppMsg::StatusSet(s) => {
                    let done = s.contains("Klar") || s.contains("Fel");
                    self.status = s;
                    if done { self.busy = false; }
                }
                AppMsg::DiffSet(d) => {
                    self.diff = d;
                    self.panel = Panel::Diff;
                }
                AppMsg::GitLogSet(log) => { self.git_log = log; }
                AppMsg::CargoRunning(running) => {
                    self.cargo_busy = running;
                    if !running { self.busy = false; }
                }
                AppMsg::ToolchainsLoaded(list) => {
                    self.installed_toolchains = list;
                }
                AppMsg::Error(e) => {
                    self.chat.push(ChatLine {
                        role: "sys".into(),
                        text: format!("⚠  {e}"),
                    });
                    self.busy = false;
                    self.status = "Fel ✗".into();
                }
            }
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.busy { return; }
        self.input.clear();
        self.chat.push(ChatLine { role: "du".into(), text: text.clone() });
        self.busy = true;
        self.status = "Tänker…".into();
        self.request_focus = true;

        let req = AiderRequest {
            goal: text,
            context_files: self.context_files
                .iter()
                .map(|p| self.repo_path.join(p))
                .collect(),
            repo_path: self.repo_path.clone(),
            model: self.cfg.model.clone(),
            api_key: self.cfg.api_key.clone(),
            base_url: self.cfg.base_url.clone(),
        };
        let _ = self.req_tx.try_send(req);
    }
}

// ─── eframe::App — the GUI update loop ───────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();

        // Keep repainting while busy so we pick up aider messages quickly.
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }

        // ── Title bar ─────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("titlebar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("matsux-term")
                        .strong()
                        .color(egui::Color32::from_rgb(100, 200, 255)),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(self.repo_path.display().to_string())
                        .monospace()
                        .color(egui::Color32::LIGHT_GRAY),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("[{}]", self.model))
                        .color(egui::Color32::GOLD),
                );
                if self.busy {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(&self.status)
                            .color(egui::Color32::YELLOW),
                    );
                }
            });
        });

        // ── Status bar ────────────────────────────────────────────────────────
        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let status_color = if self.status.contains("Fel") {
                    egui::Color32::RED
                } else if self.busy {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::LIGHT_GREEN
                };
                ui.label(egui::RichText::new(&self.status).color(status_color));

                let n = self.context_files.len();
                if n > 0 {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("{n} fil(er) i kontext"))
                            .color(egui::Color32::LIGHT_GREEN),
                    );
                } else {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("välj filer till vänster")
                            .color(egui::Color32::DARK_GRAY),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("? hjälp").clicked() {
                        self.panel = if self.panel == Panel::Help { Panel::None } else { Panel::Help };
                    }
                    if ui.small_button("g git log").clicked() {
                        self.panel = if self.panel == Panel::GitLog { Panel::None } else { Panel::GitLog };
                    }
                    if ui.small_button("d diff").clicked() {
                        self.panel = if self.panel == Panel::Diff { Panel::None } else { Panel::Diff };
                    }
                });
            });
        });

        // ── Bottom panel (diff / git log / help) ──────────────────────────────
        if self.panel != Panel::None {
            egui::TopBottomPanel::bottom("bottompanel")
                .resizable(true)
                .default_height(180.0)
                .show(ctx, |ui| {
                    match &self.panel {
                        Panel::Diff => {
                            ui.heading("Diff");
                            ui.separator();
                            egui::ScrollArea::both().show(ui, |ui| {
                                ui.add(egui::Label::new(
                                    egui::RichText::new(self.diff.as_str())
                                        .monospace()
                                        .color(egui::Color32::LIGHT_GREEN),
                                ));
                            });
                        }
                        Panel::GitLog => {
                            ui.heading("Git log");
                            ui.separator();
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for entry in &self.git_log {
                                    ui.label(
                                        egui::RichText::new(entry.as_str())
                                            .monospace()
                                            .color(egui::Color32::GOLD),
                                    );
                                }
                            });
                        }
                        Panel::Help => {
                            ui.heading("Hjälp");
                            ui.separator();
                            egui::Grid::new("helpgrid").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
                                ui.strong("Välj filer");
                                ui.label("Klicka i checkboxen bredvid filnamnet för att lägga till/ta bort");
                                ui.end_row();
                                ui.strong("Skicka");
                                ui.label("Skriv i textfältet längst ner → tryck Enter eller klicka Skicka →");
                                ui.end_row();
                                ui.strong("API-nyckel");
                                ui.label("export OPENAI_API_KEY=sk-...   (starta sedan om appen)");
                                ui.end_row();
                                ui.strong("Ollama");
                                ui.label("export OPENAI_BASE_URL=http://localhost:11434/v1");
                                ui.end_row();
                                ui.strong("Modell");
                                ui.label("export MATSUX_MODEL=gpt-4o-mini  (eller llama3, mistral, osv.)");
                                ui.end_row();
                                ui.strong("Stäng panel");
                                ui.label("Klicka på knappen igen nere till höger");
                                ui.end_row();
                            });
                        }
                        Panel::None => {}
                    }
                });
        }

        // ── Left panel: file browser ──────────────────────────────────────────
        egui::SidePanel::left("filebrowser")
            .resizable(true)
            .default_width(270.0)
            .max_width(320.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Filer");
                    ui.separator();
                    if ui.small_button("⟳ skanna").clicked() {
                        self.scan_files();
                    }
                    if ui.small_button("alla").clicked() {
                        let existing: HashSet<_> = self.context_files.iter().cloned().collect();
                        for f in &self.files {
                            if !existing.contains(f) {
                                self.context_files.push(f.clone());
                            }
                        }
                    }
                    if ui.small_button("rensa").clicked() {
                        self.context_files.clear();
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    // Collect toggle actions to avoid double-borrow.
                    let mut toggle: Option<PathBuf> = None;

                    for file in &self.files {
                        let in_ctx = self.context_files.contains(file);
                        let name = file.display().to_string();

                        ui.horizontal(|ui| {
                            let mut checked = in_ctx;
                            if ui.checkbox(&mut checked, "").changed() {
                                toggle = Some(file.clone());
                            }
                            let label = egui::RichText::new(&name)
                                .monospace()
                                .size(12.0)
                                .color(if in_ctx {
                                    egui::Color32::LIGHT_GREEN
                                } else {
                                    egui::Color32::from_gray(160)
                                });
                            // Clicking the name also toggles.
                            if ui.label(label).clicked() {
                                toggle = Some(file.clone());
                            }
                        });
                    }

                    if let Some(f) = toggle {
                        if let Some(pos) = self.context_files.iter().position(|x| x == &f) {
                            self.context_files.remove(pos);
                        } else {
                            self.context_files.push(f);
                        }
                    }

                    // ── Rust toolchain section ────────────────────────────
                    ui.add_space(8.0);
                    ui.separator();
                    ui.collapsing("🦀 Rust-verktygskedja", |ui| {
                        // Current toolchain from rust-toolchain.toml
                        ui.horizontal(|ui| {
                            ui.label("Kanal:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.toolchain_input)
                                    .desired_width(90.0)
                                    .hint_text("1.94.0 / stable / nightly"),
                            );
                            if ui.button("Spara").clicked() {
                                let ch = self.toolchain_input.trim().to_string();
                                if !ch.is_empty() {
                                    match cargo_runner::write_toolchain(&self.repo_path, &ch) {
                                        Ok(_) => {
                                            self.toolchain_channel = ch.clone();
                                            self.chat.push(ChatLine {
                                                role: "sys".into(),
                                                text: format!("✅ rust-toolchain.toml → channel = \"{ch}\""),
                                            });
                                        }
                                        Err(e) => {
                                            self.chat.push(ChatLine {
                                                role: "sys".into(),
                                                text: format!("⚠  Kunde inte skriva rust-toolchain.toml: {e}"),
                                            });
                                        }
                                    }
                                }
                            }
                        });

                        // Installed toolchains (from rustup toolchain list)
                        if !self.installed_toolchains.is_empty() {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Installerade:")
                                    .small()
                                    .color(egui::Color32::DARK_GRAY),
                            );
                            let toolchains = self.installed_toolchains.clone();
                            for tc in &toolchains {
                                let active = tc == &self.toolchain_channel;
                                let label = egui::RichText::new(tc)
                                    .monospace()
                                    .size(11.0)
                                    .color(if active {
                                        egui::Color32::LIGHT_GREEN
                                    } else {
                                        egui::Color32::GRAY
                                    });
                                if ui.selectable_label(active, label).clicked() {
                                    self.toolchain_input = tc.clone();
                                }
                            }
                        }
                    });

                    // ── API-inställningar ─────────────────────────────
                    ui.add_space(4.0);
                    ui.collapsing("🔑 API-inställningar", |ui| {
                        // Provider preset buttons
                        ui.label(egui::RichText::new("Leverantör:").small());
                        ui.horizontal_wrapped(|ui| {
                            for (i, p) in PROVIDERS.iter().enumerate() {
                                let active = i == self.selected_provider;
                                if ui
                                    .add(egui::SelectableLabel::new(active, p.name))
                                    .clicked()
                                {
                                    self.selected_provider = i;
                                    if !p.base_url.is_empty() {
                                        self.cfg.base_url = p.base_url.to_string();
                                    }
                                    if !p.key_hint.is_empty() && self.cfg.api_key.is_empty() {
                                        // Only set key hint if field is empty
                                    }
                                    // Set first model as default
                                    if let Some(m) = p.models.first() {
                                        self.cfg.model = m.to_string();
                                    }
                                }
                            }
                        });

                        ui.add_space(4.0);

                        // API-nyckel
                        ui.label(egui::RichText::new("API-nyckel:").small());
                        let hint = PROVIDERS
                            .get(self.selected_provider)
                            .map(|p| p.key_hint)
                            .unwrap_or("...");
                        ui.horizontal(|ui| {
                            let key_field = egui::TextEdit::singleline(&mut self.cfg.api_key)
                                .desired_width(210.0)
                                .hint_text(hint)
                                .password(!self.show_key);
                            ui.add(key_field);
                            if ui
                                .small_button(if self.show_key { "🙈" } else { "👁" })
                                .clicked()
                            {
                                self.show_key = !self.show_key;
                            }
                        });

                        // Bas-URL
                        ui.label(egui::RichText::new("Bas-URL:").small());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cfg.base_url)
                                .desired_width(240.0)
                                .hint_text("https://api.openai.com/v1"),
                        );

                        // Modell
                        ui.label(egui::RichText::new("Modell:").small());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cfg.model)
                                .desired_width(240.0)
                                .hint_text("gpt-4o"),
                        );

                        // Model shortcuts for current provider
                        let provider_models: Vec<&str> = PROVIDERS
                            .get(self.selected_provider)
                            .map(|p| p.models.to_vec())
                            .unwrap_or_default();
                        if !provider_models.is_empty() {
                            ui.add_space(2.0);
                            ui.horizontal_wrapped(|ui| {
                                for m in provider_models {
                                    let active = self.cfg.model == m;
                                    if ui
                                        .add(egui::SelectableLabel::new(
                                            active,
                                            egui::RichText::new(m).monospace().size(11.0),
                                        ))
                                        .clicked()
                                    {
                                        self.cfg.model = m.to_string();
                                    }
                                }
                            });
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("💾 Spara").clicked() {
                                match config::save(&self.cfg) {
                                    Ok(_) => {
                                        self.chat.push(ChatLine {
                                            role: "sys".into(),
                                            text: "✅ Inställningar sparade till ~/.matsux-term.toml".into(),
                                        });
                                    }
                                    Err(e) => {
                                        self.chat.push(ChatLine {
                                            role: "sys".into(),
                                            text: format!("⚠  Kunde inte spara: {e}"),
                                        });
                                    }
                                }
                            }
                            // Show key status
                            if self.cfg.api_key.is_empty() {
                                ui.colored_label(
                                    egui::Color32::RED,
                                    "⚠ Nyckel saknas",
                                );
                            } else {
                                ui.colored_label(
                                    egui::Color32::LIGHT_GREEN,
                                    "✓ Nyckel inställd",
                                );
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.collapsing("⚙ Cargo", |ui| {
                        let busy = self.cargo_busy || self.busy;

                        const CMDS: &[(&str, &[&str])] = &[
                            ("check",   &["check"]),
                            ("build",   &["build"]),
                            ("release", &["build", "--release"]),
                            ("test",    &["test"]),
                            ("clippy",  &["clippy"]),
                            ("clean",   &["clean"]),
                        ];

                        let mut send_cmd: Option<Vec<String>> = None;

                        ui.horizontal_wrapped(|ui| {
                            for (label, args) in CMDS {
                                if ui
                                    .add_enabled(
                                        !busy,
                                        egui::Button::new(
                                            egui::RichText::new(*label).monospace().size(12.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    send_cmd =
                                        Some(args.iter().map(|s| s.to_string()).collect());
                                }
                            }
                        });

                        if let Some(args) = send_cmd {
                            self.cargo_busy = true;
                            let _ = self.cargo_tx.try_send(CargoCmd {
                                repo_path: self.repo_path.clone(),
                                args,
                            });
                        }
                    });
                });
            });

        // ── Central panel: chat + input ───────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            let total_h = ui.available_height();
            let input_h = 50.0;

            // Chat history (scrollable, sticks to bottom).
            egui::ScrollArea::vertical()
                .max_height(total_h - input_h - 8.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.chat {
                        render_chat_line(ui, line);
                    }
                });

            ui.separator();

            // Input row.
            ui.horizontal(|ui| {
                let input_widget = egui::TextEdit::singleline(&mut self.input)
                    .desired_width(ui.available_width() - 90.0)
                    .hint_text(if self.busy {
                        "⏳ AI tänker…"
                    } else {
                        "Skriv din instruktion till AI…"
                    })
                    .font(egui::TextStyle::Monospace);

                let resp = ui.add_enabled(!self.busy, input_widget);

                // Auto-focus on start and after submit.
                if self.request_focus {
                    resp.request_focus();
                    self.request_focus = false;
                }

                // Enter key submits.
                if resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.submit();
                }

                if ui
                    .add_enabled(!self.busy, egui::Button::new("Skicka →"))
                    .clicked()
                {
                    self.submit();
                }
            });
        });
    }
}

// ─── Rendering helpers ────────────────────────────────────────────────────────

fn render_chat_line(ui: &mut egui::Ui, line: &ChatLine) {
    let (role_color, role_label) = match line.role.as_str() {
        "du"  => (egui::Color32::from_rgb(80, 200, 255), " Du "),
        "ai"  => (egui::Color32::GOLD, " AI "),
        _     => (egui::Color32::DARK_GRAY, "sys "),
    };

    ui.horizontal_top(|ui| {
        ui.label(egui::RichText::new(role_label).strong().color(role_color));
        ui.separator();

        // AI messages get monospace (looks like code).
        let text_style = if line.role == "ai" {
            egui::RichText::new(&line.text)
                .monospace()
                .size(12.5)
                .color(egui::Color32::LIGHT_YELLOW)
        } else if line.role == "du" {
            egui::RichText::new(&line.text)
                .size(14.0)
                .color(egui::Color32::WHITE)
        } else {
            egui::RichText::new(&line.text)
                .size(12.0)
                .color(egui::Color32::DARK_GRAY)
        };

        ui.add(egui::Label::new(text_style).wrap());
    });
    ui.add_space(4.0);
}

// ─── File scanner ─────────────────────────────────────────────────────────────

const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "jsx", "tsx", "go", "c", "cpp", "h", "hpp",
    "java", "kt", "cs", "rb", "php", "swift", "zig", "lua", "sh", "bash",
    "toml", "yaml", "yml", "json", "md", "txt",
];

const SKIP_DIRS: &[&str] = &[
    ".git", "target", "node_modules", ".cache", "dist", "build",
    "__pycache__", ".venv", "venv",
];

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') && name != ".env" { continue; }
        if path.is_dir() {
            if SKIP_DIRS.contains(&name) { continue; }
            walk_dir(root, &path, out, depth + 1);
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if CODE_EXTS.contains(&ext) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_path_buf());
                }
            }
        }
    }
}
