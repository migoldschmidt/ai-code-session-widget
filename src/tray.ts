// Status in the Windows taskbar (fork):
// - tray icon in the notification area: the widget's mascot in the pose of
//   the most urgent status present (Waiting > Question > Working > Idle),
//   with a dot in that status' colour bottom left; tooltip with all counts
// - single click on the tray icon hides the widget or shows it again where
//   it was; double click shows it right above the tray icon (it stays open
//   and keeps sitting on the taskbar while it grows or shrinks, until you
//   drag or resize it). Right click: "Show / hide widget" and "Quit".
// - taskbar button: the mascot in the most urgent pose replaces the app
//   icon; an overlay badge on top shows the number of sessions that need
//   you (Waiting + Question), red if any is Waiting, else blue
// - no sessions: tray and taskbar show the default picture, the logo that
//   is also the app icon (src/public/logo.png), without dot or badge
// Icons are drawn on a canvas and handed over as raw RGBA. Pictures come
// from the app itself; nothing is downloaded.

import { Image } from "@tauri-apps/api/image";
import { Menu } from "@tauri-apps/api/menu";
import { TrayIcon } from "@tauri-apps/api/tray";
import { PhysicalPosition, currentMonitor, getCurrentWindow, monitorFromPoint } from "@tauri-apps/api/window";
import { mascotSnapshotSvg } from "./mascot";
import { STATUS_PRIORITY, type VisualStatus } from "./sessionOrder";

const TRAY_ID = "session-widget-status";
const ICON_SIZE = 32;

// Light-theme status colours from style.css (the taskbar doesn't follow the widget theme).
const STATUS_COLORS: Record<VisualStatus, string> = {
  needsInput: "#dc2626",
  waitingAlert: "#2563eb",
  working: "#16a34a",
  waiting: "#8a8580",
};

const STATUS_WORDS: Record<VisualStatus, string> = {
  needsInput: "waiting",
  waitingAlert: "question",
  working: "working",
  waiting: "idle",
};

// Animation step of the procedural mascots to freeze for the icon (idle: a
// step where the "z" is visible).
const SNAPSHOT_TICK: Record<VisualStatus, number> = {
  needsInput: 0,
  waitingAlert: 0,
  working: 0,
  waiting: 20,
};

const appWindow = getCurrentWindow();
let tray: TrayIcon | null = null;
let lastTraySignature = "";
let lastOverlaySignature = "";
let lastTaskbarStatus: VisualStatus | "none" | null = null;

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

function countLabel(count: number): string {
  return count > 9 ? "9+" : String(count);
}

function newCanvas(): { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D | null } {
  const canvas = document.createElement("canvas");
  canvas.width = ICON_SIZE;
  canvas.height = ICON_SIZE;
  return { canvas, ctx: canvas.getContext("2d") };
}

function rgbaOf(ctx: CanvasRenderingContext2D): Uint8Array {
  return new Uint8Array(ctx.getImageData(0, 0, ICON_SIZE, ICON_SIZE).data.buffer);
}

/** Filled (or, with `hollow`, outlined) circle with an optional white label, as RGBA. */
function drawBadge(color: string, label: string, hollow = false): Uint8Array {
  const { ctx } = newCanvas();
  if (!ctx) return new Uint8Array(ICON_SIZE * ICON_SIZE * 4);

  const center = ICON_SIZE / 2;
  ctx.beginPath();
  if (hollow) {
    ctx.arc(center, center, center - 4, 0, Math.PI * 2);
    ctx.lineWidth = 4;
    ctx.strokeStyle = color;
    ctx.stroke();
  } else {
    ctx.arc(center, center, center - 0.5, 0, Math.PI * 2);
    ctx.fillStyle = color;
    ctx.fill();
  }

  if (label) {
    const fontSize = Math.round(ICON_SIZE * (label.length > 1 ? 0.56 : 0.7));
    ctx.fillStyle = "#ffffff";
    ctx.font = `700 ${fontSize}px "Segoe UI", system-ui, sans-serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(label, center, center + ICON_SIZE * 0.04);
  }
  return rgbaOf(ctx);
}

/** Size the mascot is rendered at before it is cropped and scaled into the icon. */
const MASCOT_RENDER_SIZE = 128;
/** How much larger than "whole mascot fits" the tray shows it (2 = twice as large, edges cropped). */
// 1.2: mascot slightly larger than "fits"; outer edges (feet, zzz) may touch the border.
const MASCOT_ZOOM = 1.2;

/**
 * Optional own tray picture: put a PNG at src/public/tray-mascot.png (served
 * as /tray-mascot.png, git-ignored). If present it replaces the built-in
 * mascot in the tray; the status dot stays. Loaded from the app itself — the
 * widget never fetches anything from the internet.
 */
const CUSTOM_TRAY_IMAGE_URL = "/tray-mascot.png";
/** Default picture while no session runs: the logo from the app icon (bundled, src/public/logo.png). */
const DEFAULT_IMAGE_URL = "/logo.png";
const loadedImages = new Map<string, HTMLImageElement | null>();

/** Loads a bundled picture once; null if it is missing or can't be decoded. */
async function loadImage(url: string): Promise<HTMLImageElement | null> {
  if (loadedImages.has(url)) return loadedImages.get(url) ?? null;
  let result: HTMLImageElement | null = null;
  try {
    const img = document.createElement("img");
    img.src = url;
    await img.decode(); // fails if the file is missing (the dev server answers with HTML)
    result = img.naturalWidth > 0 ? img : null;
  } catch {
    result = null;
  }
  loadedImages.set(url, result);
  return result;
}

function loadCustomTrayImage(): Promise<HTMLImageElement | null> {
  return loadImage(CUSTOM_TRAY_IMAGE_URL);
}

/**
 * Bounding box of the visible (alpha > 16) pixels — the mascot only fills
 * part of its 16x16 grid, so cropping to it makes it as large as the icon.
 */
function contentBounds(ctx: CanvasRenderingContext2D, size: number): { x: number; y: number; w: number; h: number } | null {
  const data = ctx.getImageData(0, 0, size, size).data;
  let minX = size;
  let minY = size;
  let maxX = -1;
  let maxY = -1;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (data[(y * size + x) * 4 + 3] > 16) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }
  return maxX < 0 ? null : { x: minX, y: minY, w: maxX - minX + 1, h: maxY - minY + 1 };
}

interface MascotIconOptions {
  /** Output edge length in px. */
  size: number;
  /** 1 = whole mascot fits, 2 = twice as large (edges cropped). */
  zoom: number;
  /** Status-coloured dot in the bottom-right corner. */
  dot: boolean;
  /** Fill the icon with a rounded square in the status colour, mascot on top with a white outline. */
  background: boolean;
}

// Tray: mascot slightly larger than "fits", status as a dot bottom left.
// (background: true fills the icon with the status colour instead.)
const TRAY_MASCOT: MascotIconOptions = { size: ICON_SIZE, zoom: MASCOT_ZOOM, dot: true, background: false };
// Taskbar button: bigger icon (24–48 px), whole mascot, no dot — the
// overlay badge sits in that corner.
const TASKBAR_MASCOT: MascotIconOptions = { size: 64, zoom: 1, dot: false, background: false };
// No sessions: the whole logo, no dot (there is no status to show).
const TRAY_DEFAULT: MascotIconOptions = { size: ICON_SIZE, zoom: 1, dot: false, background: false };
const TASKBAR_DEFAULT: MascotIconOptions = TASKBAR_MASCOT;

/** The mascot in `status`' pose, cropped to the icon, as RGBA. */
async function drawMascotIcon(status: VisualStatus, options: MascotIconOptions): Promise<Uint8Array> {
  const blob = new Blob([mascotSnapshotSvg(status, SNAPSHOT_TICK[status])], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  try {
    const img = document.createElement("img");
    img.src = url;
    await img.decode();
    return drawPictureIcon(img, status, options);
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Rounded-square path covering the whole icon. */
function roundedSquare(ctx: CanvasRenderingContext2D, size: number, radius: number): void {
  ctx.beginPath();
  ctx.moveTo(radius, 0);
  ctx.arcTo(size, 0, size, size, radius);
  ctx.arcTo(size, size, 0, size, radius);
  ctx.arcTo(0, size, 0, 0, radius);
  ctx.arcTo(0, 0, size, 0, radius);
  ctx.closePath();
}

/**
 * Draws `layer` onto `ctx` with a white outline around its visible pixels,
 * so the orange mascot stays readable on every status colour (also red).
 */
function drawWithOutline(ctx: CanvasRenderingContext2D, layer: HTMLCanvasElement, width: number): void {
  const silhouette = document.createElement("canvas");
  silhouette.width = layer.width;
  silhouette.height = layer.height;
  const sctx = silhouette.getContext("2d");
  if (sctx) {
    sctx.drawImage(layer, 0, 0);
    sctx.globalCompositeOperation = "source-in";
    sctx.fillStyle = "#ffffff";
    sctx.fillRect(0, 0, silhouette.width, silhouette.height);
    for (let i = 0; i < 8; i++) {
      const angle = (i / 8) * Math.PI * 2;
      ctx.drawImage(silhouette, Math.cos(angle) * width, Math.sin(angle) * width);
    }
  }
  ctx.drawImage(layer, 0, 0);
}

/** Any picture (mascot SVG or own PNG), cropped to its visible pixels, with status background or dot, as RGBA. */
function drawPictureIcon(picture: HTMLImageElement, status: VisualStatus, options: MascotIconOptions): Uint8Array {
  const { size, zoom, dot, background } = options;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  const render = document.createElement("canvas");
  render.width = MASCOT_RENDER_SIZE;
  render.height = MASCOT_RENDER_SIZE;
  const renderCtx = render.getContext("2d");
  if (!ctx || !renderCtx) return new Uint8Array(size * size * 4);

  // Fit the picture into the square render canvas, keeping its aspect ratio.
  const w = picture.naturalWidth || MASCOT_RENDER_SIZE;
  const h = picture.naturalHeight || MASCOT_RENDER_SIZE;
  const fit = MASCOT_RENDER_SIZE / Math.max(w, h);
  renderCtx.imageSmoothingQuality = "high";
  renderCtx.drawImage(
    picture,
    (MASCOT_RENDER_SIZE - w * fit) / 2,
    (MASCOT_RENDER_SIZE - h * fit) / 2,
    w * fit,
    h * fit,
  );

  // Square crop around the visible pixels, zoomed in `zoom` times — Windows
  // fixes the tray icon at 16–24 px, so "bigger" means cropping in. Centred
  // slightly above the middle so the face stays in view; with zoom > 1 arms,
  // feet and the edges of the sign/zzz may be cut off.
  const box = contentBounds(renderCtx, MASCOT_RENDER_SIZE) ?? { x: 0, y: 0, w: MASCOT_RENDER_SIZE, h: MASCOT_RENDER_SIZE };
  const side = (Math.max(box.w, box.h) * 1.04) / zoom;
  const sx = box.x + box.w / 2 - side / 2;
  const sy = box.y + box.h * (zoom > 1 ? 0.45 : 0.5) - side / 2;

  if (background) {
    // Status colour fills the icon; the mascot sits on top with a white outline.
    roundedSquare(ctx, size, size * 0.22);
    ctx.fillStyle = STATUS_COLORS[status];
    ctx.fill();
    ctx.save();
    roundedSquare(ctx, size, size * 0.22);
    ctx.clip();
    const layer = document.createElement("canvas");
    layer.width = size;
    layer.height = size;
    const lctx = layer.getContext("2d");
    if (lctx) {
      lctx.imageSmoothingQuality = "high";
      lctx.drawImage(render, sx, sy, side, side, 0, 0, size, size);
      drawWithOutline(ctx, layer, Math.max(1, size * 0.045));
    }
    ctx.restore();
  } else {
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(render, sx, sy, side, side, 0, 0, size, size);
  }

  if (dot) {
    // Bottom-left status dot: keeps the urgency readable at 16 px, where the
    // mascot poses look alike.
    const r = size * 0.16;
    ctx.beginPath();
    ctx.arc(r + 0.5, size - r - 0.5, r, 0, Math.PI * 2);
    ctx.fillStyle = STATUS_COLORS[status];
    ctx.fill();
    ctx.lineWidth = size * 0.047;
    ctx.strokeStyle = "#ffffff";
    ctx.stroke();
  }
  return new Uint8Array(ctx.getImageData(0, 0, size, size).data.buffer);
}

/**
 * Default picture (no sessions): an own tray picture if present and
 * allowed, else the logo. Null if neither can be drawn.
 */
async function defaultIconRgba(options: MascotIconOptions, allowCustom: boolean): Promise<Uint8Array | null> {
  try {
    const picture = (allowCustom ? await loadCustomTrayImage() : null) ?? (await loadImage(DEFAULT_IMAGE_URL));
    if (picture) return drawPictureIcon(picture, "waiting", options);
  } catch (err) {
    console.error("Failed to draw the default icon", err);
  }
  return null;
}

/** Tray without sessions: default picture, else an empty grey ring. */
async function trayDefaultRgba(): Promise<Uint8Array> {
  return (await defaultIconRgba(TRAY_DEFAULT, true)) ?? drawBadge(STATUS_COLORS.waiting, "", true);
}

async function trayIconRgba(top: VisualStatus | undefined, count: number): Promise<Uint8Array> {
  if (!top) return trayDefaultRgba();
  try {
    const custom = await loadCustomTrayImage();
    if (custom) return drawPictureIcon(custom, top, TRAY_MASCOT);
    return await drawMascotIcon(top, TRAY_MASCOT);
  } catch (err) {
    // e.g. the SVG could not be decoded — fall back to a coloured dot.
    console.error("Failed to draw the mascot tray icon", err);
    return drawBadge(STATUS_COLORS[top], countLabel(count));
  }
}

// ---------------------------------------------------------------------------
// Flyout above the tray icon
// ---------------------------------------------------------------------------

/**
 * Physical-pixel anchor while the widget sits above the tray icon (after a
 * double click); null when it is wherever the user put it.
 */
interface FlyoutAnchor {
  centerX: number;
  minX: number;
  maxX: number;
  /** Bottom edge to keep (taskbar at the bottom) … */
  bottom: number;
  /** … or top edge to keep (taskbar at the top). */
  top: number | null;
}

let flyout: FlyoutAnchor | null = null;

// A double click arrives as Click(Up), DoubleClick, Click(Up). The single
// click toggle therefore waits briefly and is dropped if a double click
// follows; the trailing Click(Up) after a double click is ignored.
const SINGLE_CLICK_DELAY_MS = 250;
const AFTER_DOUBLE_CLICK_IGNORE_MS = 600;
let singleClickTimer: number | undefined;
let lastDoubleClick = 0;

async function placeFlyout(): Promise<void> {
  if (!flyout) return;
  const size = await appWindow.outerSize();
  const x = Math.max(flyout.minX, Math.min(Math.round(flyout.centerX - size.width / 2), flyout.maxX - size.width));
  const y = flyout.top !== null ? flyout.top : Math.round(flyout.bottom - size.height);
  await appWindow.setPosition(new PhysicalPosition(x, y));
}

async function showWidget(): Promise<void> {
  await appWindow.show();
  await appWindow.unminimize();
  await appWindow.setFocus();
}

async function openFlyout(iconX: number, iconY: number, iconW: number, iconH: number): Promise<void> {
  const monitor = (await monitorFromPoint(iconX + iconW / 2, iconY + iconH / 2)) ?? (await currentMonitor());
  if (monitor) {
    const work = monitor.workArea;
    const margin = Math.round(8 * monitor.scaleFactor);
    const iconInUpperHalf = iconY + iconH / 2 < monitor.position.y + monitor.size.height / 2;
    flyout = {
      centerX: iconX + iconW / 2,
      minX: work.position.x + margin,
      maxX: work.position.x + work.size.width - margin,
      bottom: work.position.y + work.size.height - margin,
      top: iconInUpperHalf ? work.position.y + margin : null,
    };
    await placeFlyout();
  }
  await showWidget();
}

/** Keeps an open flyout attached to the tray after the window changed size (auto size). */
export async function reanchorFlyout(): Promise<void> {
  try {
    await placeFlyout();
  } catch (err) {
    console.error("Failed to re-anchor the flyout", err);
  }
}

/** The user moved or resized the window: from now on it is a normal widget. */
export function pinFlyout(): void {
  flyout = null;
}

/** Single click (and tray menu): hide the widget, or show it again where it was. */
async function toggleWidget(): Promise<void> {
  try {
    if (await appWindow.isVisible()) {
      await appWindow.hide();
    } else {
      await showWidget();
      await reanchorFlyout(); // still above the tray if it was opened there
    }
  } catch (err) {
    console.error("Failed to toggle the widget window", err);
  }
}

function onTrayClick(): void {
  if (Date.now() - lastDoubleClick < AFTER_DOUBLE_CLICK_IGNORE_MS) return;
  window.clearTimeout(singleClickTimer);
  singleClickTimer = window.setTimeout(() => void toggleWidget(), SINGLE_CLICK_DELAY_MS);
}

/** Double click: show the widget right above the tray icon (it stays open). */
function onTrayDoubleClick(iconX: number, iconY: number, iconW: number, iconH: number): void {
  window.clearTimeout(singleClickTimer);
  lastDoubleClick = Date.now();
  openFlyout(iconX, iconY, iconW, iconH).catch((err) => console.error("Failed to open above the tray", err));
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

function tooltipFor(counts: Record<VisualStatus, number>): string {
  const parts = STATUS_PRIORITY.filter((s) => counts[s] > 0).map((s) => `${counts[s]} ${STATUS_WORDS[s]}`);
  return `AI Code Session Widget — ${parts.length > 0 ? parts.join(", ") : "no active sessions"}`;
}

/** Creates the tray icon. `onQuit` closes the app. Safe to call again after a page reload. */
export async function initTray(onQuit: () => void): Promise<void> {
  try {
    // A dev-mode page reload keeps the old tray alive with a dead click handler.
    await TrayIcon.removeById(TRAY_ID).catch(() => undefined);
    const menu = await Menu.new({
      items: [
        { id: "toggle", text: "Show / hide widget", action: () => void toggleWidget() },
        { item: "Separator" },
        { id: "quit", text: "Quit", action: () => onQuit() },
      ],
    });
    tray = await TrayIcon.new({
      id: TRAY_ID,
      tooltip: "AI Code Session Widget",
      icon: await Image.new(await trayDefaultRgba(), ICON_SIZE, ICON_SIZE),
      menu,
      showMenuOnLeftClick: false,
      action: (event) => {
        if (event.type === "Click" && event.button === "Left" && event.buttonState === "Up") {
          onTrayClick();
        } else if (event.type === "DoubleClick" && event.button === "Left") {
          const { position, size } = event.rect;
          onTrayDoubleClick(position.x, position.y, size.width, size.height);
        }
      },
    });
    lastTraySignature = "";
  } catch (err) {
    console.error("Failed to create the tray icon", err);
  }
}

/** Updates tray icon, tooltip and taskbar badge. Cheap when nothing changed. */
export async function updateStatusIndicators(counts: Record<VisualStatus, number>): Promise<void> {
  const top = STATUS_PRIORITY.find((s) => counts[s] > 0);

  const traySignature = STATUS_PRIORITY.map((s) => counts[s]).join(",");
  if (tray && traySignature !== lastTraySignature) {
    lastTraySignature = traySignature;
    try {
      const image = await Image.new(await trayIconRgba(top, top ? counts[top] : 0), ICON_SIZE, ICON_SIZE);
      await tray.setIcon(image);
      await image.close();
      await tray.setTooltip(tooltipFor(counts));
    } catch (err) {
      console.error("Failed to update the tray icon", err);
    }
  }

  // Taskbar button icon: the mascot in the most urgent pose, the logo when
  // there are no sessions. Only redrawn when that changes.
  const taskbarStatus: VisualStatus | "none" = top ?? "none";
  if (taskbarStatus !== lastTaskbarStatus) {
    lastTaskbarStatus = taskbarStatus;
    try {
      // No sessions: logo; if it can't be drawn, the idle mascot as before.
      const rgba =
        (taskbarStatus === "none" ? await defaultIconRgba(TASKBAR_DEFAULT, false) : null) ??
        (await drawMascotIcon(taskbarStatus === "none" ? "waiting" : taskbarStatus, TASKBAR_MASCOT));
      const image = await Image.new(rgba, TASKBAR_MASCOT.size, TASKBAR_MASCOT.size);
      await appWindow.setIcon(image);
      await image.close();
    } catch (err) {
      console.error("Failed to set the taskbar icon", err);
    }
  }

  const attention = counts.needsInput + counts.waitingAlert;
  const overlaySignature = attention === 0 ? "none" : `${counts.needsInput > 0 ? "red" : "blue"}:${countLabel(attention)}`;
  if (overlaySignature !== lastOverlaySignature) {
    lastOverlaySignature = overlaySignature;
    try {
      if (attention === 0) {
        await appWindow.setOverlayIcon(undefined);
      } else {
        const color = counts.needsInput > 0 ? STATUS_COLORS.needsInput : STATUS_COLORS.waitingAlert;
        const image = await Image.new(drawBadge(color, countLabel(attention)), ICON_SIZE, ICON_SIZE);
        await appWindow.setOverlayIcon(image);
        await image.close();
      }
    } catch (err) {
      console.error("Failed to update the taskbar badge", err);
    }
  }
}
