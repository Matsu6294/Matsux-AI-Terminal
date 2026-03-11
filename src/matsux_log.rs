//! matsux-log — persistent aktivitetslogg.
//!
//! Sparas i /home/matsu/matsux-os/filer/matsux-log.json (maskinläsbar)
//! och       /home/matsu/matsux-os/filer/matsux-log.md  (läsbar för AI och människa)
//!
//! Varje post innehåller: tid, mål, filer som skapades/ändrades, resultat.
//! Loggen inkluderas i LLM-kontexten så AI:n alltid vet vad den gjort tidigare.

use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};

const LOG_JSON: &str = "/home/matsu/matsux-os/filer/matsux-log.json";
const LOG_MD:   &str = "/home/matsu/matsux-os/filer/matsux-log.md";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub goal:      String,
    pub files:     Vec<String>,
    pub result:    String, // "ok" | "fel: ..."
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn fmt_time(ts: u64) -> String {
    // Enkel UTC-formattering utan externa beroenden.
    let secs = ts % 60;
    let mins = (ts / 60) % 60;
    let hours = (ts / 3600) % 24;
    let days = ts / 86400;
    // Ungefärligt datum från Unix epoch (2 jan 1970 + days)
    format!("dag {} {:02}:{:02}:{:02} UTC", days, hours, mins, secs)
}

/// Läs alla loggposter från disk.
pub fn load_all() -> Vec<LogEntry> {
    std::fs::read_to_string(LOG_JSON)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Lägg till en ny post och skriv till disk.
pub fn append(goal: &str, files: &[&Path], result: &str) -> Result<()> {
    let entry = LogEntry {
        timestamp: now_secs(),
        goal:      goal.to_string(),
        files:     files.iter().map(|p| p.display().to_string()).collect(),
        result:    result.to_string(),
    };

    // Uppdatera JSON-loggen.
    let mut entries = load_all();
    entries.push(entry.clone());
    // Håll max 500 poster.
    if entries.len() > 500 {
        entries.drain(0..entries.len() - 500);
    }
    std::fs::create_dir_all(
        Path::new(LOG_JSON).parent().unwrap_or(Path::new(".")),
    )?;
    std::fs::write(LOG_JSON, serde_json::to_string_pretty(&entries)?)?;

    // Lägg till rad i markdown-loggen.
    let files_str = if entry.files.is_empty() {
        "–".to_string()
    } else {
        entry.files
            .iter()
            .map(|f| {
                Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(f)
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let md_line = format!(
        "| {} | {} | {} | {} |\n",
        fmt_time(entry.timestamp),
        entry.goal.chars().take(60).collect::<String>(),
        files_str,
        entry.result
    );

    // Skapa header om filen inte finns.
    if !Path::new(LOG_MD).exists() {
        std::fs::write(
            LOG_MD,
            "# matsux-log\n\n| Tid | Mål | Filer | Resultat |\n|-----|-----|-------|----------|\n",
        )?;
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(LOG_MD)?;
    f.write_all(md_line.as_bytes())?;

    Ok(())
}

/// Returnera en komprimerad sammanfattning av de senaste `n` posterna
/// att inkludera i LLM-kontexten.
pub fn summary_for_context(n: usize) -> String {
    let entries = load_all();
    if entries.is_empty() { return String::new(); }

    let recent: Vec<&LogEntry> = entries.iter().rev().take(n).collect();
    let mut out = String::from("### matsux-log (senaste aktiviteter)\n\n");
    for e in recent.iter().rev() {
        let files = if e.files.is_empty() {
            "–".to_string()
        } else {
            e.files
                .iter()
                .map(|f| {
                    Path::new(f)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(f)
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "- **{}** → filer: {} ({})\n",
            e.goal.chars().take(60).collect::<String>(),
            files,
            e.result
        ));
    }
    out.push('\n');
    out
}
