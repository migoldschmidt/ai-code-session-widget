// Pixel-grid mascot renderer — builds an SVG of <rect>/<circle>/<text>
// elements per status animation, no image assets. Style inspired by a
// reference (not reused code): blocky quadruped, warm orange palette.
//
// SVG over canvas: each body part is a real DOM node, so you can open
// devtools, click a <rect>, and tweak its x/y/width/height live to dial in
// proportions — canvas gives you none of that, only pixels.

import workingSvgRaw from "./mascot-svgs/working.svg?raw";
import figurVenterSvgRaw from "./mascot-svgs/figur-venter.svg?raw";

// "waitingAlert" is a frontend-only presentation variant (not a backend
// status): "waiting" + activity "Ended with a question" renders differently
// from plain "waiting", even though both are SessionStatus::Waiting.
export type MascotStatus = "working" | "needsInput" | "waiting" | "waitingAlert";

const GRID = { w: 16, h: 16, cell: 4.5 };
export const MASCOT_WIDTH = GRID.w * GRID.cell;
export const MASCOT_HEIGHT = GRID.h * GRID.cell;

// "working" and "waitingAlert" are fully authored, self-contained SVGs with
// native SMIL <animate>/<animateTransform> timelines (hand-built, frame-
// accurate to a reference video) — not drawn procedurally like the other
// two statuses. They're parsed once into detached templates here and cloned
// per mascot instance; their own timelines run entirely in the browser and
// are never touched by this module's JS tick loop.
function parseSvgTemplate(raw: string): SVGSVGElement {
  const doc = new DOMParser().parseFromString(raw, "image/svg+xml");
  return doc.documentElement as unknown as SVGSVGElement;
}

interface ExternalSvgSpec {
  template: SVGSVGElement;
  viewBox: { x: number; y: number; w: number; h: number };
}

function readViewBox(svg: SVGSVGElement): { x: number; y: number; w: number; h: number } {
  const attr = svg.getAttribute("viewBox");
  if (!attr) return { x: 0, y: 0, w: MASCOT_WIDTH, h: MASCOT_HEIGHT };
  const [x, y, w, h] = attr.split(/\s+/).map(Number);
  return { x, y, w, h };
}

const workingSvg: ExternalSvgSpec = (() => {
  const template = parseSvgTemplate(workingSvgRaw);
  return { template, viewBox: readViewBox(template) };
})();

const figurVenterSvg: ExternalSvgSpec = (() => {
  const template = parseSvgTemplate(figurVenterSvgRaw);
  return { template, viewBox: readViewBox(template) };
})();

/** Clones an external template's children into a <g>, scaled/centered to fit our canvas. */
function mountExternalSvg(spec: ExternalSvgSpec): SVGGElement {
  const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
  Array.from(spec.template.childNodes).forEach((node) => {
    group.appendChild(node.cloneNode(true));
  });
  const { x, y, w, h } = spec.viewBox;
  // 1.5x on top of the fit-to-canvas scale — these two are meant to read
  // larger/bolder than the procedural mascots, per explicit request.
  const scale = Math.min(MASCOT_WIDTH / w, MASCOT_HEIGHT / h) * 1.5;
  const offsetX = (MASCOT_WIDTH - w * scale) / 2;
  const offsetY = (MASCOT_HEIGHT - h * scale) / 2;
  group.setAttribute(
    "transform",
    `translate(${offsetX}, ${offsetY}) scale(${scale}) translate(${-x}, ${-y})`,
  );
  return group;
}

// Staggered "zzz" sleep effect: this many "z"s rise and fade in a loop,
// each offset from the next so more than one is visible at once.
const Z_COUNT = 3;
const Z_CYCLE = 45;
const Z_STAGGER = 15;

const PALETTE = {
  body: "#d97757",
  eye: "#221f1c",
  badgeRed: "#dc2626",
  badgeWhite: "#ffffff",
  z: "#8a8580",
};

// Body-part geometry, in grid units (not pixels) — tweak these directly,
// or open devtools and drag the resulting <rect> attributes instead.
const BODY = {
  torso: { x: 3, y: 6, w: 8, h: 6 },
  armL: { x: 1, y: 8, w: 2, h: 2 },
  armR: { x: 11, y: 8, w: 2, h: 2 },
  eyeL: { x: 4, y: 7 },
  eyeR: { x: 9, y: 7 },
  legs: [
    { x: 3, y: 12 },
    { x: 5, y: 12 },
    { x: 8, y: 12 },
    { x: 10, y: 12 },
  ],
  legH: 2,
};

const SVG_NS = "http://www.w3.org/2000/svg";

function px(gridUnits: number): number {
  return gridUnits * GRID.cell;
}

function svgEl<K extends keyof SVGElementTagNameMap>(tag: K): SVGElementTagNameMap[K] {
  return document.createElementNS(SVG_NS, tag);
}

function rect(className: string, fill: string): SVGRectElement {
  const el = svgEl("rect");
  el.setAttribute("class", className);
  el.setAttribute("fill", fill);
  return el;
}

function setRect(el: SVGRectElement, gx: number, gy: number, gw: number, gh: number): void {
  el.setAttribute("x", String(px(gx)));
  el.setAttribute("y", String(px(gy)));
  el.setAttribute("width", String(px(gw)));
  el.setAttribute("height", String(px(gh)));
}

function setVisible(el: SVGElement, visible: boolean): void {
  el.setAttribute("opacity", visible ? "1" : "0");
}

interface Refs {
  bodyGroup: SVGGElement;
  workingSvgGroup: SVGGElement;
  figurVenterSvgGroup: SVGGElement;
  swayGroup: SVGGElement;
  torso: SVGRectElement;
  armL: SVGRectElement;
  armR: SVGRectElement;
  legs: SVGRectElement[];
  legTops: SVGRectElement[];
  eyeL: SVGRectElement;
  eyeR: SVGRectElement;
  signGroup: SVGGElement;
  pole: SVGRectElement;
  badgeCircle: SVGCircleElement;
  badgeText: SVGTextElement;
  zTexts: SVGTextElement[];
}

const refsByElement = new WeakMap<SVGSVGElement, Refs>();

/** Builds one mascot's full DOM structure. Call once per session card. */
export function createMascotElement(): SVGSVGElement {
  const svg = svgEl("svg");
  svg.setAttribute("viewBox", `0 0 ${MASCOT_WIDTH} ${MASCOT_HEIGHT}`);
  svg.setAttribute("width", String(MASCOT_WIDTH));
  svg.setAttribute("height", String(MASCOT_HEIGHT));
  svg.classList.add("mascot-svg");

  const workingSvgGroup = mountExternalSvg(workingSvg);
  workingSvgGroup.classList.add("m-working-svg");
  const figurVenterSvgGroup = mountExternalSvg(figurVenterSvg);
  figurVenterSvgGroup.classList.add("m-figur-venter-svg");

  const bodyGroup = svgEl("g");
  bodyGroup.classList.add("m-body-group");

  // Sway group: the parts that lean together during needsInput's side-to-side
  // rock (torso, left hand, eyes, and the upper half of each leg), while the
  // feet (legs, below) and the right hand/sign (its own animation) stay out
  // of this group so they can move independently or stay planted.
  const swayGroup = svgEl("g");
  swayGroup.classList.add("m-sway-group");

  const torso = rect("m-torso", PALETTE.body);
  const armL = rect("m-arm-l", PALETTE.body);
  const armR = rect("m-arm-r", PALETTE.body);
  const legs = BODY.legs.map((_, i) => rect(`m-leg m-leg-${i}`, PALETTE.body));
  const legTops = BODY.legs.map((_, i) => rect(`m-leg-top m-leg-top-${i}`, PALETTE.body));
  const eyeL = rect("m-eye-l", PALETTE.eye);
  const eyeR = rect("m-eye-r", PALETTE.eye);

  swayGroup.append(torso, ...legTops, armL, eyeL, eyeR);
  bodyGroup.append(swayGroup, ...legs, armR);

  const signGroup = svgEl("g");
  signGroup.classList.add("m-sign-group");

  const pole = rect("m-sign-pole", PALETTE.eye);

  const badgeCircle = svgEl("circle");
  badgeCircle.setAttribute("class", "m-badge-circle");
  badgeCircle.setAttribute("fill", PALETTE.badgeRed);

  const badgeText = svgEl("text");
  badgeText.setAttribute("class", "m-badge-text");
  badgeText.setAttribute("fill", PALETTE.badgeWhite);
  badgeText.setAttribute("text-anchor", "middle");
  badgeText.setAttribute("dominant-baseline", "central");
  badgeText.setAttribute("font-size", String(GRID.cell * 1.5));
  badgeText.setAttribute("font-weight", "bold");
  badgeText.textContent = "!";

  signGroup.append(pole, badgeCircle, badgeText);

  const zTexts = Array.from({ length: Z_COUNT }, () => {
    const z = svgEl("text");
    z.setAttribute("class", "m-z");
    z.setAttribute("fill", PALETTE.z);
    z.setAttribute("text-anchor", "middle");
    z.setAttribute("font-weight", "bold");
    z.textContent = "z";
    return z;
  });

  svg.append(bodyGroup, signGroup, ...zTexts, workingSvgGroup, figurVenterSvgGroup);

  refsByElement.set(svg, {
    workingSvgGroup,
    figurVenterSvgGroup,
    bodyGroup,
    swayGroup,
    torso,
    armL,
    armR,
    legs,
    legTops,
    eyeL,
    eyeR,
    signGroup,
    pole,
    badgeCircle,
    badgeText,
    zTexts,
  });

  return svg;
}

function updateBody(refs: Refs, armLift: [number, number] = [0, 0]): void {
  const { torso, legs, armL, armR } = refs;
  setRect(torso, BODY.torso.x, BODY.torso.y, BODY.torso.w, BODY.torso.h);
  BODY.legs.forEach((leg, i) => setRect(legs[i], leg.x, leg.y, 1, BODY.legH));
  setRect(armL, BODY.armL.x, BODY.armL.y - armLift[0], BODY.armL.w, BODY.armL.h);
  setRect(armR, BODY.armR.x, BODY.armR.y - armLift[1], BODY.armR.w, BODY.armR.h);
}

function updateEyes(refs: Refs, closed: boolean): void {
  const { eyeL, eyeR } = refs;
  if (closed) {
    setRect(eyeL, BODY.eyeL.x, BODY.eyeL.y + 0.4, 1, 0.25);
    setRect(eyeR, BODY.eyeR.x, BODY.eyeR.y + 0.4, 1, 0.25);
    return;
  }
  setRect(eyeL, BODY.eyeL.x, BODY.eyeL.y, 1, 1);
  setRect(eyeR, BODY.eyeR.x, BODY.eyeR.y, 1, 1);
}

function hideExtras(refs: Refs): void {
  setVisible(refs.signGroup, false);
  refs.signGroup.removeAttribute("transform");
  refs.armR.removeAttribute("transform");
  refs.zTexts.forEach((z) => setVisible(z, false));
  refs.legTops.forEach((legTop) => setVisible(legTop, false));
  refs.bodyGroup.removeAttribute("transform");
  refs.swayGroup.removeAttribute("transform");
}

// --- Needs input: rocks side to side, holds up a warning sign that waves ---

const NEEDS_INPUT_CYCLE = 5; // ~130ms/tick -> ~650ms/cycle, ~6 cycles per 4s

// Body rock: torso, left hand, eyes and the upper half of each leg lean
// together, swinging smoothly between -0.5 and +0.5 grid units, while the
// feet (lower half of each leg) and the right hand/sign (animated separately
// below) stay out of this group. Continuous sine rather than a discrete
// step list — a stepped [0,-0.5,0,0.5,0] sequence repeats 0 back-to-back at
// the loop seam, which reads as a stutter/pause at center.
const SWAY_AMPLITUDE = 0.5;

function updateNeedsInput(refs: Refs, tick: number): void {
  hideExtras(refs);
  updateBody(refs);
  updateEyes(refs, false);

  const swayPhase = ((tick % NEEDS_INPUT_CYCLE) / NEEDS_INPUT_CYCLE) * Math.PI * 2;

  // Left hand grows to match the right hand's 2x2 size while the sign is
  // being held/waved (see mascot-hand-size-rule memory note) — it doesn't
  // do anything itself, but staying its resting 1x2 size would look
  // lopsided next to the raised, enlarged right hand.
  // Grow outward from the body (right edge anchored at the torso's left
  // edge), not inward — growing inward would just merge into the torso and
  // only the original 1-wide sliver would read as a visible hand.
  // It only drops when the body leans left, and never rises past its
  // resting position — clamping to one direction (rather than a full
  // symmetric bob) is also what reads as the slower of the two motions,
  // since it sits still at rest for the other half of the sway cycle.
  const armLBobY = Math.max(0, -Math.sin(swayPhase)) * 0.5;
  const armLX = BODY.torso.x - 2;
  setRect(refs.armL, armLX, BODY.armL.y + armLBobY, 2, 2);

  const legHalfH = BODY.legH / 2;
  BODY.legs.forEach((leg, i) => {
    setRect(refs.legTops[i], leg.x, leg.y, 1, legHalfH);
    setVisible(refs.legTops[i], true);
    setRect(refs.legs[i], leg.x, leg.y + legHalfH, 1, legHalfH);
  });

  const swayDx = String(px(Math.sin(swayPhase) * SWAY_AMPLITUDE));
  refs.swayGroup.setAttribute("transform", `translate(${swayDx}, 0)`);

  // Right arm is raised straight up, hand resting on top of the head, holding
  // a sign (pole + flag). The whole hand+sign swings side to side together as
  // one rigid unit, so it reads as a waved sign, not a balloon on a string.
  // Swing range is defined relative to the hand's own width: at its
  // rightmost point the hand protrudes half a hand-width past the head's
  // right edge; at its leftmost point it sits a full hand-width further in
  // (i.e. its rightmost extreme, one hand-width to the left).
  // Hand grows to 2x2 while actively holding/waving the sign, rather than
  // staying its resting size (see mascot-hand-size-rule memory note).
  const handW = 2;
  const handH = 2;
  const headRightEdge = BODY.torso.x + BODY.torso.w;
  const handX = headRightEdge - handW;
  const swingAmplitude = handW / 2;
  const handY = BODY.torso.y - handH;
  setRect(refs.armR, handX, handY, handW, handH);

  const poleHeight = 2.2;
  const poleIntoHand = 0.5;
  setRect(refs.pole, handX + handW / 2 - 0.1, handY - poleHeight, 0.2, poleHeight + poleIntoHand);

  const flagX = px(handX + handW / 2);
  const flagY = px(handY - poleHeight);
  refs.badgeCircle.setAttribute("cx", String(flagX));
  refs.badgeCircle.setAttribute("cy", String(flagY));
  refs.badgeCircle.setAttribute("r", String(px(1.3)));
  refs.badgeText.setAttribute("x", String(flagX));
  refs.badgeText.setAttribute("y", String(flagY));

  setVisible(refs.signGroup, true);
  const wavePhase = swayPhase;
  const dx = String(px(Math.sin(wavePhase) * swingAmplitude));
  refs.armR.setAttribute("transform", `translate(${dx}, 0)`);
  refs.signGroup.setAttribute("transform", `translate(${dx}, 0)`);
}

// --- Waiting (plain): lies still, eyes closed, a slow "z" floats and fades ---

const IDLE_CYCLE = 60;

function updateIdle(refs: Refs, tick: number): void {
  hideExtras(refs);
  updateBody(refs);
  updateEyes(refs, true);

  // Plain waiting/sleeping keeps the smaller resting hands — the 2x2 size
  // elsewhere is for actively doing/holding something (see
  // mascot-hand-size-rule memory note), which doesn't apply here.
  setRect(refs.armL, BODY.armL.x + 1, BODY.armL.y, 1, BODY.armL.h);
  setRect(refs.armR, BODY.armR.x, BODY.armR.y, 1, BODY.armR.h);

  refs.zTexts.forEach((z, i) => {
    const local = (tick + i * Z_STAGGER) % Z_CYCLE;
    const p = local / Z_CYCLE;
    const alpha = Math.sin(p * Math.PI);
    if (alpha <= 0.05) {
      setVisible(z, false);
      return;
    }
    setVisible(z, true);
    z.setAttribute("opacity", String(alpha));
    z.setAttribute("font-size", String(px(1.6 + p * 1.4)));
    z.setAttribute("x", String(px(BODY.torso.x + BODY.torso.w / 2 + (i - 1) * 0.8)));
    z.setAttribute("y", String(px(BODY.torso.y - 1 - p * 3)));
  });
}


function update(refs: Refs, status: MascotStatus, tick: number): void {
  // "working" and "waitingAlert" are fully authored external SVGs with their
  // own native SMIL timelines — just show the right one and hide our
  // procedural rig entirely; no per-tick JS drawing for these two.
  if (status === "working" || status === "waitingAlert") {
    setVisible(refs.bodyGroup, false);
    hideExtras(refs);
    setVisible(refs.workingSvgGroup, status === "working");
    setVisible(refs.figurVenterSvgGroup, status === "waitingAlert");
    return;
  }

  setVisible(refs.bodyGroup, true);
  setVisible(refs.workingSvgGroup, false);
  setVisible(refs.figurVenterSvgGroup, false);

  if (status === "needsInput") updateNeedsInput(refs, tick);
  else updateIdle(refs, tick % IDLE_CYCLE);
}

/**
 * Static SVG markup of the mascot as shown for `status` at animation step
 * `tick` (fork: used to draw the tray icon, see tray.ts). Visibility is set
 * via attributes, so the markup renders correctly outside the page's CSS.
 */
export function mascotSnapshotSvg(status: MascotStatus, tick = 0): string {
  const svg = createMascotElement();
  const refs = refsByElement.get(svg);
  if (refs) update(refs, status, tick);
  return new XMLSerializer().serializeToString(svg);
}

// --- Registry: keeps animation phase alive across DOM re-renders ---
//
// main.ts rebuilds the session list (and its mascot elements) on every 2s
// poll. Animation state (per-pid tick counters) lives here instead, so
// swapping in a fresh element doesn't restart the animation from frame 0.

interface Entry {
  refs: Refs;
  status: MascotStatus;
}

const TICK_MS = 130;

class MascotRegistry {
  private entries = new Map<string | number, Entry>();
  private ticks = new Map<string | number, number>();
  private acc = 0;
  private last = performance.now();
  private started = false;

  set(pid: string | number, element: SVGSVGElement, status: MascotStatus): void {
    const refs = refsByElement.get(element);
    if (!refs) return;
    this.entries.set(pid, { refs, status });
    if (!this.ticks.has(pid)) this.ticks.set(pid, 0);
    this.ensureLoop();
  }

  prune(activePids: Set<string | number>): void {
    for (const pid of this.entries.keys()) {
      if (!activePids.has(pid)) {
        this.entries.delete(pid);
        this.ticks.delete(pid);
      }
    }
  }

  private ensureLoop(): void {
    if (this.started) return;
    this.started = true;
    requestAnimationFrame(this.frame);
  }

  private frame = (now: number): void => {
    const dt = now - this.last;
    this.last = now;
    this.acc += dt;
    while (this.acc >= TICK_MS) {
      this.acc -= TICK_MS;
      for (const pid of this.ticks.keys()) {
        this.ticks.set(pid, (this.ticks.get(pid) ?? 0) + 1);
      }
    }
    for (const [pid, entry] of this.entries) {
      update(entry.refs, entry.status, this.ticks.get(pid) ?? 0);
    }
    requestAnimationFrame(this.frame);
  };
}

export const mascots = new MascotRegistry();
