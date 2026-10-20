"""Generates the AI Code Widget logo as SVG + PNG (transparent) and as an app icon."""
import re, sys
import numpy as np
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.path import Path
from matplotlib.patches import PathPatch
from PIL import Image

OUT = sys.argv[1] if len(sys.argv) > 1 else "."

# ---------- Geometry (coordinates in the space 0..720 x -80..640) ----------
VB = (0, -80, 720, 720)  # x, y, w, h

BRACE = "M222 34 H205 Q132 34 132 112 V198 L46 275 L132 352 V438 Q132 516 205 516 H222"
BRACE_R = "M498 34 H515 Q588 34 588 112 V198 L674 275 L588 352 V438 Q588 516 515 516 H498"

FOREHEAD = ("M207 160 C230 108 295 88 360 88 C425 88 490 108 513 160 "
            "Q437 148 387 164 L333 164 Q283 148 207 160 Z")
FACE = ("M200 222 C220 204 245 212 270 223 C295 234 315 228 333 214 L333 158 L387 158 L387 214 "
        "C405 228 425 234 450 223 C475 212 500 204 520 222 L520 268 "
        "C518 345 470 495 360 495 C250 495 202 345 200 268 Z")
# Nose & mouth as cut-out lines (stroke width NM_W, round caps)
NOSE = "M341 318 L360 341 L379 318"          # slim chevron
MOUTH = "M322 408 Q360 408 398 408"          # neutral, straight
NM_W = 14
CHEEK_L = "M194 226 L194 268 C196 315 208 365 228 406 C254 340 246 272 194 226 Z"
CHEEK_R = "M526 226 L526 268 C524 315 512 365 492 406 C466 340 474 272 526 226 Z"
EYE_L = "M244 192 Q274 198 304 192"
EYE_R = "M416 192 Q446 198 476 192"
SPARK = ("M360 104 C363.5 121 367 124.5 384 128 C367 131.5 363.5 135 360 152 "
         "C356.5 135 353 131.5 336 128 C353 124.5 356.5 121 360 104 Z")
SPARK_S = ("M640 -8 C641.8 1 643 2.2 652 4 C643 5.8 641.8 7 640 16 "
           "C638.2 7 637 5.8 628 4 C637 2.2 638.2 1 640 -8 Z")

# Colors
# Gradient of the right brace (top -> bottom) for the whole logo
G_A, G_B = "#40A1EF", "#2DC1EE"
G_Y0, G_Y1 = 34, 516
CK, CK_OP = "#1E3A8A", 0.35           # cheek shading (subtle, blue-tinted)


def svg(icon=False):
    x, y, w, h = VB
    bg = ""
    if icon:
        bg = (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="160" fill="url(#bg)"/>')
    scale = ' transform="translate(360 280) scale(0.8) translate(-360 -280)"' if icon else ""
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x} {y} {w} {h}" width="1024" height="1024">
  <defs>
    <linearGradient id="g" x1="0" y1="{G_Y0}" x2="0" y2="{G_Y1}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="{G_A}"/><stop offset="1" stop-color="{G_B}"/>
    </linearGradient>
    <linearGradient id="bg" x1="0" y1="-80" x2="720" y2="640" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1E1B4B"/><stop offset="1" stop-color="#0F172A"/>
    </linearGradient>
    <clipPath id="faceclip"><path d="{FACE}"/></clipPath>
    <mask id="cut" maskUnits="userSpaceOnUse" x="0" y="-80" width="720" height="720">
      <rect x="0" y="-80" width="720" height="720" fill="#fff"/>
      <g fill="none" stroke="#000" stroke-width="{NM_W}" stroke-linecap="round" stroke-linejoin="round">
        <path d="{NOSE}"/><path d="{MOUTH}"/>
      </g>
    </mask>
  </defs>
  {bg}
  <g{scale}>
    <g fill="none" stroke="url(#g)" stroke-width="44" stroke-linecap="round" stroke-linejoin="miter">
      <path d="{BRACE}"/>
      <path d="{BRACE_R}"/>
    </g>
    <g fill="url(#g)" fill-rule="evenodd">
      <path d="{FOREHEAD} {SPARK}"/>
      <path d="{FACE}" mask="url(#cut)"/>
    </g>
    <g fill="{CK}" fill-opacity="{CK_OP}" clip-path="url(#faceclip)" mask="url(#cut)">
      <path d="{CHEEK_L}"/>
      <path d="{CHEEK_R}"/>
    </g>
    <g fill="none" stroke="url(#g)" stroke-width="9" stroke-linecap="round">
      <path d="{EYE_L}"/>
      <path d="{EYE_R}"/>
    </g>
  </g>
</svg>
'''


# ---------- Rasterization ----------
def parse(d):
    toks = re.findall(r"[MLHVCQZ]|-?\d*\.?\d+", d)
    verts, codes, i, cur, start, cmd = [], [], 0, (0, 0), (0, 0), None
    num = lambda: float(toks[i])
    while i < len(toks):
        t = toks[i]
        if t.isalpha():
            cmd = t; i += 1
            if cmd == "Z":
                verts.append(start); codes.append(Path.CLOSEPOLY); cur = start
            continue
        if cmd == "M":
            cur = (float(toks[i]), float(toks[i + 1])); i += 2
            verts.append(cur); codes.append(Path.MOVETO); start = cur; cmd = "L"
        elif cmd == "L":
            cur = (float(toks[i]), float(toks[i + 1])); i += 2
            verts.append(cur); codes.append(Path.LINETO)
        elif cmd == "H":
            cur = (float(toks[i]), cur[1]); i += 1; verts.append(cur); codes.append(Path.LINETO)
        elif cmd == "V":
            cur = (cur[0], float(toks[i])); i += 1; verts.append(cur); codes.append(Path.LINETO)
        elif cmd == "C":
            p = [float(v) for v in toks[i:i + 6]]; i += 6
            verts += [(p[0], p[1]), (p[2], p[3]), (p[4], p[5])]; codes += [Path.CURVE4] * 3
            cur = (p[4], p[5])
        elif cmd == "Q":
            p = [float(v) for v in toks[i:i + 4]]; i += 4
            verts += [(p[0], p[1]), (p[2], p[3])]; codes += [Path.CURVE3] * 2
            cur = (p[2], p[3])
    return Path(verts, codes)


def mask(d, size, xf, stroke=None, cap="round", join="miter"):
    """Alpha mask (0..1) of a path. xf=(scale, ox, oy) viewbox->pixels."""
    s, ox, oy = xf
    fig = plt.figure(figsize=(size / 72, size / 72), dpi=72)
    ax = fig.add_axes([0, 0, 1, 1]); ax.axis("off")
    ax.set_xlim(0, size); ax.set_ylim(size, 0)
    p = parse(d)
    p = Path((p.vertices - [ox, oy]) * s + 0, p.codes)
    if stroke:
        patch = PathPatch(p, fill=False, ec="white", lw=stroke * s, capstyle=cap, joinstyle=join)
    else:
        patch = PathPatch(p, fc="white", ec="none")
    ax.add_patch(patch)
    fig.patch.set_alpha(0); ax.patch.set_alpha(0)
    fig.canvas.draw()
    a = np.asarray(fig.canvas.buffer_rgba())[..., 3].astype(float) / 255
    plt.close(fig)
    return a


def hexrgb(h):
    h = h.lstrip("#"); return np.array([int(h[i:i + 2], 16) for i in (0, 2, 4)], float) / 255


def grad(size, xf, p0, p1, c0, c1):
    s, ox, oy = xf
    yy, xx = np.mgrid[0:size, 0:size] + 0.5
    X, Y = xx / s + ox, yy / s + oy
    v = np.array(p1, float) - p0
    t = np.clip(((X - p0[0]) * v[0] + (Y - p0[1]) * v[1]) / (v @ v), 0, 1)[..., None]
    return hexrgb(c0) * (1 - t) + hexrgb(c1) * t


def over(dst, rgb, a):
    a = a[..., None]
    out_a = a + dst[..., 3:] * (1 - a)
    out_rgb = (rgb * a + dst[..., :3] * dst[..., 3:] * (1 - a)) / np.maximum(out_a, 1e-6)
    return np.concatenate([out_rgb, out_a], -1)


def render(size, icon=False):
    x, y, w, h = VB
    s = size / w
    if icon:
        k = 0.8
        # Scaling content around (360,280) by 0.8 = enlarging the viewbox accordingly
        s2 = s * k
        ox = 360 - (360 - x) / k; oy = 280 - (280 - y) / k
        xf = (s2, ox, oy)
    else:
        xf = (s, x, y)
    img = np.zeros((size, size, 4))
    if icon:
        bgm = rounded_rect(size, 160 * s)
        img = over(img, grad(size, (s, x, y), (0, -80), (720, 640), "#1E1B4B", "#0F172A"), bgm)
    br = np.maximum(mask(BRACE, size, xf, 44), mask(BRACE_R, size, xf, 44))
    g = grad(size, xf, (0, G_Y0), (0, G_Y1), G_A, G_B)
    img = over(img, g, br)
    face = mask(FACE, size, xf)
    holes = np.maximum(mask(NOSE, size, xf, NM_W, join="round"), mask(MOUTH, size, xf, NM_W))
    fh = mask(FOREHEAD, size, xf) * (1 - mask(SPARK, size, xf))
    fc = np.maximum(fh, face * (1 - holes))
    img = over(img, g, fc)
    ck = np.maximum(mask(CHEEK_L, size, xf), mask(CHEEK_R, size, xf)) * face * (1 - holes) * CK_OP
    img = over(img, np.broadcast_to(hexrgb(CK), img[..., :3].shape), ck)
    eyes = np.maximum(mask(EYE_L, size, xf, 9), mask(EYE_R, size, xf, 9))
    img = over(img, g, eyes)
    return Image.fromarray((np.clip(img, 0, 1) * 255 + 0.5).astype(np.uint8), "RGBA")


def rounded_rect(size, r):
    yy, xx = np.mgrid[0:size, 0:size] + 0.5
    dx = np.maximum(np.maximum(r - xx, xx - (size - r)), 0)
    dy = np.maximum(np.maximum(r - yy, yy - (size - r)), 0)
    return np.clip(r - np.hypot(dx, dy) + 0.5, 0, 1)


if __name__ == "__main__":
    open(f"{OUT}/logo.svg", "w").write(svg())
    open(f"{OUT}/app-icon.svg", "w").write(svg(icon=True))
    render(1024).save(f"{OUT}/logo.png")
    render(1024, icon=True).save(f"{OUT}/app-icon.png")
    for n in (512, 256, 128, 64):
        render(n * 2).resize((n, n), Image.LANCZOS).save(f"{OUT}/logo-{n}.png")
    print("ok")
