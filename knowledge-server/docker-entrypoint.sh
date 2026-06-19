#!/bin/sh
set -e

# If the user mounted a full config file, use it directly.
if [ -f /etc/harvest/server.toml ]; then
  exec knowledge-server --config /etc/harvest/server.toml "$@"
fi

# Validate required variables.
: "${JWT_SECRET:?JWT_SECRET environment variable is required}"
: "${LLM_API_KEY:?LLM_API_KEY environment variable is required}"

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
EOF

# LLM section — provider-specific fields differ.
LLM_PROVIDER="${LLM_PROVIDER:-anthropic}"
LLM_MAX_ITERATIONS="${LLM_MAX_ITERATIONS:-20}"
LLM_TIMEOUT_SECS="${LLM_TIMEOUT_SECS:-120}"
LLM_MAX_RETRIES="${LLM_MAX_RETRIES:-3}"

case "$LLM_PROVIDER" in
  anthropic)
    cat >> "$CONFIG" << EOF

[llm]
provider       = "anthropic"
model          = "${LLM_MODEL:-claude-sonnet-4-6}"
api_key        = "${LLM_API_KEY}"
max_iterations = ${LLM_MAX_ITERATIONS}
timeout_secs   = ${LLM_TIMEOUT_SECS}
max_retries    = ${LLM_MAX_RETRIES}
EOF
    ;;
  gemini)
    cat >> "$CONFIG" << EOF

[llm]
provider       = "gemini"
model          = "${LLM_MODEL:-gemini-2.5-flash-preview-05-20}"
api_key        = "${LLM_API_KEY}"
max_iterations = ${LLM_MAX_ITERATIONS}
timeout_secs   = ${LLM_TIMEOUT_SECS}
max_retries    = ${LLM_MAX_RETRIES}
EOF
    ;;
  openai-compatible)
    : "${LLM_BASE_URL:?LLM_BASE_URL is required when LLM_PROVIDER=openai-compatible}"
    cat >> "$CONFIG" << EOF

[llm]
provider       = "openai-compatible"
base_url       = "${LLM_BASE_URL}"
api_key        = "${LLM_API_KEY}"
model          = "${LLM_MODEL}"
max_iterations = ${LLM_MAX_ITERATIONS}
timeout_secs   = ${LLM_TIMEOUT_SECS}
max_retries    = ${LLM_MAX_RETRIES}
EOF
    ;;
  *)
    echo "ERROR: unknown LLM_PROVIDER=${LLM_PROVIDER} (expected: anthropic, gemini, openai-compatible)" >&2
    exit 1
    ;;
esac

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

exec knowledge-server --config "$CONFIG" "$@"
