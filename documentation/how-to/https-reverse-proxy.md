# Put Harvest behind HTTPS

**Goal:** serve Harvest on a public HTTPS address.

Terminate TLS in a reverse proxy in front of the web UI (snap: port 3000). The proxy must:

- keep the `Host` header,
- not buffer responses: chat answers and events are streamed (server-sent events),
- forward websocket upgrades: the agent console and port forwarding use them,
- allow long reads, for example one hour.

```nginx
server {
    listen 443 ssl;
    server_name harvest.example.com;
    ssl_certificate     /etc/ssl/harvest.pem;
    ssl_certificate_key /etc/ssl/harvest-key.pem;
    client_max_body_size 64m;

    location / {
        proxy_pass         http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header   Host              $host;
        proxy_set_header   X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
        proxy_set_header   Upgrade           $http_upgrade;
        proxy_set_header   Connection        $http_connection;
        proxy_buffering    off;
        proxy_read_timeout 3600s;
    }
}
```

Then tell Harvest its public address, used in sign-in redirects, agent install commands and preview links:

```bash
sudo harvest auth public-url set https://harvest.example.com
sudo harvest agents set-public-url https://harvest.example.com
```

For previews, add the wildcard server from [Serve web previews](serve-previews.md).
