# Usage: python tools/make_themes.py assets/themes
"""Write the gpui-kit theme files for Riffly's colour themes from the
colours each theme publishes. Every theme has a light and a dark side; the
light/dark switch moves between them and the theme menu picks the family.

  GitHub     Primer's light and "dark dimmed"     github.com/primer/primitives (MIT)
  Nord       Polar Night / Snow Storm / Frost     nordtheme.com (MIT)
  One        Atom's One Light / One Dark          github.com/atom/atom (MIT)
  Solarized  base03…base3 and the accents        ethanschoonover.com/solarized (MIT)
"""
import json
import sys
from pathlib import Path

OUT = Path(sys.argv[1])

# Each side names: page, text, border, muted surface, muted text, accent
# (links, focus, checkboxes), the colour on the accent, sidebar/tab bar,
# popover, selection wash and caret.
THEMES = {
    "github": ("GitHub", {
        "light": dict(page="#ffffff", text="#1f2328", border="#d0d7de", muted="#f6f8fa",
                      muted_text="#59636e", accent="#0969da", on_accent="#ffffff",
                      chrome="#f6f8fa", popover="#ffffff", selection="#0969da33", caret="#1f2328",
                      hover="#eaeef2"),
        "dark": dict(page="#22272e", text="#adbac7", border="#444c56", muted="#2d333b",
                     muted_text="#768390", accent="#539bf5", on_accent="#ffffff",
                     chrome="#1c2128", popover="#2d333b", selection="#539bf544", caret="#adbac7",
                     hover="#373e47"),
    }),
    "nord": ("Nord", {
        "light": dict(page="#eceff4", text="#2e3440", border="#d8dee9", muted="#e5e9f0",
                      muted_text="#4c566a", accent="#5e81ac", on_accent="#eceff4",
                      chrome="#e5e9f0", popover="#eceff4", selection="#81a1c140", caret="#2e3440",
                      hover="#d8dee9"),
        "dark": dict(page="#2e3440", text="#d8dee9", border="#434c5e", muted="#3b4252",
                     muted_text="#a3abbb", accent="#88c0d0", on_accent="#2e3440",
                     chrome="#292e39", popover="#3b4252", selection="#88c0d040", caret="#d8dee9",
                     hover="#434c5e"),
    }),
    "one": ("One", {
        "light": dict(page="#fafafa", text="#383a42", border="#dbdbdc", muted="#f0f0f1",
                      muted_text="#696c77", accent="#4078f2", on_accent="#ffffff",
                      chrome="#eaeaeb", popover="#ffffff", selection="#4078f233", caret="#526fff",
                      hover="#e5e5e6"),
        "dark": dict(page="#282c34", text="#abb2bf", border="#3e4451", muted="#2c313a",
                     muted_text="#7f848e", accent="#61afef", on_accent="#282c34",
                     chrome="#21252b", popover="#21252b", selection="#61afef40", caret="#528bff",
                     hover="#2c313a"),
    }),
    "solarized": ("Solarized", {
        "light": dict(page="#fdf6e3", text="#586e75", border="#e6dfc8", muted="#eee8d5",
                      muted_text="#93a1a1", accent="#268bd2", on_accent="#fdf6e3",
                      chrome="#eee8d5", popover="#fdf6e3", selection="#268bd233", caret="#586e75",
                      hover="#e6dfc8"),
        "dark": dict(page="#002b36", text="#93a1a1", border="#0e4452", muted="#073642",
                     muted_text="#839496", accent="#268bd2", on_accent="#fdf6e3",
                     chrome="#00252f", popover="#073642", selection="#268bd255", caret="#93a1a1",
                     hover="#0e4452"),
    }),
}


def side(name, mode, c):
    return {
        "name": f"{name} {'Dark' if mode == 'dark' else 'Light'}",
        "mode": mode,
        "is_default": True,
        "radius": 6,
        "radius.lg": 8,
        "colors": {
            "background": c["page"],
            "foreground": c["text"],
            "border": c["border"],
            "input.border": c["border"],
            "muted.background": c["muted"],
            "muted.foreground": c["muted_text"],
            "popover.background": c["popover"],
            "popover.foreground": c["text"],
            "accent.background": c["hover"],
            "accent.foreground": c["text"],
            "primary.background": c["accent"],
            "primary.hover.background": c["accent"],
            "primary.active.background": c["accent"],
            "primary.foreground": c["on_accent"],
            "secondary.background": c["muted"],
            "secondary.foreground": c["text"],
            "selection.background": c["selection"],
            "caret": c["caret"],
            "link": c["accent"],
            "link.hover": c["accent"],
            "link.active": c["accent"],
            "ring": c["accent"],
            "drag.border": c["accent"],
            "drop_target.background": c["selection"],
            "table.head.background": c["muted"],
            "table.row.border": c["border"],
            "title_bar.background": c["page"],
            "title_bar.border": c["border"],
            "sidebar.background": c["chrome"],
            "sidebar.foreground": c["muted_text"],
            "sidebar.accent.background": c["hover"],
            "sidebar.accent.foreground": c["text"],
            "sidebar.border": c["border"],
            "tab_bar.background": c["chrome"],
            "tab.background": c["chrome"],
            "tab.foreground": c["muted_text"],
            "tab.active.background": c["page"],
            "tab.active.foreground": c["text"],
        },
    }


OUT.mkdir(parents=True, exist_ok=True)
for key, (name, sides) in THEMES.items():
    doc = {
        "$schema": "https://gpui-kit.com/schema/themes.json",
        "name": name,
        "author": "Riffly",
        "themes": [side(name, "light", sides["light"]), side(name, "dark", sides["dark"])],
    }
    (OUT / f"{key}.json").write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
print("wrote", ", ".join(f"{k}.json" for k in THEMES))
