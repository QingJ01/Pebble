//! Evaluate Pebble's smart notifications on the synthetic sets in
//! `crates/pebble-notify/eval/`.
//!
//! ```text
//! cargo run -p pebble-notify-eval                  # the shipped rules
//! cargo run -p pebble-notify-eval -- --model       # rules + your model
//! cargo run -p pebble-notify-eval -- --model --out eval-report.md
//! ```
//!
//! With `--model`, the model comes from the environment (never printed):
//! `PEBBLE_EVAL_ENDPOINT` (an OpenAI-compatible base URL),
//! `PEBBLE_EVAL_MODEL` and `PEBBLE_EVAL_API_KEY` (optional for local
//! servers). Prompts, parsing and the final decision are the app's own
//! (`pebble_notify::triage_model`, `apply_model_verdict`), so the numbers
//! describe what ships.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use pebble_notify::eval::{
    bundled_awaiting_cases, bundled_deadline_set, bundled_notify_cases, deadline_outcome,
    rule_deadline, score_triage, smart_prefs, tier_name, BinaryScore, DeadlineCase,
    DeadlineOutcome, DeadlineScore, NotifyCase, TriageScore, EVAL_MINUTE,
};
use pebble_notify::triage_model::{
    parse_response, prepare_body, system_prompt, user_prompt, TriageAnswer, TriageItem,
    MIN_CONFIDENCE,
};
use pebble_notify::{apply_model_verdict, is_automated, Decision, Reason, Tier};
use serde_json::{json, Value};

/// The app waits this long for the model before using the rules.
const SHIPPED_TIMEOUT: Duration = Duration::from_secs(8);

struct Options {
    model: bool,
    out: Option<String>,
    batch: usize,
    timeout: Duration,
    bulk: bool,
}

fn options() -> Options {
    let mut options = Options {
        model: false,
        out: None,
        batch: 10,
        timeout: Duration::from_secs(60),
        bulk: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => options.model = true,
            "--out" => options.out = args.next(),
            "--batch" => {
                options.batch = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n: &usize| *n > 0)
                    .unwrap_or(10)
            }
            "--timeout" => {
                options.timeout =
                    Duration::from_secs(args.next().and_then(|v| v.parse().ok()).unwrap_or(60))
            }
            "--no-bulk" => options.bulk = false,
            "-h" | "--help" => {
                println!(
                    "pebble-notify-eval [--model] [--out FILE] [--batch N] [--timeout SECS] [--no-bulk]\n\
                     --model uses PEBBLE_EVAL_ENDPOINT, PEBBLE_EVAL_MODEL, PEBBLE_EVAL_API_KEY"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown option {other} (see --help)");
                std::process::exit(2);
            }
        }
    }
    options
}

// ------------------------------------------------------------------ model

struct Model {
    client: reqwest::Client,
    url: String,
    name: String,
    api_key: String,
}

impl Model {
    fn from_env(timeout: Duration) -> Result<Self, String> {
        let endpoint =
            std::env::var("PEBBLE_EVAL_ENDPOINT").map_err(|_| "PEBBLE_EVAL_ENDPOINT is not set")?;
        let name =
            std::env::var("PEBBLE_EVAL_MODEL").map_err(|_| "PEBBLE_EVAL_MODEL is not set")?;
        let api_key = std::env::var("PEBBLE_EVAL_API_KEY").unwrap_or_default();
        let trimmed = endpoint.trim().trim_end_matches('/');
        // Same rule as the app's `chat_completions_url`.
        let url = if trimmed.ends_with("/chat/completions") {
            trimmed.to_string()
        } else if trimmed.ends_with("/v1") {
            format!("{trimmed}/chat/completions")
        } else {
            format!("{trimmed}/v1/chat/completions")
        };
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            url,
            name,
            api_key,
        })
    }

    /// One triage request, as the app sends it. Returns the reply text and
    /// how long it took.
    async fn triage(
        &self,
        now_local: &str,
        items: &[TriageItem],
    ) -> (Result<String, String>, Duration) {
        let body = json!({
            "model": self.name,
            "messages": [
                {"role": "system", "content": system_prompt(now_local)},
                {"role": "user", "content": user_prompt(items, &[])},
            ],
            "temperature": 0.2,
        });
        let started = Instant::now();
        let mut request = self.client.post(&self.url).json(&body);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let result = async {
            let response = request.send().await.map_err(|e| e.to_string())?;
            let status = response.status();
            let value: Value = response.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!(
                    "HTTP {status}: {}",
                    value.to_string().chars().take(200).collect::<String>()
                ));
            }
            Ok(value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string())
        }
        .await;
        (result, started.elapsed())
    }
}

/// Answers for a list of items, sent in batches. Each answer carries the
/// batch latency; failed batches give no answers.
struct BatchRun {
    answers: Vec<Option<TriageAnswer>>,
    latency: Vec<Option<Duration>>,
    errors: Vec<String>,
    batches: Vec<Duration>,
}

async fn run_batches(
    model: &Model,
    now_local: &str,
    items: Vec<TriageItem>,
    batch: usize,
) -> BatchRun {
    let mut run = BatchRun {
        answers: vec![None; items.len()],
        latency: vec![None; items.len()],
        errors: Vec::new(),
        batches: Vec::new(),
    };
    for (chunk_index, chunk) in items.chunks(batch).enumerate() {
        let offset = chunk_index * batch;
        let local: Vec<TriageItem> = chunk
            .iter()
            .enumerate()
            .map(|(i, item)| TriageItem {
                id: (i + 1).to_string(),
                ..item.clone()
            })
            .collect();
        let ids: Vec<String> = local.iter().map(|i| i.id.clone()).collect();
        let (reply, took) = model.triage(now_local, &local).await;
        run.batches.push(took);
        eprintln!(
            "  batch {} ({} items): {:.1}s",
            chunk_index + 1,
            local.len(),
            took.as_secs_f64()
        );
        match reply.and_then(|text| parse_response(&text, &ids)) {
            Ok(mut answers) => {
                for i in 0..local.len() {
                    run.answers[offset + i] = answers.remove(&(i + 1).to_string());
                    run.latency[offset + i] = Some(took);
                }
                let missing = (0..local.len())
                    .filter(|i| run.answers[offset + i].is_none())
                    .count();
                if missing > 0 {
                    run.errors.push(format!(
                        "batch {}: {missing} items without a usable answer",
                        chunk_index + 1
                    ));
                }
            }
            Err(e) => run.errors.push(format!("batch {}: {e}", chunk_index + 1)),
        }
    }
    run
}

// ----------------------------------------------------------------- triage

fn item_for(case: &NotifyCase) -> TriageItem {
    let facts = case.facts();
    TriageItem {
        id: String::new(),
        from: case.mail.from_address.clone(),
        from_name: case.mail.from_name.clone(),
        subject: case.mail.subject.clone(),
        to_me: case.mail.to_me,
        known_contact: case.facts.favorite || facts.sender_known,
        times_user_wrote: case.facts.times_user_wrote,
        automated: is_automated(&facts),
        body: Some(prepare_body(&case.mail.body)),
    }
}

fn settle(decision: &Decision, answer: Option<&TriageAnswer>, bulk: bool) -> Decision {
    match answer {
        Some(answer) => apply_model_verdict(
            decision,
            &answer.verdict,
            &smart_prefs(bulk),
            EVAL_MINUTE,
            MIN_CONFIDENCE,
        ),
        None => Decision {
            reason: Reason::ModelFallback,
            model_candidate: false,
            ..decision.clone()
        },
    }
}

fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

fn triage_table(out: &mut String, rows: &[(&str, &TriageScore)]) {
    let _ = writeln!(
        out,
        "| | Accuracy | Notify precision | Notify recall | F1 | Missed | Noisy | Lost |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|");
    for (name, s) in rows {
        let _ = writeln!(
            out,
            "| {name} | {} ({}/{}) | {} | {} | {:.2} | {} | {} | {} |",
            pct(s.accuracy()),
            s.correct(),
            s.total(),
            pct(s.interrupt_precision()),
            pct(s.interrupt_recall()),
            s.interrupt_f1(),
            s.missed(),
            s.noisy(),
            s.lost()
        );
    }
}

fn deadline_row(out: &mut String, name: &str, s: &DeadlineScore) {
    let _ = writeln!(
        out,
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {} |",
        s.exact,
        s.same_day,
        s.wrong_day,
        s.missed,
        s.false_positive,
        s.correct_none,
        pct(s.day_recall()),
        pct(s.day_precision())
    );
}

fn fmt_due(due: Option<NaiveDateTime>) -> String {
    due.map(|d| d.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

fn outcome_name(o: DeadlineOutcome) -> &'static str {
    match o {
        DeadlineOutcome::Exact => "exact",
        DeadlineOutcome::SameDay => "same day",
        DeadlineOutcome::WrongDay => "wrong day",
        DeadlineOutcome::Missed => "missed",
        DeadlineOutcome::FalsePositive => "false positive",
        DeadlineOutcome::CorrectNone => "ok",
    }
}

fn snippet(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s: String = one.chars().take(48).collect();
    if one.chars().count() > 48 {
        s.push('…');
    }
    s.replace('|', "/")
}

#[tokio::main]
async fn main() {
    let opts = options();
    let cases = bundled_notify_cases();
    let deadline_set = bundled_deadline_set();
    let awaiting = bundled_awaiting_cases();
    let received = deadline_set.received_at();
    let now_local = received.format("%Y-%m-%d %H:%M (%A)").to_string();

    let model = if opts.model {
        match Model::from_env(opts.timeout) {
            Ok(model) => Some(model),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    } else {
        None
    };

    let mut out = String::new();
    let _ = writeln!(out, "# Smart notification evaluation\n");
    let _ = writeln!(
        out,
        "Synthetic data: {} triage cases, {} deadline cases, {} awaiting-reply cases (zh and en). Mode: Smart.{}\n",
        cases.len(),
        deadline_set.cases.len(),
        awaiting.len(),
        match &model {
            Some(m) => format!(" Model: `{}`, batches of {}.", m.name, opts.batch),
            None => String::new(),
        }
    );

    // ---- triage: rules
    let prefs = smart_prefs(opts.bulk);
    let rule_decisions: Vec<Decision> = cases.iter().map(|c| c.decide(&prefs)).collect();
    let rule_tiers: Vec<Tier> = rule_decisions.iter().map(|d| d.tier).collect();
    let (rules_score, rules_by_cat) = score_triage(&cases, &rule_tiers);
    let candidates: Vec<usize> = (0..cases.len())
        .filter(|&i| rule_decisions[i].model_candidate)
        .collect();

    // ---- triage: model
    let mut model_tiers: Option<(Vec<Tier>, Vec<Tier>)> = None;
    let mut model_notes: HashMap<usize, String> = HashMap::new();
    let mut latencies: Vec<Duration> = Vec::new();
    let mut model_errors: Vec<String> = Vec::new();
    if let Some(model) = &model {
        eprintln!("Triage: {} candidates for the model", candidates.len());
        let items: Vec<TriageItem> = candidates.iter().map(|&i| item_for(&cases[i])).collect();
        let run = run_batches(model, &now_local, items, opts.batch).await;
        latencies.extend(run.batches.iter().copied());
        model_errors.extend(run.errors.iter().map(|e| format!("triage {e}")));
        let mut unconstrained = rule_tiers.clone();
        let mut shipped = rule_tiers.clone();
        for (k, &i) in candidates.iter().enumerate() {
            let answer = run.answers[k].as_ref();
            unconstrained[i] = settle(&rule_decisions[i], answer, opts.bulk).tier;
            let in_time = run.latency[k].is_some_and(|l| l <= SHIPPED_TIMEOUT);
            shipped[i] = settle(&rule_decisions[i], answer.filter(|_| in_time), opts.bulk).tier;
            if let Some(a) = answer {
                model_notes.insert(
                    i,
                    format!(
                        "{} {:.2}{}",
                        a.verdict.label.code(),
                        a.verdict.confidence,
                        a.verdict
                            .why
                            .as_deref()
                            .map(|w| format!(" “{w}”"))
                            .unwrap_or_default()
                    ),
                );
            }
        }
        model_tiers = Some((unconstrained, shipped));
    }

    let _ = writeln!(out, "## Notification triage\n");
    let _ = writeln!(
        out,
        "Expected is what a careful person would want. **Missed** = deserved a notification and didn't get one; **Noisy** = notified although it could wait; **Lost** = mail that should reach you went silent (must be 0). {} of {} messages are ones the rules can't place and would go to the model{}.\n",
        candidates.len(),
        cases.len(),
        if opts.bulk { ", including bulk mail (\"Also check newsletters\" on)" } else { "" }
    );
    let mut rows: Vec<(&str, TriageScore)> =
        vec![("Rules only (shipped default)", rules_score.clone())];
    let mut by_cat_model: Option<std::collections::BTreeMap<String, TriageScore>> = None;
    if let Some((unconstrained, shipped)) = &model_tiers {
        let (s_un, by_cat) = score_triage(&cases, unconstrained);
        let (s_sh, _) = score_triage(&cases, shipped);
        rows.push(("Rules + model", s_un));
        rows.push(("Rules + model, 8 s limit (as shipped)", s_sh));
        by_cat_model = Some(by_cat);
    }
    let refs: Vec<(&str, &TriageScore)> = rows.iter().map(|(n, s)| (*n, s)).collect();
    triage_table(&mut out, &refs);

    let _ = writeln!(out, "\n### By category\n");
    let _ = writeln!(
        out,
        "| Category | Cases | Rules correct |{}",
        if by_cat_model.is_some() {
            " Model correct |"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "|---|---|---|{}",
        if by_cat_model.is_some() { "---|" } else { "" }
    );
    for (cat, s) in &rules_by_cat {
        let model_cell = by_cat_model
            .as_ref()
            .and_then(|m| m.get(cat))
            .map(|m| format!(" {}/{} |", m.correct(), m.total()))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "| {cat} | {} | {}/{} |{model_cell}",
            s.total(),
            s.correct(),
            s.total()
        );
    }

    let _ = writeln!(out, "\n### Disagreements with the expected tier\n");
    let _ = writeln!(
        out,
        "| Case | Category | Mail | Expected | Rules |{}",
        if model_tiers.is_some() {
            " Model | Model said |"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "|---|---|---|---|---|{}",
        if model_tiers.is_some() {
            "---|---|"
        } else {
            ""
        }
    );
    for (i, case) in cases.iter().enumerate() {
        let model_tier = model_tiers.as_ref().map(|(u, _)| u[i]);
        if rule_tiers[i] == case.expected && model_tier.is_none_or(|t| t == case.expected) {
            continue;
        }
        let model_cells = match model_tier {
            Some(t) => format!(
                " {} | {} |",
                tier_name(t),
                model_notes
                    .get(&i)
                    .map(|n| snippet(n))
                    .unwrap_or_else(|| "—".into())
            ),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "| {} | {} | {}: {} | {} | {} |{model_cells}",
            case.id,
            case.category,
            snippet(&case.mail.from_name),
            snippet(&case.mail.subject),
            tier_name(case.expected),
            tier_name(rule_tiers[i]),
        );
    }

    // ---- deadlines
    let rule_due: Vec<Option<NaiveDateTime>> = deadline_set
        .cases
        .iter()
        .map(|c| rule_deadline(c, received))
        .collect();
    let mut model_due: Option<Vec<Option<NaiveDateTime>>> = None;
    if let Some(model) = &model {
        eprintln!("Deadlines: {} cases", deadline_set.cases.len());
        let items: Vec<TriageItem> = deadline_set.cases.iter().map(deadline_item).collect();
        let run = run_batches(model, &now_local, items, opts.batch).await;
        latencies.extend(run.batches.iter().copied());
        model_errors.extend(run.errors.iter().map(|e| format!("deadlines {e}")));
        model_due = Some(
            run.answers
                .iter()
                .map(|a| a.as_ref().and_then(|a| a.deadline))
                .collect(),
        );
    }
    let score_of = |dues: &[Option<NaiveDateTime>]| {
        let mut s = DeadlineScore::default();
        for (case, due) in deadline_set.cases.iter().zip(dues) {
            s.add(deadline_outcome(case.expected_at(), *due));
        }
        s
    };
    let _ = writeln!(out, "\n## Deadlines in mail\n");
    let _ = writeln!(
        out,
        "Mail received {}. A deadline on the right day counts as found; the app reminds ahead of it. In the app the model's deadline is used only when the rules find none (\"Rules, then model\").\n",
        deadline_set.received
    );
    let _ = writeln!(out, "| | Exact | Same day | Wrong day | Missed | False positive | Correct none | Day recall | Day precision |");
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
    deadline_row(&mut out, "Rules (shipped)", &score_of(&rule_due));
    let combined: Option<Vec<Option<NaiveDateTime>>> = model_due
        .as_ref()
        .map(|m| rule_due.iter().zip(m).map(|(r, m)| r.or(*m)).collect());
    if let (Some(m), Some(c)) = (&model_due, &combined) {
        deadline_row(&mut out, "Model alone", &score_of(m));
        deadline_row(&mut out, "Rules, then model (as shipped)", &score_of(c));
    }
    let _ = writeln!(
        out,
        "\n| Case | Text | Expected | Rules |{}",
        if model_due.is_some() { " Model |" } else { "" }
    );
    let _ = writeln!(
        out,
        "|---|---|---|---|{}",
        if model_due.is_some() { "---|" } else { "" }
    );
    for (i, case) in deadline_set.cases.iter().enumerate() {
        let r = deadline_outcome(case.expected_at(), rule_due[i]);
        let m = model_due
            .as_ref()
            .map(|m| deadline_outcome(case.expected_at(), m[i]));
        let fine =
            |o: DeadlineOutcome| matches!(o, DeadlineOutcome::Exact | DeadlineOutcome::CorrectNone);
        if fine(r) && m.is_none_or(fine) {
            continue;
        }
        let model_cell = match (&model_due, m) {
            (Some(due), Some(o)) => format!(" {} ({}) |", fmt_due(due[i]), outcome_name(o)),
            _ => String::new(),
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} ({}) |{model_cell}",
            case.id,
            snippet(&format!("{} {}", case.subject, case.body)),
            fmt_due(case.expected_at()),
            fmt_due(rule_due[i]),
            outcome_name(r),
        );
    }

    // ---- awaiting reply
    let mut aw = BinaryScore::default();
    let mut aw_errors = Vec::new();
    for case in &awaiting {
        let got = case.nudges();
        aw.add(case.expected, got);
        if got != case.expected {
            aw_errors.push(format!(
                "| {} | {} | {} | {:.2} | {} |",
                case.id,
                snippet(&format!("{} {}", case.subject, case.body)),
                if case.expected { "nudge" } else { "no nudge" },
                case.score(),
                snippet(&case.note)
            ));
        }
    }
    let _ = writeln!(out, "\n## Waiting for your reply (rules)\n");
    let _ = writeln!(
        out,
        "Precision {} ({} of {} nudges wanted), recall {} ({} of {} found).\n",
        pct(aw.precision()),
        aw.tp,
        aw.tp + aw.fp,
        pct(aw.recall()),
        aw.tp,
        aw.tp + aw.fn_
    );
    if !aw_errors.is_empty() {
        let _ = writeln!(
            out,
            "| Case | Text | Expected | Score | Note |\n|---|---|---|---|---|"
        );
        for row in aw_errors {
            let _ = writeln!(out, "{row}");
        }
    }

    if model.is_some() {
        let mut sorted = latencies.clone();
        sorted.sort();
        let at = |q: f64| {
            sorted
                .get(((sorted.len() as f64 - 1.0) * q).round() as usize)
                .map(|d| format!("{:.1}s", d.as_secs_f64()))
                .unwrap_or_else(|| "—".into())
        };
        let over = sorted.iter().filter(|d| **d > SHIPPED_TIMEOUT).count();
        let _ = writeln!(out, "\n## Model latency and errors\n");
        let _ = writeln!(
            out,
            "{} requests: median {}, p90 {}, slowest {}; {} over the app's 8 s limit (those batches use the rules in the app).\n",
            sorted.len(),
            at(0.5),
            at(0.9),
            at(1.0),
            over
        );
        for e in &model_errors {
            let _ = writeln!(out, "- {e}");
        }
    }

    match &opts.out {
        Some(path) => match std::fs::write(path, &out) {
            Ok(()) => eprintln!("Report written to {path}"),
            Err(e) => {
                eprintln!("Couldn't write {path}: {e}");
                print!("{out}");
            }
        },
        None => print!("{out}"),
    }
}

fn deadline_item(case: &DeadlineCase) -> TriageItem {
    TriageItem {
        id: String::new(),
        from: "colleague@work.example".into(),
        from_name: "Colleague".into(),
        subject: case.subject.clone(),
        to_me: true,
        known_contact: true,
        times_user_wrote: 3,
        automated: false,
        body: Some(prepare_body(&case.body)),
    }
}
