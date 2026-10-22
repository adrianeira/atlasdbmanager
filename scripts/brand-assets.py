"""Gera os recursos de marca do app a partir dos arquivos de `branding/`.

Uso (na raiz do projeto; precisa do Pillow: `pip install pillow`):

    python scripts/brand-assets.py .
    npx tauri icon branding/logo-1024.png -o <pasta-temporaria>
    # copie os arquivos (não as pastas android/ios) da pasta temporária para src-tauri/icons/

Entradas  : branding/logo-mini.png (logo, PNG com fundo transparente) e branding/splachscreen.png (splash).
Saídas    : branding/logo-1024.png (quadrado 1024x1024 transparente, com respiro, fonte dos ícones),
            public/splash.jpg (splash leve) e public/logo.png (256x256, janela Sobre e favicon).
"""
import os
import sys

from PIL import Image

root = sys.argv[1] if len(sys.argv) > 1 else "."
branding = os.path.join(root, "branding")
public = os.path.join(root, "public")
os.makedirs(public, exist_ok=True)

# Logo quadrado: recorta a margem transparente e centraliza com ~9% de respiro (macOS/Android recortam cantos).
logo = Image.open(os.path.join(branding, "logo-mini.png")).convert("RGBA")
box = logo.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
logo = logo.crop(box)
size, margin = 1024, 0.09
inner = int(size * (1 - 2 * margin))
scale = min(inner / logo.width, inner / logo.height)
resized = logo.resize((round(logo.width * scale), round(logo.height * scale)), Image.LANCZOS)
canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
canvas.paste(resized, ((size - resized.width) // 2, (size - resized.height) // 2), resized)
canvas.save(os.path.join(branding, "logo-1024.png"), optimize=True)
canvas.resize((256, 256), Image.LANCZOS).save(os.path.join(public, "logo.png"), optimize=True)

# Splash: JPEG de alta qualidade (~200 KB em vez de ~1,4 MB), para a tela de abertura aparecer rápido.
Image.open(os.path.join(branding, "splachscreen.png")).convert("RGB").save(
    os.path.join(public, "splash.jpg"), quality=92, subsampling=0, optimize=True
)
print("ok: branding/logo-1024.png, public/logo.png, public/splash.jpg")
