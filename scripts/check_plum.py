import sqlite3, os

path = os.path.expandvars(
    r"%LOCALAPPDATA%\Packages\Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe\LocalState\plum.sqlite"
)
conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
cur = conn.cursor()
rows = cur.execute(
    "SELECT Id, Text, Theme, UpdatedAt FROM Note WHERE DeletedAt IS NULL ORDER BY UpdatedAt DESC LIMIT 5"
).fetchall()
for r in rows:
    text = r[1] if r[1] is not None else "(null)"
    print(f"{r[0][:8]} | theme={r[2]} | {text[:60]!r}")
