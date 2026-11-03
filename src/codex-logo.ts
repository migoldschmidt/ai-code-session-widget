import codexSvg from "./mascot-svgs/codex.svg?raw";
import type { MascotStatus } from "./mascot";
import "./codex-logo.css";

const SVG_NS = "http://www.w3.org/2000/svg";
const template = new DOMParser().parseFromString(codexSvg, "image/svg+xml").documentElement;
const CENTER = 12;
const SLICE_RADIUS = 40;
let nextLogoId = 0;

export type CodexLogoVariant = "engine" | "portal" | "terminal";

function svgElement<K extends keyof SVGElementTagNameMap>(tag: K): SVGElementTagNameMap[K] {
  return document.createElementNS(SVG_NS, tag);
}

function useElement(id: string, className: string): SVGUseElement {
  const use = svgElement("use");
  use.setAttribute("href", `#${id}`);
  use.setAttribute("class", className);
  return use;
}

function slicePoints(index: number): string {
  // Slightly overlap neighbouring slices to prevent hairline gaps at
  // fractional Windows display scales.
  const overlap = 0.8;
  const start = ((index * 60 - 90 - overlap) * Math.PI) / 180;
  const end = (((index + 1) * 60 - 90 + overlap) * Math.PI) / 180;
  const point = (angle: number): string =>
    `${CENTER + Math.cos(angle) * SLICE_RADIUS},${CENTER + Math.sin(angle) * SLICE_RADIUS}`;
  return `${CENTER},${CENTER} ${point(start)} ${point(end)}`;
}

function statusLabel(status: MascotStatus): string {
  switch (status) {
    case "working":
      return "Codex — working";
    case "needsInput":
      return "Codex — waiting for input";
    case "waitingAlert":
      return "Codex — waiting for an answer";
    case "waiting":
      return "Codex — idle";
  }
}

const TYPE_CHARACTER_POOL = [
  "a", "b", "x", "k",
  "3", "7", "0", "9",
  "{", "}", "[", "]",
  "(", ")", "/", "*",
  "#", "+", "=", "?",
];

function typingCharacters(seed: string): string[] {
  let hash = 2166136261;
  for (const character of seed) {
    hash ^= character.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  const available = [...TYPE_CHARACTER_POOL];
  const result: string[] = [];
  for (let index = 0; index < 8; index += 1) {
    hash = Math.imul(hash ^ (index + 1), 2246822519);
    const poolIndex = Math.abs(hash) % available.length;
    result.push(available.splice(poolIndex, 1)[0]);
  }
  return result;
}

/** Builds a status-reactive Codex mark whose six sectors act like an engine. */
export function createCodexLogoElement(
  status: MascotStatus,
  variant: CodexLogoVariant = "engine",
  animationSeed: string = status,
): SVGSVGElement {
  const logo = document.importNode(template, true) as unknown as SVGSVGElement;
  const originalPath = logo.querySelector("path");
  if (!originalPath) return logo;

  const instanceId = `codex-engine-${nextLogoId++}`;
  const markId = `${instanceId}-mark`;

  logo.replaceChildren();
  logo.classList.add("codex-logo", "codex-engine");
  logo.dataset.status = status;
  logo.dataset.variant = variant;
  logo.setAttribute("role", "img");
  logo.setAttribute("aria-label", statusLabel(status));
  // A globally aligned negative delay keeps the phase continuous when the
  // session list is rebuilt by the two-second poll.
  logo.style.setProperty("--engine-clock", `${-performance.now()}ms`);

  // No SVG <title>: it would replace the card's tooltip (session details)
  // while hovering the logo; aria-label above names the status.

  const defs = svgElement("defs");
  const mark = originalPath.cloneNode(true) as SVGPathElement;
  mark.id = markId;
  defs.appendChild(mark);

  const clips: string[] = [];
  for (let index = 0; index < 6; index += 1) {
    const clipId = `${instanceId}-slice-${index}`;
    const clip = svgElement("clipPath");
    clip.id = clipId;
    clip.setAttribute("clipPathUnits", "userSpaceOnUse");
    const polygon = svgElement("polygon");
    polygon.setAttribute("points", slicePoints(index));
    clip.appendChild(polygon);
    defs.appendChild(clip);
    clips.push(clipId);
  }

  const halo = svgElement("circle");
  halo.setAttribute("class", "codex-engine-halo");
  halo.setAttribute("cx", String(CENTER));
  halo.setAttribute("cy", String(CENTER));
  halo.setAttribute("r", "10.5");

  const base = useElement(markId, "codex-engine-base");

  const segments = svgElement("g");
  segments.setAttribute("class", "codex-engine-segments");
  clips.forEach((clipId, index) => {
    const segment = useElement(markId, "codex-engine-segment");
    segment.setAttribute("clip-path", `url(#${clipId})`);
    segment.style.setProperty(
      "--segment-clock",
      `calc(var(--engine-clock) - ${index * 105}ms)`,
    );
    segments.appendChild(segment);
  });

  const core = svgElement("circle");
  core.setAttribute("class", "codex-engine-core");
  core.setAttribute("cx", String(CENTER));
  core.setAttribute("cy", String(CENTER));
  core.setAttribute("r", "1.35");

  const portalParticles = svgElement("g");
  portalParticles.setAttribute("class", "codex-portal-particles");
  const particlePositions = [
    { x: 12, y: -1 },
    { x: 25, y: 12 },
    { x: 1, y: 22 },
  ];
  particlePositions.forEach(({ x, y }, index) => {
    const particle = svgElement("circle");
    particle.setAttribute("class", `codex-portal-particle codex-portal-particle-${index + 1}`);
    particle.setAttribute("cx", String(x));
    particle.setAttribute("cy", String(y));
    particle.setAttribute("r", "0.9");
    particle.style.setProperty(
      "--particle-clock",
      `calc(var(--engine-clock) - ${index * 520}ms)`,
    );
    portalParticles.appendChild(particle);
  });

  // The prompt is cut out of the source logo's single compound path. For the
  // glyph study we paint those holes closed, then redraw the exact two
  // subpaths independently so `>` and `_` can move without changing the mark.
  const glyph = svgElement("g");
  glyph.setAttribute("class", "codex-glyph");
  glyph.setAttribute("aria-hidden", "true");
  const glyphPatch = svgElement("rect");
  glyphPatch.setAttribute("class", "codex-glyph-patch");
  glyphPatch.setAttribute("x", "5.25");
  glyphPatch.setAttribute("y", "7.5");
  glyphPatch.setAttribute("width", "13.25");
  glyphPatch.setAttribute("height", "9.15");
  const chevron = svgElement("path");
  chevron.setAttribute("class", "codex-glyph-chevron");
  chevron.setAttribute("d", "M7.282 8.307a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393z");
  const cursor = svgElement("path");
  cursor.setAttribute("class", "codex-glyph-cursor");
  cursor.setAttribute("d", "M12.728 14.547a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z");
  const alertDot = svgElement("circle");
  alertDot.setAttribute("class", "codex-glyph-alert-dot");
  alertDot.setAttribute("cx", "12");
  alertDot.setAttribute("cy", "18");
  alertDot.setAttribute("r", "1");
  const sleepZ = svgElement("path");
  sleepZ.setAttribute("class", "codex-glyph-sleep-z");
  sleepZ.setAttribute("d", "M8.1 8.7h7.8L8.1 16.3h7.8");
  sleepZ.setAttribute("pathLength", "1");
  const questionMark = svgElement("g");
  questionMark.setAttribute("class", "codex-glyph-question-mark");
  const questionHook = svgElement("path");
  questionHook.setAttribute("class", "codex-glyph-question-hook");
  questionHook.setAttribute("d", "M8.2 9.5c0-2.25 1.55-3.7 3.9-3.7 2.3 0 3.9 1.35 3.9 3.45 0 1.75-.9 2.55-2.15 3.35-1.05.68-1.55 1.25-1.55 2.35");
  questionHook.setAttribute("pathLength", "1");
  const questionDot = svgElement("circle");
  questionDot.setAttribute("class", "codex-glyph-question-dot");
  questionDot.setAttribute("cx", "12.3");
  questionDot.setAttribute("cy", "18");
  questionDot.setAttribute("r", "1.05");
  questionMark.append(questionHook, questionDot);
  const typing = svgElement("g");
  typing.setAttribute("class", "codex-glyph-typing");
  const typingClipId = `${instanceId}-typing-clip`;
  const typingClip = svgElement("clipPath");
  typingClip.id = typingClipId;
  typingClip.setAttribute("clipPathUnits", "userSpaceOnUse");
  const typingWindow = svgElement("rect");
  typingWindow.setAttribute("x", "8.4");
  typingWindow.setAttribute("y", "9.5");
  typingWindow.setAttribute("width", "8.8");
  typingWindow.setAttribute("height", "6.2");
  typingClip.appendChild(typingWindow);
  defs.appendChild(typingClip);
  typing.setAttribute("clip-path", `url(#${typingClipId})`);

  const sequence = typingCharacters(animationSeed).join("");
  const ribbon = svgElement("text");
  ribbon.setAttribute("class", "codex-glyph-type-ribbon");
  ribbon.setAttribute("x", "8.7");
  ribbon.setAttribute("y", "12.6");
  ribbon.setAttribute("dominant-baseline", "central");
  ribbon.textContent = sequence + sequence;
  typing.appendChild(ribbon);
  glyph.append(glyphPatch, chevron, cursor, alertDot, sleepZ, questionMark, typing);

  const signal = svgElement("g");
  signal.setAttribute("class", "codex-engine-signal");
  signal.setAttribute("aria-hidden", "true");
  const signalDisc = svgElement("circle");
  signalDisc.setAttribute("class", "codex-engine-signal-disc");
  signalDisc.setAttribute("cx", "21");
  signalDisc.setAttribute("cy", "3");
  signalDisc.setAttribute("r", "3.4");
  const signalText = svgElement("text");
  signalText.setAttribute("class", "codex-engine-signal-text");
  signalText.setAttribute("x", "21");
  signalText.setAttribute("y", "3.25");
  signalText.setAttribute("text-anchor", "middle");
  signalText.setAttribute("dominant-baseline", "central");
  signalText.textContent = status === "needsInput" ? "!" : "?";
  signal.append(signalDisc, signalText);

  logo.append(defs, halo, base, segments, portalParticles, core, glyph, signal);
  return logo;
}
