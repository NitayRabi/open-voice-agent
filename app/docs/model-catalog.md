# Downloadable model selection

The app offers models that passed all eight intended delegation requests in our
local smoke test. E4B uses Apache 2.0; LFM uses the LFM Open License v1.0.
Each model retains its own terms independently of the open-source app. Weights are downloaded
on demand, not bundled. This is a curated download policy; existing local GGUF
paths and remote endpoints remain supported.

| Model | Download | Badges | Intended delegations | Unwanted delegations |
| --- | ---: | --- | ---: | ---: |
| Gemma 4 E4B Instruct Q4_0 | 4.59 GB | Recommended | 8/8 | 0/8 |
| Liquid AI LFM2.5 1.2B Instruct Q4_K_M | 731 MB | Smallest | 8/8 | 4/8 |

“Smallest” compares downloadable files in this catalog, not all available models.
“Recommended” favors E4B's successful conversational routing.
Download size is not a RAM/VRAM estimate. LFM offers a smaller footprint but
was more likely to delegate casual conversation unnecessarily.

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

LFM uses the previously tested file, whose local SHA-256 matches the pinned
upstream download. It completed all eight intended delegations, but also
unnecessarily delegated four of eight casual requests. It remains an optional
small model rather than the default. Qwen3.6 passed the routing test but is
excluded from downloads because its 22.29 GB file is too large for this catalog.
Its historical results remain in the evidence folder.
Raw routing evidence and the prompt harness are in [model-evidence](model-evidence/).

## Licenses and exclusions

License sources checked 2026-09-06:

- [Gemma 4 E4B upstream](https://huggingface.co/google/gemma-4-E4B-it): Apache-2.0.
- [Qwen3.6 upstream license](https://huggingface.co/Qwen/Qwen3.6-35B-A3B/blob/main/LICENSE): Apache-2.0.
- [LFM2.5 1.2B license](https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct/blob/main/LICENSE):
  LFM Open License includes a commercial revenue threshold requiring additional
  licensing for commercial use at $10M or more annual revenue. Included as an
  optional download with this condition disclosed. Referencing the model by name
  does not remove the model user’s license obligations.
- [Qwen2.5 3B license](https://huggingface.co/Qwen/Qwen2.5-3B-Instruct/blob/main/LICENSE):
  Qwen Research License requires a separate commercial agreement. Excluded despite
  8/8 intended delegations (also 4/8 unwanted). Its former Apache label was incorrect.

Qwen3.5 2B and Granite 4.2 3B did not achieve 8/8. Other former catalog entries
lack qualifying test evidence. The catalog pins repository revisions, exact
byte sizes and SHA-256 hashes in `src-tauri/assets/catalog.json` so later upstream
changes cannot silently replace the selected files.
