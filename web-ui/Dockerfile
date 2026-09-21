# ── Stage 1: build ────────────────────────────────────────────────────────────
FROM node:22-alpine AS builder

WORKDIR /app

# Install deps first for better layer caching.
COPY web-ui/package.json web-ui/package-lock.json ./
RUN npm ci

# Build the Vue app.
COPY web-ui/ .
RUN npm run build

# ── Stage 2: serve ────────────────────────────────────────────────────────────
FROM nginx:alpine

COPY --from=builder /app/dist          /usr/share/nginx/html
COPY web-ui/nginx.conf                 /etc/nginx/conf.d/default.conf

# The stock entrypoint's IPv6 script shells out to `apk manifest` to patch the
# packaged default.conf, which can hang and stop nginx from ever starting. We
# ship our own config, so the script has nothing to do.
RUN rm -f /docker-entrypoint.d/10-listen-on-ipv6-by-default.sh

EXPOSE 80
