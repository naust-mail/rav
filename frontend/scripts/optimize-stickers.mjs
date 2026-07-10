// Resizes/re-encodes public/stickers/*.{png,gif} to WebP at their actual max
// render size (StickerCell/StickerPicker render at most ~90px; 192px covers
// 2x HiDPI). Run this whenever a new oversized sticker is added - it's
// idempotent, skipping manifest entries that already point at a .webp file.
//
// Usage: node scripts/optimize-stickers.mjs
import sharp from "sharp";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const STICKERS_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "public", "stickers");
const MANIFEST_PATH = path.join(STICKERS_DIR, "manifest.json");
const MAX_DIM = 192;

const manifest = JSON.parse(await fs.readFile(MANIFEST_PATH, "utf8"));

let converted = 0;
let totalBefore = 0;
let totalAfter = 0;

for (const entry of manifest) {
  if (entry.file.endsWith(".webp")) continue;

  const srcPath = path.join(STICKERS_DIR, entry.file);
  const srcStat = await fs.stat(srcPath);

  const newFile = entry.file.replace(/\.(png|gif|jpe?g)$/i, ".webp");
  const outPath = path.join(STICKERS_DIR, newFile);

  await sharp(srcPath, { animated: entry.animated })
    .resize(MAX_DIM, MAX_DIM, { fit: "inside", withoutEnlargement: true })
    .webp({ quality: 82 })
    .toFile(outPath);

  const outStat = await fs.stat(outPath);
  totalBefore += srcStat.size;
  totalAfter += outStat.size;
  converted++;

  await fs.unlink(srcPath);
  entry.file = newFile;
  console.log(`${entry.id}: ${(srcStat.size / 1024).toFixed(1)} KB -> ${(outStat.size / 1024).toFixed(1)} KB`);
}

if (converted > 0) {
  await fs.writeFile(MANIFEST_PATH, JSON.stringify(manifest, null, 2) + "\n");
  console.log(`\nConverted ${converted} file(s): ${(totalBefore / 1024).toFixed(1)} KB -> ${(totalAfter / 1024).toFixed(1)} KB`);
} else {
  console.log("Nothing to convert - all manifest entries already point at .webp files.");
}
