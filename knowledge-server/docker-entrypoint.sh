#!/bin/sh
set -e

# If the user mounted a full config file, use it directly.
if [ -f /etc/harvest/server.toml ]; then
  exec knowledge-server --config /etc/harvest/server.toml "$@"
fi

# Validate required variables.
: "${JWT_SECRET:?JWT_SECRET environment variable is required}"

CONFIG=/tmp/server.toml

cat > "$CONFIG" << EOF
[server]
host = "0.0.0.0"
port = 8080

[ui]
enable_docs = ${ENABLE_DOCS:-false}

[agents]
public_url  = "${PUBLIC_URL:-}"
binary_path = "/usr/local/bin/harvest-agent"

[neo4j]
uri      = "${NEO4J_URI:-bolt://neo4j:7687}"
user     = "${NEO4J_USER:-neo4j}"
password = "${NEO4J_PASSWORD:-devpassword}"

[auth]
jwt_secret        = "${JWT_SECRET}"
allow_local_login = ${ALLOW_LOCAL_LOGIN:-true}

[agent]
max_iterations = ${LLM_MAX_ITERATIONS:-20}
EOF

# Appends one [[llm]] block, reading fields from the env vars named
# ${1}_PROVIDER, ${1}_MODEL, ${1}_API_KEY, etc. (indirect lookup via eval,
# since POSIX sh has no arrays/namerefs). $1 is "LLM" for the legacy flat
# single-provider vars, or "LLM_<NAME>" for a named multi-provider block.
# Note: max_iterations is NOT a per-provider field — it's set once above,
# under [agent], since LlmProviderConfig has no such field (config.rs).
#
# ${1}_ID/_EXPOSE_TO_UI/_NAME/_MODELS map to the id/expose_to_ui/name/models
# fields the chat page's provider/model picker relies on (config.rs). For a
# named multi-provider block, id defaults to <NAME> lowercased so it doesn't
# need to be set explicitly; the legacy flat LLM_* block leaves id blank so
# config.rs auto-derives it from provider+priority.
emit_llm_block() {
  prefix="$1"
  eval "provider=\"\${${prefix}_PROVIDER:-}\""
  eval "model=\"\${${prefix}_MODEL:-}\""
  eval "api_key=\"\${${prefix}_API_KEY:-}\""
  eval "base_url=\"\${${prefix}_BASE_URL:-}\""
  eval "priority=\"\${${prefix}_PRIORITY:-0}\""
  eval "timeout_secs=\"\${${prefix}_TIMEOUT_SECS:-120}\""
  eval "max_retries=\"\${${prefix}_MAX_RETRIES:-3}\""
  eval "id=\"\${${prefix}_ID:-}\""
  eval "expose_to_ui=\"\${${prefix}_EXPOSE_TO_UI:-}\""
  eval "display_name=\"\${${prefix}_NAME:-}\""
  eval "models=\"\${${prefix}_MODELS:-}\""

  : "${provider:?${prefix}_PROVIDER environment variable is required}"

  if [ -z "$id" ] && [ "$prefix" != "LLM" ]; then
    id=$(printf '%s' "${prefix#LLM_}" | tr '[:upper:]' '[:lower:]')
  fi

  case "$provider" in
    anthropic)
      : "${api_key:?${prefix}_API_KEY environment variable is required}"
      default_model="claude-sonnet-4-6"
      ;;
    gemini)
      : "${api_key:?${prefix}_API_KEY environment variable is required}"
      default_model="gemini-2.5-flash-preview-05-20"
      ;;
    openai-compatible)
      # api_key may be left empty for self-hosted endpoints that don't
      # require auth — openai_compat.rs skips the Authorization header
      # when it's empty rather than sending it blank.
      : "${base_url:?${prefix}_BASE_URL is required when ${prefix}_PROVIDER=openai-compatible}"
      default_model=""
      ;;
    *)
      echo "ERROR: unknown ${prefix}_PROVIDER=${provider} (expected: anthropic, gemini, openai-compatible)" >&2
      exit 1
      ;;
  esac

  cat >> "$CONFIG" << EOF

[[llm]]
provider       = "${provider}"
model          = "${model:-$default_model}"
api_key        = "${api_key}"
timeout_secs   = ${timeout_secs}
max_retries    = ${max_retries}
priority       = ${priority}
EOF

  [ "$provider" = "openai-compatible" ] && printf 'base_url       = "%s"\n' "${base_url}" >> "$CONFIG"
  [ -n "$id" ]           && printf 'id             = "%s"\n' "${id}" >> "$CONFIG"
  [ -n "$expose_to_ui" ] && printf 'expose_to_ui   = %s\n' "${expose_to_ui}" >> "$CONFIG"
  [ -n "$display_name" ] && printf 'name           = "%s"\n' "${display_name}" >> "$CONFIG"
  if [ -n "$models" ]; then
    models_toml=$(printf '%s' "$models" | awk -F',' '{for(i=1;i<=NF;i++){gsub(/^ +| +$/,"",$i); printf "\"%s\"%s", $i, (i<NF?", ":"")}}')
    printf 'models         = [%s]\n' "${models_toml}" >> "$CONFIG"
  fi
}

# Multi-provider: any LLM_<NAME>_PROVIDER var (e.g. LLM_GEMINI_PROVIDER,
# LLM_CLAUDE_PROVIDER) defines a named provider block. Falls back to the
# legacy flat LLM_PROVIDER/LLM_MODEL/LLM_API_KEY vars when none are set.
llm_names=$(env | sed -n 's/^LLM_\(.*\)_PROVIDER=.*/\1/p' | sort)

if [ -n "$llm_names" ]; then
  for name in $llm_names; do
    emit_llm_block "LLM_${name}"
  done
else
  emit_llm_block "LLM"
fi

# Optional: Google OAuth
if [ -n "${GOOGLE_CLIENT_ID:-}" ]; then
  : "${GOOGLE_CLIENT_SECRET:?GOOGLE_CLIENT_SECRET required when GOOGLE_CLIENT_ID is set}"
  : "${GOOGLE_REDIRECT_URI:?GOOGLE_REDIRECT_URI required when GOOGLE_CLIENT_ID is set}"
  cat >> "$CONFIG" << EOF

[auth.google]
client_id     = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
redirect_uri  = "${GOOGLE_REDIRECT_URI}"
EOF
fi

# Optional: OIDC
if [ -n "${OIDC_ISSUER_URL:-}" ]; then
  : "${OIDC_CLIENT_ID:?OIDC_CLIENT_ID required when OIDC_ISSUER_URL is set}"
  : "${OIDC_CLIENT_SECRET:?OIDC_CLIENT_SECRET required when OIDC_ISSUER_URL is set}"
  : "${OIDC_REDIRECT_URI:?OIDC_REDIRECT_URI required when OIDC_ISSUER_URL is set}"
  cat >> "$CONFIG" << EOF

[auth.oidc]
issuer_url    = "${OIDC_ISSUER_URL}"
client_id     = "${OIDC_CLIENT_ID}"
client_secret = "${OIDC_CLIENT_SECRET}"
redirect_uri  = "${OIDC_REDIRECT_URI}"
EOF
  if [ -n "${OIDC_DISPLAY_NAME:-}" ]; then
    printf 'display_name  = "%s"\n' "${OIDC_DISPLAY_NAME}" >> "$CONFIG"
  fi
fi

# Optional: LXD (enables "Let Harvest create and manage agent" in the web UI).
# Set LXD_TRUST_TOKEN (from `lxc config trust add --name harvest`, run once
# against the cluster) to let Harvest generate and self-register its own
# client identity — or set LXD_CLIENT_CERT/LXD_CLIENT_KEY instead to manage
# the identity yourself, which skips the trust-token flow entirely.
if [ -n "${LXD_ENDPOINT:-}" ]; then
  cat >> "$CONFIG" << EOF

[lxd]
endpoint    = "${LXD_ENDPOINT}"
EOF
  if [ -n "${LXD_CLIENT_CERT:-}" ] && [ -n "${LXD_CLIENT_KEY:-}" ]; then
    cat >> "$CONFIG" << EOF
client_cert = """
${LXD_CLIENT_CERT}
"""
client_key  = """
${LXD_CLIENT_KEY}
"""
EOF
  elif [ -n "${LXD_TRUST_TOKEN:-}" ]; then
    printf 'trust_token = "%s"\n' "${LXD_TRUST_TOKEN}" >> "$CONFIG"
  fi
  if [ -n "${LXD_CA_CERT:-}" ]; then
    cat >> "$CONFIG" << EOF
ca_cert = """
${LXD_CA_CERT}
"""
EOF
  fi
  [ -n "${LXD_INSECURE:-}" ]     && printf 'insecure     = %s\n' "${LXD_INSECURE}"     >> "$CONFIG"
  [ -n "${LXD_PROJECT:-}" ]      && printf 'project      = "%s"\n' "${LXD_PROJECT}"      >> "$CONFIG"
  [ -n "${LXD_IMAGE_ALIAS:-}" ]  && printf 'image_alias  = "%s"\n' "${LXD_IMAGE_ALIAS}"  >> "$CONFIG"
  [ -n "${LXD_IMAGE_SERVER:-}" ] && printf 'image_server = "%s"\n' "${LXD_IMAGE_SERVER}" >> "$CONFIG"
  [ -n "${LXD_PROFILE:-}" ]      && printf 'profile      = "%s"\n' "${LXD_PROFILE}"      >> "$CONFIG"
fi

exec knowledge-server --config "$CONFIG" "$@"
