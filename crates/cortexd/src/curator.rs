//! The curator behind `brain.ask`.
//!
//! Retrieval already finds the right material most of the time; what it cannot do is *decide*.
//! It returns thirty candidates in a defensible order, and an agent then reads ten of them to find
//! the two that answer its question — which is exactly the token cost this memory exists to save.
//! The curator does that reading once, locally, and hands back the few that matter with a line
//! each saying why.
//!
//! Two modes, and the second is not a degraded first:
//!
//! - **model** — a small local language model, reached over an OpenAI-compatible endpoint on the
//!   loopback interface, is shown the question and the numbered candidates and answers with the
//!   numbers it picks. Its output is constrained by a JSON schema, so a two-billion-parameter
//!   model cannot answer in prose. It may only *choose*: an item it names must be one of the
//!   candidates, and a number it invents is dropped.
//! - **ranked** — the model is off, unreachable, slow or confused: the fused, reranked order
//!   stands and the top items are returned as they are. The note says which of those it was.
//!
//! Nothing here is stored. The call is audited like every other tool call.

use std::time::{Duration, Instant};

use cortex_core::config::CuratorConfig;
use cortex_rpc::protocol::{AskItem, AskResult, CuratorInfo};

/// One thing retrieval found, as the curator is shown it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: &'static str,
    pub path: Option<String>,
    pub memory_id: Option<i64>,
    pub title: String,
    pub snippet: Option<String>,
    pub zone: Option<String>,
}

impl Candidate {
    fn item(&self, why: Option<String>) -> AskItem {
        AskItem {
            kind: self.kind.to_string(),
            path: self.path.clone(),
            memory_id: self.memory_id,
            title: self.title.clone(),
            why,
            snippet: self.snippet.clone(),
            zone: self.zone.clone(),
        }
    }
}

/// Characters of each candidate's passage the model sees. Enough for a paragraph.
const PASSAGE_CHARS: usize = 360;

fn shorten(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max).collect();
    out.push('…');
    out
}

const SYSTEM: &str = "You are the curator of a personal knowledge index. You are given a question \
and numbered candidates (files and remembered facts) that a search engine found. Choose the few \
candidates that actually answer the question or are the place to look, best first, in `choice`. \
Never choose a candidate only because it shares words with the question. For each chosen candidate \
write a reason of at most twelve words, in the language of the question. Then, in `summary`, answer \
the question itself in one to three sentences using only what the chosen passages say, citing them \
like [2]. Never add facts that are not in a passage; if the passages do not contain the answer, name \
the file to read instead.";

fn prompt(question: &str, candidates: &[Candidate], limit: usize) -> String {
    let mut out = format!("Question: {question}\nPick at most {limit}.\n\nCandidates:\n");
    for (i, c) in candidates.iter().enumerate() {
        let place = c.path.as_deref().unwrap_or("(memory)");
        out.push_str(&format!("[{}] {} — {}", i + 1, c.kind, place));
        if let Some(zone) = &c.zone {
            out.push_str(&format!(" (project: {zone})"));
        }
        out.push('\n');
        if let Some(s) = &c.snippet {
            out.push_str("    ");
            out.push_str(&shorten(s, PASSAGE_CHARS));
            out.push('\n');
        }
    }
    out
}

/// The keys are named so that their alphabetical order is the order they must be written in:
/// `serde_json` sorts object keys, llama.cpp's grammar follows the order it receives, and a model
/// that writes its summary before its choice has already decided without choosing.
fn schema(limit: usize) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "choice": {
                "type": "array",
                "minItems": 1,
                "maxItems": limit,
                "items": {
                    "type": "object",
                    "properties": {
                        "n": { "type": "integer" },
                        "why": { "type": "string" }
                    },
                    "required": ["n", "why"]
                }
            },
            "summary": { "type": "string" }
        },
        "required": ["choice", "summary"]
    })
}

#[derive(Debug, serde::Deserialize)]
struct Pick {
    n: i64,
    #[serde(default)]
    why: String,
}

#[derive(Debug, serde::Deserialize)]
struct Decision {
    #[serde(default, rename = "choice")]
    picks: Vec<Pick>,
    #[serde(default, rename = "summary")]
    answer: String,
}

/// Candidate numbers the summary cites, like `[2]`, in order of first mention.
fn cited(answer: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        rest = &rest[open + 1..];
        if let Some(close) = rest.find(']') {
            if let Ok(n) = rest[..close].trim().parse::<i64>() {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
            rest = &rest[close..];
        }
    }
    out
}

/// Ask the model. Every failure is a string for the note, never an error for the caller.
fn call_model(
    config: &CuratorConfig,
    question: &str,
    candidates: &[Candidate],
    limit: usize,
) -> Result<(Decision, Option<String>), String> {
    let body = serde_json::json!({
        "model": config.model,
        "temperature": 0,
        "max_tokens": 120 + 40 * limit,
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": prompt(question, candidates, limit) }
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "curation", "schema": schema(limit) }
        },
        // Reasoning models spend the whole budget thinking unless told not to; the choice here
        // is a reading task, not a reasoning one.
        "chat_template_kwargs": { "enable_thinking": false }
    });

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(config.timeout_ms)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(&config.endpoint)
        .send_json(&body)
        .map_err(|e| format!("decision model unreachable: {e}"))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("decision model answer unreadable: {e}"))?;
    if status != 200 {
        return Err(format!("decision model answered HTTP {status}: {}", shorten(&text, 160)));
    }
    let reply: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("decision model answer is not JSON: {e}"))?;
    let model = reply.get("model").and_then(|m| m.as_str()).map(|s| s.to_string());
    let content = reply
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "decision model answer has no content".to_string())?;
    let decision: Decision = serde_json::from_str(content.trim())
        .map_err(|e| format!("decision model broke the schema: {e}"))?;
    Ok((decision, model))
}

/// Turn a decision into items: only numbers that name a candidate, each at most once. A summary
/// that cites candidates the choice left out adds them after it — the model read them.
fn apply(decision: &Decision, candidates: &[Candidate], limit: usize) -> Vec<AskItem> {
    let mut used = std::collections::HashSet::new();
    let citations: Vec<Pick> = cited(&decision.answer)
        .into_iter()
        .map(|n| Pick { n, why: String::new() })
        .collect();
    decision
        .picks
        .iter()
        .chain(citations.iter())
        .filter_map(|p| {
            let index = usize::try_from(p.n).ok()?.checked_sub(1)?;
            let c = candidates.get(index)?;
            used.insert(index).then(|| {
                let why = p.why.trim();
                c.item((!why.is_empty()).then(|| shorten(why, 120)))
            })
        })
        .take(limit)
        .collect()
}

/// The block an agent reads. Paths first, because a path is what saves the search.
pub fn render(question: &str, answer: Option<&str>, items: &[AskItem], info: &CuratorInfo) -> String {
    let mut out = format!("brain_ask: {question}\n");
    if let Some(a) = answer {
        out.push_str(&format!("Answer: {a}\n"));
    }
    if items.is_empty() && answer.is_none() {
        out.push_str("Nothing in the index answers this. Search the files directly.\n");
    }
    for (i, item) in items.iter().enumerate() {
        let place = match (&item.path, item.memory_id) {
            (Some(p), _) => p.clone(),
            (None, Some(id)) => format!("memory #{id}"),
            _ => item.title.clone(),
        };
        out.push_str(&format!("[{}] {}", i + 1, place));
        if let Some(why) = &item.why {
            out.push_str(&format!(" — {why}"));
        }
        out.push('\n');
        if let Some(s) = &item.snippet {
            out.push_str(&format!("    {}\n", shorten(s, 220)));
        }
    }
    let how = match info.mode.as_str() {
        "model" => format!(
            "chosen by {} from {} candidates in {} ms",
            info.model.as_deref().unwrap_or("the decision model"),
            info.candidates,
            info.retrieval_ms + info.model_ms
        ),
        _ => format!(
            "ranked order of {} candidates, {} ms{}",
            info.candidates,
            info.retrieval_ms,
            info.note.as_deref().map(|n| format!(" ({n})")).unwrap_or_default()
        ),
    };
    out.push_str(&format!("({how})\n"));
    out
}

/// Whether the decision model is there, and whether it is asleep. Blocking, and quick: a local
/// server answers in milliseconds, and one that does not within half a second is reported as
/// unreachable rather than waited for.
pub fn health(config: &CuratorConfig) -> cortex_rpc::protocol::CuratorService {
    let mut out = cortex_rpc::protocol::CuratorService {
        enabled: config.enabled,
        reachable: false,
        sleeping: None,
        model: None,
    };
    if !config.enabled || !config.endpoint_is_loopback() {
        return out;
    }
    // `http://host:port/v1/chat/completions` -> `http://host:port/props`
    let base = match config.endpoint.find("/v1/") {
        Some(i) => &config.endpoint[..i],
        None => config.endpoint.trim_end_matches('/'),
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(500)))
        .http_status_as_error(false)
        .build()
        .into();
    let Ok(mut response) = agent.get(&format!("{base}/props")).call() else {
        return out;
    };
    out.reachable = response.status().as_u16() == 200;
    if let Ok(text) = response.body_mut().read_to_string() {
        if let Ok(props) = serde_json::from_str::<serde_json::Value>(&text) {
            out.sleeping = props.get("is_sleeping").and_then(|v| v.as_bool());
            out.model = props
                .get("model_path")
                .and_then(|v| v.as_str())
                .map(|p| p.rsplit('/').next().unwrap_or(p).trim_end_matches(".gguf").to_string());
        }
    }
    out
}

/// Decide. Blocking: call it from `spawn_blocking`.
pub fn decide(
    config: &CuratorConfig,
    question: &str,
    candidates: Vec<Candidate>,
    limit: usize,
    use_model: bool,
    retrieval_ms: u64,
) -> AskResult {
    let limit = limit.clamp(1, 12);
    let shown: Vec<Candidate> = candidates.into_iter().take(config.candidates.max(limit)).collect();

    let ranked = |note: Option<String>| -> (Vec<AskItem>, CuratorInfo) {
        (
            shown.iter().take(limit).map(|c| c.item(None)).collect(),
            CuratorInfo {
                mode: "ranked".into(),
                model: None,
                note,
                candidates: shown.len(),
                retrieval_ms,
                model_ms: 0,
            },
        )
    };

    let (answer, items, info) = if shown.is_empty() {
        let (items, info) = ranked(None);
        (None, items, info)
    } else if !use_model {
        let (items, info) = ranked(Some("the caller asked for retrieval only".into()));
        (None, items, info)
    } else if !config.enabled {
        let (items, info) = ranked(Some("no decision model is configured".into()));
        (None, items, info)
    } else if !config.endpoint_is_loopback() {
        let (items, info) = ranked(Some("the curator endpoint is not a loopback address".into()));
        (None, items, info)
    } else {
        let started = Instant::now();
        match call_model(config, question, &shown, limit) {
            Ok((decision, reported)) => {
                // A server that reports its model as a file path says nothing a reader wants.
                let model = reported.filter(|m| !m.contains('/'));
                let model_ms = started.elapsed().as_millis() as u64;
                let items = apply(&decision, &shown, limit);
                let answer = decision.answer.trim();
                // A model that picked nothing although retrieval found candidates is not
                // trusted to have read them: the ranked list is returned and the refusal noted.
                if items.is_empty() && answer.is_empty() {
                    let (items, mut info) = ranked(Some("the decision model picked nothing".into()));
                    info.model_ms = model_ms;
                    (None, items, info)
                } else {
                    (
                        (!answer.is_empty()).then(|| shorten(answer, 600)),
                        items,
                        CuratorInfo {
                            mode: "model".into(),
                            model: model.or_else(|| Some(config.model.clone())),
                            note: None,
                            candidates: shown.len(),
                            retrieval_ms,
                            model_ms,
                        },
                    )
                }
            }
            Err(note) => {
                tracing::info!(%note, "curator fell back to the ranked order");
                let (items, mut info) = ranked(Some(note));
                info.model_ms = started.elapsed().as_millis() as u64;
                (None, items, info)
            }
        }
    };

    let text = render(question, answer.as_deref(), &items, &info);
    AskResult {
        question: question.to_string(),
        answer,
        items,
        curator: info,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(n: usize) -> Vec<Candidate> {
        (0..n)
            .map(|i| Candidate {
                kind: "file",
                path: Some(format!("/p/{i}.md")),
                memory_id: None,
                title: format!("{i}.md"),
                snippet: Some(format!("passage {i}")),
                zone: None,
            })
            .collect()
    }

    #[test]
    fn an_invented_or_repeated_number_is_dropped() {
        let d = Decision {
            picks: vec![
                Pick { n: 2, why: "b".into() },
                Pick { n: 99, why: "x".into() },
                Pick { n: 0, why: "x".into() },
                Pick { n: -3, why: "x".into() },
                Pick { n: 2, why: "again".into() },
                Pick { n: 1, why: "a".into() },
            ],
            answer: String::new(),
        };
        let items = apply(&d, &cands(3), 5);
        let paths: Vec<_> = items.iter().map(|i| i.path.clone().unwrap()).collect();
        assert_eq!(paths, vec!["/p/1.md", "/p/0.md"]);
        assert_eq!(items[0].why.as_deref(), Some("b"));
    }

    #[test]
    fn a_summary_cites_what_the_choice_left_out() {
        let d = Decision {
            picks: vec![Pick { n: 3, why: "c".into() }],
            answer: "Siehe [1] und [3], nicht [x] oder [7].".into(),
        };
        let items = apply(&d, &cands(4), 5);
        let paths: Vec<_> = items.iter().map(|i| i.path.clone().unwrap()).collect();
        assert_eq!(paths, vec!["/p/2.md", "/p/0.md"]);
        assert_eq!(cited("a [2] b [2] c [10]"), vec![2, 10]);
    }

    #[test]
    fn the_schema_writes_the_choice_before_the_summary() {
        let text = serde_json::to_string(&schema(3)).unwrap();
        assert!(text.find("\"choice\"").unwrap() < text.find("\"summary\"").unwrap());
    }

    #[test]
    fn without_a_model_the_ranked_order_stands_and_says_why() {
        let config = CuratorConfig::default();
        let r = decide(&config, "q", cands(8), 3, true, 12);
        assert_eq!(r.curator.mode, "ranked");
        assert_eq!(r.items.len(), 3);
        assert_eq!(r.items[0].path.as_deref(), Some("/p/0.md"));
        assert!(r.curator.note.as_deref().unwrap().contains("no decision model"));
        assert!(r.text.contains("/p/0.md"));
    }

    #[test]
    fn an_unreachable_model_falls_back_quickly() {
        let config = CuratorConfig {
            enabled: true,
            // Port 9 (discard) on loopback: nothing listens, the connection is refused at once.
            endpoint: "http://127.0.0.1:9/v1/chat/completions".into(),
            timeout_ms: 2_000,
            ..CuratorConfig::default()
        };
        let started = Instant::now();
        let r = decide(&config, "q", cands(4), 2, true, 5);
        assert_eq!(r.curator.mode, "ranked");
        assert!(r.curator.note.as_deref().unwrap().contains("unreachable"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn nothing_found_is_said_plainly() {
        let r = decide(&CuratorConfig::default(), "q", Vec::new(), 3, true, 1);
        assert!(r.items.is_empty());
        assert!(r.text.contains("Nothing in the index answers this"));
    }

    #[test]
    fn the_prompt_numbers_candidates_from_one_and_shortens_passages() {
        let mut c = cands(2);
        c[1].snippet = Some("word ".repeat(400));
        let p = prompt("where?", &c, 2);
        assert!(p.contains("[1] file — /p/0.md"));
        assert!(p.contains("[2] file — /p/1.md"));
        assert!(p.lines().all(|l| l.chars().count() < PASSAGE_CHARS + 40));
    }
}
