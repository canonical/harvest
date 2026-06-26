#!/usr/bin/env python3
# Reads the `llm` snap config subtree as JSON from stdin and writes
# [[llm]] TOML blocks to stdout, sorted by priority then name.
# Called by the configure hook:
#   snapctl get --document llm | python3 "$SNAP/lib/harvest/gen-llm-config.py"
import json
import sys

PROVIDER_DEFAULTS = {
    'anthropic':        {'model': 'claude-sonnet-4-6'},
    'gemini':           {'model': 'gemini-2.5-flash'},
    'openai-compatible': {'model': ''},
}

try:
    providers = json.load(sys.stdin)
except (json.JSONDecodeError, ValueError):
    providers = {}

if not isinstance(providers, dict):
    providers = {}

sorted_providers = sorted(
    providers.items(),
    key=lambda kv: (int(str(kv[1].get('priority', 0))), kv[0]),
)

for name, cfg in sorted_providers:
    if not isinstance(cfg, dict):
        continue
    api_key = cfg.get('api-key', '')
    if not api_key:
        continue

    provider = cfg.get('provider', 'anthropic')
    defaults = PROVIDER_DEFAULTS.get(provider, {})
    model    = cfg.get('model', '') or defaults.get('model', '')
    timeout  = cfg.get('timeout-secs', 120)
    retries  = cfg.get('max-retries', 3)
    priority = cfg.get('priority', 0)
    base_url = cfg.get('base-url', '')

    print()
    print('[[llm]]')
    print('provider     = "%s"' % provider)
    print('api_key      = "%s"' % api_key)
    if model:
        print('model        = "%s"' % model)
    print('timeout_secs = %s' % timeout)
    print('max_retries  = %s' % retries)
    print('priority     = %s' % priority)
    if provider == 'openai-compatible' and base_url:
        print('base_url     = "%s"' % base_url)
