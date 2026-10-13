The Codex CLI base instructions in this folder (`prompt.md`) are copied unchanged
from https://github.com/openai/codex at tag `rust-v0.162.0`
(`codex-rs/models-manager/prompt.md`), Copyright OpenAI, licensed under the
Apache License, Version 2.0 (the same license as this project; see `LICENSE`).

`mxl models sync` writes them into the Codex model catalog it generates for
Mixlayer models. These are the instructions Codex itself uses for a model it has
no metadata for, so a Mixlayer model behaves the same way it did before the
catalog existed, but with its real context window and a place in `/model`.
When updating, copy the file from the Codex release you tested against and
update the tag above.
