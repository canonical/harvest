# Benchmark coding sessions

**Goal:** measure the quality, speed and token use of Code mode before and after a change, on the same tasks.

`harvest-eval code-bench` runs each task in a fresh session through the HTTP API, as a user would, then checks the result with acceptance tests the agent never sees.

```bash
export HARVEST_TOKEN=<session token of a user with a model key>
harvest-eval code-bench \
  --config /etc/harvest/server.toml \
  --server http://127.0.0.1:8080 \
  --tasks harvest-eval/tasks/code \
  --runs 3 \
  --out bench.json
```

| Option | Default | Meaning |
|---|---|---|
| `--task <name>` | all | run one task |
| `--runs <n>` | `3` | runs per task; results vary between runs, so use 3 or more |
| `--keep-sessions` | off | leave sessions open to inspect them |

It prints one row per task with the median and p90 of time, model calls, input, cached and output tokens and cost, the acceptance and checks pass rates, and the mean number of test lines the agent added. `--out` keeps every run.

## Task files

```toml
name = "itsdangerous-rotation-bug"
kind = "bug"                       # free text: feature, tweak, bug, design…
repository = "https://github.com/pallets/itsdangerous"
base = "main"
setup = ["git … commit …"]        # run in the container before the first message, e.g. to plant a bug
prompts = ["…", "…"]               # one message per turn, in one chat

[acceptance]                       # run on a copy of /workspace after the last turn
prepare = ["python3 -m venv /tmp/bench-venv", "/tmp/bench-venv/bin/pip install -q -e . pytest"]
command = "/tmp/bench-venv/bin/pytest -q tests"
[acceptance.files]                 # hidden tests written into the copy
"tests/test_bench_acceptance.py" = '''…'''

[checks]                           # the repository's own checks, on another copy
prepare = ["/tmp/bench-venv/bin/pip install -q ruff"]
commands = ["/tmp/bench-venv/bin/ruff check src tests"]
```

The bundled tasks cover a tweak, a planted bug, a feature and a design-then-implement task. The tool needs the server configuration because it runs setup and acceptance commands in the session containers through Harvest's LXD identity.
