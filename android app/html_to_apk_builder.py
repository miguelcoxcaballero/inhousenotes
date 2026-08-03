import json
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import time
import tkinter as tk
import xml.etree.ElementTree as ET
from pathlib import Path
from tkinter import filedialog, messagebox, ttk


ROOT = Path(__file__).resolve().parent
OUTPUT_ROOT = ROOT / "apk-output"


def safe_work_root() -> Path:
    """Return a working directory guaranteed to be ASCII-only.

    Why: Gradle / Android build tooling refuses paths containing non-ASCII
    characters on Windows, so we cannot build inside the user's project folder
    if its path has accents or other non-ASCII chars (e.g. "Código").
    """
    if os.name == "nt":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
        candidate = base / "HtmlApkBuilder"
    else:
        candidate = Path.home() / ".cache" / "htmlapkbuilder"
    try:
        str(candidate).encode("ascii")
    except UnicodeEncodeError:
        candidate = Path("C:/HtmlApkBuilder") if os.name == "nt" else Path("/tmp/htmlapkbuilder")
    return candidate


WORK_ROOT = safe_work_root()


# Theme tokens — matches the inhouse stencil (cream + orange accent)
THEME = {
    "bg": "#f5f5f0",
    "panel": "#ffffff",
    "panel2": "#ffffff",
    "border": "#e9e9e3",
    "border_soft": "#f0f0ea",
    "text": "#1a1a1a",
    "muted": "#666666",
    "accent": "#E07A3C",
    "accent_hover": "#d06a2c",
    "accent_pressed": "#b85a22",
    "ok": "#2f9e6b",
    "warn": "#c97a1c",
    "err": "#c0392b",
    "field": "#faf9f4",
    "field_focus": "#ffffff",
    "log_bg": "#ffffff",
    "step_hover": "#faf8f2",
}


# Font helpers — Comfortaa / DM Sans if installed (matches stencil), else Segoe UI / Menlo.
def _pick_font(candidates: list[str], default: str) -> str:
    try:
        from tkinter import font as tkfont
        available = set(tkfont.families())
    except Exception:
        return default
    for name in candidates:
        if name in available:
            return name
    return default


def fonts():
    return {
        "brand": _pick_font(["Comfortaa", "Segoe UI Semibold"], "Segoe UI"),
        "body": _pick_font(["DM Sans", "Segoe UI"], "Segoe UI"),
        "mono": _pick_font(["Cascadia Mono", "Consolas", "Menlo"], "Consolas" if os.name == "nt" else "Menlo"),
    }


# Build phases shown in the timeline
PHASES = [
    ("prepare", "Prepare workspace"),
    ("copy", "Copy web assets"),
    ("npm", "Install Capacitor"),
    ("cap_add", "Add Android project"),
    ("cap_sync", "Sync web app"),
    ("patch", "Patch manifest & gradle"),
    ("icon", "Apply launcher icon"),
    ("sign", "Configure signing"),
    ("gradle", "Compile APK with Gradle"),
    ("collect", "Collect APK"),
]


# Android launcher icon densities — (mipmap folder suffix, side length px)
ICON_DENSITIES = [
    ("mdpi", 48),
    ("hdpi", 72),
    ("xhdpi", 96),
    ("xxhdpi", 144),
    ("xxxhdpi", 192),
]
# Foreground (adaptive icon) needs to be 108dp at each density.
ADAPTIVE_FOREGROUND_DENSITIES = [
    ("mdpi", 108),
    ("hdpi", 162),
    ("xhdpi", 216),
    ("xxhdpi", 324),
    ("xxxhdpi", 432),
]


def slugify(value: str, fallback: str = "htmlapk") -> str:
    value = re.sub(r"[^a-zA-Z0-9]+", "-", value.strip()).strip("-").lower()
    return value or fallback


def package_from_name(value: str) -> str:
    slug = slugify(value, "app").replace("-", "")
    if not slug or not slug[0].isalpha():
        slug = f"app{slug}"
    return f"com.local.{slug}"


def validate_package(value: str) -> bool:
    part = r"[A-Za-z][A-Za-z0-9_]*"
    return re.fullmatch(rf"{part}(\.{part})+", value or "") is not None


def fmt_duration(seconds: float) -> str:
    seconds = int(seconds)
    if seconds < 60:
        return f"{seconds}s"
    m, s = divmod(seconds, 60)
    if m < 60:
        return f"{m}m {s:02d}s"
    h, m = divmod(m, 60)
    return f"{h}h {m:02d}m {s:02d}s"


def run_capture(command, cwd=None):
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            shell=False,
            encoding="utf-8",
            errors="replace",
        )
        return result.returncode, (result.stdout or "").strip()
    except Exception as exc:
        return 1, str(exc)


def tool(name: str) -> str:
    if os.name == "nt":
        for candidate in (f"{name}.cmd", f"{name}.exe", name):
            resolved = shutil.which(candidate)
            if resolved:
                return resolved
    return shutil.which(name) or name


def find_android_sdk() -> Path | None:
    candidates = [
        os.environ.get("ANDROID_HOME"),
        os.environ.get("ANDROID_SDK_ROOT"),
        str(Path.home() / "AppData" / "Local" / "Android" / "Sdk"),
        str(Path.home() / "Library" / "Android" / "sdk"),
        "/opt/android-sdk",
    ]
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate)
        if (path / "platform-tools").exists() and (path / "platforms").exists():
            return path
    return None


def find_android_studio_jdk() -> Path | None:
    candidates = [
        os.environ.get("JAVA_HOME"),
        r"C:\Program Files\Android\Android Studio\jbr",
        r"C:\Program Files\Android\Android Studio\jre",
        r"C:\Program Files\Android\Android Studio\jbr\Contents\Home",
    ]
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate)
        java_name = "java.exe" if os.name == "nt" else "java"
        if (path / "bin" / java_name).exists():
            return path
    return None


def java_tool(name: str) -> str:
    jdk = find_android_studio_jdk()
    executable = f"{name}.exe" if os.name == "nt" else name
    if jdk and (jdk / "bin" / executable).exists():
        return str(jdk / "bin" / executable)
    return tool(name)


class RoundedCard(tk.Frame):
    """A panel with rounded corners, a hairline border, and a body frame
    that children pack/grid into.

    Implementation: a Canvas at the back draws a rounded-rect with stroke;
    a tk.Frame at the front (with the same fill color) is packed with an
    inset equal to the radius, so the canvas's rounded corners remain
    visible while the body's rectangular interior blends seamlessly.
    """

    def __init__(
        self,
        parent,
        radius: int = 14,
        fill: str | None = None,
        stroke: str | None = None,
        body_pad: int = 14,
    ):
        try:
            under = parent.cget("bg")
        except tk.TclError:
            under = THEME["bg"]
        super().__init__(parent, bg=under, bd=0, highlightthickness=0)
        self._radius = radius
        self._fill = fill or THEME["panel"]
        self._stroke = stroke or THEME["border"]

        self._canvas = tk.Canvas(self, bg=under, highlightthickness=0, bd=0)
        self._canvas.place(x=0, y=0, relwidth=1, relheight=1)

        # Body inset so the rounded corners stay visible. Body's bg matches
        # the rounded fill so the rectangle's edges blend in.
        inset = max(radius - 2, 4)
        self.body = tk.Frame(self, bg=self._fill, bd=0, highlightthickness=0)
        self.body.pack(fill="both", expand=True, padx=inset, pady=inset)

        # Internal padding (inside body, around children)
        self._inner = tk.Frame(self.body, bg=self._fill)
        self._inner.pack(fill="both", expand=True, padx=body_pad - inset, pady=body_pad - inset)

        self._bg_id = None
        self.bind("<Configure>", self._on_configure)

    @property
    def content(self) -> tk.Frame:
        return self._inner

    def _on_configure(self, event):
        self._redraw(event.width, event.height)

    def _redraw(self, w: int, h: int):
        if self._bg_id is not None:
            self._canvas.delete(self._bg_id)
            self._bg_id = None
        if w <= 2 or h <= 2:
            return
        r = min(self._radius, w // 2, h // 2)
        # Rounded rect via smoothed polygon — anchor pts at corners give
        # near-perfect arcs at radius r.
        x0, y0, x1, y1 = 0.5, 0.5, w - 0.5, h - 0.5
        pts = [
            x0 + r, y0,
            x1 - r, y0,
            x1, y0,
            x1, y0 + r,
            x1, y1 - r,
            x1, y1,
            x1 - r, y1,
            x0 + r, y1,
            x0, y1,
            x0, y1 - r,
            x0, y0 + r,
            x0, y0,
        ]
        self._bg_id = self._canvas.create_polygon(
            pts,
            smooth=True,
            splinesteps=36,
            fill=self._fill,
            outline=self._stroke,
            width=1,
        )
        self._canvas.tag_lower(self._bg_id)


class Toggle(tk.Frame):
    """Custom checkbox: a small canvas-drawn box + label.

    Avoids the clam-theme rendering quirk on Windows where ttk.Checkbutton
    sometimes draws a red × instead of a check.
    """

    BOX = 18

    def __init__(self, parent, variable: tk.BooleanVar, text: str, command=None):
        bg = parent.cget("bg") if hasattr(parent, "cget") else THEME["panel"]
        super().__init__(parent, bg=bg, bd=0, highlightthickness=0, cursor="hand2")
        self.var = variable
        self._cmd = command
        self._enabled = True

        self.canvas = tk.Canvas(
            self, width=self.BOX, height=self.BOX, bg=bg, highlightthickness=0, bd=0
        )
        self.canvas.pack(side="left")
        self.label = tk.Label(
            self,
            text=text,
            bg=bg,
            fg=THEME["text"],
            font=("Segoe UI", 10),
            padx=8,
            cursor="hand2",
        )
        self.label.pack(side="left")

        for w in (self, self.canvas, self.label):
            w.bind("<Button-1>", self._toggle)
        self.var.trace_add("write", lambda *_: self._render())
        self._render()

    def configure_state(self, state: str):
        self._enabled = state != "disabled"
        fg = THEME["text"] if self._enabled else THEME["muted"]
        cur = "hand2" if self._enabled else "arrow"
        self.label.configure(fg=fg, cursor=cur)
        self.canvas.configure(cursor=cur)
        self.configure(cursor=cur)
        self._render()

    def _toggle(self, _event=None):
        if not self._enabled:
            return
        self.var.set(not self.var.get())
        if self._cmd:
            self._cmd()

    def _render(self):
        c = self.canvas
        c.delete("all")
        s = self.BOX
        r = 4
        checked = bool(self.var.get())
        # Rounded square
        fill = THEME["accent"] if checked else THEME["field"]
        outline = THEME["accent"] if checked else THEME["border"]
        if not self._enabled:
            fill = THEME["field"]
            outline = THEME["border"]
        x0, y0, x1, y1 = 1, 1, s - 1, s - 1
        pts = [
            x0 + r, y0, x1 - r, y0, x1, y0, x1, y0 + r,
            x1, y1 - r, x1, y1, x1 - r, y1, x0 + r, y1,
            x0, y1, x0, y1 - r, x0, y0 + r, x0, y0,
        ]
        c.create_polygon(pts, smooth=True, splinesteps=18, fill=fill, outline=outline, width=1)
        if checked:
            # white checkmark
            c.create_line(4, 9, 8, 13, 14, 5, fill="#ffffff", width=2, capstyle="round", joinstyle="round")


class StepRow(tk.Frame):
    """A single row in the build phase timeline. One line: icon · label · detail."""

    def __init__(self, parent, label: str):
        bg = parent.cget("bg") if hasattr(parent, "cget") else THEME["panel"]
        super().__init__(parent, bg=bg, bd=0, highlightthickness=0)
        self._bg = bg
        self._state = "pending"
        self.icon_canvas = tk.Canvas(self, width=16, height=16, bg=bg, highlightthickness=0, bd=0)
        self.icon_canvas.grid(row=0, column=0, padx=(2, 12), pady=7)
        self.label = tk.Label(
            self, text=label, bg=bg, fg=THEME["text"], font=("Segoe UI", 10), anchor="w"
        )
        self.label.grid(row=0, column=1, sticky="w")
        self.detail = tk.Label(
            self, text="", bg=bg, fg=THEME["muted"], font=("Segoe UI", 9), anchor="e"
        )
        self.detail.grid(row=0, column=2, sticky="e", padx=(8, 4))
        self.columnconfigure(1, weight=1)
        self._draw()

    def set_state(self, state: str, detail: str = ""):
        self._state = state
        self._draw()
        if state == "pending":
            self.label.configure(fg=THEME["muted"])
        elif state == "skip":
            self.label.configure(fg=THEME["muted"])
        else:
            self.label.configure(fg=THEME["text"])
        if detail or state in ("ok", "skip", "fail"):
            self.detail.configure(text=detail)

    def _draw(self):
        c = self.icon_canvas
        c.delete("all")
        cx, cy, r = 8, 8, 6
        if self._state == "pending":
            c.create_oval(cx - r, cy - r, cx + r, cy + r, outline=THEME["border"], width=1.5)
        elif self._state == "running":
            c.create_oval(cx - r, cy - r, cx + r, cy + r, outline=THEME["accent"], width=1.5)
            c.create_oval(cx - 2, cy - 2, cx + 2, cy + 2, fill=THEME["accent"], outline="")
        elif self._state == "ok":
            c.create_oval(cx - r, cy - r, cx + r, cy + r, fill=THEME["ok"], outline="")
            c.create_line(4, 8, 7, 11, 12, 5, fill="#ffffff", width=2, capstyle="round", joinstyle="round")
        elif self._state == "skip":
            c.create_oval(cx - r, cy - r, cx + r, cy + r, fill="#dcdcd6", outline="")
            c.create_line(4, 8, 12, 8, fill="#ffffff", width=2, capstyle="round")
        elif self._state == "fail":
            c.create_oval(cx - r, cy - r, cx + r, cy + r, fill=THEME["err"], outline="")
            c.create_line(5, 5, 11, 11, fill="#ffffff", width=2, capstyle="round")
            c.create_line(11, 5, 5, 11, fill="#ffffff", width=2, capstyle="round")


class ApkBuilderApp(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("inhouse apk builder")
        self.geometry("1180x780")
        self.minsize(1040, 700)
        self.configure(bg=THEME["bg"])
        self.fonts = fonts()

        self.log_queue = queue.Queue()
        self.worker = None
        self.running = False
        self.build_start = 0.0
        self.phase_start = 0.0
        self.current_phase = None
        self.phase_rows: dict[str, StepRow] = {}

        # Gradle progress tracking
        self.gradle_total_tasks = 0
        self.gradle_done_tasks = 0
        self.gradle_active = False

        html_files = sorted(ROOT.glob("*.html"))
        default_html = html_files[0] if html_files else ROOT / "maps.html"

        self.html_path = tk.StringVar(value=str(default_html))
        self.icon_path = tk.StringVar(value="")
        self.app_name = tk.StringVar(value=default_html.stem.replace("-", " ").replace("_", " ").title())
        self.package_id = tk.StringVar(value=package_from_name(self.app_name.get()))
        self.version_name = tk.StringVar(value="1.0.0")
        self.version_code = tk.IntVar(value=1)

        self.permission_internet = tk.BooleanVar(value=True)
        self.permission_location = tk.BooleanVar(value=True)
        self.copy_sibling_assets = tk.BooleanVar(value=True)
        self.clean_build = tk.BooleanVar(value=False)
        self.release_build = tk.BooleanVar(value=False)
        self.generate_keystore = tk.BooleanVar(value=True)
        self.open_output = tk.BooleanVar(value=True)

        self.keystore_path = tk.StringVar(value=str(ROOT / "html-to-apk-release.keystore"))
        self.keystore_password = tk.StringVar(value="changeit123")
        self.key_alias = tk.StringVar(value="htmltoapk")

        self.progress = tk.DoubleVar(value=0)
        self.status = tk.StringVar(value="Ready to build")
        self.elapsed = tk.StringVar(value="")
        self.eta = tk.StringVar(value="")

        self._setup_styles()
        self.create_widgets()
        self.after(100, self.drain_log_queue)
        self.after(500, self._tick_elapsed)

    def _setup_styles(self):
        style = ttk.Style(self)
        try:
            style.theme_use("clam")
        except tk.TclError:
            pass

        s = THEME
        f = self.fonts

        # Base
        style.configure(".", background=s["bg"], foreground=s["text"], borderwidth=0)
        style.configure("TFrame", background=s["bg"])
        style.configure(
            "Card.TFrame",
            background=s["panel"],
            bordercolor=s["border"],
            lightcolor=s["border"],
            darkcolor=s["border"],
            relief="solid",
            borderwidth=1,
        )
        style.configure("Card2.TFrame", background=s["panel2"])
        style.configure("Step.TFrame", background=s["panel"])
        style.configure("Header.TFrame", background=s["panel2"])

        style.configure("TLabel", background=s["bg"], foreground=s["text"], font=(f["body"], 10))
        style.configure("Card.TLabel", background=s["panel"], foreground=s["text"], font=(f["body"], 10))
        style.configure("Card2.TLabel", background=s["panel2"], foreground=s["text"], font=(f["body"], 10))
        style.configure("Muted.TLabel", background=s["panel"], foreground=s["muted"], font=(f["body"], 9))
        style.configure("Muted2.TLabel", background=s["panel2"], foreground=s["muted"], font=(f["body"], 9))
        style.configure(
            "Brand.TLabel",
            background=s["panel2"],
            foreground=s["text"],
            font=(f["brand"], 17, "bold"),
        )
        style.configure(
            "Title.TLabel",
            background=s["panel2"],
            foreground=s["text"],
            font=(f["body"], 14, "bold"),
        )
        style.configure(
            "Subtitle.TLabel",
            background=s["panel2"],
            foreground=s["muted"],
            font=(f["body"], 9),
        )
        style.configure(
            "Section.TLabel",
            background=s["panel"],
            foreground=s["muted"],
            font=(f["body"], 9, "bold"),
        )
        style.configure(
            "Status.TLabel",
            background=s["panel2"],
            foreground=s["text"],
            font=(f["body"], 10, "bold"),
        )
        style.configure(
            "StatusMuted.TLabel",
            background=s["panel2"],
            foreground=s["muted"],
            font=(f["body"], 9),
        )
        style.configure("StepLabel.TLabel", background=s["panel"], foreground=s["text"], font=(f["body"], 10))
        style.configure("StepDetail.TLabel", background=s["panel"], foreground=s["muted"], font=(f["body"], 9))
        style.configure("StepIconPending.TLabel", background=s["panel"], foreground=s["muted"], font=(f["body"], 13))
        style.configure("StepIconRunning.TLabel", background=s["panel"], foreground=s["accent"], font=(f["body"], 13, "bold"))
        style.configure("StepIconOk.TLabel", background=s["panel"], foreground=s["ok"], font=(f["body"], 13, "bold"))
        style.configure("StepIconSkip.TLabel", background=s["panel"], foreground=s["muted"], font=(f["body"], 13))
        style.configure("StepIconFail.TLabel", background=s["panel"], foreground=s["err"], font=(f["body"], 13, "bold"))

        # Entries — soft cream field, orange focus ring
        style.configure(
            "TEntry",
            fieldbackground=s["field"],
            background=s["field"],
            foreground=s["text"],
            insertcolor=s["text"],
            bordercolor=s["border"],
            lightcolor=s["border"],
            darkcolor=s["border"],
            padding=8,
        )
        style.map(
            "TEntry",
            fieldbackground=[("focus", s["field_focus"])],
            bordercolor=[("focus", s["accent"])],
            lightcolor=[("focus", s["accent"])],
            darkcolor=[("focus", s["accent"])],
        )
        style.configure(
            "TSpinbox",
            fieldbackground=s["field"],
            background=s["field"],
            foreground=s["text"],
            arrowcolor=s["muted"],
            bordercolor=s["border"],
            lightcolor=s["border"],
            darkcolor=s["border"],
            padding=6,
        )
        style.map(
            "TSpinbox",
            fieldbackground=[("focus", s["field_focus"])],
            bordercolor=[("focus", s["accent"])],
        )

        # Buttons — secondary (translucent dark) and primary (dark)
        style.configure(
            "TButton",
            background="#ececea",
            foreground=s["text"],
            bordercolor="#ececea",
            lightcolor="#ececea",
            darkcolor="#ececea",
            padding=(14, 8),
            font=(f["body"], 10),
            relief="flat",
        )
        style.map(
            "TButton",
            background=[("active", "#e2e2de"), ("pressed", "#d8d8d4")],
        )
        # Primary CTA — dark like stencil's btn-primary
        style.configure(
            "Accent.TButton",
            background="#1a1a1a",
            foreground="#ffffff",
            bordercolor="#1a1a1a",
            lightcolor="#1a1a1a",
            darkcolor="#1a1a1a",
            padding=(20, 10),
            font=(f["body"], 11, "bold"),
            relief="flat",
        )
        style.map(
            "Accent.TButton",
            background=[
                ("disabled", "#cfcfca"),
                ("active", "#2a2a2a"),
                ("pressed", "#000000"),
            ],
            foreground=[("disabled", "#ffffff")],
        )
        # Orange button (used for highlighted secondary actions)
        style.configure(
            "Orange.TButton",
            background=s["accent"],
            foreground="#ffffff",
            bordercolor=s["accent"],
            lightcolor=s["accent"],
            darkcolor=s["accent"],
            padding=(14, 8),
            font=(f["body"], 10, "bold"),
            relief="flat",
        )
        style.map(
            "Orange.TButton",
            background=[("active", s["accent_hover"]), ("pressed", s["accent_pressed"])],
        )

        # Checkbutton
        style.configure(
            "TCheckbutton",
            background=s["panel"],
            foreground=s["text"],
            focuscolor=s["panel"],
            font=(f["body"], 10),
            indicatorbackground=s["field"],
            indicatorforeground=s["accent"],
        )
        style.map(
            "TCheckbutton",
            background=[("active", s["panel"])],
            foreground=[("disabled", s["muted"])],
            indicatorcolor=[("selected", s["accent"]), ("!selected", s["field"])],
        )

        # Progressbar — orange on cream
        style.configure(
            "Accent.Horizontal.TProgressbar",
            background=s["accent"],
            troughcolor=s["field"],
            bordercolor=s["border"],
            lightcolor=s["accent"],
            darkcolor=s["accent"],
            thickness=8,
        )

        # Notebook
        style.configure("TNotebook", background=s["bg"], borderwidth=0, tabmargins=(0, 0, 0, 0))
        style.configure(
            "TNotebook.Tab",
            background=s["bg"],
            foreground=s["muted"],
            padding=(16, 8),
            font=(f["body"], 10, "bold"),
            borderwidth=0,
        )
        style.map(
            "TNotebook.Tab",
            background=[("selected", s["panel"])],
            foreground=[("selected", s["text"])],
        )

        # Scrollbar
        style.configure(
            "Vertical.TScrollbar",
            background="#dcdcd6",
            troughcolor=s["bg"],
            bordercolor=s["bg"],
            arrowcolor=s["muted"],
            relief="flat",
        )

    # ------------------------------------------------------------------
    # UI construction
    # ------------------------------------------------------------------

    def create_widgets(self):
        outer = ttk.Frame(self, style="TFrame")
        outer.pack(fill="both", expand=True)

        self._build_header(outer)

        # Pinned footer at the bottom — Build button can never be clipped.
        self._build_footer(outer)

        # Scrollable body between header and footer.
        body_wrap = ttk.Frame(outer, style="TFrame")
        body_wrap.pack(fill="both", expand=True)
        self._build_body(body_wrap)

        self.toggle_release()

    def _build_body(self, parent):
        body = ttk.Frame(parent, style="TFrame")
        body.pack(fill="both", expand=True, padx=14, pady=(12, 10))
        body.columnconfigure(0, weight=0, minsize=420)
        body.columnconfigure(1, weight=1)
        body.rowconfigure(0, weight=1)
        self._build_left_panel(body)
        self._build_right_panel(body)

    def _build_footer(self, parent):
        sep = tk.Frame(parent, height=1, bg=THEME["border"])
        sep.pack(side="bottom", fill="x")
        bar = tk.Frame(parent, bg=THEME["panel2"])
        bar.pack(side="bottom", fill="x")
        inner = tk.Frame(bar, bg=THEME["panel2"], padx=16, pady=10)
        inner.pack(fill="x")
        ttk.Button(inner, text="Check requirements", command=self.check_requirements).pack(side="left")
        self.build_button = ttk.Button(
            inner, text="Build APK", style="Accent.TButton", command=self.start_build
        )
        self.build_button.pack(side="right")

    def _build_header(self, parent):
        header = ttk.Frame(parent, style="Header.TFrame")
        header.pack(fill="x")

        inner = ttk.Frame(header, style="Header.TFrame", padding=(18, 12))
        inner.pack(fill="x")
        inner.columnconfigure(2, weight=1)

        # Chevron logo (matches stencil's M4 22 L20 6 L36 22 path)
        logo = tk.Canvas(
            inner,
            width=36,
            height=26,
            bg=THEME["panel2"],
            highlightthickness=0,
            bd=0,
        )
        logo.create_line(
            4, 22, 20, 6, 36, 22,
            fill=THEME["accent"],
            width=4,
            capstyle="round",
            joinstyle="round",
            smooth=False,
        )
        logo.grid(row=0, column=0, rowspan=2, sticky="w", padx=(0, 12))

        brand_wrap = ttk.Frame(inner, style="Header.TFrame")
        brand_wrap.grid(row=0, column=1, rowspan=2, sticky="w")
        ttk.Label(brand_wrap, text="inhouse apk builder", style="Brand.TLabel").pack(anchor="w")
        ttk.Label(
            brand_wrap,
            text="Wrap a single .html file into a signed Android app",
            style="Subtitle.TLabel",
        ).pack(anchor="w", pady=(2, 0))

        right = ttk.Frame(inner, style="Header.TFrame")
        right.grid(row=0, column=2, rowspan=2, sticky="e")
        ttk.Label(right, textvariable=self.elapsed, style="Status.TLabel").pack(anchor="e")
        ttk.Label(right, textvariable=self.eta, style="StatusMuted.TLabel").pack(anchor="e")

        sep = tk.Frame(parent, height=1, bg=THEME["border"])
        sep.pack(fill="x")

    def _card(self, parent, title: str) -> RoundedCard:
        card = RoundedCard(parent)
        tk.Label(
            card.content,
            text=title.upper(),
            bg=THEME["panel"],
            fg=THEME["muted"],
            font=(self.fonts["body"], 9, "bold"),
            anchor="w",
        ).grid(row=0, column=0, columnspan=4, sticky="w", pady=(0, 12))
        card.content.columnconfigure(0, weight=1)
        return card

    def _muted(self, parent, text: str) -> tk.Label:
        return tk.Label(
            parent,
            text=text,
            bg=parent.cget("bg"),
            fg=THEME["muted"],
            font=(self.fonts["body"], 9),
        )

    def _build_left_panel(self, parent):
        # Scrollable container: Canvas + inner Frame + mousewheel binding.
        container = tk.Frame(parent, bg=THEME["bg"], bd=0, highlightthickness=0)
        container.grid(row=0, column=0, sticky="nsew", padx=(0, 12))
        container.rowconfigure(0, weight=1)
        container.columnconfigure(0, weight=1)

        canvas = tk.Canvas(container, bg=THEME["bg"], highlightthickness=0, bd=0)
        canvas.grid(row=0, column=0, sticky="nsew")
        sb = ttk.Scrollbar(container, orient="vertical", command=canvas.yview)
        sb.grid(row=0, column=1, sticky="ns")
        canvas.configure(yscrollcommand=sb.set)

        left = tk.Frame(canvas, bg=THEME["bg"], bd=0, highlightthickness=0)
        win_id = canvas.create_window((0, 0), window=left, anchor="nw")
        left.columnconfigure(0, weight=1)

        def _on_inner_configure(_e):
            canvas.configure(scrollregion=canvas.bbox("all"))
        left.bind("<Configure>", _on_inner_configure)

        def _on_canvas_configure(e):
            canvas.itemconfigure(win_id, width=e.width)
        canvas.bind("<Configure>", _on_canvas_configure)

        def _on_wheel(e):
            canvas.yview_scroll(int(-1 * (e.delta / 120)), "units")
        # Only scroll while the cursor is over the left panel.
        container.bind("<Enter>", lambda _e: canvas.bind_all("<MouseWheel>", _on_wheel))
        container.bind("<Leave>", lambda _e: canvas.unbind_all("<MouseWheel>"))

        # Source
        src = self._card(left, "Source")
        src.grid(row=0, column=0, sticky="ew")
        src_row = tk.Frame(src.content, bg=THEME["panel"])
        src_row.grid(row=1, column=0, sticky="ew")
        src_row.columnconfigure(0, weight=1)
        ttk.Entry(src_row, textvariable=self.html_path).grid(row=0, column=0, sticky="ew")
        ttk.Button(src_row, text="Choose…", command=self.choose_html).grid(row=0, column=1, padx=(10, 0))

        # App identity
        det = self._card(left, "App identity")
        det.grid(row=1, column=0, sticky="ew", pady=(10, 0))
        dgrid = tk.Frame(det.content, bg=THEME["panel"])
        dgrid.grid(row=1, column=0, sticky="ew")
        dgrid.columnconfigure(0, weight=1)
        dgrid.columnconfigure(1, weight=1)

        self._muted(dgrid, "App name").grid(row=0, column=0, sticky="w")
        self._muted(dgrid, "Package ID").grid(row=0, column=1, sticky="w", padx=(12, 0))
        ttk.Entry(dgrid, textvariable=self.app_name).grid(row=1, column=0, sticky="ew", pady=(4, 12))
        ttk.Entry(dgrid, textvariable=self.package_id).grid(
            row=1, column=1, sticky="ew", padx=(12, 0), pady=(4, 12)
        )
        self._muted(dgrid, "Version name").grid(row=2, column=0, sticky="w")
        self._muted(dgrid, "Version code").grid(row=2, column=1, sticky="w", padx=(12, 0))
        ttk.Entry(dgrid, textvariable=self.version_name).grid(row=3, column=0, sticky="ew", pady=(4, 0))
        ttk.Spinbox(dgrid, textvariable=self.version_code, from_=1, to=999999).grid(
            row=3, column=1, sticky="ew", padx=(12, 0), pady=(4, 0)
        )

        # Icon picker — spans both columns underneath
        self._muted(dgrid, "App icon (optional, square PNG/JPG ≥ 192px)").grid(
            row=4, column=0, columnspan=2, sticky="w", pady=(14, 4)
        )
        irow = tk.Frame(dgrid, bg=THEME["panel"])
        irow.grid(row=5, column=0, columnspan=2, sticky="ew")
        irow.columnconfigure(1, weight=1)
        self.icon_preview = tk.Canvas(
            irow, width=44, height=44, bg=THEME["field"], highlightthickness=1,
            highlightbackground=THEME["border"], bd=0,
        )
        self.icon_preview.grid(row=0, column=0, padx=(0, 10))
        self._draw_icon_placeholder()
        ttk.Entry(irow, textvariable=self.icon_path).grid(row=0, column=1, sticky="ew")
        ttk.Button(irow, text="Choose…", command=self.choose_icon).grid(row=0, column=2, padx=(10, 0))
        ttk.Button(irow, text="Clear", command=self.clear_icon).grid(row=0, column=3, padx=(6, 0))
        # Update preview when path changes (e.g. typed manually).
        self.icon_path.trace_add("write", lambda *_: self._refresh_icon_preview())

        # Options — custom Toggle widgets in a 2-column flow
        opts = self._card(left, "Options")
        opts.grid(row=2, column=0, sticky="ew", pady=(10, 0))
        ogrid = tk.Frame(opts.content, bg=THEME["panel"])
        ogrid.grid(row=1, column=0, sticky="ew")
        ogrid.columnconfigure(0, weight=1)
        ogrid.columnconfigure(1, weight=1)
        checks = [
            (self.permission_internet, "Internet permission", None),
            (self.permission_location, "Location permissions", None),
            (self.copy_sibling_assets, "Copy sibling assets", None),
            (self.clean_build, "Clean build folder first", None),
            (self.release_build, "Build signed release APK", self.toggle_release),
            (self.generate_keystore, "Generate keystore if missing", None),
            (self.open_output, "Open output folder when done", None),
        ]
        for i, (var, text, cmd) in enumerate(checks):
            Toggle(ogrid, var, text, command=cmd).grid(
                row=i // 2, column=i % 2, sticky="w", padx=(0, 8), pady=6
            )

        # Signing
        sign = self._card(left, "Release signing")
        sign.grid(row=3, column=0, sticky="ew", pady=(10, 0))
        self.signing_frame = sign
        sgrid = tk.Frame(sign.content, bg=THEME["panel"])
        sgrid.grid(row=1, column=0, sticky="ew")
        sgrid.columnconfigure(0, weight=1)
        sgrid.columnconfigure(1, weight=1)

        self._muted(sgrid, "Keystore").grid(row=0, column=0, columnspan=2, sticky="w")
        krow = tk.Frame(sgrid, bg=THEME["panel"])
        krow.grid(row=1, column=0, columnspan=2, sticky="ew", pady=(4, 12))
        krow.columnconfigure(0, weight=1)
        ttk.Entry(krow, textvariable=self.keystore_path).grid(row=0, column=0, sticky="ew")
        ttk.Button(krow, text="Choose…", command=self.choose_keystore).grid(row=0, column=1, padx=(10, 0))

        self._muted(sgrid, "Password").grid(row=2, column=0, sticky="w")
        self._muted(sgrid, "Alias").grid(row=2, column=1, sticky="w", padx=(12, 0))
        ttk.Entry(sgrid, textvariable=self.keystore_password, show="•").grid(
            row=3, column=0, sticky="ew", pady=(4, 0)
        )
        ttk.Entry(sgrid, textvariable=self.key_alias).grid(
            row=3, column=1, sticky="ew", padx=(12, 0), pady=(4, 0)
        )

    def _build_right_panel(self, parent):
        right = ttk.Frame(parent, style="TFrame")
        right.grid(row=0, column=1, sticky="nsew")
        right.columnconfigure(0, weight=1)
        right.rowconfigure(1, weight=1)

        # Progress card
        prog = RoundedCard(right)
        prog.grid(row=0, column=0, sticky="ew")
        tk.Label(
            prog.content,
            textvariable=self.status,
            bg=THEME["panel"],
            fg=THEME["text"],
            font=(self.fonts["body"], 11, "bold"),
            anchor="w",
        ).pack(anchor="w")
        self.progress_bar = ttk.Progressbar(
            prog.content,
            variable=self.progress,
            maximum=100,
            style="Accent.Horizontal.TProgressbar",
        )
        self.progress_bar.pack(fill="x", pady=(10, 0))

        # Body card with tabbed content (Steps / Log)
        main = RoundedCard(right)
        main.grid(row=1, column=0, sticky="nsew", pady=(10, 0))

        nb = ttk.Notebook(main.content)
        nb.pack(fill="both", expand=True)

        # Steps
        steps = tk.Frame(nb, bg=THEME["panel"])
        nb.add(steps, text="  Steps  ")
        for key, label in PHASES:
            r = StepRow(steps, label)
            r.pack(fill="x", padx=4, pady=1)
            self.phase_rows[key] = r

        # Log
        log = tk.Frame(nb, bg=THEME["panel"])
        nb.add(log, text="  Build log  ")
        log.rowconfigure(0, weight=1)
        log.columnconfigure(0, weight=1)
        self.log_text = tk.Text(
            log,
            wrap="word",
            font=(self.fonts["mono"], 9),
            background=THEME["log_bg"],
            foreground=THEME["text"],
            insertbackground=THEME["text"],
            selectbackground=THEME["accent"],
            selectforeground="#ffffff",
            relief="flat",
            borderwidth=0,
            padx=14,
            pady=12,
        )
        self.log_text.grid(row=0, column=0, sticky="nsew")
        self.log_text.tag_configure("muted", foreground=THEME["muted"])
        self.log_text.tag_configure("ok", foreground=THEME["ok"])
        self.log_text.tag_configure("warn", foreground=THEME["warn"])
        self.log_text.tag_configure("err", foreground=THEME["err"])
        self.log_text.tag_configure("cmd", foreground=THEME["accent"])
        sb = ttk.Scrollbar(log, orient="vertical", command=self.log_text.yview)
        sb.grid(row=0, column=1, sticky="ns")
        self.log_text.configure(yscrollcommand=sb.set)

    # ------------------------------------------------------------------
    # Logging / queue plumbing
    # ------------------------------------------------------------------

    def log(self, text: str = "", tag: str | None = None):
        timestamp = time.strftime("%H:%M:%S")
        self.log_queue.put(("log", f"[{timestamp}] {text}\n", tag))

    def set_progress(self, value: float, status: str):
        self.log_queue.put(("progress", value, status))

    def set_phase(self, key: str, state: str, detail: str = ""):
        self.log_queue.put(("phase", key, state, detail))

    def drain_log_queue(self):
        try:
            while True:
                item = self.log_queue.get_nowait()
                kind = item[0]
                if kind == "log":
                    _, text, tag = item
                    self.log_text.insert("end", text, tag or ())
                    self.log_text.see("end")
                elif kind == "progress":
                    self.progress.set(item[1])
                    self.status.set(item[2])
                elif kind == "phase":
                    _, key, state, detail = item
                    row = self.phase_rows.get(key)
                    if row is not None:
                        row.set_state(state, detail)
                elif kind == "done":
                    self.running = False
                    self.build_button.configure(state="normal")
        except queue.Empty:
            pass
        self.after(100, self.drain_log_queue)

    def _tick_elapsed(self):
        if self.running and self.build_start:
            self.elapsed.set(f"Elapsed  {fmt_duration(time.time() - self.build_start)}")
        self.after(500, self._tick_elapsed)

    # ------------------------------------------------------------------
    # Form actions
    # ------------------------------------------------------------------

    def choose_html(self):
        selected = filedialog.askopenfilename(
            initialdir=ROOT,
            title="Choose HTML file",
            filetypes=[("HTML files", "*.html;*.htm"), ("All files", "*.*")],
        )
        if selected:
            self.html_path.set(selected)
            stem = Path(selected).stem.replace("-", " ").replace("_", " ").title()
            self.app_name.set(stem)
            self.package_id.set(package_from_name(stem))

    def choose_icon(self):
        selected = filedialog.askopenfilename(
            initialdir=ROOT,
            title="Choose app icon",
            filetypes=[("Image files", "*.png;*.jpg;*.jpeg;*.webp"), ("All files", "*.*")],
        )
        if selected:
            self.icon_path.set(selected)

    def clear_icon(self):
        self.icon_path.set("")

    def _draw_icon_placeholder(self):
        c = self.icon_preview
        c.delete("all")
        c.create_text(22, 22, text="?", fill=THEME["muted"], font=(self.fonts["body"], 16, "bold"))

    def _refresh_icon_preview(self):
        path = self.icon_path.get().strip()
        c = self.icon_preview
        c.delete("all")
        if not path or not Path(path).is_file():
            self._draw_icon_placeholder()
            return
        try:
            # tk.PhotoImage handles PNG/GIF natively (no Pillow needed for preview).
            img = tk.PhotoImage(file=path)
            # Subsample to fit 40px box.
            w, h = img.width(), img.height()
            factor = max(1, max(w, h) // 40)
            img = img.subsample(factor, factor)
            self._icon_preview_img = img  # keep ref
            c.create_image(22, 22, image=img)
        except Exception:
            # Pillow path for non-PNG (jpg/webp).
            try:
                from PIL import Image, ImageTk  # type: ignore
                im = Image.open(path).convert("RGBA")
                im.thumbnail((40, 40), Image.LANCZOS)
                self._icon_preview_img = ImageTk.PhotoImage(im)
                c.create_image(22, 22, image=self._icon_preview_img)
            except Exception:
                self._draw_icon_placeholder()

    def choose_keystore(self):
        selected = filedialog.asksaveasfilename(
            initialdir=ROOT,
            title="Choose release keystore",
            defaultextension=".keystore",
            filetypes=[("Keystore files", "*.keystore;*.jks"), ("All files", "*.*")],
        )
        if selected:
            self.keystore_path.set(selected)

    def toggle_release(self):
        state = "normal" if self.release_build.get() else "disabled"

        def walk(widget):
            try:
                widget.configure(state=state)
            except tk.TclError:
                pass
            for child in widget.winfo_children():
                walk(child)

        for child in self.signing_frame.winfo_children():
            walk(child)

    def check_requirements(self):
        self.log_text.delete("1.0", "end")
        self.log("Checking requirements…", "cmd")
        sdk = find_android_sdk()
        studio_jdk = find_android_studio_jdk()
        checks = [
            ("Node", [tool("node"), "--version"]),
            ("npm", [tool("npm"), "--version"]),
            ("Java", [java_tool("java"), "-version"]),
        ]
        all_ok = True
        for name, command in checks:
            code, output = run_capture(command, ROOT)
            if code == 0:
                first = output.splitlines()[0] if output else ""
                self.log(f"  ✓  {name}  {first}", "ok")
            else:
                all_ok = False
                self.log(f"  ✕  {name}  not found", "err")
                if output:
                    self.log(f"     {output.splitlines()[0]}", "muted")

        if sdk:
            self.log(f"  ✓  Android SDK  {sdk}", "ok")
        else:
            all_ok = False
            self.log("  ✕  Android SDK  not detected (set ANDROID_HOME)", "err")
        if studio_jdk:
            self.log(f"  ✓  Android Studio JDK  {studio_jdk}", "ok")

        # Path safety
        try:
            str(ROOT).encode("ascii")
            self.log(f"  ✓  Project path is ASCII-safe", "ok")
        except UnicodeEncodeError:
            self.log(
                f"  !  Project path contains non-ASCII characters; building inside {WORK_ROOT}",
                "warn",
            )

        self.log(("Ready to build." if all_ok else "Fix the issues above before building."),
                 "ok" if all_ok else "warn")

    # ------------------------------------------------------------------
    # Build pipeline
    # ------------------------------------------------------------------

    def start_build(self):
        if self.running:
            return
        self.log_text.delete("1.0", "end")
        for row in self.phase_rows.values():
            row.set_state("pending", "")
        error = self.validate_form()
        if error:
            messagebox.showerror("Cannot build", error)
            return

        self.running = True
        self.build_button.configure(state="disabled")
        self.progress.set(0)
        self.build_start = time.time()
        self.elapsed.set("Elapsed  0s")
        self.eta.set("")
        self.worker = threading.Thread(target=self.build_apk, daemon=True)
        self.worker.start()

    def validate_form(self):
        html = Path(self.html_path.get())
        if not html.exists() or html.suffix.lower() not in {".html", ".htm"}:
            return "Choose a valid .html file."
        if not self.app_name.get().strip():
            return "Enter an app name."
        if not validate_package(self.package_id.get().strip()):
            return "Package ID must look like com.example.app."
        if self.release_build.get():
            if len(self.keystore_password.get()) < 6:
                return "Keystore password must be at least 6 characters."
            if not self.key_alias.get().strip():
                return "Enter a key alias."
        return None

    def _begin_phase(self, key: str, status: str, percent: float):
        self.current_phase = key
        self.phase_start = time.time()
        self.set_phase(key, "running")
        self.set_progress(percent, status)

    def _end_phase(self, key: str, ok: bool = True, detail: str | None = None):
        elapsed = time.time() - self.phase_start
        if detail is None:
            detail = fmt_duration(elapsed)
        self.set_phase(key, "ok" if ok else "fail", detail)

    def _skip_phase(self, key: str, detail: str = "skipped"):
        self.set_phase(key, "skip", detail)

    def build_apk(self):
        try:
            self._build_apk()
        except Exception as exc:
            self.log(f"ERROR: {exc}", "err")
            if self.current_phase:
                self._end_phase(self.current_phase, ok=False, detail="failed")
            self.set_progress(self.progress.get(), "Build failed")
        finally:
            total = fmt_duration(time.time() - self.build_start) if self.build_start else "0s"
            self.elapsed.set(f"Total  {total}")
            self.log_queue.put(("done",))

    def _build_apk(self):
        html = Path(self.html_path.get()).resolve()
        app_name = self.app_name.get().strip()
        package_id = self.package_id.get().strip()
        project_slug = slugify(app_name)
        project_dir = WORK_ROOT / project_slug
        www_dir = project_dir / "www"

        # Phase: prepare
        self._begin_phase("prepare", "Preparing workspace", 2)
        self.log(f"Source HTML  {html}", "muted")
        self.log(f"Build folder  {project_dir}", "muted")
        try:
            str(ROOT).encode("ascii")
        except UnicodeEncodeError:
            self.log(
                f"Project path contains non-ASCII characters — building inside {WORK_ROOT} to keep Gradle happy.",
                "warn",
            )
        WORK_ROOT.mkdir(parents=True, exist_ok=True)
        OUTPUT_ROOT.mkdir(exist_ok=True)
        if self.clean_build.get() and project_dir.exists():
            self.log("Removing old build folder…", "muted")
            shutil.rmtree(project_dir)
        project_dir.mkdir(parents=True, exist_ok=True)
        www_dir.mkdir(parents=True, exist_ok=True)
        self._end_phase("prepare")

        # Phase: copy
        self._begin_phase("copy", "Copying web assets", 8)
        copied = self.copy_web_files(html, www_dir)
        self.write_node_project(project_dir, app_name, package_id, html)
        self._end_phase("copy", detail=f"{copied} item(s)")

        # Phase: npm install (skip if cached)
        self._begin_phase("npm", "Installing Capacitor packages", 14)
        node_modules = project_dir / "node_modules"
        if node_modules.exists() and (project_dir / "package-lock.json").exists() and not self.clean_build.get():
            self.log("node_modules cache found — skipping npm install.", "muted")
            self._skip_phase("npm", "cached")
        else:
            self.run_step([tool("npm"), "install", "--no-audit", "--no-fund", "--loglevel=error"], project_dir)
            self._end_phase("npm")

        # Phase: cap add (skip if exists)
        self._begin_phase("cap_add", "Adding Android project", 24)
        if (project_dir / "android").exists() and not self.clean_build.get():
            self.log("Android project already exists — reusing it.", "muted")
            self._skip_phase("cap_add", "cached")
        else:
            self.run_step([tool("npx"), "cap", "add", "android"], project_dir)
            self._end_phase("cap_add")

        # Phase: cap sync
        self._begin_phase("cap_sync", "Syncing web app", 32)
        self.run_step([tool("npx"), "cap", "sync", "android"], project_dir)
        self._end_phase("cap_sync")

        # Phase: patch
        self._begin_phase("patch", "Applying Android settings", 38)
        self.patch_manifest(project_dir, app_name, package_id)
        self.patch_oauth_persistence(project_dir, package_id)
        self.patch_startup_theme(project_dir)
        self.patch_gradle_versions(project_dir)
        self.patch_android_dependencies(project_dir)
        self.write_gradle_properties(project_dir)
        self._end_phase("patch")

        # Phase: launcher icon
        icon = self.icon_path.get().strip()
        if icon and Path(icon).is_file():
            self._begin_phase("icon", "Generating launcher icons", 42)
            self.apply_launcher_icon(project_dir, Path(icon))
            self._end_phase("icon")
        else:
            self._skip_phase("icon", "default icon")

        # Phase: signing
        build_type = "Release" if self.release_build.get() else "Debug"
        if self.release_build.get():
            self._begin_phase("sign", "Configuring release signing", 44)
            self.ensure_keystore()
            self.write_signing_properties(project_dir)
            self.patch_release_signing(project_dir)
            self._end_phase("sign")
        else:
            self._skip_phase("sign", "debug build")

        # Phase: gradle (most of the time lives here)
        self._begin_phase("gradle", f"Compiling {build_type} APK with Gradle", 50)
        gradlew = "gradlew.bat" if os.name == "nt" else "./gradlew"
        task = f"assemble{build_type}"
        self.gradle_active = True
        self.gradle_done_tasks = 0
        self.gradle_total_tasks = 0
        try:
            self.run_step(
                [str(project_dir / "android" / gradlew), task, "--console=plain", "--warning-mode=none"],
                project_dir / "android",
                phase_key="gradle",
                phase_low=50,
                phase_high=92,
            )
        finally:
            self.gradle_active = False
        self._end_phase("gradle")

        # Phase: collect
        self._begin_phase("collect", "Collecting APK", 95)
        apk = self.find_apk(project_dir, build_type.lower())
        final_name = f"{project_slug}-{build_type.lower()}-v{self.version_name.get().strip()}.apk"
        final_apk = OUTPUT_ROOT / final_name
        shutil.copy2(apk, final_apk)
        size_mb = final_apk.stat().st_size / (1024 * 1024)
        self._end_phase("collect", detail=f"{size_mb:.1f} MB")

        self.set_progress(100, "Build complete")
        self.log(f"APK created  {final_apk}  ({size_mb:.1f} MB)", "ok")
        if self.open_output.get():
            try:
                os.startfile(OUTPUT_ROOT)
            except Exception:
                pass

    # ------------------------------------------------------------------
    # File / project setup
    # ------------------------------------------------------------------

    def copy_web_files(self, html: Path, www_dir: Path):
        for item in www_dir.iterdir():
            if item.is_dir():
                shutil.rmtree(item)
            else:
                item.unlink()

        shutil.copy2(html, www_dir / "index.html")
        count = 1
        if not self.copy_sibling_assets.get():
            return count

        ignored = {".apk_builder", "apk-output", "__pycache__", "node_modules", ".git"}
        skip_files = {"html_to_apk_builder.py", "HTML to APK Builder.bat"}
        for item in html.parent.iterdir():
            if item.name in ignored or item.name in skip_files:
                continue
            if item.resolve() == html:
                continue
            destination = www_dir / item.name
            try:
                if item.is_dir():
                    shutil.copytree(item, destination, ignore=shutil.ignore_patterns(".git", "node_modules"))
                    count += 1
                elif item.is_file():
                    shutil.copy2(item, destination)
                    count += 1
            except Exception as exc:
                self.log(f"  could not copy {item.name}: {exc}", "warn")
        return count

    def write_node_project(self, project_dir: Path, app_name: str, package_id: str, html: Path):
        package_json = {
            "name": slugify(app_name),
            "version": self.version_name.get().strip(),
            "private": True,
            "scripts": {"sync": "cap sync android", "open": "cap open android"},
            "dependencies": {
                "@capacitor/android": "latest",
                "@capacitor/cli": "latest",
                "@capacitor/core": "latest",
            },
            "devDependencies": {},
        }
        capacitor_config = {
            "appId": package_id,
            "appName": app_name,
            "webDir": "www",
            "server": {"androidScheme": "https"},
        }
        try:
            html_text = html.read_text(encoding="utf-8", errors="ignore").lower()
        except Exception:
            html_text = ""
        if "inhousenotes.com" in html_text:
            allow_navigation = [
                "inhousenotes.com",
                "*.inhousenotes.com",
                "accounts.google.com",
                "*.google.com",
                "*.googleusercontent.com",
                "*.gstatic.com",
            ]
            capacitor_config["allowNavigation"] = allow_navigation
            capacitor_config["server"]["allowNavigation"] = allow_navigation
            self.log("Configured the local Inhouse Notes launch shell and remote WebView navigation.", "ok")
        (project_dir / "package.json").write_text(json.dumps(package_json, indent=2), encoding="utf-8")
        (project_dir / "capacitor.config.json").write_text(
            json.dumps(capacitor_config, indent=2), encoding="utf-8"
        )

    def patch_manifest(self, project_dir: Path, app_name: str, package_id: str):
        manifest = project_dir / "android" / "app" / "src" / "main" / "AndroidManifest.xml"
        ET.register_namespace("android", "http://schemas.android.com/apk/res/android")
        tree = ET.parse(manifest)
        root = tree.getroot()
        ns = "{http://schemas.android.com/apk/res/android}"

        wanted = []
        if package_id == "com.local.inhousenotes":
            wanted.append("android.permission.REQUEST_INSTALL_PACKAGES")
        if self.permission_internet.get():
            wanted.append("android.permission.INTERNET")
        if self.permission_location.get():
            wanted.extend(
                [
                    "android.permission.ACCESS_FINE_LOCATION",
                    "android.permission.ACCESS_COARSE_LOCATION",
                ]
            )
        existing = {n.attrib.get(f"{ns}name") for n in root.findall("uses-permission")}
        for permission in wanted:
            if permission not in existing:
                node = ET.Element("uses-permission")
                node.set(f"{ns}name", permission)
                root.insert(0, node)

        application = root.find("application")
        if application is not None:
            application.set(f"{ns}label", app_name)
            provider_authority = f"{package_id}.fileprovider"
            provider_exists = any(
                provider.attrib.get(f"{ns}authorities") == provider_authority
                for provider in application.findall("provider")
            )
            if not provider_exists:
                provider = ET.Element("provider")
                provider.set(f"{ns}name", "androidx.core.content.FileProvider")
                provider.set(f"{ns}authorities", provider_authority)
                provider.set(f"{ns}exported", "false")
                provider.set(f"{ns}grantUriPermissions", "true")
                meta = ET.SubElement(provider, "meta-data")
                meta.set(f"{ns}name", "android.support.FILE_PROVIDER_PATHS")
                meta.set(f"{ns}resource", "@xml/file_paths")
                application.append(provider)
            activity = None
            for candidate in application.findall("activity"):
                name = candidate.attrib.get(f"{ns}name", "")
                if name.endswith("MainActivity"):
                    activity = candidate
                    break
            if activity is not None:
                activity.set(f"{ns}exported", "true")
                # Two redirect schemes are registered:
                #   1. inhousenotes://oauth2callback — legacy implicit flow.
                #   2. <package_id>:/oauth2redirect — PKCE flow (Google requires reverse-DNS scheme
                #      that matches package name for Android OAuth clients).
                wanted_schemes = [
                    ("inhousenotes", "oauth2callback"),
                    (package_id, None),  # PKCE: no host check, reverse-DNS scheme only
                ]
                existing_schemes = set()
                for intent_filter in activity.findall("intent-filter"):
                    for data_node in intent_filter.findall("data"):
                        existing_schemes.add((
                            data_node.attrib.get(f"{ns}scheme"),
                            data_node.attrib.get(f"{ns}host"),
                        ))
                for scheme, host in wanted_schemes:
                    if (scheme, host) in existing_schemes:
                        continue
                    intent_filter = ET.Element("intent-filter")
                    action = ET.SubElement(intent_filter, "action")
                    action.set(f"{ns}name", "android.intent.action.VIEW")
                    category_default = ET.SubElement(intent_filter, "category")
                    category_default.set(f"{ns}name", "android.intent.category.DEFAULT")
                    category_browsable = ET.SubElement(intent_filter, "category")
                    category_browsable.set(f"{ns}name", "android.intent.category.BROWSABLE")
                    data = ET.SubElement(intent_filter, "data")
                    data.set(f"{ns}scheme", scheme)
                    if host:
                        data.set(f"{ns}host", host)
                    activity.append(intent_filter)

        tree.write(manifest, encoding="utf-8", xml_declaration=True)

        xml_dir = manifest.parent / "res" / "xml"
        xml_dir.mkdir(parents=True, exist_ok=True)
        file_paths = xml_dir / "file_paths.xml"
        file_paths.write_text(
            """<?xml version="1.0" encoding="utf-8"?>
<paths xmlns:android="http://schemas.android.com/apk/res/android">
    <cache-path name="exports" path="exports/" />
    <cache-path name="updates" path="updates/" />
</paths>
""",
            encoding="utf-8"
        )

    def patch_oauth_persistence(self, project_dir: Path, package_id: str):
        main_src_root = project_dir / "android" / "app" / "src" / "main"
        java_file = main_src_root / "java" / Path(*package_id.split(".")) / "MainActivity.java"
        kotlin_file = main_src_root / "kotlin" / Path(*package_id.split(".")) / "MainActivity.kt"

        if java_file.exists():
            java_file.write_text(
                f"""package {package_id};

import android.content.Intent;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.net.Uri;
import android.util.Base64;
import android.webkit.CookieManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.provider.MediaStore;
import android.provider.Settings;

import androidx.browser.customtabs.CustomTabsIntent;
import androidx.core.content.ContextCompat;
import androidx.core.content.FileProvider;

import com.getcapacitor.BridgeActivity;
import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStream;
import java.io.BufferedInputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import android.widget.Toast;
import org.json.JSONObject;

public class MainActivity extends BridgeActivity {{
    @Override
    public void onCreate(Bundle savedInstanceState) {{
        super.onCreate(savedInstanceState);

        WebView webView = getBridge().getWebView();
        int bootColor = ContextCompat.getColor(this, R.color.ihn_boot_background);
        getWindow().setStatusBarColor(bootColor);
        getWindow().setNavigationBarColor(bootColor);
        webView.setBackgroundColor(bootColor);
        WebSettings settings = webView.getSettings();
        settings.setDomStorageEnabled(true);
        settings.setDatabaseEnabled(true);
        String appUserAgent = settings.getUserAgentString()
            .replace("; wv", "")
            .replace("Version/4.0 ", "");
        settings.setUserAgentString(appUserAgent + " InhouseNotesApp/{self.version_name.get().strip()}");
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE);
        settings.setJavaScriptEnabled(true);

        CookieManager cookieManager = CookieManager.getInstance();
        cookieManager.setAcceptCookie(true);
        cookieManager.setAcceptThirdPartyCookies(webView, true);
        cookieManager.flush();

        webView.addJavascriptInterface(new InhouseNativeBridge(), "InhouseNative");
        handleAppCallback(getIntent());
    }}

    @Override
    public void onNewIntent(Intent intent) {{
        super.onNewIntent(intent);
        setIntent(intent);
        handleAppCallback(intent);
    }}

    private void handleAppCallback(Intent intent) {{
        if (intent == null || intent.getData() == null) return;
        Uri data = intent.getData();
        String scheme = data.getScheme();
        boolean legacyMatch = "inhousenotes".equals(scheme) && "oauth2callback".equals(data.getHost());
        boolean pkceMatch = "{package_id}".equals(scheme);
        if (!legacyMatch && !pkceMatch) return;
        String fragment = data.getEncodedFragment();
        String query = data.getEncodedQuery();
        // PKCE flow returns ?code=...; implicit flow returns #access_token=...
        // Pass whichever is present to the JS handler.
        String payload = "";
        if (fragment != null && !fragment.isEmpty()) {{
            payload = fragment;
        }} else if (query != null && !query.isEmpty()) {{
            payload = query;
        }}
        String target = "https://inhousenotes.com/?inhouse_app=1";
        if (fragment != null && !fragment.isEmpty()) {{
            target += "#" + fragment;
        }}
        final String finalTarget = target;
        final String finalPayload = payload;
        WebView webView = getBridge().getWebView();
        webView.post(() -> {{
            webView.evaluateJavascript(
                "window.handleInhouseNativeOAuth && window.handleInhouseNativeOAuth(" + JSONObject.quote(finalPayload) + ");",
                null
            );
            // Only navigate the WebView for the legacy implicit flow; PKCE handles state in-place.
            if (finalTarget.contains("#access_token=")) {{
                webView.loadUrl(finalTarget);
            }}
        }});
    }}

    public class InhouseNativeBridge {{
        private String pendingPdfName = "cuaderno.pdf";
        private StringBuilder pendingPdfBase64 = new StringBuilder();
        private volatile boolean updateDownloadRunning = false;

        @JavascriptInterface
        public int getPdfExportApiVersion() {{
            return 2;
        }}

        @JavascriptInterface
        public String getAppVersion() {{
            return "{self.version_name.get().strip()}";
        }}

        @JavascriptInterface
        public void installAppUpdate(String url) {{
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
                    && !getPackageManager().canRequestPackageInstalls()) {{
                notifyAppUpdateResult("permission_required", "Allow Inhouse Notes to install updates");
                runOnUiThread(() -> startActivity(new Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + getPackageName())
                )));
                return;
            }}
            if (updateDownloadRunning) {{
                notifyAppUpdateResult("downloading", "The update is already downloading");
                return;
            }}
            updateDownloadRunning = true;
            notifyAppUpdateResult("downloading", "Downloading update");
            new Thread(() -> {{
                HttpURLConnection connection = null;
                try {{
                    Uri parsed = Uri.parse(url);
                    String host = parsed.getHost();
                    boolean allowedHost = "inhousenotes.com".equalsIgnoreCase(host)
                        || "github.com".equalsIgnoreCase(host)
                        || "raw.githubusercontent.com".equalsIgnoreCase(host);
                    if (!"https".equalsIgnoreCase(parsed.getScheme()) || !allowedHost) {{
                        throw new Exception("Update URL is not allowed");
                    }}
                    connection = (HttpURLConnection) new URL(url).openConnection();
                    connection.setInstanceFollowRedirects(true);
                    connection.setConnectTimeout(15000);
                    connection.setReadTimeout(45000);
                    connection.setRequestProperty("Accept", "application/vnd.android.package-archive");
                    int responseCode = connection.getResponseCode();
                    if (responseCode < 200 || responseCode >= 300) {{
                        throw new Exception("Update download failed (" + responseCode + ")");
                    }}
                    File updateDir = new File(getCacheDir(), "updates");
                    if (!updateDir.exists() && !updateDir.mkdirs()) {{
                        throw new Exception("Could not prepare update storage");
                    }}
                    File apkFile = new File(updateDir, "inhouse-notes-update.apk");
                    long totalBytes = 0;
                    try (InputStream input = new BufferedInputStream(connection.getInputStream());
                         FileOutputStream output = new FileOutputStream(apkFile)) {{
                        byte[] buffer = new byte[32768];
                        int read;
                        while ((read = input.read(buffer)) != -1) {{
                            output.write(buffer, 0, read);
                            totalBytes += read;
                        }}
                    }}
                    if (totalBytes < 100000) throw new Exception("Downloaded update is incomplete");
                    Uri apkUri = FileProvider.getUriForFile(
                        MainActivity.this,
                        getPackageName() + ".fileprovider",
                        apkFile
                    );
                    notifyAppUpdateResult("ready", "Update downloaded");
                    runOnUiThread(() -> {{
                        Intent installIntent = new Intent(Intent.ACTION_VIEW);
                        installIntent.setDataAndType(apkUri, "application/vnd.android.package-archive");
                        installIntent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
                        startActivity(installIntent);
                    }});
                }} catch (Exception error) {{
                    error.printStackTrace();
                    notifyAppUpdateResult("error", error.getMessage());
                }} finally {{
                    updateDownloadRunning = false;
                    if (connection != null) connection.disconnect();
                }}
            }}, "InhouseNotesUpdate").start();
        }}

        private void notifyAppUpdateResult(String status, String message) {{
            JSONObject payload = new JSONObject();
            try {{
                payload.put("status", status);
                payload.put("message", message == null ? "" : message);
            }} catch (Exception ignored) {{}}
            WebView webView = getBridge().getWebView();
            webView.post(() -> webView.evaluateJavascript(
                "window.handleInhouseUpdateResult && window.handleInhouseUpdateResult(" + payload.toString() + ");",
                null
            ));
        }}

        @JavascriptInterface
        public void openAuthUrl(String url) {{
            Uri uri = Uri.parse(url);
            CustomTabsIntent customTabsIntent = new CustomTabsIntent.Builder()
                .setShowTitle(true)
                .build();
            customTabsIntent.intent.addFlags(Intent.FLAG_ACTIVITY_NO_HISTORY);
            customTabsIntent.launchUrl(MainActivity.this, uri);
        }}

        @JavascriptInterface
        public void savePdf(String fileName, String base64Pdf) {{
            try {{
                savePdfBytes(fileName, Base64.decode(base64Pdf, Base64.DEFAULT));
            }} catch (Exception e) {{
                e.printStackTrace();
                notifyPdfExportResult(false, fileName, e.getMessage());
            }}
        }}

        @JavascriptInterface
        public void beginPdfExport(String fileName) {{
            pendingPdfName = fileName;
            pendingPdfBase64 = new StringBuilder();
        }}

        @JavascriptInterface
        public void appendPdfExportChunk(String chunk) {{
            if (chunk != null) pendingPdfBase64.append(chunk);
        }}

        @JavascriptInterface
        public void finishPdfExport() {{
            try {{
                byte[] bytes = Base64.decode(pendingPdfBase64.toString(), Base64.DEFAULT);
                savePdfBytes(pendingPdfName, bytes);
            }} catch (Exception e) {{
                e.printStackTrace();
                notifyPdfExportResult(false, pendingPdfName, e.getMessage());
            }} finally {{
                pendingPdfBase64 = new StringBuilder();
            }}
        }}

        private void savePdfBytes(String fileName, byte[] bytes) throws Exception {{
            String safeName = fileName == null || fileName.trim().isEmpty()
                ? "cuaderno.pdf"
                : fileName.replaceAll("[\\\\/:*?\\\"<>|]+", "_");
            if (!safeName.toLowerCase().endsWith(".pdf")) safeName += ".pdf";

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {{
                ContentResolver resolver = getContentResolver();
                ContentValues values = new ContentValues();
                values.put(MediaStore.Downloads.DISPLAY_NAME, safeName);
                values.put(MediaStore.Downloads.MIME_TYPE, "application/pdf");
                values.put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS);
                values.put(MediaStore.Downloads.IS_PENDING, 1);
                Uri uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
                if (uri == null) throw new Exception("Android could not create the download");
                try {{
                    try (OutputStream out = resolver.openOutputStream(uri)) {{
                        if (out == null) throw new Exception("Android could not open the download");
                        out.write(bytes);
                    }}
                    values.clear();
                    values.put(MediaStore.Downloads.IS_PENDING, 0);
                    resolver.update(uri, values, null, null);
                }} catch (Exception error) {{
                    resolver.delete(uri, null, null);
                    throw error;
                }}
                openPdfChooser(uri, safeName);
                notifyPdfExportResult(true, safeName, "PDF saved; choose an app to open or share it");
                return;
            }}

            File dir = new File(getCacheDir(), "exports");
            if (!dir.exists()) dir.mkdirs();
            File file = new File(dir, safeName);
            try (FileOutputStream out = new FileOutputStream(file)) {{
                out.write(bytes);
            }}
            Uri uri = FileProvider.getUriForFile(
                MainActivity.this,
                getPackageName() + ".fileprovider",
                file
            );
            openPdfChooser(uri, safeName);
            notifyPdfExportResult(true, safeName, "PDF saved; choose an app to open or share it");
        }}

        private void openPdfChooser(Uri uri, String safeName) {{
            Intent viewIntent = new Intent(Intent.ACTION_VIEW);
            viewIntent.setDataAndType(uri, "application/pdf");
            viewIntent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            Intent sendIntent = new Intent(Intent.ACTION_SEND);
            sendIntent.setType("application/pdf");
            sendIntent.putExtra(Intent.EXTRA_STREAM, uri);
            sendIntent.putExtra(Intent.EXTRA_TITLE, safeName);
            sendIntent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            Intent chooser = Intent.createChooser(viewIntent, "Abrir o compartir PDF");
            chooser.putExtra(Intent.EXTRA_INITIAL_INTENTS, new Intent[] {{ sendIntent }});
            runOnUiThread(() -> startActivity(chooser));
        }}

        private void notifyPdfExportResult(boolean ok, String fileName, String message) {{
            JSONObject payload = new JSONObject();
            try {{
                payload.put("ok", ok);
                payload.put("name", fileName == null ? "cuaderno.pdf" : fileName);
                payload.put("message", message == null ? (ok ? "PDF saved" : "PDF export failed") : message);
            }} catch (Exception ignored) {{}}
            WebView webView = getBridge().getWebView();
            webView.post(() -> webView.evaluateJavascript(
                "window.handleInhousePdfExportResult && window.handleInhousePdfExportResult(" + payload.toString() + ");",
                null
            ));
            final String toastMessage = ok ? "PDF guardado en Descargas" : "No se pudo guardar el PDF";
            runOnUiThread(() -> Toast.makeText(MainActivity.this, toastMessage, Toast.LENGTH_LONG).show());
        }}
    }}

    @Override
    public void onPause() {{
        CookieManager.getInstance().flush();
        super.onPause();
    }}

    @Override
    public void onStop() {{
        CookieManager.getInstance().flush();
        super.onStop();
    }}
}}
""",
                encoding="utf-8",
            )
            self.log("Enabled WebView cookie and storage persistence for OAuth sessions.", "ok")
            return

        if kotlin_file.exists():
            kotlin_file.write_text(
                f"""package {package_id}

import android.content.Intent
import android.content.ContentValues
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.net.Uri
import android.util.Base64
import android.provider.MediaStore
import android.widget.Toast
import android.webkit.CookieManager
import android.webkit.JavascriptInterface
import android.webkit.WebSettings
import androidx.browser.customtabs.CustomTabsIntent
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import com.getcapacitor.BridgeActivity
import java.io.File

class MainActivity : BridgeActivity() {{
    override fun onCreate(savedInstanceState: Bundle?) {{
        super.onCreate(savedInstanceState)

        val webView = bridge.webView
        val bootColor = ContextCompat.getColor(this, R.color.ihn_boot_background)
        window.statusBarColor = bootColor
        window.navigationBarColor = bootColor
        webView.setBackgroundColor(bootColor)
        webView.settings.domStorageEnabled = true
        webView.settings.databaseEnabled = true
        val appUserAgent = webView.settings.userAgentString
            .replace("; wv", "")
            .replace("Version/4.0 ", "")
        webView.settings.userAgentString = "$appUserAgent InhouseNotesApp/{self.version_name.get().strip()}"
        webView.settings.mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
        webView.settings.javaScriptEnabled = true

        CookieManager.getInstance().apply {{
            setAcceptCookie(true)
            setAcceptThirdPartyCookies(webView, true)
            flush()
        }}

        webView.addJavascriptInterface(InhouseNativeBridge(), "InhouseNative")
        handleAppCallback(intent)
    }}

    override fun onNewIntent(intent: Intent) {{
        super.onNewIntent(intent)
        setIntent(intent)
        handleAppCallback(intent)
    }}

    private fun handleAppCallback(intent: Intent?) {{
        val data: Uri = intent?.data ?: return
        val legacyMatch = data.scheme == "inhousenotes" && data.host == "oauth2callback"
        val pkceMatch = data.scheme == "{package_id}"
        if (!legacyMatch && !pkceMatch) return
        val fragment = data.encodedFragment
        val query = data.encodedQuery
        // PKCE flow returns ?code=...; implicit flow returns #access_token=...
        val payload: String = when {{
            !fragment.isNullOrEmpty() -> fragment
            !query.isNullOrEmpty() -> query
            else -> ""
        }}
        var target = "https://inhousenotes.com/?inhouse_app=1"
        if (!fragment.isNullOrEmpty()) {{
            target += "#$fragment"
        }}
        val finalTarget = target
        val finalPayload = payload
        bridge.webView.post {{
            val escapedPayload = org.json.JSONObject.quote(finalPayload)
            bridge.webView.evaluateJavascript(
                "window.handleInhouseNativeOAuth && window.handleInhouseNativeOAuth($escapedPayload);",
                null
            )
            // Only navigate the WebView for legacy implicit flow; PKCE handles state in place.
            if (finalTarget.contains("#access_token=")) {{
                bridge.webView.loadUrl(finalTarget)
            }}
        }}
    }}

    inner class InhouseNativeBridge {{
        private var pendingPdfName: String? = "cuaderno.pdf"
        private var pendingPdfBase64 = StringBuilder()

        @JavascriptInterface
        fun getPdfExportApiVersion(): Int {{
            return 2
        }}

        @JavascriptInterface
        fun openAuthUrl(url: String) {{
            val customTabsIntent = CustomTabsIntent.Builder()
                .setShowTitle(true)
                .build()
            customTabsIntent.intent.addFlags(Intent.FLAG_ACTIVITY_NO_HISTORY)
            customTabsIntent.launchUrl(this@MainActivity, Uri.parse(url))
        }}

        @JavascriptInterface
        fun savePdf(fileName: String?, base64Pdf: String?) {{
            try {{
                savePdfBytes(fileName, Base64.decode(base64Pdf ?: "", Base64.DEFAULT))
            }} catch (e: Exception) {{
                e.printStackTrace()
                notifyPdfExportResult(false, fileName, e.message)
            }}
        }}

        @JavascriptInterface
        fun beginPdfExport(fileName: String?) {{
            pendingPdfName = fileName
            pendingPdfBase64 = StringBuilder()
        }}

        @JavascriptInterface
        fun appendPdfExportChunk(chunk: String?) {{
            if (chunk != null) pendingPdfBase64.append(chunk)
        }}

        @JavascriptInterface
        fun finishPdfExport() {{
            try {{
                val bytes = Base64.decode(pendingPdfBase64.toString(), Base64.DEFAULT)
                savePdfBytes(pendingPdfName, bytes)
            }} catch (e: Exception) {{
                e.printStackTrace()
                notifyPdfExportResult(false, pendingPdfName, e.message)
            }} finally {{
                pendingPdfBase64 = StringBuilder()
            }}
        }}

        private fun savePdfBytes(fileName: String?, bytes: ByteArray) {{
            var safeName = (fileName ?: "cuaderno.pdf")
                .ifBlank {{ "cuaderno.pdf" }}
                .replace(Regex("[\\\\/:*?\\\"<>|]+"), "_")
            if (!safeName.lowercase().endsWith(".pdf")) safeName += ".pdf"

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {{
                val values = ContentValues().apply {{
                    put(MediaStore.Downloads.DISPLAY_NAME, safeName)
                    put(MediaStore.Downloads.MIME_TYPE, "application/pdf")
                    put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }}
                val uri = contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                    ?: error("Android could not create the download")
                try {{
                    contentResolver.openOutputStream(uri)?.use {{ it.write(bytes) }}
                        ?: error("Android could not open the download")
                    values.clear()
                    values.put(MediaStore.Downloads.IS_PENDING, 0)
                    contentResolver.update(uri, values, null, null)
                }} catch (error: Exception) {{
                    contentResolver.delete(uri, null, null)
                    throw error
                }}
                openPdfChooser(uri, safeName)
                notifyPdfExportResult(true, safeName, "PDF saved; choose an app to open or share it")
                return
            }}

            val dir = File(cacheDir, "exports").apply {{ mkdirs() }}
            val file = File(dir, safeName)
            file.writeBytes(bytes)
            val uri = FileProvider.getUriForFile(
                this@MainActivity,
                "$packageName.fileprovider",
                file
            )
            openPdfChooser(uri, safeName)
            notifyPdfExportResult(true, safeName, "PDF saved; choose an app to open or share it")
        }}

        private fun openPdfChooser(uri: Uri, safeName: String) {{
            val viewIntent = Intent(Intent.ACTION_VIEW).apply {{
                setDataAndType(uri, "application/pdf")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }}
            val sendIntent = Intent(Intent.ACTION_SEND).apply {{
                type = "application/pdf"
                putExtra(Intent.EXTRA_STREAM, uri)
                putExtra(Intent.EXTRA_TITLE, safeName)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }}
            val chooser = Intent.createChooser(viewIntent, "Abrir o compartir PDF").apply {{
                putExtra(Intent.EXTRA_INITIAL_INTENTS, arrayOf<android.os.Parcelable>(sendIntent))
            }}
            runOnUiThread {{ startActivity(chooser) }}
        }}

        private fun notifyPdfExportResult(ok: Boolean, fileName: String?, message: String?) {{
            val payload = org.json.JSONObject().apply {{
                put("ok", ok)
                put("name", fileName ?: "cuaderno.pdf")
                put("message", message ?: if (ok) "PDF saved" else "PDF export failed")
            }}
            bridge.webView.post {{
                bridge.webView.evaluateJavascript(
                    "window.handleInhousePdfExportResult && window.handleInhousePdfExportResult($payload);",
                    null
                )
            }}
            val toastMessage = if (ok) "PDF guardado en Descargas" else "No se pudo guardar el PDF"
            runOnUiThread {{ Toast.makeText(this@MainActivity, toastMessage, Toast.LENGTH_LONG).show() }}
        }}
    }}

    override fun onPause() {{
        CookieManager.getInstance().flush()
        super.onPause()
    }}

    override fun onStop() {{
        CookieManager.getInstance().flush()
        super.onStop()
    }}
}}
""",
                encoding="utf-8",
            )
            self.log("Enabled WebView cookie and storage persistence for OAuth sessions.", "ok")
            return

        self.log("MainActivity not found; skipped OAuth persistence patch.", "warn")

    def patch_startup_theme(self, project_dir: Path):
        """Use one launch/WebView colour so Android never exposes black or white frames."""
        res_root = project_dir / "android" / "app" / "src" / "main" / "res"
        values_dir = res_root / "values"
        values_night_dir = res_root / "values-night"
        values_dir.mkdir(parents=True, exist_ok=True)
        values_night_dir.mkdir(parents=True, exist_ok=True)
        (values_dir / "ihn_boot_colors.xml").write_text(
            """<?xml version="1.0" encoding="utf-8"?>
<resources>
    <color name="ihn_boot_background">#F5F5F0</color>
</resources>
""",
            encoding="utf-8",
        )
        (values_night_dir / "ihn_boot_colors.xml").write_text(
            """<?xml version="1.0" encoding="utf-8"?>
<resources>
    <color name="ihn_boot_background">#151515</color>
</resources>
""",
            encoding="utf-8",
        )

        styles_path = values_dir / "styles.xml"
        if not styles_path.exists():
            self.log("Android styles.xml not found; skipped launch-theme colour patch.", "warn")
            return
        tree = ET.parse(styles_path)
        root = tree.getroot()
        style_count = 0
        for style in root.findall("style"):
            name = style.get("name", "")
            if not (name.startswith("AppTheme") or name.startswith("Theme.")):
                continue
            desired = {
                "android:windowBackground": "@color/ihn_boot_background",
                "android:statusBarColor": "@color/ihn_boot_background",
                "android:navigationBarColor": "@color/ihn_boot_background",
                "android:windowSplashScreenBackground": "@color/ihn_boot_background",
            }
            existing = {item.get("name"): item for item in style.findall("item")}
            for item_name, item_value in desired.items():
                item = existing.get(item_name)
                if item is None:
                    item = ET.SubElement(style, "item", {"name": item_name})
                item.text = item_value
            style_count += 1
        ET.indent(tree, space="    ")
        tree.write(styles_path, encoding="utf-8", xml_declaration=True)
        self.log(f"Aligned {style_count} Android launch theme(s) with the first web frame.", "ok")

    def patch_gradle_versions(self, project_dir: Path):
        gradle_file = project_dir / "android" / "app" / "build.gradle"
        text = gradle_file.read_text(encoding="utf-8")
        version_name = self.version_name.get().strip()
        version_code = str(int(self.version_code.get()))
        text = re.sub(r"versionCode\s+\d+", f"versionCode {version_code}", text)
        text = re.sub(r'versionName\s+"[^"]+"', f'versionName "{version_name}"', text)
        gradle_file.write_text(text, encoding="utf-8")

    def patch_android_dependencies(self, project_dir: Path):
        gradle_file = project_dir / "android" / "app" / "build.gradle"
        text = gradle_file.read_text(encoding="utf-8")
        dependencies = [
            'implementation "androidx.browser:browser:1.8.0"',
            'implementation "androidx.core:core:1.13.1"',
        ]
        if "dependencies {" not in text:
            text += "\n\ndependencies {\n" + "\n".join(f"    {dep}" for dep in dependencies) + "\n}\n"
        else:
            missing = [dep for dep in dependencies if dep not in text]
            if not missing:
                return
            text = text.replace(
                "dependencies {",
                "dependencies {\n" + "\n".join(f"    {dep}" for dep in missing),
                1
            )
        gradle_file.write_text(text, encoding="utf-8")

    def apply_launcher_icon(self, project_dir: Path, source: Path):
        """Resize the user's source image into every Android launcher density
        and overwrite the Capacitor template's mipmap PNGs.

        Auto-installs Pillow if missing.
        """
        try:
            from PIL import Image  # type: ignore
        except ImportError:
            self.log("Pillow not found — installing it (one-time)…", "muted")
            try:
                subprocess.run(
                    [sys.executable, "-m", "pip", "install", "--quiet", "pillow"],
                    check=True,
                )
                from PIL import Image  # type: ignore
            except Exception as exc:
                raise RuntimeError(
                    f"Could not install Pillow automatically: {exc}. "
                    "Run `pip install pillow` and try again."
                ) from exc

        res_root = project_dir / "android" / "app" / "src" / "main" / "res"
        if not res_root.is_dir():
            raise RuntimeError(f"Missing Android res folder: {res_root}")

        src = Image.open(source).convert("RGBA")
        # Square-crop centered if the source isn't square.
        w, h = src.size
        if w != h:
            side = min(w, h)
            left = (w - side) // 2
            top = (h - side) // 2
            src = src.crop((left, top, left + side, top + side))

        from PIL import ImageDraw

        for suffix, size in ICON_DENSITIES:
            folder = res_root / f"mipmap-{suffix}"
            folder.mkdir(parents=True, exist_ok=True)
            square = src.resize((size, size), Image.LANCZOS)
            square.save(folder / "ic_launcher.png", "PNG", optimize=True)
            # ic_launcher_round: same image masked to a circle.
            mask = Image.new("L", (size, size), 0)
            ImageDraw.Draw(mask).ellipse((0, 0, size, size), fill=255)
            round_img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
            round_img.paste(square, (0, 0), mask)
            round_img.save(folder / "ic_launcher_round.png", "PNG", optimize=True)

        # Adaptive icon foreground (108dp) — Capacitor templates often include
        # ic_launcher_foreground.png; keep parity if those files exist.
        for suffix, size in ADAPTIVE_FOREGROUND_DENSITIES:
            folder = res_root / f"mipmap-{suffix}"
            fg_file = folder / "ic_launcher_foreground.png"
            if not fg_file.exists():
                continue
            # Place artwork centered at ~66% so safe-zone is preserved.
            inner = int(size * 0.66)
            inner_img = src.resize((inner, inner), Image.LANCZOS)
            canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
            offset = (size - inner) // 2
            canvas.paste(inner_img, (offset, offset), inner_img)
            canvas.save(fg_file, "PNG", optimize=True)

        self.log(f"Wrote launcher icons in {len(ICON_DENSITIES)} densities", "ok")

    def write_gradle_properties(self, project_dir: Path):
        """Tune Gradle for speed and disable the non-ASCII path check (defensive)."""
        props_file = project_dir / "android" / "gradle.properties"
        existing = props_file.read_text(encoding="utf-8") if props_file.exists() else ""
        wanted = {
            "org.gradle.jvmargs": "-Xmx2048m -Dfile.encoding=UTF-8",
            "org.gradle.daemon": "true",
            "org.gradle.parallel": "true",
            "org.gradle.caching": "true",
            "org.gradle.configureondemand": "true",
            "android.useAndroidX": "true",
            "android.overridePathCheck": "true",
        }
        lines = existing.splitlines()
        present_keys = {line.split("=", 1)[0].strip() for line in lines if "=" in line and not line.strip().startswith("#")}
        for key, value in wanted.items():
            if key not in present_keys:
                lines.append(f"{key}={value}")
        props_file.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")

    def ensure_keystore(self):
        keystore = Path(self.keystore_path.get()).resolve()
        if keystore.exists():
            self.log(f"Using existing keystore  {keystore}", "muted")
            return
        if not self.generate_keystore.get():
            raise RuntimeError(f"Keystore does not exist: {keystore}")
        self.log(f"Generating release keystore  {keystore}", "muted")
        keystore.parent.mkdir(parents=True, exist_ok=True)
        self.run_step(
            [
                java_tool("keytool"),
                "-genkeypair",
                "-v",
                "-keystore",
                str(keystore),
                "-alias",
                self.key_alias.get().strip(),
                "-keyalg",
                "RSA",
                "-keysize",
                "2048",
                "-validity",
                "10000",
                "-storepass",
                self.keystore_password.get(),
                "-keypass",
                self.keystore_password.get(),
                "-dname",
                "CN=Local HTML APK, OU=Local, O=Local, L=Local, ST=Local, C=ES",
            ],
            ROOT,
        )

    def write_signing_properties(self, project_dir: Path):
        props = project_dir / "android" / "keystore.properties"
        keystore = Path(self.keystore_path.get()).resolve()

        def java_props_escape(s: str) -> str:
            # Java's Properties.load() reads files as ISO-8859-1 by default.
            # Encode anything outside printable ASCII as \uXXXX so non-ASCII
            # characters (e.g. accented folder names) survive the round-trip.
            out = []
            for ch in s:
                code = ord(ch)
                if ch == "\\":
                    out.append("\\\\")
                elif code < 0x20 or code > 0x7E:
                    out.append(f"\\u{code:04x}")
                else:
                    out.append(ch)
            return "".join(out)

        text = "\n".join(
            [
                f"storeFile={java_props_escape(keystore.as_posix())}",
                f"storePassword={java_props_escape(self.keystore_password.get())}",
                f"keyAlias={java_props_escape(self.key_alias.get().strip())}",
                f"keyPassword={java_props_escape(self.keystore_password.get())}",
                "",
            ]
        )
        # Plain ASCII after escaping; ISO-8859-1 is fine to write as.
        props.write_text(text, encoding="iso-8859-1")

    def patch_release_signing(self, project_dir: Path):
        gradle_file = project_dir / "android" / "app" / "build.gradle"
        text = gradle_file.read_text(encoding="utf-8")

        # 1. Prepend the keystore.properties loader once.
        if "keystorePropertiesFile" not in text:
            text = (
                'def keystorePropertiesFile = rootProject.file("keystore.properties")\n'
                "def keystoreProperties = new Properties()\n"
                "if (keystorePropertiesFile.exists()) {\n"
                "    keystoreProperties.load(new FileInputStream(keystorePropertiesFile))\n"
                "}\n\n"
            ) + text

        # 2. Add signingConfigs block right after `android {` if missing.
        if "signingConfigs {" not in text:
            text = text.replace(
                "android {",
                "android {\n"
                "    signingConfigs {\n"
                "        release {\n"
                "            storeFile file(keystoreProperties['storeFile'])\n"
                "            storePassword keystoreProperties['storePassword']\n"
                "            keyAlias keystoreProperties['keyAlias']\n"
                "            keyPassword keystoreProperties['keyPassword']\n"
                "        }\n"
                "    }",
                1,
            )

        # 3. Make the `signingConfig signingConfigs.release` line idempotent and
        # always land inside `buildTypes { release { ... } }`. Strip any prior
        # occurrences first (they may be in the wrong block from earlier runs),
        # then insert exactly once after the buildTypes.release opening brace.
        text = re.sub(
            r"\n[ \t]*signingConfig[ \t]+signingConfigs\.release[ \t]*\n",
            "\n",
            text,
        )
        text = re.sub(
            r"(buildTypes\s*\{\s*release\s*\{)",
            r"\1\n            signingConfig signingConfigs.release",
            text,
            count=1,
        )

        gradle_file.write_text(text, encoding="utf-8")

    def find_apk(self, project_dir: Path, build_type: str) -> Path:
        apk_dir = project_dir / "android" / "app" / "build" / "outputs" / "apk" / build_type
        apks = sorted(apk_dir.glob("*.apk"), key=lambda p: p.stat().st_mtime, reverse=True)
        if not apks:
            raise RuntimeError(f"No APK found in {apk_dir}")
        return apks[0]

    # ------------------------------------------------------------------
    # Subprocess execution + output parsing
    # ------------------------------------------------------------------

    GRADLE_TASK_RE = re.compile(r"^> Task (:[\w:\-]+)")
    GRADLE_DOWNLOAD_RE = re.compile(r"^Download(?:ing)? (.+)$")
    GRADLE_PERCENT_RE = re.compile(r"<.*?(\d+)% .*?>")

    def run_step(
        self,
        command,
        cwd,
        phase_key: str | None = None,
        phase_low: float = 0,
        phase_high: float = 0,
    ):
        self.log(f"›  {self.mask_command(command)}", "cmd")
        env = os.environ.copy()
        env["PYTHONIOENCODING"] = "utf-8"
        env["JAVA_TOOL_OPTIONS"] = "-Dfile.encoding=UTF-8"
        sdk = find_android_sdk()
        studio_jdk = find_android_studio_jdk()
        if sdk:
            env["ANDROID_HOME"] = str(sdk)
            env["ANDROID_SDK_ROOT"] = str(sdk)
        if studio_jdk:
            env["JAVA_HOME"] = str(studio_jdk)
            env["PATH"] = str(studio_jdk / "bin") + os.pathsep + env.get("PATH", "")

        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            shell=False,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        assert process.stdout is not None
        is_gradle = phase_key == "gradle"
        last_task = ""
        for raw in process.stdout:
            line = raw.rstrip()
            if not line:
                continue
            tag = None
            lower = line.lower()
            if "build successful" in lower or "[success]" in lower:
                tag = "ok"
            elif "build failed" in lower or "error" in lower or "failure" in lower or line.startswith("✕"):
                tag = "err"
            elif line.startswith("> Task ") or line.startswith("Download"):
                tag = "muted"
            self.log(line, tag)

            if is_gradle:
                self._update_gradle_progress(line, phase_low, phase_high)
                m = self.GRADLE_TASK_RE.match(line)
                if m:
                    last_task = m.group(1)
                    self.set_phase(phase_key, "running", last_task)

        code = process.wait()
        if code != 0:
            raise RuntimeError(f"Command failed with exit code {code}: {' '.join(map(str, command))}")

    def _update_gradle_progress(self, line: str, low: float, high: float):
        # Detect Gradle's own "<====...> NN% EXECUTING [...]" progress strings
        m = self.GRADLE_PERCENT_RE.search(line)
        if m:
            pct = int(m.group(1))
            mapped = low + (high - low) * (pct / 100.0)
            self.set_progress(mapped, f"Gradle  {pct}%")
            return
        m = self.GRADLE_TASK_RE.match(line)
        if m:
            self.gradle_done_tasks += 1
            # We don't know total upfront; cap progress increments at high-2.
            target = min(high - 2, low + min(self.gradle_done_tasks * 0.6, high - low - 2))
            self.set_progress(target, f"Gradle  {m.group(1)}")

    def mask_command(self, command):
        command = [str(part) for part in command]
        hidden_after = {"-storepass", "-keypass"}
        masked = []
        hide_next = False
        for part in command:
            if hide_next:
                masked.append("********")
                hide_next = False
                continue
            masked.append(part)
            if part in hidden_after:
                hide_next = True
        return " ".join(masked)


if __name__ == "__main__":
    app = ApkBuilderApp()
    app.mainloop()
