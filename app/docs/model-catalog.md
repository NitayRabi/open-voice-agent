# Downloadable model selection

The app offers models that passed all eight intended delegation requests in our
local smoke test and require no separate commercial license. Both selections use
Apache 2.0: normal license and notice obligations still apply. Weights are downloaded
on demand, not bundled. This is a curated download policy; existing local GGUF
paths and remote endpoints remain supported.

| Model | Download | Badges | Intended delegations | Unwanted delegations |
| --- | ---: | --- | ---: | ---: |
| Gemma 4 E4B Instruct Q4_0 | 4.59 GB | Recommended, Smallest | 8/8 | 0/8 |
| Qwen3.6 35B A3B Q4_K_M | 22.29 GB | — | 8/8 | 0/8 |

“Smallest” compares downloadable files in this catalog, not all available models.
“Recommended” favors E4B's smaller footprint and successful conversational routing.
Download size is not a RAM/VRAM estimate. Qwen's active parameter count does not
remove the need to store all its weights.

## Evidence and limits

The test used two delegation prompts (research a Rust release; inspect a project
for performance bottlenecks) and two casual conversation prompts (unwind after a
long day; explain the blue sky), each repeated four times. Temperature was zero,
output capped at 128 tokens, with a `delegate_task(request: string)` tool and a
system instruction to delegate research, coding, calculations and actions.
No delegated actions were executed. Eight successes are repeated trials of two
prompts, not eight independent tasks or a broad capability benchmark.

The exact pinned E4B file was re-downloaded and re-tested on 2026-09-06 because
it differs from the earlier local Q4_0 file. It produced eight valid tool calls
and eight ordinary text answers. Warm medians, excluding the first repetition:
53 ms to conversational text, 302 ms to complete delegation. Hardware was an
AMD Radeon AI PRO R9700; llama.cpp build 10558, Vulkan, 32K context, one slot,
all GPU layers, no speculative decoding, reasoning off. These measurements
exclude speech recognition, text-to-speech and delegated task execution.

Qwen uses the previously tested file, whose local SHA-256 was checked against
the pinned download. Its earlier warm medians were 179 ms conversational text
and 441 ms complete delegation. That run used a different runtime configuration
(256K context and MTP), so the timings are not a controlled speed ranking.
Raw routing evidence and the prompt harness are in [model-evidence](model-evidence/).

## Licenses and exclusions

License sources checked 2026-09-06:

- [Gemma 4 E4B upstream](https://huggingface.co/google/gemma-4-E4B-it): Apache-2.0.
- [Qwen3.6 upstream license](https://huggingface.co/Qwen/Qwen3.6-35B-A3B/blob/main/LICENSE): Apache-2.0.
- [LFM2.5 1.2B license](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct/blob/main/LICENSE):
  LFM Open License includes a commercial revenue threshold requiring additional
  licensing above $10M. Excluded despite 8/8 intended delegations (also 4/8 unwanted).
- [Qwen2.5 3B license](https://huggingface.co/Qwen/Qwen2.5-3B-Instruct/blob/main/LICENSE):
  Qwen Research License requires a separate commercial agreement. Excluded despite
  8/8 intended delegations (also 4/8 unwanted). Its former Apache label was incorrect.

Qwen3.5 2B and Granite 4.2 3B did not achieve 8/8. Other former catalog entries
lack qualifying test evidence. The catalog pins repository revisions, exact
byte sizes and SHA-256 hashes in `src-tauri/assets/catalog.json` so later upstream
changes cannot silently replace the selected files.
