#!/usr/bin/env python3
"""Generate RUSTARUS logo bitmap using Spleen 32x64 PSF font via psftools."""
import struct, sys, subprocess, os

FONT = "/opt/homebrew/Caskroom/font-spleen/2.2.0/spleen-2.2.0/spleen-32x64.psfu"
TXT = "/tmp/spleen32x64.txt"

def export_psf_txt():
    if not os.path.exists(TXT):
        subprocess.run(["psf2txt", FONT, TXT], check=True)

def parse_glyphs():
    """Parse psf2txt output into {unicode_char: [[bool]]}."""
    export_psf_txt()
    glyphs = {}
    with open(TXT) as f:
        text = f.read()

    for block in text.split("%\n"):
        if "Bitmap:" not in block:
            continue
        # Extract bitmap rows
        bmp_start = block.index("Bitmap:") + len("Bitmap:")
        bmp_end = block.index("Unicode:")
        bmp_text = bmp_start
        bitmap_str = block[bmp_start:bmp_end].replace("\\\n", "").replace("\\", "")
        rows = []
        for piece in bitmap_str.strip().split():
            rows.append([c == '#' for c in piece])

        # Extract unicode
        uni_line = block[bmp_end:]
        # Parse [XXXXXX] hex values
        import re
        codes = re.findall(r'\[([0-9a-fA-F]+)\]', uni_line)
        for code in codes:
            cp = int(code, 16)
            if cp < 128:
                glyphs[chr(cp)] = rows
    return glyphs

def render_text(text, glyphs, spacing=2):
    if not text:
        return []
    height = len(glyphs.get('A', []))
    char_grids = []
    for ch in text:
        g = glyphs.get(ch)
        if g is None:
            # Space: use glyph width of empty pixels
            w = len(glyphs.get('A', [[]])[0])
            char_grids.append([[False] * w for _ in range(height)])
        else:
            char_grids.append(g)

    rows = []
    for y in range(height):
        row = []
        for i, g in enumerate(char_grids):
            if i > 0:
                row.extend([False] * spacing)
            if y < len(g):
                row.extend(g[y])
        rows.append(row)
    return rows

def rainbow_color(y, height):
    """Generate Atari-style rainbow: warm colors at top, cool at bottom."""
    # Hue sweep from red/orange through yellow, green, cyan, blue, purple
    t = y / max(height - 1, 1)
    # HSV to RGB with full saturation and brightness
    hue = t * 270  # 0=red, 60=yellow, 120=green, 180=cyan, 240=blue, 270=purple
    h = hue / 60.0
    i = int(h)
    f = h - i
    q = int(255 * (1 - f))
    t_val = int(255 * f)
    if i == 0:   return (255, t_val, 0)
    elif i == 1: return (q, 255, 0)
    elif i == 2: return (0, 255, t_val)
    elif i == 3: return (0, q, 255)
    elif i == 4: return (t_val, 0, 255)
    else:        return (255, 0, q)

def write_bmp(filename, grid, bg=(0, 0, 0)):
    height = len(grid)
    width = len(grid[0]) if grid else 0
    row_bytes = width * 3
    row_pad = (4 - row_bytes % 4) % 4
    pixel_size = (row_bytes + row_pad) * height
    file_size = 54 + pixel_size

    with open(filename, 'wb') as f:
        f.write(b'BM')
        f.write(struct.pack('<I', file_size))
        f.write(struct.pack('<HH', 0, 0))
        f.write(struct.pack('<I', 54))
        f.write(struct.pack('<I', 40))
        f.write(struct.pack('<i', width))
        f.write(struct.pack('<i', height))
        f.write(struct.pack('<HH', 1, 24))
        f.write(struct.pack('<I', 0))
        f.write(struct.pack('<I', pixel_size))
        f.write(struct.pack('<ii', 2835, 2835))
        f.write(struct.pack('<II', 0, 0))

        for y in range(height - 1, -1, -1):
            fg = rainbow_color(y, height)
            for x in range(width):
                c = fg if grid[y][x] else bg
                f.write(bytes([c[2], c[1], c[0]]))
            f.write(b'\x00' * row_pad)

if __name__ == '__main__':
    out = 'assets/logo.bmp'
    for arg in sys.argv[1:]:
        if not arg.startswith('-'):
            out = arg

    glyphs = parse_glyphs()
    grid = render_text("RUSTARUS", glyphs, spacing=2)

    # Trim top/bottom to match side margins
    h = len(grid)
    w = len(grid[0]) if grid else 0
    top = next((y for y in range(h) if any(grid[y])), 0)
    bot = next((y for y in range(h - 1, -1, -1) if any(grid[y])), h - 1)
    side_margin = min(
        next(x for x in range(w) if grid[y][x])
        for y in range(top, bot + 1) if any(grid[y])
    )
    grid = grid[top - side_margin : bot + side_margin + 1]

    write_bmp(out, grid, bg=(0, 0, 0))
    w = len(grid[0]) if grid else 0
    h = len(grid)
    print(f"Generated {out}: {w}x{h} pixels")
