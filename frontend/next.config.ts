import type { NextConfig } from "next";

const isDev = process.env.NODE_ENV === "development";
const stickersEnabled = process.env.NEXT_PUBLIC_FEATURE_STICKERS !== "false";

const nextConfig: NextConfig = {
  output: isDev ? undefined : 'export',
  basePath: process.env.NEXT_PUBLIC_BASE_PATH || '',
  env: {
    NEXT_PUBLIC_FEATURE_STICKERS: stickersEnabled ? "true" : "false",
  },
  experimental: {
    optimizePackageImports: ['lucide-react'],
  },
  // In dev, proxy /api and /ws to the local Rust backend.
  // Ignored in production builds (output: export has no server).
  ...(isDev && {
    async rewrites() {
      const backend = process.env.NEXT_PUBLIC_API_URL || "http://localhost:3001";
      return [
        { source: "/api/:path*", destination: `${backend}/api/:path*` },
        { source: "/ws", destination: `${backend}/ws` },
      ];
    },
  }),
  allowedDevOrigins: [
    '192.168.68.63'
  ]
};

export default nextConfig;
