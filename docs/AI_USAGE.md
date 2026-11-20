# AI explanations (BYOK / local)

`csb explain` adds an optional, advisory explanation to an existing review.
It is off unless you run it, uses only the evidence already in the review, and
never changes the deterministic report, manifest or verdict.

The deterministic artifacts remain the source of truth. The explanation is
labeled advisory and cannot change scope, design, checks or merge state.

## Prerequisite

Generate a review with design artifacts:

```sh
csb review --repo . --base "$BASE" --head "$HEAD" --out-dir review/ --design auto
```

List boundaries from `review/artifact.json`:

```sh
jq -r '.hld.delta.changes[] | .id + "  " + .kind' review/artifact.json
```

## Preview first

```sh
csb explain --dir review/ --boundary <hld-change-id> \
  --model qwen2.5-coder:7b --preview
```

This writes `review/ai-preview.json` and performs **no network request**. It
shows the provider, model, endpoint, evidence paths/commits/lines and byte
counts. Source text is not included.

## Local Ollama

```sh
ollama pull qwen2.5-coder:7b
csb explain --dir review/ --boundary <id> \
  --provider ollama --model qwen2.5-coder:7b --send
```

Default endpoint: `http://127.0.0.1:11434`. Plain HTTP is allowed only on
loopback. The request sets `stream: false`, a JSON schema and temperature 0.

## OpenAI-compatible providers

```sh
export CSB_AI_API_KEY=...
csb explain --dir review/ --boundary <id> \
  --provider openai --model gpt-4.1-mini --send
```

Default endpoint: `https://api.openai.com/v1`. The adapter calls
`POST <endpoint>/chat/completions` and expects an OpenAI-compatible response.

Works with OpenAI, Azure-compatible gateways, OpenRouter, vLLM, LM Studio,
llama.cpp and similar services. Claude can be used through an
OpenAI-compatible gateway; there is no native Anthropic adapter yet.

Keys can also come from `--key-file <path>`; on Unix the file must be regular,
at most 4 KiB and mode `0600`. Keys are never accepted as CLI values, written to
logs, prompts or output, or stored by the tool.

**Not supported:** GitHub Copilot and Codex are IDE/agent products without a
third-party inference API. Chat subscriptions do not grant API access.

## Model selection (measured locally)

Tested on this repository's M2 merge diff with Ollama 0.40 on an Apple-silicon Mac:

| Model | Narrow declaration boundary | Broad 16-span boundary | Recommendation |
|---|---|---|---|
| `qwen2.5-coder:1.5b` | 4–12 s, valid, generic | fails validation (invented citation) | fast; narrow boundaries only |
| `qwen2.5-coder:3b` | 6–17 s, valid, more specific | fails validation (invented citation) | narrow boundaries only |
| `qwen2.5-coder:7b` | 8–10 s, best wording | 14–16 s after the v0.4.1 guards | recommended default |
| `qwen3:14b` | exceeded the 60 s deadline cold | not tested | raise the deadline before using |

Small models mostly restate the declaration and ask generic questions. The
largest quality limit is evidence granularity: HLD sources are individual
declaration lines. Richer LLD/function-body evidence is the next improvement;
a larger model alone does not fix it.

From a container, `127.0.0.1` is the container itself. Use `--network host` on
Linux or an HTTPS endpoint. Plain-HTTP `host.docker.internal` is intentionally
rejected, and an Ollama listener must never be exposed beyond the local host.

## Outputs

| File | Content |
|---|---|
| `ai-preview.json` | exact destination and evidence inventory; no source text |
| `ai.md` | advisory explanation with evidence indexes |
| `ai.json` | versioned result, bundle identity, provider/model, evidence hashes |

`ai.md`/`ai.json` are not listed in `manifest.json`; the deterministic review
still verifies independently.

## Limits and failure behavior

- Evidence combines HLD source spans with LLD evidence: the most specific
  declaration body from the retained syntax inputs, plus deterministic lexical
  call observations for that declaration. Each item records `source_kind`
  (`hld`, `lld_body`, `lld_calls`); identical HLD/LLD text is never duplicated.
- At most 16 evidence spans and 16 KiB of redacted source per request.
- 60-second deadline, 1024 generated tokens, 64 KiB response cap. Truncated
  output fails as `provider_output_truncated`; it is never parsed as a result.
- Boundaries marked `uncertain` must return no claims; they get a neutral
  headline and only questions/unknowns. Asserted claims fail validation.
- No retries, no redirects, no proxy.
- Only cited evidence indexes are accepted; foreign or stale citations fail.
- Invalid JSON, schema, markup or an oversized response fails with exit code 5.
- A provider failure leaves the review untouched; rerun `csb review` only if the
  deterministic review itself changed.
- Redaction is defense in depth, not a guarantee; preview and scope authorization
  remain the primary controls.

## Static reports

The self-contained report and GitHub Pages publication are static. They never
call a model. AI explanations are a local CLI action only.
