"""Optimize GIF for web - reduce size while maintaining quality"""
from PIL import Image
import os

def optimize_gif_web(input_path, output_path, max_width=800):
    """Optimize GIF for web with best compression"""
    print(f"Loading {input_path}...")
    img = Image.open(input_path)

    frames = []
    durations = []

    try:
        while True:
            frame = img.copy().convert('RGB')
            frames.append(frame)
            durations.append(img.info.get('duration', 125))
            img.seek(img.tell() + 1)
    except EOFError:
        pass

    print(f"  Original: {frames[0].width}x{frames[0].height}, {len(frames)} frames")

    # Resize using nearest neighbor for crisp text (no antialiasing artifacts)
    if frames[0].width > max_width:
        ratio = max_width / frames[0].width
        new_size = (max_width, int(frames[0].height * ratio))
        frames = [f.resize(new_size, Image.Resampling.NEAREST) for f in frames]
        print(f"  Resized to: {new_size[0]}x{new_size[1]}")

    # Create global palette from first frame for consistent colors
    palette_img = frames[0].quantize(colors=64, method=Image.Quantize.MEDIANCUT)
    palette = palette_img.getpalette()

    # Apply same palette to all frames (much better compression)
    frames_p = []
    for frame in frames:
        frame_p = frame.quantize(colors=64, palette=palette_img, dither=Image.Dither.NONE)
        frames_p.append(frame_p)

    # Save with optimization
    frames_p[0].save(
        output_path,
        save_all=True,
        append_images=frames_p[1:],
        duration=durations[0],
        loop=0,
        optimize=True,
        disposal=2  # Restore to background - helps with compression
    )

    orig_size = os.path.getsize(input_path) / 1024
    new_size_kb = os.path.getsize(output_path) / 1024
    print(f"  Original: {orig_size:.0f} KB")
    print(f"  Optimized: {new_size_kb:.0f} KB ({(1 - new_size_kb/orig_size)*100:.0f}% reduction)")
    print(f"  Saved: {output_path}")

if __name__ == '__main__':
    # Web optimized version
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_dir = os.path.dirname(script_dir)
    images_dir = os.path.join(project_dir, 'images')

    optimize_gif_web(
        os.path.join(images_dir, 'demo.gif'),
        os.path.join(images_dir, 'demo_web.gif'),
        max_width=800
    )
