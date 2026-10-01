"""Draw the JustTools context-menu icons.

    python explorer/icons/build.py

Each icon is a rounded colour tile with a white glyph, drawn on a 32-unit grid
and rendered separately at every size so the 16 px menu version stays crisp.
The tile carries its own contrast, so one set serves light and dark menus.
Requires Pillow. The generated .ico and .png files are committed.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
SUPERSAMPLE = 8
WHITE = (255, 255, 255, 255)


class Canvas:
    """A drawing surface addressed in 32-unit grid coordinates."""

    def __init__(self, size):
        self.pixels = size * SUPERSAMPLE
        self.scale = self.pixels / 32
        self.image = Image.new("RGBA", (self.pixels, self.pixels), (0, 0, 0, 0))
        self.draw = ImageDraw.Draw(self.image)

    def p(self, value):
        return value * self.scale

    def box(self, x0, y0, x1, y1):
        return [self.p(x0), self.p(y0), self.p(x1), self.p(y1)]

    def rect(self, x0, y0, x1, y1, radius=0, fill=WHITE):
        self.draw.rounded_rectangle(self.box(x0, y0, x1, y1), self.p(radius), fill=fill)

    def ellipse(self, x0, y0, x1, y1, fill=WHITE):
        self.draw.ellipse(self.box(x0, y0, x1, y1), fill=fill)

    def polygon(self, points, fill=WHITE):
        self.draw.polygon([(self.p(x), self.p(y)) for x, y in points], fill=fill)

    def line(self, points, width, fill=WHITE):
        scaled = [(self.p(x), self.p(y)) for x, y in points]
        self.draw.line(scaled, fill=fill, width=round(self.p(width)), joint="curve")
        radius = self.p(width) / 2
        for x, y in (scaled[0], scaled[-1]):
            self.draw.ellipse([x - radius, y - radius, x + radius, y + radius], fill=fill)

    def arc(self, x0, y0, x1, y1, start, end, width, fill=WHITE):
        self.draw.arc(self.box(x0, y0, x1, y1), start, end, fill=fill, width=round(self.p(width)))

    def text(self, value, size, x, y):
        font = ImageFont.truetype("consolab.ttf", round(self.p(size)))
        self.draw.text((self.p(x), self.p(y)), value, font=font, fill=WHITE, anchor="mm")

    def tile(self, top, bottom):
        gradient = Image.new("RGBA", (1, self.pixels))
        for y in range(self.pixels):
            mix = y / (self.pixels - 1)
            gradient.putpixel(
                (0, y),
                tuple(round(a + (b - a) * mix) for a, b in zip(top, bottom)) + (255,),
            )
        gradient = gradient.resize((self.pixels, self.pixels))
        mask = Image.new("L", (self.pixels, self.pixels), 0)
        ImageDraw.Draw(mask).rounded_rectangle(self.box(1, 1, 31, 31), self.p(7.5), fill=255)
        self.image.paste(gradient, (0, 0), mask)

    def finish(self, size):
        return self.image.resize((size, size), Image.LANCZOS)


def rgb(value):
    return tuple(int(value[index : index + 2], 16) for index in (1, 3, 5))


def brand(c):
    c.rect(12, 8, 22.5, 11.6, 1.8)
    c.rect(18.4, 8, 22.5, 19, 1.8)
    c.arc(10.2, 11.6, 22.5, 24.4, 0, 180, 4.1)
    c.ellipse(10.2, 16.6, 14.3, 20.7)


def video(c):
    c.polygon([(12.2, 8.6), (24.2, 16), (12.2, 23.4)])
    c.line([(12.2, 8.6), (24.2, 16), (12.2, 23.4), (12.2, 8.6)], 1.6)


def audio(c):
    for x, top in ((8.4, 13), (13.2, 8.5), (18, 11), (22.8, 14)):
        c.rect(x - 1.5, top, x + 1.5, 32 - top, 1.5)


def optimize(c):
    c.ellipse(19.2, 8.6, 23.6, 13)
    c.polygon([(7, 24), (13.4, 13.2), (19.8, 24)])
    c.polygon([(15.4, 24), (20.4, 16.8), (25.4, 24)])
    c.rect(7, 22.6, 25.4, 24.4, 0.9)


def resize(c):
    c.line([(10, 22), (22, 10)], 2.4)
    c.line([(15.6, 9.6), (22.4, 9.6), (22.4, 16.4)], 2.4)
    c.line([(9.6, 15.6), (9.6, 22.4), (16.4, 22.4)], 2.4)


def convert(c):
    c.line([(8.5, 12), (23, 12)], 2.4)
    c.line([(19, 8), (23.4, 12), (19, 16)], 2.4)
    c.line([(23.5, 20), (9, 20)], 2.4)
    c.line([(13, 16), (8.6, 20), (13, 24)], 2.4)


def crop(c):
    c.line([(11.5, 7.5), (11.5, 20.5), (24.5, 20.5)], 2.4)
    c.line([(7.5, 11.5), (20.5, 11.5), (20.5, 24.5)], 2.4)


def cutout(c):
    c.ellipse(12.4, 7.6, 19.6, 14.8)
    c.draw.pieslice(c.box(8, 16.6, 24, 32.6), 180, 360, fill=WHITE)


def pdf(c):
    c.polygon([(9.5, 7), (18.6, 7), (22.5, 10.9), (22.5, 25), (9.5, 25)])
    c.polygon([(18.6, 7), (22.5, 10.9), (18.6, 10.9)], fill=(255, 255, 255, 120))
    for y in (14.4, 17.6, 20.8):
        c.rect(12, y - 0.8, 20, y + 0.8, 0.8, fill=TILES["pdf"][1] + (255,))


def svg(c):
    c.arc(4, 9.5, 23, 28.5, 270, 360, 2.2)
    c.rect(10.7, 6.8, 16.3, 12.4, 1)
    c.rect(19.6, 15.7, 25.2, 21.3, 1)
    c.line([(8, 23.5), (13.4, 23.5)], 2.2)


def json(c):
    c.text("{}", 17.5, 16, 15.2)


def links(c):
    c.line([(13.4, 18.6), (18.6, 13.4)], 2.4)
    c.arc(7, 13.2, 18.8, 25, 60, 300, 2.4)
    c.arc(13.2, 7, 25, 18.8, 240, 480, 2.4)


def zip_(c):
    c.rect(8, 9, 24, 14, 1.4)
    c.rect(9.4, 15.2, 22.6, 24.5, 1.4)
    c.rect(13.6, 17.6, 18.4, 19.8, 1.1, fill=TILES["zip"][1] + (255,))


def console(c):
    c.line([(9.5, 11), (15, 16), (9.5, 21)], 2.4)
    c.line([(17.2, 21.4), (23, 21.4)], 2.4)


def options(c):
    for y, knob in ((10.5, 19.5), (16, 12), (21.5, 17)):
        c.line([(8.5, y), (23.5, y)], 1.8)
        c.ellipse(knob - 2.7, y - 2.7, knob + 2.7, y + 2.7)


def paste(c):
    c.line([(16, 8), (16, 18.5)], 2.4)
    c.line([(11.4, 14.4), (16, 19), (20.6, 14.4)], 2.4)
    c.line([(8.5, 19.5), (8.5, 23.5), (23.5, 23.5), (23.5, 19.5)], 2.4)


TILES = {
    "brand": (rgb("#9d6bff"), rgb("#6d28d9")),
    "video": (rgb("#fb7185"), rgb("#e11d48")),
    "audio": (rgb("#34d399"), rgb("#059669")),
    "optimize": (rgb("#60a5fa"), rgb("#2563eb")),
    "resize": (rgb("#22d3ee"), rgb("#0891b2")),
    "convert": (rgb("#818cf8"), rgb("#4f46e5")),
    "crop": (rgb("#2dd4bf"), rgb("#0d9488")),
    "cutout": (rgb("#f472b6"), rgb("#db2777")),
    "pdf": (rgb("#f87171"), rgb("#dc2626")),
    "svg": (rgb("#fb923c"), rgb("#ea580c")),
    "json": (rgb("#fbbf24"), rgb("#d97706")),
    "links": (rgb("#38bdf8"), rgb("#0284c7")),
    "zip": (rgb("#94a3b8"), rgb("#475569")),
    "console": (rgb("#64748b"), rgb("#1e293b")),
    "options": (rgb("#9ca3af"), rgb("#4b5563")),
    "paste": (rgb("#e879f9"), rgb("#c026d3")),
}

GLYPHS = {
    "brand": brand,
    "video": video,
    "audio": audio,
    "optimize": optimize,
    "resize": resize,
    "convert": convert,
    "crop": crop,
    "cutout": cutout,
    "pdf": pdf,
    "svg": svg,
    "json": json,
    "links": links,
    "zip": zip_,
    "console": console,
    "options": options,
    "paste": paste,
}


def render(name, size):
    canvas = Canvas(size)
    canvas.tile(*TILES[name])
    GLYPHS[name](canvas)
    return canvas.finish(size)


def main():
    for name in GLYPHS:
        frames = [render(name, size) for size in SIZES]
        frames[-1].save(
            HERE / f"{name}.ico",
            sizes=[(size, size) for size in SIZES],
            append_images=frames[:-1],
        )
    # The package identity needs loose logos beside its manifest.
    for stem, size in (("logo-44", 44), ("logo-150", 150), ("logo-store", 50)):
        render("brand", size).save(HERE / f"{stem}.png")
    sheet = Image.new("RGBA", (len(GLYPHS) * 72 + 8, 2 * 72 + 44), (32, 32, 32, 255))
    sheet.paste((243, 243, 243, 255), (0, 72 + 22, sheet.width, sheet.height))
    for row in range(2):
        for index, name in enumerate(GLYPHS):
            top = row * (72 + 22)
            large = render(name, 48)
            small = render(name, 16)
            sheet.alpha_composite(large, (index * 72 + 8, top + 8))
            sheet.alpha_composite(small, (index * 72 + 8, top + 64))
    sheet.save(HERE.parent.parent / "docs" / "images" / "context-menu-icons.png")


if __name__ == "__main__":
    main()
