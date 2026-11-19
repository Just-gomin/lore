"""고정 이미지를 만든다. Pillow가 필요하다. 다시 만들려면 이 폴더에서 `python3 make.py`를 실행한다.

- text.png: 영어 글자가 든 그림. EXIF가 없다(스크린숏처럼).
- photo.jpg: 같은 그림에 EXIF 촬영 시각(DateTimeOriginal)과 DateTime이 있다.
- datetime-only.jpg: EXIF에 DateTime만 있다.
"""

from PIL import Image, ImageDraw, ImageFont

TEXT = "RETRY FIVE TIMES"


def picture():
    image = Image.new("RGB", (900, 160), "white")
    draw = ImageDraw.Draw(image)
    font = ImageFont.load_default(size=64)
    draw.text((40, 40), TEXT, fill="black", font=font)
    return image


def exif(**tags):
    data = Image.Exif()
    if "datetime" in tags:
        data[0x0132] = tags["datetime"]
    if "original" in tags:
        data.get_ifd(0x8769)[0x9003] = tags["original"]
    return data


if __name__ == "__main__":
    image = picture()
    image.save("text.png", optimize=False)
    image.save(
        "photo.jpg",
        quality=90,
        exif=exif(datetime="2026:11:05 18:00:00", original="2026:11:03 10:20:30"),
    )
    image.save("datetime-only.jpg", quality=90, exif=exif(datetime="2026:11:04 09:00:00"))
