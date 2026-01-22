#!/bin/bash
# Create a simple app icon for OllamaBar

set -e

ICON_DIR="assets/OllamaBar.iconset"
mkdir -p "$ICON_DIR"

# Create a simple icon using sips and imagemagick or just colored squares
# For now, create placeholder PNGs using Python

python3 << 'EOF'
import os

# Create a simple gradient icon with a circle
def create_icon(size, output_path):
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        # Fallback: create a simple solid color icon
        import subprocess
        # Use sips to create a colored square
        subprocess.run([
            'convert', '-size', f'{size}x{size}',
            'xc:#34C759',  # Green color
            '-fill', 'white',
            '-draw', f'circle {size//2},{size//2} {size//2},{size//6}',
            output_path
        ], check=False)
        return

    # Create image with gradient background
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Draw a filled circle (green for running state)
    center = size // 2
    radius = size // 3
    draw.ellipse(
        [center - radius, center - radius, center + radius, center + radius],
        fill=(52, 199, 89, 255)  # Apple green
    )

    img.save(output_path)

sizes = [16, 32, 64, 128, 256, 512, 1024]
icon_dir = "assets/OllamaBar.iconset"

for size in sizes:
    create_icon(size, f"{icon_dir}/icon_{size}x{size}.png")
    if size <= 512:
        create_icon(size * 2, f"{icon_dir}/icon_{size}x{size}@2x.png")

print("Icon images created")
EOF

# Convert to icns if iconutil is available
if command -v iconutil &> /dev/null; then
    iconutil -c icns "$ICON_DIR" -o assets/OllamaBar.icns
    echo "Created assets/OllamaBar.icns"
else
    echo "iconutil not found - icon images are in $ICON_DIR"
fi
