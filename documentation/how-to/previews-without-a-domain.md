# Use previews without a domain

**Goal:** framed web previews on a test or private-network instance without your own DNS.

1. Use a wildcard DNS service. `sslip.io` resolves any name containing an IP address to that address:
   ```toml
   [code]
   preview_domain = "preview.10-1-2-3.sslip.io"   # for a host at 10.1.2.3
   preview_scheme = "https"
   ```
   Preview host names are then looked up through that public service.
2. Create a local certificate authority and a wildcard certificate with `mkcert`:
   ```bash
   sudo apt install mkcert
   sudo mkdir -p /etc/harvest/preview-tls
   sudo CAROOT=/etc/harvest/preview-tls mkcert \
     -cert-file /etc/harvest/preview-tls/preview.pem \
     -key-file  /etc/harvest/preview-tls/preview-key.pem \
     "*.preview.10-1-2-3.sslip.io"
   ```
3. Use them in the proxy from [Serve web previews](serve-previews.md).
4. On each computer that opens Harvest, trust `/etc/harvest/preview-tls/rootCA.pem` (import it as a trusted root certificate, or run `mkcert -install` with `CAROOT` pointing at a copy). Never share `rootCA-key.pem`.

Without step 4, browsers reject the preview certificate and the Preview tab stays blank.
