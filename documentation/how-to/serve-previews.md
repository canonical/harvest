# Serve web previews on their own domain

**Goal:** web previews that render inside Harvest and work with any framework.

Each web preview is served at `p-<preview id>.<preview domain>`. Web previews stay off until `preview_domain` is set; API and command-line previews work without it.

1. **DNS:** a wildcard record, `*.preview.harvest.example.com`, pointing at your reverse proxy.
2. **Certificate:** a wildcard certificate for `*.preview.harvest.example.com` (Let's Encrypt with a DNS-01 challenge works).
3. **Proxy:** send those hosts to the Harvest server unchanged, keeping `Host` and websocket upgrades:

```nginx
map $http_upgrade $harvest_preview_connection {
    default upgrade;
    ''      '';
}

server {
    listen 443 ssl;
    server_name "~^p-[0-9a-f]{32}\.preview\.";
    ssl_certificate     /etc/ssl/preview.pem;
    ssl_certificate_key /etc/ssl/preview-key.pem;
    client_max_body_size 64m;

    location / {
        proxy_pass         http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header   Host       $host;
        proxy_set_header   Upgrade    $http_upgrade;
        proxy_set_header   Connection $harvest_preview_connection;
        proxy_buffering    off;
        proxy_read_timeout 3600s;
    }
}
```

4. **Harvest:**

```toml
[code]
preview_domain = "preview.harvest.example.com"
preview_scheme = "https"
```

Set `auth.public_url` (or `agents.public_url`) to Harvest's own address: previews only allow Harvest's origin to frame them.

With `preview_scheme = "https"`, previews show inside Harvest's Preview tab. With `"http"`, browsers refuse the preview's sign-in cookie inside a frame, so the tab offers **Open the app** in a new tab instead.

Background: [How previews stay isolated](../explanation/preview-isolation.md).
