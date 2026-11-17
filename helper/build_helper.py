r"""Freeze tapo_helper.py (PyInstaller, one folder) and zip it for the desktop build.

Output: vendor/helper/tapo-helper.zip, which desktop/build.rs packs into TapFeed.exe.
Run with the project's .venv:  .venv\Scripts\python.exe helper\build_helper.py
"""

import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "build" / "helper"
DIST = WORK / "dist" / "tapo-helper"
OUT = ROOT / "vendor" / "helper" / "tapo-helper.zip"


def main() -> None:
    shutil.rmtree(WORK, ignore_errors=True)
    subprocess.run(
        [
            sys.executable, "-m", "PyInstaller",
            "--noconfirm", "--clean", "--onedir", "--console",
            "--name", "tapo-helper",
            "--paths", str(ROOT / "helper"),
            "--hidden-import", "errors",
            "--exclude-module", "tkinter",
            "--exclude-module", "customtkinter",
            "--exclude-module", "PIL",
            "--exclude-module", "vlc",
            "--distpath", str(WORK / "dist"),
            "--workpath", str(WORK / "work"),
            "--specpath", str(WORK),
            str(ROOT / "helper" / "tapo_helper.py"),
        ],
        check=True,
    )
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in sorted(DIST.rglob("*")):
            if f.is_file():
                z.write(f, f.relative_to(DIST).as_posix())
    print(f"wrote {OUT} ({OUT.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
