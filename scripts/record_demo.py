"""
Record htop-win demo as animated GIF using Python
"""
import subprocess
import time
import mss
import pyautogui
from PIL import Image
import os

def find_window_region(crop_chrome=True):
    """Find the htop-win window region, optionally cropping window chrome"""
    try:
        import pygetwindow as gw
        windows = gw.getWindowsWithTitle('htop-win')
        if windows:
            win = windows[0]
            win.activate()  # Bring to front
            time.sleep(0.3)

            if crop_chrome:
                # Crop out Windows Terminal chrome (tabs + title bar + borders)
                # Windows Terminal has: ~48px top (tabs+titlebar), ~1-2px borders
                # Be aggressive to ensure no chrome shows
                top_chrome = 52      # Title bar + tabs
                left_border = 10
                right_border = 10
                bottom_border = 10

                left = win.left + left_border
                top = win.top + top_chrome
                width = win.width - left_border - right_border
                height = win.height - top_chrome - bottom_border

                return (left, top, width, height)
            else:
                return (win.left, win.top, win.width, win.height)
    except:
        pass
    return None

def capture_frames(duration=15, fps=4):
    """Capture frames from screen"""
    frames = []
    interval = 1.0 / fps

    with mss.mss() as sct:
        # Try to find htop-win window, otherwise use primary monitor
        region = find_window_region()
        if region:
            monitor = {"left": region[0], "top": region[1],
                       "width": region[2], "height": region[3]}
            print(f"Recording htop-win window: {region}")
        else:
            # Use a reasonable portion of the screen
            monitor = sct.monitors[1]  # Primary monitor
            # Crop to a reasonable size
            monitor = {"left": monitor["left"], "top": monitor["top"],
                       "width": min(monitor["width"], 1200),
                       "height": min(monitor["height"], 800)}
            print(f"Recording screen region: {monitor}")

        start_time = time.time()
        frame_count = 0

        while time.time() - start_time < duration:
            # Capture screenshot
            screenshot = sct.grab(monitor)
            img = Image.frombytes("RGB", screenshot.size, screenshot.bgra, "raw", "BGRX")
            frames.append(img)
            frame_count += 1

            # Wait for next frame
            elapsed = time.time() - start_time
            next_frame_time = frame_count * interval
            if next_frame_time > elapsed:
                time.sleep(next_frame_time - elapsed)

            if frame_count % fps == 0:
                print(f"  Recorded {frame_count} frames ({int(elapsed)}s)...")

    return frames

def simulate_demo():
    """5-second seamless loop demo"""
    time.sleep(0.8)  # Brief pause at start

    # Smooth scroll down 3 steps
    for _ in range(3):
        pyautogui.press('down')
        time.sleep(0.3)

    time.sleep(0.6)

    # Smooth scroll back up 3 steps (return to start position)
    for _ in range(3):
        pyautogui.press('up')
        time.sleep(0.3)

    time.sleep(0.8)  # End matches start for seamless loop

def create_gif(frames, output_path, fps=4):
    """Create animated GIF from frames at full resolution"""
    if not frames:
        print("No frames to save!")
        return

    # Keep full resolution - no downscaling
    print(f"  Frame size: {frames[0].width}x{frames[0].height}")

    # Convert to palette mode for GIF format
    frames_p = []
    for frame in frames:
        frame_p = frame.convert('P', palette=Image.Palette.ADAPTIVE, colors=256)
        frames_p.append(frame_p)

    # Save as GIF
    duration = int(1000 / fps)  # ms per frame
    frames_p[0].save(
        output_path,
        save_all=True,
        append_images=frames_p[1:],
        duration=duration,
        loop=0,
        optimize=True
    )

    size_mb = os.path.getsize(output_path) / (1024 * 1024)
    print(f"Saved: {output_path} ({len(frames)} frames, {size_mb:.1f} MB)")

def main():
    print("=" * 50)
    print("htop-win Demo Recorder")
    print("=" * 50)

    # Launch htop-win in a new console window
    print("\n1. Launching htop-win.exe in new console...")
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_dir = os.path.dirname(script_dir)
    htop_exe = os.path.join(project_dir, 'target', 'release', 'htop-win.exe')

    htop_process = subprocess.Popen(
        [htop_exe],
        creationflags=subprocess.CREATE_NEW_CONSOLE
    )

    time.sleep(2)  # Wait for window to open

    print("\n2. Starting recording (5 seconds)...")
    print("   Simulating navigation...\n")

    # Start demo simulation in background
    import threading
    demo_thread = threading.Thread(target=simulate_demo)
    demo_thread.start()

    # Capture frames (5 second seamless loop)
    frames = capture_frames(duration=5, fps=8)

    demo_thread.join()

    print(f"\n3. Creating GIF from {len(frames)} frames...")
    output_path = os.path.join(project_dir, 'images', 'demo.gif')
    create_gif(frames, output_path, fps=8)

    print("\n4. Closing htop-win...")
    pyautogui.press('q')
    time.sleep(0.5)

    print("\nDone! Check demo.gif")

if __name__ == '__main__':
    main()
