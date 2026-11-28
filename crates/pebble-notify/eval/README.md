# Smart notification evaluation

Synthetic, hand-labelled sets for the parts of #107 that decide when Pebble
interrupts you. No real mail is used.

| File | What | Labels |
|---|---|---|
| `notify_cases.json` | 78 new messages (zh/en) in 10 categories, with what the store would know about each (known sender, VIP, bulk headers, rules…) | the tier a careful person wants in **Smart** mode: `interrupt`, `digest` or `silent` |
| `deadline_cases.json` | 38 mail texts received Wed 2026-12-02 10:00, including past references and dates without a deadline | the local due time, or `null` |
| `awaiting_cases.json` | 24 messages that might be waiting for your reply | whether a nudge is welcome |

## Rules only

The shipped rules are checked on every `cargo test` (`tests/eval_rules.rs`):
no mail may be lost, certain categories must be perfect, and accuracy,
deadline precision and nudge precision/recall may not drop below the
recorded floors. The full report:

```bash
cargo run -p pebble-notify-eval
```

`report-rules.md` is a snapshot of that report.

## Rules plus a model

Set these in your own terminal (they are never printed or written to the
report), then run with `--model`:

```bash
export PEBBLE_EVAL_ENDPOINT=https://api.example.com   # OpenAI-compatible base URL
export PEBBLE_EVAL_MODEL=model-name
export PEBBLE_EVAL_API_KEY=...                        # optional for local servers
cargo run -p pebble-notify-eval -- --model --out eval-report.md
```

The tool sends the synthetic messages the way the app does: the same
prompt, body preparation, parsing and decision (`triage_model`,
`apply_model_verdict`). It reports:

- **Rules + model**: every candidate answered, however long it took.
- **As shipped**: batches slower than the app's 8 s limit use the rules,
  as in the app.
- Deadlines from the model alone and combined with the rules (the app uses
  the model's deadline only when the rules found none).
- Latency per request and any malformed answers.

Options: `--batch N` (messages per request, default 10), `--timeout SECS`
(per request, default 60), `--no-bulk` (don't send newsletters and
automated mail to the model, the app's default).
