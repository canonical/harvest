# Security model

## Identity

Users sign in with a local password, Google or OIDC. Sessions are signed JWTs (`auth.jwt_secret`). Applications use API keys (`hk_live_…`), stored hashed and shown once. Machine agents have their own tokens.

## Authorisation

- **Personal spaces** (chats, sessions, personal artifacts and skills) belong to one user.
- **Deploy projects** use viewer, editor and owner roles, checked on every request.
- **Repositories** are public or private. Private access is granted by proving access at the source with the user's own credentials, and re-checked periodically, so revoking upstream revokes in Harvest. Every agent tool call is filtered by the same rules.

See [permissions](../reference/permissions.md).

## Secrets

Users' model keys, SSH keys and git tokens are encrypted with AES-256-GCM under `security.user_key_encryption_key`. They are decrypted only on the server, for the request that needs them. They never enter coding containers or machine agents.

## Running untrusted code

Code written in sessions runs only in LXD containers, as root inside the container, never on the Harvest host. Previews run on separate origins with framing locked to Harvest ([details](preview-isolation.md)). Shared previews give each viewer a separate copy.

## Machines

Agents connect outbound; Harvest holds no credentials to your machines. Actions that change infrastructure in Deploy chat require the user's confirmation, and generated artifacts are reviewed before they run.

## Models

Prompts include code and conversation content and go to the model providers you configure. Choose providers, regions and data agreements accordingly, or use an `openai-compatible` endpoint you host.
