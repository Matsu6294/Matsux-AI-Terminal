//! Lokal kunskapsbas (RAG) — sparar och söker kodexempel.
//!
//! Filer sparas i /home/matsu/matsux-os/filer/kb/
//! Format: kb/<timestamp>_<stem>.<ext>
//! Metadata: kb/<timestamp>_<stem>.meta.json

use std::path::{Path, PathBuf};
use anyhow::Result;

pub const KB_DIR: &str = "/home/matsu/matsux-os/filer/kb";

#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    goal: String,
    language: String,
    timestamp: u64,
}

/// Spara en genererad fil till kunskapsbasen.
pub fn save(goal: &str, file_path: &Path, content: &str) -> Result<()> {
    let kb = PathBuf::from(KB_DIR);
    std::fs::create_dir_all(&kb)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("txt");
    let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("fil");
    let lang = ext.to_string();

    let code_file = kb.join(format!("{ts}_{stem}.{ext}"));
    let meta_file = kb.join(format!("{ts}_{stem}.meta.json"));

    std::fs::write(&code_file, content)?;
    let meta = Meta { goal: goal.to_string(), language: lang, timestamp: ts };
    std::fs::write(&meta_file, serde_json::to_string_pretty(&meta)?)?;

    Ok(())
}

/// Sök i kunskapsbasen efter relevanta exempel baserat på mål och språk.
/// Returnerar max `limit` kodsnippets som kontext-sträng.
pub fn search(goal: &str, limit: usize) -> String {
    let kb = PathBuf::from(KB_DIR);
    if !kb.exists() { return String::new(); }

    let goal_words: Vec<&str> = goal.split_whitespace().collect();

    let mut hits: Vec<(usize, PathBuf)> = std::fs::read_dir(&kb)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x != "json")
                .unwrap_or(false)
        })
        .filter_map(|entry| {
            let code_path = entry.path();
            let stem = code_path.file_stem()?.to_str()?.to_string();

            // Läs metadata om den finns.
            let meta_path = kb.join(format!("{}.meta.json", stem));
            let meta_goal = std::fs::read_to_string(&meta_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Meta>(&s).ok())
                .map(|m| m.goal)
                .unwrap_or_default();

            // Räkna nyckelordsträffar.
            let combined = format!("{stem} {meta_goal}").to_lowercase();
            let score = goal_words
                .iter()
                .filter(|w| w.len() > 3 && combined.contains(&w.to_lowercase()))
                .count();

            if score > 0 { Some((score, code_path)) } else { None }
        })
        .collect();

    hits.sort_by(|a, b| b.0.cmp(&a.0));
    hits.truncate(limit);

    if hits.is_empty() { return String::new(); }

    let mut out = String::from("### Relevanta exempel från kunskapsbasen\n\n");
    for (_, path) in &hits {
        if let Ok(code) = std::fs::read_to_string(path) {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            out.push_str(&format!("#### {name}\n```{ext}\n{code}\n```\n\n"));
        }
    }
    out
}
