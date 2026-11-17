// 用代码绘制 FanGuard 程序图标（无第三方依赖）
// 输出：src-tauri/icons/ 下的 PNG 与多尺寸 ICO
//
// 绘制方式：在单位坐标系 [0,1]² 中用几何函数描述图形，
// 每个像素 4×4 超采样抗锯齿，按 "over" 规则逐层合成。

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync, readdirSync, unlinkSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "icons");

// ---------------------------------------------------------------------------
// 几何
// ---------------------------------------------------------------------------

const TAU = Math.PI * 2;

/** 圆角矩形（中心 0.5，半边长 h，圆角 r）内部判定 */
function inRoundRect(x, y, h, r) {
  const dx = Math.abs(x - 0.5) - (h - r);
  const dy = Math.abs(y - 0.5) - (h - r);
  const ox = Math.max(dx, 0);
  const oy = Math.max(dy, 0);
  return Math.hypot(ox, oy) + Math.min(Math.max(dx, dy), 0) <= r;
}

function polar(x, y) {
  const dx = x - 0.5;
  const dy = y - 0.5;
  return { r: Math.hypot(dx, dy), t: Math.atan2(dy, dx) };
}

/** 三片弯曲扇叶 */
const BLADES = 3;
const R_IN = 0.085;
const R_OUT = 0.315;
function inBlade(x, y) {
  const { r, t } = polar(x, y);
  if (r < R_IN || r > R_OUT) return false;
  const u = (r - R_IN) / (R_OUT - R_IN); // 0..1 沿叶片方向
  // 随半径旋转，形成弯曲的叶片
  const swirl = t + u * 1.15;
  const seg = TAU / BLADES;
  let a = ((swirl % seg) + seg) % seg - seg / 2;
  // 叶片角宽：根部窄、中部宽、尖端圆收
  const half = 0.62 * Math.pow(Math.sin(Math.PI * Math.min(1, u * 0.92 + 0.06)), 0.75) * (1 - 0.25 * u);
  // 前缘略微偏移，使叶片不对称更有动感
  a += 0.12 * u;
  return Math.abs(a) <= half;
}

function lerp(a, b, t) {
  return a + (b - a) * t;
}

function mix(c1, c2, t) {
  return [lerp(c1[0], c2[0], t), lerp(c1[1], c2[1], t), lerp(c1[2], c2[2], t)];
}

const hex = (h) => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
const C_TOP = hex("#22d3ee");
const C_MID = hex("#3b82f6");
const C_BOT = hex("#4338ca");
const WHITE = [255, 255, 255];
const HUB = hex("#1e40af");
const ACCENT = hex("#22c55e");

/**
 * 图层列表：每层返回 (x,y) 处的 [r,g,b,a]（a 为 0..1）或 null
 */
const layers = [
  // 背景：圆角方形 + 对角渐变
  (x, y) => {
    if (!inRoundRect(x, y, 0.47, 0.2)) return null;
    const t = Math.min(1, Math.max(0, (x * 0.35 + y * 0.65 - 0.05) / 0.9));
    const c = t < 0.5 ? mix(C_TOP, C_MID, t * 2) : mix(C_MID, C_BOT, (t - 0.5) * 2);
    return [...c, 1];
  },
  // 顶部高光
  (x, y) => {
    if (!inRoundRect(x, y, 0.47, 0.2)) return null;
    const a = Math.max(0, 0.16 * (1 - y / 0.55));
    return a > 0 ? [...WHITE, a] : null;
  },
  // 外圈导风环
  (x, y) => {
    const { r } = polar(x, y);
    return r >= 0.345 && r <= 0.385 ? [...WHITE, 0.4] : null;
  },
  // 扇叶阴影
  (x, y) => (inBlade(x - 0.012, y - 0.018) ? [0, 0, 40, 0.22] : null),
  // 扇叶
  (x, y) => {
    if (!inBlade(x, y)) return null;
    const { r } = polar(x, y);
    const shade = 0.92 + 0.08 * ((r - R_IN) / (R_OUT - R_IN));
    return [255 * shade, 255 * shade, 255, 1];
  },
  // 轴心
  (x, y) => (polar(x, y).r <= 0.085 ? [...WHITE, 1] : null),
  (x, y) => (polar(x, y).r <= 0.038 ? [...HUB, 1] : null),
  // 右下角状态指示灯（守护 / 正常）
  (x, y) => (Math.hypot(x - 0.795, y - 0.795) <= 0.085 ? [...WHITE, 1] : null),
  (x, y) => (Math.hypot(x - 0.795, y - 0.795) <= 0.062 ? [...ACCENT, 1] : null),
];

// ---------------------------------------------------------------------------
// 光栅化
// ---------------------------------------------------------------------------

function render(size) {
  const SS = size <= 32 ? 6 : 4;
  const px = Buffer.alloc(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let pxx = 0; pxx < size; pxx++) {
      // 预乘 alpha 累加
      let R = 0,
        G = 0,
        B = 0,
        A = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const x = (pxx + (sx + 0.5) / SS) / size;
          const y = (py + (sy + 0.5) / SS) / size;
          let r = 0,
            g = 0,
            b = 0,
            a = 0;
          for (const layer of layers) {
            const c = layer(x, y);
            if (!c) continue;
            const ca = c[3];
            r = c[0] * ca + r * (1 - ca);
            g = c[1] * ca + g * (1 - ca);
            b = c[2] * ca + b * (1 - ca);
            a = ca + a * (1 - ca);
          }
          R += r;
          G += g;
          B += b;
          A += a;
        }
      }
      const n = SS * SS;
      const i = (py * size + pxx) * 4;
      const alpha = A / n;
      px[i] = alpha > 0 ? Math.round(R / n / alpha) : 0;
      px[i + 1] = alpha > 0 ? Math.round(G / n / alpha) : 0;
      px[i + 2] = alpha > 0 ? Math.round(B / n / alpha) : 0;
      px[i + 3] = Math.round(alpha * 255);
    }
  }
  return px;
}

// ---------------------------------------------------------------------------
// PNG / ICO 编码
// ---------------------------------------------------------------------------

const CRC_TABLE = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}

function encodePng(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function encodeIco(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  const entries = [];
  let offset = 6 + images.length * 16;
  for (const { size, png } of images) {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e.writeUInt16LE(1, 4); // planes
    e.writeUInt16LE(32, 6); // bpp
    e.writeUInt32LE(png.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += png.length;
    entries.push(e);
  }
  return Buffer.concat([header, ...entries, ...images.map((i) => i.png)]);
}

// ---------------------------------------------------------------------------

mkdirSync(outDir, { recursive: true });
// 清理旧的图标文件
for (const f of readdirSync(outDir)) {
  if (/\.(png|ico|icns)$/i.test(f)) unlinkSync(join(outDir, f));
}

const cache = new Map();
const png = (size) => {
  if (!cache.has(size)) cache.set(size, encodePng(size, render(size)));
  return cache.get(size);
};

const files = {
  "32x32.png": 32,
  "64x64.png": 64,
  "128x128.png": 128,
  "128x128@2x.png": 256,
  "icon.png": 512,
};
for (const [name, size] of Object.entries(files)) {
  writeFileSync(join(outDir, name), png(size));
  console.log(`  ${name}`);
}
const icoSizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
writeFileSync(join(outDir, "icon.ico"), encodeIco(icoSizes.map((size) => ({ size, png: png(size) }))));
console.log(`  icon.ico (${icoSizes.join(", ")})`);
console.log("图标已生成：" + outDir);
