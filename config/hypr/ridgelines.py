# The wallpaper: the old dithered Matterhorn redrawn as ridgelines. Each line is a row of the old picture's
# brightness, blurred back out of its dots, lifted by how bright it is, with nearer rows hiding further ones.
# The old picture is in git: git show f69cf93:config/hypr/wallpaper.png > /tmp/matterhorn.png
#   python3 config/hypr/ridgelines.py 2560 1440 config/hypr/wallpaper.png /tmp/matterhorn.png
import numpy as np, sys
from PIL import Image, ImageDraw, ImageFilter
W, H = int(sys.argv[1]), int(sys.argv[2]); out = sys.argv[3]
S = 2; k = H / 1080
# The brightest a line gets: under the bar's white, so the wallpaper never outshines it
PEAK = 125
src = Image.open(sys.argv[4]).convert('L')
lum = np.asarray(src.filter(ImageFilter.GaussianBlur(6)).resize((W*S, H*S), Image.LANCZOS)).astype(float)/255
# Local detail, lifted most where it's dark, so the shadowed face has terrain too rather than flat lines
broad = np.asarray(src.filter(ImageFilter.GaussianBlur(28)).resize((W*S, H*S), Image.LANCZOS)).astype(float)/255
lift = lum + (lum - broad) * (1.0 + 4.0 * (1 - np.clip(broad * 2.5, 0, 1)))
# Where the mountain is: broadly, so the sparse dots of its shadowed face count, and closely, so lines start right at
# its lit edge rather than trailing out into the sky
sil = np.asarray(src.filter(ImageFilter.GaussianBlur(14)).resize((W*S, H*S), Image.LANCZOS)).astype(float)/255
near = np.asarray(src.filter(ImageFilter.GaussianBlur(5)).resize((W*S, H*S), Image.LANCZOS)).astype(float)/255
step, amp = 9*k, 30*k
img = Image.new('L', (W*S, H*S), 0); d = ImageDraw.Draw(img)
xs = np.arange(0, W*S, 2)
lw = max(1, round(S*k))
y0 = 60*k*S
while y0 < H*S + amp*S:
    yy = min(int(y0), H*S-1)
    l = lum[yy, xs]; m = np.minimum(sil[yy, xs] / 0.035, near[yy, xs] / 0.025)
    pts = list(zip(xs.tolist(), (y0 - amp*S*np.clip(lift[yy, xs], 0, 1.2)).tolist()))
    d.polygon(pts + [(W*S, H*S), (0, H*S)], fill=0)
    fade = min(1, max(0, (H*S - y0) / (110*k*S)))
    for i in range(len(xs)-1):
        if m[i] > 1 and m[i+1] > 1:
            b = min(1, 0.22 + 1.3*l[i]) * fade
            d.line([pts[i], pts[i+1]], fill=int(PEAK*b), width=lw)
    y0 += step*S
img = img.resize((W, H), Image.LANCZOS)
if out.endswith('.ppm'):
    img.convert('RGB').save(out)
else:
    img.save(out, optimize=True)
