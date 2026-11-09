import type { NextConfig } from "next";

// The browser only ever talks to the Next.js server; API and avatar requests
// are proxied to the Rust backend. This avoids CORS entirely.
// NOTE: rewrites are resolved at build time, so BACKEND_URL must be set when
// running `next build` (the Docker image passes it as a build arg).
const backend = process.env.BACKEND_URL || "http://localhost:8080";

const nextConfig: NextConfig = {
  output: "standalone",
  poweredByHeader: false,
  async rewrites() {
    return [
      { source: "/api/:path*", destination: `${backend}/api/:path*` },
      { source: "/uploads/:path*", destination: `${backend}/uploads/:path*` },
    ];
  },
};

export default nextConfig;
