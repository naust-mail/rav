/**
 * Compile-time feature flags. Values are baked in at build time via env vars.
 * Tree-shaking eliminates dead branches when a flag is false.
 */
export const FEATURES = {
  /** Calendar stickers (piyotaso). Disable with NEXT_PUBLIC_FEATURE_STICKERS=false. */
  stickers: process.env.NEXT_PUBLIC_FEATURE_STICKERS !== "false",
} as const;
