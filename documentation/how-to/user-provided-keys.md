# Let users bring their own API keys

**Goal:** each user pays for their own model usage with their own key.

1. Generate an encryption key and set it. Harvest encrypts every stored user key with it (AES-256-GCM).
   ```bash
   sudo harvest security user-key-encryption-key set "$(openssl rand -hex 32)"
   ```
   In `server.toml`: `[security] user_key_encryption_key = "<64 hex characters>"`.
2. Mark the provider as user-keyed and leave its key empty:
   ```bash
   sudo harvest llm provider add --id primary --provider anthropic --model claude-sonnet-5-5 --user-provided-key
   ```
   In `server.toml`: `user_provided_key = true`, `api_key = ""`.
3. Each user opens **Settings → LLM API keys**, selects **Edit** on the provider and pastes their key.

Pages that need a model ask users without a key to add one. Keep the encryption key safe: without it, stored keys and git credentials cannot be decrypted.
