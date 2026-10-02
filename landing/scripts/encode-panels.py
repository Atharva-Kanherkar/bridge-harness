# Encodes the frames from record-panels.mjs into looping animated WebPs for the README.
import glob
import os

from PIL import Image

frames_root = "/tmp/bridge-panels"
media = os.path.join(os.path.dirname(__file__), "..", "..", "docs", "media")
width = 1000  # 2x captures downscaled: sharp at the README's 680px display width

for panel in sorted(os.listdir(frames_root)):
    paths = sorted(glob.glob(f"{frames_root}/{panel}/*.png"))
    if not paths:
        continue
    frames = []
    for path in paths:
        image = Image.open(path).convert("RGB")
        frames.append(image.resize((width, round(image.height * width / image.width)), Image.LANCZOS))
    # One tick is 80ms in the page; hold the finished frame a beat before looping.
    durations = [80] * (len(frames) - 1) + [1600]
    target = os.path.join(media, f"feature-{panel}.webp")
    frames[0].save(target, save_all=True, append_images=frames[1:], duration=durations, loop=0, quality=72, method=4)
    print(f"{panel}: {os.path.getsize(target) // 1024} KB")
