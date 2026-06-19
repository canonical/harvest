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

EXPOSE 80
