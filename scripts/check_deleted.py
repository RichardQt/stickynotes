import sqlite3, os

path = os.path.expandvars(
    r"%LOCALAPPDATA%\Packages\Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe\LocalState\plum.sqlite"
)
conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
cur = conn.cursor()
rows = cur.execute(
    "SELECT Id, DeletedAt, RemoteId, Text FROM Note WHERE Id IN (?, ?)",
    ("699f8a6f-21b9-4202-a146-267b57e96586", "4c6817b1-93e8-4871-a2e6-cdce7584be62"),
).fetchall()
for r in rows:
    text = (r[3] or "")[:50]
    print(f"{r[0][:8]} | DeletedAt={r[1]} | RemoteId={r[2]!r} | {text!r}")
