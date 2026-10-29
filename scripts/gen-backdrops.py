# Generates the built-in backdrop pictures (app/public/backdrops/*.jpg and
# thumbs/). Procedural, no source images: the output is original work under
# the repo's MIT licence. Needs numpy + Pillow; not part of the build.
#   python scripts/gen-backdrops.py app/public/backdrops
import numpy as np, sys, os
from PIL import Image

W, H = 2560, 1600

def hex2lin(h):
    h = h.lstrip('#')
    c = np.array([int(h[i:i+2], 16) for i in (0, 2, 4)], dtype=np.float32) / 255
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)

def lin2srgb(c):
    c = np.clip(c, 0, 1)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(c, 1 / 2.4) - 0.055)

def smooth(t):
    t = np.clip(t, 0, 1)
    return t * t * (3 - 2 * t)

def render(spec, w=W, h=H, seed=0):
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    u = x / w          # 0..1
    v = y / h          # 0..1
    aspect = w / h
    # Background: vertical blend of two colours plus soft glow blobs.
    top, bottom = hex2lin(spec['bg'][0]), hex2lin(spec['bg'][1])
    t = smooth(v)[..., None]
    img = top * (1 - t) + bottom * t
    for (bx, by, r, col, a) in spec.get('blobs', []):
        d2 = ((u - bx) * aspect) ** 2 + (v - by) ** 2
        g = np.exp(-d2 / (2 * r * r))[..., None] * a
        img = img * (1 - g) + hex2lin(col) * g
    # Wave bands, back to front: each band fills below a warped curve
    # with a soft edge and a lit rim, like the macOS abstract statics.
    for i, band in enumerate(spec['bands']):
        base, amp, freq, phase, col_a, col_b = band[:6]
        tilt = band[6] if len(band) > 6 else 0.0
        warp = 0.35 * np.sin(2.1 * u * np.pi + phase * 1.7) + 0.2 * np.sin(5.3 * u + phase)
        curve = (base + tilt * (u - 0.5)
                 + amp * np.sin(freq * np.pi * u + phase + warp)
                 + amp * 0.35 * np.sin(freq * 2.3 * np.pi * u - phase * 0.6))
        dist = (v - curve) * h  # px below the curve
        edge = 1 / (1 + np.exp(np.clip(-dist / 1.6, -60, 60)))  # anti-aliased edge
        depth = np.clip(dist / (h * 0.55), 0, 1)
        ca, cb = hex2lin(col_a), hex2lin(col_b)
        fill = ca * (1 - depth[..., None]) + cb * depth[..., None]
        # Lit rim just under the edge.
        rim = np.exp(-np.maximum(dist, 0) / (h * 0.012))[..., None] * 0.55
        lit = np.clip(ca * 1.9 + 0.02, 0, 1)
        fill = fill * (1 - rim) + lit * rim
        # Shadow cast above the edge on what is behind.
        shadow = np.exp(-np.maximum(-dist, 0) / (h * 0.022))[..., None] * 0.38 * (1 - edge[..., None])
        img = img * (1 - shadow)
        e = edge[..., None]
        img = img * (1 - e) + fill * e
    # Vignette.
    d = np.sqrt(((u - 0.5) * 1.2) ** 2 + (v - 0.5) ** 2)
    img = img * (1 - 0.35 * smooth((d - 0.35) / 0.5))[..., None]
    out = lin2srgb(img)
    # Fine grain so JPEG keeps the gradients free of banding.
    out = out + rng.normal(0, 1.1 / 255, out.shape).astype(np.float32)
    return Image.fromarray((np.clip(out, 0, 1) * 255 + 0.5).astype(np.uint8))

SPECS = {
  'aurora':   dict(bg=('#04121f', '#062a2e'), blobs=[(0.25, 0.25, 0.25, '#1fd1a5', 0.35), (0.8, 0.2, 0.3, '#2f6bff', 0.3)],
                   bands=[(0.42, 0.08, 1.2, 0.3, '#0e5f6b', '#04222b'), (0.58, 0.07, 1.6, 2.1, '#14a38b', '#063a3c'), (0.74, 0.06, 1.1, 4.0, '#3be3a6', '#0a4a44', -0.1)]),
  'dune':     dict(bg=('#f3c27a', '#c9692f'), blobs=[(0.7, 0.2, 0.3, '#ffe2a8', 0.5)],
                   bands=[(0.40, 0.07, 1.0, 0.8, '#e9944a', '#b4521f', 0.12), (0.56, 0.08, 1.3, 2.6, '#d4733a', '#8f3b17'), (0.72, 0.06, 0.9, 5.1, '#b8562a', '#5e2410', -0.08)]),
  'lagoon':   dict(bg=('#063a6b', '#0a7f9e'), blobs=[(0.3, 0.3, 0.3, '#35d7e8', 0.35)],
                   bands=[(0.45, 0.09, 1.1, 1.0, '#1b8fc4', '#08406e'), (0.62, 0.07, 1.5, 3.3, '#2fc3d6', '#0b5c7c', 0.1), (0.78, 0.05, 1.0, 0.2, '#7ee8e0', '#1386a0')]),
  'ember':    dict(bg=('#1a0505', '#4a0d0a'), blobs=[(0.75, 0.3, 0.28, '#ff6a2a', 0.35)],
                   bands=[(0.44, 0.08, 1.3, 0.4, '#8f1d14', '#2b0606'), (0.6, 0.07, 1.0, 2.9, '#d2451f', '#5a1009', -0.12), (0.76, 0.06, 1.4, 4.6, '#ff8a3d', '#8c2410')]),
  'orchid':   dict(bg=('#2a0f45', '#6b2a8c'), blobs=[(0.2, 0.2, 0.3, '#ff7ad9', 0.3)],
                   bands=[(0.42, 0.08, 1.2, 2.0, '#8e3fbf', '#3b1460'), (0.58, 0.07, 1.6, 4.4, '#c45bd6', '#5d1f7d', 0.1), (0.74, 0.06, 1.0, 1.1, '#f08ad8', '#8a2f8f')]),
  'midnight': dict(bg=('#05060f', '#0d1330'), blobs=[(0.7, 0.25, 0.3, '#2b3fa8', 0.35)],
                   bands=[(0.46, 0.07, 1.1, 3.5, '#17205a', '#070a1f'), (0.62, 0.06, 1.4, 0.9, '#243289', '#0b1033', -0.1), (0.78, 0.05, 0.9, 2.2, '#3c56c9', '#121a4d')]),
  'meadow':   dict(bg=('#0f2e1a', '#2f6b2c'), blobs=[(0.75, 0.2, 0.3, '#d8f06a', 0.35)],
                   bands=[(0.44, 0.08, 1.0, 1.5, '#2f7d3a', '#123d1e', 0.1), (0.6, 0.07, 1.4, 3.8, '#62a83e', '#1f5426'), (0.76, 0.06, 1.2, 0.6, '#b5d95a', '#3d7a2a', -0.06)]),
  'sunset':   dict(bg=('#2b1055', '#d1495b'), blobs=[(0.5, 0.35, 0.3, '#ffb36b', 0.45)],
                   bands=[(0.44, 0.08, 1.2, 0.2, '#e0605e', '#7a2357'), (0.6, 0.07, 1.0, 2.4, '#f08a5d', '#a0344f', 0.12), (0.76, 0.06, 1.5, 4.9, '#ffc07a', '#c8553d')]),
  'graphite': dict(bg=('#0e0f12', '#24262b'), blobs=[(0.7, 0.2, 0.3, '#4a4f5a', 0.35)],
                   bands=[(0.45, 0.07, 1.1, 2.3, '#2c2f35', '#121316'), (0.61, 0.06, 1.4, 4.2, '#3d4149', '#1a1c20', -0.1), (0.77, 0.05, 1.0, 0.7, '#5a5f69', '#26292e')]),
  'glacier':  dict(bg=('#bcd9f0', '#5f95c8'), bands=[], blobs=[(0.15, 0.2, 0.32, '#ffffff', 0.7), (0.85, 0.35, 0.3, '#8ec5ff', 0.6), (0.4, 0.85, 0.35, '#3d6fb0', 0.6), (0.9, 0.9, 0.3, '#b7e3f5', 0.6)]),
  'nebula':   dict(bg=('#0b0420', '#1d0b44'), bands=[], blobs=[(0.2, 0.3, 0.28, '#ff3fa4', 0.7), (0.8, 0.2, 0.3, '#3f6bff', 0.65), (0.55, 0.8, 0.32, '#8a2be2', 0.7), (0.05, 0.95, 0.25, '#1b0b5a', 0.6)]),
  'sakura':   dict(bg=('#fbe3e8', '#f2a7b8'), bands=[], blobs=[(0.2, 0.25, 0.3, '#fff3ea', 0.75), (0.85, 0.3, 0.3, '#ff9fbf', 0.6), (0.45, 0.9, 0.35, '#f07aa0', 0.55), (0.95, 0.95, 0.25, '#ffc9a8', 0.6)]),
}

if __name__ == '__main__':
    out = sys.argv[1]
    names = sys.argv[2:] or list(SPECS)  # the 12 shipped ids (settings.rs BUILTIN_BACKDROPS)
    os.makedirs(os.path.join(out, 'thumbs'), exist_ok=True)
    for name in names:
        # Seeded by name so a re-run reproduces the shipped files.
        img = render(SPECS[name], seed=sum(map(ord, name)))
        img.save(os.path.join(out, f'{name}.jpg'), quality=88, optimize=True, progressive=True)
        thumb = img.resize((240, 150), Image.LANCZOS)
        thumb.save(os.path.join(out, 'thumbs', f'{name}.jpg'), quality=85, optimize=True)
        print(name, os.path.getsize(os.path.join(out, f'{name}.jpg')) // 1024, 'KB')
