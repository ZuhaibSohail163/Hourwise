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
COPY --chown=nextjs:nodejs docker/healthcheck.mjs ./healthcheck.mjs
USER nextjs
# Git commit of this build, reported by /healthz. CI sets it (docs/CI-CD.md); other builds
# report "unknown". Declared last so it never invalidates the cached layers above.
ARG HOURWISE_REVISION=unknown
ENV HOURWISE_REVISION=$HOURWISE_REVISION
EXPOSE 3000
# Health check (no curl in the image): /healthz must answer, and a NEW container only becomes
# healthy once it has reached the backend through its /api proxy. Swarm therefore never sends
# traffic to a frontend that can't reach the backend (see docker/healthcheck.mjs).
HEALTHCHECK --interval=15s --timeout=10s --start-period=30s --retries=3 \
    CMD ["node", "/app/healthcheck.mjs"]
CMD ["node", "server.js"]
