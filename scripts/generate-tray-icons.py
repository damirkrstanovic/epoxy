"""Regenerate committed SNI ARGB buffers; build/runtime needs no rendering dependency.
Requires rsvg-convert and Pillow only when changing the source SVG.
"""
from pathlib import Path
import subprocess
import tempfile
from PIL import Image

assets = Path(__file__).resolve().parent.parent / 'epoxy' / 'assets'
with tempfile.TemporaryDirectory(prefix='epoxy-icon-render-') as directory:
    for size in (16, 24, 32, 48):
        png = Path(directory) / f'{size}.png'
        subprocess.run(['rsvg-convert', '--width', str(size), '--height', str(size),
                        '--output', str(png), str(assets / 'tray.svg')], check=True)
        with Image.open(png) as image:
            rgba = image.convert('RGBA').tobytes()
        argb = bytearray()
        for offset in range(0, len(rgba), 4):
            r, g, b, a = rgba[offset:offset + 4]
            argb.extend((a, r, g, b))  # SNI ARGB32, network byte order
        (assets / f'tray-{size}.argb').write_bytes(argb)
