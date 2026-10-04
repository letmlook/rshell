// scripts/gen-icons.mjs
//
// 生成 RShell 的应用图标（PNG / ICO / ICNS）。
//
// 为什么需要脚本而不是手搓二进制：仓库里原来的 src-tauri/icons/*.png 是
// **占位图**（32x32.png 只有 104 字节、128x128.png 299 字节，纯色/全透明），
// 所以 dev 与打包后的窗口/任务栏/exe 都不显示任何图标。图标是二进制产物，
// 手改不可复现也不可评审；这里用纯 Node（只用内置 zlib）把矢量标记光栅化
// 成真正的 PNG，再组装 ICO/ICNS 容器，改设计只需改 drawMark()。
//
// 用法：npm run icons
//
// 标记设计（与 src/components/CustomTitleBar.vue 里的内联 SVG 保持一致）：
// 圆角方形 + 蓝色渐变底 + 近白的终端提示符 ">" + 光标块。深色任务栏上也有对比度。

import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync, utimesSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const ICON_DIR = join(ROOT, "src-tauri", "icons");

// ── 标记绘制 ──────────────────────────────────────────────────────────

/** 圆角矩形内部判定（用于底板） */
function insideRoundedRect(x, y, size, radius) {
  const inset = size * 0.045;
  const min = inset;
  const max = size - inset;
  if (x < min || x > max || y < min || y > max) return false;
  const cx = Math.min(Math.max(x, min + radius), max - radius);
  const cy = Math.min(Math.max(y, min + radius), max - radius);
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= radius * radius;
}

/** 点到线段的距离（用于画 ">" 的两条笔画） */
function distToSegment(px, py, ax, ay, bx, by) {
  const vx = bx - ax;
  const vy = by - ay;
  const wx = px - ax;
  const wy = py - ay;
  const len2 = vx * vx + vy * vy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, (wx * vx + wy * vy) / len2));
  const dx = px - (ax + t * vx);
  const dy = py - (ay + t * vy);
  return Math.sqrt(dx * dx + dy * dy);
}

/**
 * 生成 size×size 的 RGBA 像素缓冲。
 * 4× 超采样做抗锯齿：小尺寸（16/32）下边缘不会出现锯齿。
 */
function drawMark(size) {
  const px = Buffer.alloc(size * size * 4);
  const S = 4;
  const radius = size * 0.22;
  const stroke = size * 0.085;
  const glyph = { x: size * 0.36, y: size * 0.5 };
  const arm = size * 0.13;
  const reach = size * 0.17;
  const barW = size * 0.09;
  const barH = size * 0.17;
  const barX = size * 0.58;
  const barY = size * 0.5 - barH / 2;

  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      for (let sy = 0; sy < S; sy += 1) {
        for (let sx = 0; sx < S; sx += 1) {
          const fx = x + (sx + 0.5) / S;
          const fy = y + (sy + 0.5) / S;
          if (!insideRoundedRect(fx, fy, size, radius)) continue;
          // 底色：竖向渐变 #4f8ef7 → #1d4ed8
          const t = fy / size;
          let cr = 0x4f + (0x1d - 0x4f) * t;
          let cg = 0x8e + (0x4e - 0x8e) * t;
          let cb = 0xf7 + (0xd8 - 0xf7) * t;
          // 提示符 ">"：两条笔画；光标块
          const onChevron =
            distToSegment(fx, fy, glyph.x - arm, glyph.y - arm, glyph.x + reach, glyph.y) <= stroke / 2 ||
            distToSegment(fx, fy, glyph.x + reach, glyph.y, glyph.x - arm, glyph.y + arm) <= stroke / 2;
          const onBar = fx >= barX && fx <= barX + barW && fy >= barY && fy <= barY + barH;
          if (onChevron || onBar) {
            cr = 0xf8;
            cg = 0xfa;
            cb = 0xfc;
          }
          r += cr;
          g += cg;
          b += cb;
          a += 255;
        }
      }
      const n = S * S;
      const i = (y * size + x) * 4;
      if (a === 0) {
        px[i] = 0; px[i + 1] = 0; px[i + 2] = 0; px[i + 3] = 0;
        continue;
      }
      // 预乘还原：颜色按覆盖到的采样点平均，alpha 按覆盖率
      const covered = a / 255;
      px[i] = Math.round(r / covered);
      px[i + 1] = Math.round(g / covered);
      px[i + 2] = Math.round(b / covered);
      px[i + 3] = Math.round(a / n);
    }
  }
  return px;
}

// ── PNG 编码 ──────────────────────────────────────────────────────────

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i += 1) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([len, body, crc]);
}

function encodePng(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;  // bit depth
  ihdr[9] = 6;  // color type: RGBA
  ihdr[10] = 0; // deflate
  ihdr[11] = 0; // adaptive filter
  ihdr[12] = 0; // no interlace
  // 每行前置一个 filter 字节（0 = None）
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y += 1) {
    const rowStart = y * (size * 4 + 1);
    raw[rowStart] = 0;
    rgba.copy(raw, rowStart + 1, y * size * 4, (y + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ── ICO / ICNS 容器 ───────────────────────────────────────────────────

/** ICO：Vista+ 支持直接内嵌 PNG 条目 */
function buildIco(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  const dir = Buffer.alloc(16 * entries.length);
  let offset = 6 + dir.length;
  entries.forEach((entry, i) => {
    const at = i * 16;
    dir[at] = entry.size >= 256 ? 0 : entry.size;
    dir[at + 1] = entry.size >= 256 ? 0 : entry.size;
    dir[at + 2] = 0; // 调色板数
    dir[at + 3] = 0; // reserved
    dir.writeUInt16LE(1, at + 4);  // color planes
    dir.writeUInt16LE(32, at + 6); // bits per pixel
    dir.writeUInt32LE(entry.png.length, at + 8);
    dir.writeUInt32LE(offset, at + 12);
    offset += entry.png.length;
  });
  return Buffer.concat([header, dir, ...entries.map((e) => e.png)]);
}

/** ICNS：每项是 4 字节类型 + 4 字节长度（含 8 字节头）+ PNG 数据 */
function buildIcns(entries) {
  const body = entries.map(({ type, png }) => {
    const head = Buffer.alloc(8);
    head.write(type, 0, "ascii");
    head.writeUInt32BE(png.length + 8, 4);
    return Buffer.concat([head, png]);
  });
  const total = 8 + body.reduce((sum, b) => sum + b.length, 0);
  const head = Buffer.alloc(8);
  head.write("icns", 0, "ascii");
  head.writeUInt32BE(total, 4);
  return Buffer.concat([head, ...body]);
}

// ── 输出 ──────────────────────────────────────────────────────────────

function pngFor(size) {
  return encodePng(size, drawMark(size));
}

mkdirSync(ICON_DIR, { recursive: true });

const PNG_SIZES = [16, 32, 48, 64, 128, 256, 512, 1024];
const cache = new Map();
const png = (size) => {
  if (!cache.has(size)) cache.set(size, pngFor(size));
  return cache.get(size);
};

const written = [];
function write(name, data) {
  writeFileSync(join(ICON_DIR, name), data);
  written.push(`${name} (${data.length} B)`);
}

// Tauri 约定的文件名
write("32x32.png", png(32));
write("128x128.png", png(128));
write("128x128@2x.png", png(256));
write("icon.png", png(512));
write("128x128@2x@1x.png", png(256));

// Windows: 16/32/48/64/256
write("icon.ico", buildIco([16, 32, 48, 64, 256].map((size) => ({ size, png: png(size) }))));

// macOS: icp4=16 icp5=32 ic07=128 ic08=256 ic09=512 ic10=1024
write(
  "icon.icns",
  buildIcns([
    { type: "icp4", png: png(16) },
    { type: "icp5", png: png(32) },
    { type: "ic07", png: png(128) },
    { type: "ic08", png: png(256) },
    { type: "ic09", png: png(512) },
    { type: "ic10", png: png(1024) },
  ]),
);

// console.log(`已生成 ${written.length} 个图标文件：\n  ${written.join("\n  ")}`);

// ── 让 tauri 重新嵌入图标（重要）──
// 图标是**编译期**资源：tauri 的 build script 在编译时把它们写进 OUT_DIR 生成的
// Rust 源码。但 cargo 只按 build.rs 自身的指纹决定是否重跑该脚本——改了
// src-tauri/icons/*.png 不会触发它，于是增量构建里 exe / 安装包**仍然带着旧图标**
//（实测：图标已换成真图、exe 也重新链接了，抠出来的仍是纯色方块）。
// 这里主动 touch build.rs，下一次 cargo build / tauri dev / tauri build 就会
// 重新嵌入；验证办法见 docs/08 的说明。
const BUILD_RS = join(ICON_DIR, "..", "build.rs");
try {
  const now = new Date();
  utimesSync(BUILD_RS, now, now);
  console.log("已 touch src-tauri/build.rs：下次 cargo/tauri 构建会重新嵌入图标");
} catch (error) {
  console.warn(`未能 touch build.rs（${String(error)}）——请手动 cargo clean -p rshell 后重新构建，否则图标不会更新`);
}
