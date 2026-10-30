# Packs dist/macos/AppIcon.icns from the Andromeda masters: the mark at every size.
# macOS icons sit inside the canvas with a 10% margin, so each master renders at 80% and is centred.
import struct
import subprocess
import sys
from pathlib import Path

BRANDING = Path(__file__).parent.parent / "branding"
TYPES = [("icp4", 16, "mark"), ("icp5", 32, "mark"), ("ic11", 32, "mark"), ("ic12", 64, "mark"),
         ("ic07", 128, "mark"), ("ic08", 256, "mark"), ("ic13", 256, "mark"), ("ic09", 512, "mark"),
         ("ic14", 512, "mark"), ("ic10", 1024, "mark")]


def png(size, master):
    inner = round(size * 0.8)
    art = subprocess.run(["rsvg-convert", "-w", str(inner), str(BRANDING / f"andromeda-{master}.svg")],
                         check=True, capture_output=True).stdout
    return subprocess.run(["magick", "png:-", "-background", "none", "-gravity", "center", "-extent",
                           f"{size}x{size}", "png:-"], input=art, check=True, capture_output=True).stdout


chunks = b"".join(kind.encode() + struct.pack(">I", len(data) + 8) + data
                  for kind, size, master in TYPES for data in [png(size, master)])
Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent / "AppIcon.icns").write_bytes(
    b"icns" + struct.pack(">I", len(chunks) + 8) + chunks)
