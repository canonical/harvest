# Create API keys for applications

**Goal:** let a script or service call Harvest's chat API as you.

1. Open **Settings → API keys** and select **Create API key**. Name it after the application.
2. Copy the key now: it starts with `hk_live_` and is shown only once.
3. Send it as a bearer token:
   ```bash
   curl https://harvest.example.com/v1/chat \
     -H "Authorization: Bearer hk_live_..." \
     -H "Content-Type: application/json" \
     -d '{"message": "Where is the session timeout configured?"}'
   ```

The key acts as you: it sees the repositories you see and uses your model key when the provider needs one. Conversations it creates appear in your chat history marked **API · <key name>**.

To revoke, select **Delete** next to the key. Rename with **Edit**. Request and response fields: [Call Harvest from your application](call-the-chat-api.md).
