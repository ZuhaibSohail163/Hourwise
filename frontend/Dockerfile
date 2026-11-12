# syntax=docker/dockerfile:1
# Production image for the Hourwise frontend (Next.js standalone server).
#
#   docker build --build-arg BACKEND_URL=http://hourwise-backend:8080 -t hourwise-frontend ./frontend
#
# BACKEND_URL is where the Next.js server proxies /api and /uploads. Next.js resolves rewrites
# at BUILD time, so it must be set as a build argument (it is not read at runtime). It should be
# the backend's PRIVATE address (Docker service name), never a public URL.

# ---- deps ----
FROM node:22-alpine AS deps
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci

# ---- build ----
FROM node:22-alpine AS builder
WORKDIR /app
# Where the Next.js server proxies /api and /uploads (baked in at build time).
ARG BACKEND_URL=http://backend:8080
ENV BACKEND_URL=$BACKEND_URL \
    NEXT_TELEMETRY_DISABLED=1
COPY --from=deps /app/node_modules ./node_modules
COPY . .
RUN npm run build

# ---- runtime ----
FROM node:22-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0
RUN addgroup -S nodejs && adduser -S nextjs -G nodejs
COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./
COPY --from=builder --chown=nextjs:nodejs /app/.next/static ./.next/static
USER nextjs
EXPOSE 3000
# Liveness probe (no curl in the image): GET /healthz must return 200.
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD ["node", "-e", "fetch('http://127.0.0.1:3000/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"]
CMD ["node", "server.js"]
