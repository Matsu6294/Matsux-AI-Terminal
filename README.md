# Matsux-AI-Terminal

**Standalone AI-kodassistent byggd i Rust 1.94.0** — öppnas med dubbelklick, inget terminal-fönster behövs.

Bygger på [aider](https://aider.chat)-konceptet men helt omskrivet i Rust med ett inbyggt GUI (egui/eframe). Stödjer alla OpenAI-kompatibla API:er — OpenAI, Groq, Mistral AI, Ollama (lokalt).

![Rust](https://img.shields.io/badge/Rust-1.94.0-orange?logo=rust)
![License](https://img.shields.io/badge/license-MIT-blue)

---

## Funktioner

- **Dubbelklickbar GUI-app** — inget terminalberoende
- **Inbyggd aider** — AI skriver, söker/ersätter och committar kod åt dig
- **Filväljare** — välj vilka filer som skickas som kontext till AI:n
- **API-inställningar** — byt modell/leverantör direkt i appen, sparas till `~/.matsux-term.toml`
- **Cargo-integration** — kör `check / build / release / test / clippy / clean` med ett klick
- **Rust-verktygskedja** — läs och skriv `rust-toolchain.toml`, välj toolchain-version i UI:t
- **Git-integration** — auto-commit efter varje AI-edit, diff- och loggpanel inbyggd
- **Streaming** — cargo-output streamas rad för rad till chattpanelen

---

## Snabbstart

### 1. Bygg från källkod

```bash
git clone https://github.com/Matsu6294/matsux-term.git
cd matsux-term
cargo build --release
# Binary: target/release/matsux-term
```

Kräver Rust 1.94.0 (hanteras av `rust-toolchain.toml`).

### 2. Välj API-leverantör

Starta appen och öppna **🔑 API-inställningar** i vänstra sidofältet.

| Leverantör | Kostnad | Hastighet | Modell (rekommenderad) |
|---|---|---|---|
| [Groq](https://console.groq.com) | Gratis | ~200 tok/s | `llama-3.3-70b-versatile` |
| [OpenAI](https://platform.openai.com) | Betald | ~100 tok/s | `gpt-4o` |
| [Mistral AI](https://console.mistral.ai) | Betald | ~100 tok/s | `codestral-latest` |
| [Ollama](https://ollama.com) (lokal) | Gratis | ~15 tok/s | `qwen2.5-coder:1.5b` |

#### Ollama (helt lokalt, ingen API-nyckel)

```bash
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen2.5-coder:1.5b
```

Välj sedan **Ollama (lokal)** i inställningspanelen.

### 3. Starta

```bash
# Öppna i ett specifikt repo:
./target/release/matsux-term /sökväg/till/ditt/projekt

# Eller dubbelklicka på binären
```

---

## Användning

### AI-kodning

1. Välj filer i **vänstra panelen** (checkbox) — dessa skickas som kontext
2. Skriv din instruktion i **textfältet** längst ner
3. Tryck **Enter** eller **Skicka →**
4. AI:n redigerar filerna och skapar ett git-commit automatiskt

### Rust-verktygskedja

Klicka **🦀 Rust-verktygskedja** i sidofältet:
- Ange version: `1.94.0`, `stable`, `nightly`, `beta`
- Klicka **Spara** → skriver `rust-toolchain.toml`
- Visar alla installerade toolchains (via `rustup toolchain list`)

### Cargo-kommandon

Klicka **⚙ Cargo** i sidofältet:

| Knapp | Kommando |
|---|---|
| check | `cargo check` |
| build | `cargo build` |
| release | `cargo build --release` |
| test | `cargo test` |
| clippy | `cargo clippy` |
| clean | `cargo clean` |

Output streamas direkt till chattpanelen med `🔴 error` / `🟡 warning`-markeringar.

### Paneler (knappar i statusfältet)

| Knapp | Visar |
|---|---|
| `d diff` | Senaste git-diff |
| `g git log` | De 10 senaste commits |
| `? hjälp` | Snabbreferens |

---

## Projektstruktur

```
matsux-term/
├── src/
│   ├── main.rs          # eframe setup, tokio runtime, kanaler
│   ├── app.rs           # GUI-state + egui rendering (eframe::App)
│   ├── aider.rs         # Async aider-loop: LLM → parse edits → apply → git commit
│   ├── cargo_runner.rs  # Cargo/rustup integration
│   └── config.rs        # ~/.matsux-term.toml persistence
├── Cargo.toml
└── rust-toolchain.toml  # Rust 1.94.0
```

### Beroenden (path deps)

matsux-term använder crates från [matsux-aider](../matsux-aider) som path-dependencies:

| Crate | Funktion |
|---|---|
| `llm` | `OpenAiClient` — HTTP mot valfri OpenAI-kompatibel API |
| `editor` | `parse_edits` + `apply_edits` — SEARCH/REPLACE format (portad från aider) |
| `git-ops` | `GitRepo` — commit, diff, log via libgit2 |

---

## Konfiguration

Sparas i `~/.matsux-term.toml`:

```toml
api_key = "sk-..."
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

Kan också sättas via miljövariabler (läses vid start):

```bash
export OPENAI_API_KEY=sk-...
export OPENAI_BASE_URL=https://api.groq.com/openai/v1
export MATSUX_MODEL=llama-3.3-70b-versatile
```

---

## Krav

- Rust 1.74+ (byggs med 1.94.0 via rust-toolchain.toml)
- Linux: `libGL`, `libX11` eller Wayland-bibliotek (standardpaket på de flesta skrivbordsmiljöer)
- Git (för auto-commit-funktionen)

---

## Licens

MIT
